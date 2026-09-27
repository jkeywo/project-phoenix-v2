//! Literal recipient references from the exact resolved script set. Expressions
//! are left to the runtime validator; this scanner never executes authored code.

use super::validate::{lex_significant_raw, Tok, Token};
use crate::objective_instances::ObjectiveInstanceKey;
use crate::recipients::{RecipientCatalog, RecipientSelection};
use crate::world::validate::{Severity, SourceLocation, WorldFinding};
use rhai::{Dynamic, ImmutableString, Map};
use std::collections::BTreeSet;

type Spanned = (Token, String);

#[derive(Clone, Debug, Default)]
pub struct RecipientScriptReferences {
    pub declarations: BTreeSet<ObjectiveInstanceKey>,
    pub selections: Vec<LocatedSelection>,
    pub malformed: Vec<(String, usize, String)>,
}

#[derive(Clone, Debug)]
pub struct LocatedSelection {
    pub path: String,
    pub line: usize,
    pub selection: RecipientSelection,
}

fn punctuation(token: &Spanned, wanted: char) -> bool {
    matches!(token.0.kind, Tok::Other(c) if c == wanted)
}

/// Split only at the current container's commas, retaining nested maps/arrays.
fn entries(tokens: &[Spanned]) -> Vec<&[Spanned]> {
    let mut depth = 0;
    let mut start = 0;
    let mut out = Vec::new();
    for (index, (token, _)) in tokens.iter().enumerate() {
        match token.kind {
            Tok::Other('{') | Tok::Other('(') | Tok::LBracket => depth += 1,
            Tok::Other('}') | Tok::Other(')') | Tok::RBracket => depth -= 1,
            Tok::Other(',') if depth == 0 => {
                out.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < tokens.len() {
        out.push(&tokens[start..]);
    }
    out
}

fn field(tokens: &[Spanned]) -> Option<(&str, &[Spanned])> {
    if tokens.len() < 3 || tokens[1].0.kind != Tok::Colon {
        return None;
    }
    let (Tok::Ident(name) | Tok::Str(name)) = &tokens[0].0.kind else {
        return None;
    };
    Some((name, &tokens[2..]))
}

/// Parse structural literals only. Even a function call inside a map cannot be
/// executed here. Rhai decodes a single quoted string so escaped Unicode IDs
/// have the same meaning at Save and at runtime.
fn literal(tokens: &[Spanned], engine: &rhai::Engine) -> Option<Dynamic> {
    // Preserve a numeric literal as its actual type so a malformed selector
    // such as `all_player_ships: 1` fails Save instead of looking computed.
    let spelling = tokens
        .iter()
        .map(|(_, raw)| raw.as_str())
        .collect::<String>();
    if let Ok(value) = spelling.parse::<rhai::INT>() {
        return Some(value.into());
    }
    if tokens.len() == 1 {
        match &tokens[0].0.kind {
            Tok::Str(_) if !tokens[0].1.starts_with('`') => {
                return engine
                    .eval::<ImmutableString>(&tokens[0].1)
                    .ok()
                    .map(Dynamic::from)
            }
            Tok::Ident(name) if name == "true" => return Some(true.into()),
            Tok::Ident(name) if name == "false" => return Some(false.into()),
            _ => {}
        }
    }
    if tokens.len() >= 2
        && tokens[0].0.kind == Tok::LBracket
        && tokens.last()?.0.kind == Tok::RBracket
    {
        return entries(&tokens[1..tokens.len() - 1])
            .into_iter()
            .filter(|part| !part.is_empty())
            .map(|part| literal(part, engine))
            .collect::<Option<rhai::Array>>()
            .map(Dynamic::from);
    }
    if tokens.len() >= 3
        && punctuation(&tokens[0], '#')
        && punctuation(&tokens[1], '{')
        && punctuation(tokens.last()?, '}')
    {
        let mut map = Map::new();
        for entry in entries(&tokens[2..tokens.len() - 1]) {
            if entry.is_empty() {
                continue;
            }
            let (name, value) = field(entry)?;
            map.insert(name.into(), literal(value, engine)?);
        }
        return Some(Dynamic::from(map));
    }
    None
}

pub fn scan(sources: &[vellum_script::ScriptSource]) -> RecipientScriptReferences {
    let engine = super::engine::runtime_engine();
    let mut result = RecipientScriptReferences::default();
    for source in sources {
        let tokens = lex_significant_raw(&source.source);
        for start in 0..tokens.len().saturating_sub(5) {
            let Tok::Ident(method) = &tokens[start].0.kind else {
                continue;
            };
            if !matches!(
                method.as_str(),
                "open_comms" | "addressed" | "add_objective"
            ) || start < 2
                || tokens[start - 1].0.kind != Tok::Dot
                || !matches!(&tokens[start-2].0.kind, Tok::Ident(receiver) if receiver == "effects")
                || !punctuation(&tokens[start + 1], '(')
                || !punctuation(&tokens[start + 2], '#')
                || !punctuation(&tokens[start + 3], '{')
            {
                continue;
            }
            let mut depth = 1;
            let mut end = start + 4;
            while end < tokens.len() {
                if punctuation(&tokens[end], '{') {
                    depth += 1;
                }
                if punctuation(&tokens[end], '}') {
                    depth -= 1;
                }
                if depth == 0 {
                    break;
                }
                end += 1;
            }
            if end == tokens.len() {
                continue;
            }
            let mut selected = Map::new();
            let mut objective_id = None;
            let mut instance_id = None;
            for entry in entries(&tokens[start + 4..end]) {
                let Some((name, value)) = field(entry) else {
                    continue;
                };
                let Some(value) = literal(value, &engine) else {
                    continue;
                };
                match name {
                    "id" => {
                        objective_id = value.try_cast::<ImmutableString>().map(|v| v.to_string())
                    }
                    "instance_id" => {
                        instance_id = value.try_cast::<ImmutableString>().map(|v| v.to_string())
                    }
                    "recipient_ship_slots"
                    | "recipient_factions"
                    | "all_player_ships"
                    | "recipient_objective_instances" => {
                        selected.insert(name.into(), value);
                    }
                    _ => {}
                }
            }
            if method == "add_objective" {
                if let (Some(objective_id), Some(instance_id)) = (objective_id, instance_id) {
                    result.declarations.insert(ObjectiveInstanceKey {
                        objective_id,
                        instance_id,
                    });
                }
            }
            match RecipientSelection::from_rhai_map(&selected) {
                Ok(Some(selection)) => result.selections.push(LocatedSelection {
                    path: source.path.clone(),
                    line: tokens[start].0.line,
                    selection,
                }),
                Err(message) => {
                    result
                        .malformed
                        .push((source.path.clone(), tokens[start].0.line, message))
                }
                Ok(None) => {}
            }
        }
    }
    result
}

impl RecipientScriptReferences {
    pub fn validate(&self, catalog: &RecipientCatalog) -> Vec<WorldFinding> {
        self.selections
            .iter()
            .filter_map(|usage| {
                usage
                    .selection
                    .validate(catalog)
                    .err()
                    .map(|error| WorldFinding {
                        severity: Severity::Error,
                        category: "invalid-recipient",
                        message: error.to_string(),
                        source: SourceLocation {
                            file: usage.path.clone(),
                            line: Some(usage.line),
                            reference: "recipient selection".into(),
                        },
                    })
            })
            .chain(
                self.malformed
                    .iter()
                    .map(|(path, line, message)| WorldFinding {
                        severity: Severity::Error,
                        category: "invalid-recipient",
                        message: message.clone(),
                        source: SourceLocation {
                            file: path.clone(),
                            line: Some(*line),
                            reference: "recipient selection".into(),
                        },
                    }),
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_body(source: &str) -> RecipientScriptReferences {
        scan(&[vellum_script::ScriptSource {
            path: "mission.rhai".into(),
            source: source.into(),
        }])
    }

    #[test]
    fn references_are_source_located_and_share_instance_declarations() {
        let refs = scan_body(
            r#"
            fn run(ctx) {
                ctx.effects.add_objective(#{ id: "escort", instance_id: "pair", all_player_ships: true });
                ctx.effects.open_comms(#{ from: "control", node_fn: "hail",
                    recipient_objective_instances: [#{ objective_id: "escort", instance_id: "pair" }],
                    recipient_ship_slots: ["typo"] });
            }
        "#,
        );
        let catalog = RecipientCatalog {
            objective_instances: refs.declarations.clone(),
            ..Default::default()
        };
        let findings = refs.validate(&catalog);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].source.file, "mission.rhai");
        assert_eq!(findings[0].source.line, Some(4));
        assert!(findings[0].message.contains("typo"));
    }

    #[test]
    fn comments_strings_and_computed_values_do_not_become_literal_references() {
        let refs = scan_body(
            r#"
            // ctx.effects.open_comms(#{ recipient_ship_slots: ["bad"] });
            let note = "ctx.effects.addressed(#{ recipient_factions: [\"bad\"] })";
            fn run(ctx) { ctx.effects.open_comms(#{ recipient_ship_slots: choose_slots(),
                recipient_factions: ["Alliance"] }); }
        "#,
        );
        assert_eq!(refs.selections.len(), 1);
        let catalog = RecipientCatalog {
            factions: ["Alliance".to_string()].into(),
            ..Default::default()
        };
        assert!(refs.validate(&catalog).is_empty());
    }

    #[test]
    fn malformed_literal_selection_is_a_save_error() {
        for fields in [
            "all_player_ships: 1",
            "recipient_ship_slots: false",
            "recipient_factions: [\"\"]",
            "recipient_objective_instances: [#{ objective_id: \"x\" }]",
        ] {
            let refs = scan_body(&format!(
                "fn run(ctx) {{ ctx.effects.addressed(#{{ {fields} }}); }}"
            ));
            let errors = refs.validate(&RecipientCatalog::default());
            assert_eq!(errors.len(), 1, "{fields}");
            assert_eq!(errors[0].source.line, Some(1));
        }
    }

    #[test]
    fn string_escape_decoding_matches_runtime() {
        let refs = scan_body(
            r#"fn f(ctx) { ctx.effects.addressed(#{ recipient_ship_slots: ["le\u0061d"] }); }"#,
        );
        let catalog = RecipientCatalog {
            ship_slots: ["lead".to_string()].into(),
            ..Default::default()
        };
        assert_eq!(refs.selections.len(), 1);
        assert!(refs.validate(&catalog).is_empty());
    }
}
