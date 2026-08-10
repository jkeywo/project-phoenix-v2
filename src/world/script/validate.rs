//! Cross-reference validation for loaded scripts (issue #979, milestone M1).
//!
//! Once every unit has run its top level and the defined-function set is known,
//! this pass proves each collected [`Registration`] names a function that
//! actually exists across the content set. Unresolved handlers become
//! [`WorldFinding`] errors on the **existing** authoring-validation channel, so
//! the atomic activation gate (`world::validate::has_error`) blocks a world
//! whose scripts reference functions that were never defined — nothing spawns
//! partially.
//!
//! The defined set is built from `AST::iter_functions()` (see
//! [`load`](super::load)); handler names are ordinary named functions, so name
//! resolution is exact. Anonymous `anon$…` names are irrelevant here — they are
//! never referenced by name — and the M0 spike's cross-file collision caveat
//! does not apply to named handlers.

use std::collections::BTreeSet;

use vellum_script::ScriptSource;

use crate::world::script::engine::{Registration, ScriptTrigger};
use crate::world::validate::{Severity, SourceLocation, WorldFinding};

/// Category slug for a handler that resolves to no defined function.
pub const UNRESOLVED_SCRIPT_FN: &str = "unresolved-script-fn";

/// Category slug for a `[[trigger]]` that specifies BOTH `script = "fn"` and a
/// declarative `[[trigger.action]]` array — the two front-ends are alternatives,
/// never both on one trigger (issue #980, M2).
pub const TRIGGER_SCRIPT_AND_ACTION: &str = "trigger-script-and-action";

/// Category slug for a `[[comms]]` block that specifies BOTH `script = "fn"` and
/// an inline `[[comms.response]]` dialogue tree — the two front-ends are
/// alternatives, never both on one thread (issue #982, M4). The comms analogue of
/// [`TRIGGER_SCRIPT_AND_ACTION`].
pub const COMMS_SCRIPT_AND_RESPONSE: &str = "comms-script-and-response";

/// Category slug for a compound assignment (`+=`, `-=`, `*=`, …) whose target is
/// the script `flags` accessor — `flags.<name> += n` or `flags[expr] += n`
/// (issue #994).
///
/// M3 (issue #981) made a composable increment an explicit verb
/// ([`flags.increment(name, by)`](super::flags::Flags) → `FlagMutation::Increment`)
/// because Rhai desugars a compound assignment on a custom-type indexer to
/// *get-then-set* **before** the custom type is consulted — so `flags.x += n` is
/// physically indistinguishable from `flags.x = final` and silently drains as an
/// absolute `SetValue`, re-introducing the exact clobber hazard M3 removed. This
/// lint turns that silent degradation into a blocking finding.
pub const FLAG_OPASSIGN_NOT_COMPOSABLE: &str = "flag-opassign-not-composable";

fn unresolved_finding(handler: &str, source_path: &str, context: &str) -> WorldFinding {
    WorldFinding {
        severity: Severity::Error,
        category: UNRESOLVED_SCRIPT_FN,
        message: format!("{context} references undefined function '{handler}'"),
        source: SourceLocation {
            file: source_path.to_string(),
            line: None,
            reference: handler.to_string(),
        },
    }
}

/// Prove every registration's handler resolves against `defined_fns`.
///
/// Returns one error finding per unresolved handler, located at the unit that
/// made the registration.
pub fn validate_registrations(
    registrations: &[Registration],
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for reg in registrations {
        if !defined_fns.contains(&reg.handler) {
            findings.push(unresolved_finding(
                &reg.handler,
                &reg.source_path,
                &format!("script registration for event '{}'", reg.event),
            ));
        }
    }
    findings
}

/// Prove every Rhai-authored trigger's handler (`on_destroyed("x", "fn")`, …)
/// resolves against `defined_fns` (issue #980, M2).
///
/// A trigger whose handler names no defined function is an error finding on the
/// existing authoring-validation channel, so the atomic activation gate
/// (`world::validate::has_error`) blocks a world whose scripted triggers point at
/// functions that were never defined.
pub fn validate_script_triggers(
    script_triggers: &[ScriptTrigger],
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for st in script_triggers {
        if !defined_fns.contains(&st.handler) {
            findings.push(unresolved_finding(
                &st.handler,
                &st.source_path,
                "scripted trigger",
            ));
        }
    }
    findings
}

/// Cross-reference the TOML `[[trigger]] script = "fn"` front-end (issue #980, M2)
/// against the compiled script's defined-function set.
///
/// Reads the raw world TOML rather than the parsed `WorldConfig` so this pass
/// owns the whole scripted-trigger contract in the script module, beside the Rhai
/// checks, and sees a `[[trigger]]` that carries BOTH front-ends at once (which
/// the parser silently prefers `script` for). Two error rules, both blocking:
///
/// * **both front-ends** — a trigger with `script = "fn"` AND a non-empty
///   `[[trigger.action]]` array. They are alternatives; specifying both is
///   ambiguous authoring, reported as [`TRIGGER_SCRIPT_AND_ACTION`].
/// * **unresolved handler** — a `script = "fn"` naming no defined function,
///   reported as [`UNRESOLVED_SCRIPT_FN`], exactly like a Rhai registration.
///
/// `world_path` locates the findings; a per-trigger line is not derived (the
/// loader does not carry the raw TOML text — the existing `WorldFinding` allows
/// `line: None`).
pub fn validate_toml_script_triggers(
    world_path: &str,
    world_toml: &toml::Value,
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let Some(triggers) = world_toml.get("trigger").and_then(|t| t.as_array()) else {
        return findings;
    };
    for entry in triggers {
        let Some(handler) = entry.get("script").and_then(|v| v.as_str()) else {
            continue;
        };
        let has_actions = entry
            .get("action")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        if has_actions {
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: TRIGGER_SCRIPT_AND_ACTION,
                message: format!(
                    "trigger specifies both script = '{handler}' and a declarative action \
                     array; the two front-ends are alternatives, not both on one trigger"
                ),
                source: SourceLocation {
                    file: world_path.to_string(),
                    line: None,
                    reference: handler.to_string(),
                },
            });
        }
        if !defined_fns.contains(handler) {
            findings.push(unresolved_finding(handler, world_path, "trigger script"));
        }
    }
    findings
}

/// Cross-reference the TOML `[[comms]] script = "fn"` front-end (issue #982, M4)
/// against the compiled script's defined-function set.
///
/// The comms twin of [`validate_toml_script_triggers`]. Reads the raw world TOML
/// rather than the parsed `WorldConfig` so this pass owns the whole scripted-comms
/// contract in the script module, beside the trigger checks, and sees a
/// `[[comms]]` that carries BOTH front-ends at once (which the parser silently
/// prefers `script` for, dropping the response tree). Two error rules, both
/// blocking:
///
/// * **both front-ends** — a block with `script = "fn"` AND a non-empty
///   `[[comms.response]]` array. They are alternatives; specifying both is
///   ambiguous authoring, reported as [`COMMS_SCRIPT_AND_RESPONSE`].
/// * **unresolved root fn** — a `script = "fn"` naming no defined function,
///   reported as [`UNRESOLVED_SCRIPT_FN`], exactly like a scripted trigger.
///
/// `world_path` locates the findings; a per-block line is not derived (the loader
/// does not carry the raw TOML text — the existing `WorldFinding` allows
/// `line: None`).
pub fn validate_toml_script_comms(
    world_path: &str,
    world_toml: &toml::Value,
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let Some(entries) = world_toml.get("comms").and_then(|t| t.as_array()) else {
        return findings;
    };
    for entry in entries {
        let Some(root_fn) = entry.get("script").and_then(|v| v.as_str()) else {
            continue;
        };
        let has_responses = entry
            .get("response")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        if has_responses {
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: COMMS_SCRIPT_AND_RESPONSE,
                message: format!(
                    "comms thread specifies both script = '{root_fn}' and an inline \
                     [[comms.response]] dialogue tree; the two front-ends are alternatives, \
                     not both on one thread"
                ),
                source: SourceLocation {
                    file: world_path.to_string(),
                    line: None,
                    reference: root_fn.to_string(),
                },
            });
        }
        if !defined_fns.contains(root_fn) {
            findings.push(unresolved_finding(root_fn, world_path, "comms script"));
        }
    }
    findings
}

// ── `flags` compound-assignment lint (issue #994) ─────────────────────────────
//
// A true `AST::walk` over `Stmt`/`Expr` needs Rhai's `internals` feature, which
// this build does not enable (and enabling it is out of this seam's scope). Rhai
// also gates its tokenizer behind `internals`, and `vellum_script` exposes no
// walk/lex helper. So the body walk is a small, self-contained lexical pass over
// the script source: it strips comments and string/char literals exactly the way
// Rhai's tokenizer does (nested `/* */`, `//`, `"…"` with `\` escapes, verbatim
// `` `…` `` and `#"…"#` strings, `'…'` char literals), then matches the token
// shape `flags . <name> <op=>` / `flags [ … ] <op=>` — i.e. a compound-assignment
// operator applied directly to a member reached through a `flags` segment. This
// catches both bare `flags.x += n` and the real `ctx.flags.x += n` idiom (the
// `flags` segment sits mid-chain in the latter), while never firing on a `+=` in
// a comment or string, on a plain `flags.x = v`, on `flags.increment(…)`, or on a
// non-`flags` target such as a local `x += 1`. The walk is deterministic: tokens
// are emitted in source order and matched left to right.

/// A significant lexical token for the `flags` op-assign scan. Whitespace,
/// comments, and string/char literals are dropped before these are produced, so a
/// `+=` inside a comment or string can never be mistaken for code.
#[derive(Debug, PartialEq, Eq)]
enum Tok {
    /// An identifier run (`flags`, `score`, `ctx`, `increment`, …).
    Ident(String),
    /// `.` member access.
    Dot,
    /// `[` index open.
    LBracket,
    /// `]` index close.
    RBracket,
    /// One of the compound-assignment operators (`+=`, `-=`, `*=`, `/=`, `%=`,
    /// `**=`, `<<=`, `>>=`, `&=`, `|=`, `^=`). Plain `=` and `==` are `Other`.
    OpAssign,
    /// Any other token we do not distinguish (plain `=`, `==`, numbers, `(`, `,`,
    /// `;`, braces, other operators). A separator only.
    Other,
}

/// A token plus the 1-based source line it starts on (for locating a finding).
struct Token {
    kind: Tok,
    line: usize,
}

/// If a compound-assignment operator starts at `chars[i]`, its length in chars
/// (`+=`/`-=`/… → 2, `**=`/`<<=`/`>>=` → 3); otherwise `None`. Longest match wins,
/// so `**=` beats `*=` and neither `=`, `==`, `<=`, `>=`, `**`, `<<`, `>>` is
/// treated as a compound assignment.
fn opassign_len(chars: &[char], i: usize) -> Option<usize> {
    let n = chars.len();
    if i + 2 < n {
        match (chars[i], chars[i + 1], chars[i + 2]) {
            ('*', '*', '=') | ('<', '<', '=') | ('>', '>', '=') => return Some(3),
            _ => {}
        }
    }
    if i + 1 < n {
        match (chars[i], chars[i + 1]) {
            ('+', '=')
            | ('-', '=')
            | ('*', '=')
            | ('/', '=')
            | ('%', '=')
            | ('&', '=')
            | ('|', '=')
            | ('^', '=') => return Some(2),
            _ => {}
        }
    }
    None
}

/// Lex `source` into significant tokens, discarding whitespace, comments, and
/// string/char literals (mirroring Rhai's tokenizer so a `+=` in a comment or
/// string is invisible here).
fn lex_significant(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;

    while i < n {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_whitespace() => i += 1,
            // `//` line comment.
            '/' if i + 1 < n && chars[i + 1] == '/' => {
                i += 2;
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            // `/* … */` block comment (Rhai nests them).
            '/' if i + 1 < n && chars[i + 1] == '*' => {
                i += 2;
                let mut depth = 1usize;
                while i < n && depth > 0 {
                    if chars[i] == '\n' {
                        line += 1;
                        i += 1;
                    } else if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && i + 1 < n && chars[i + 1] == '/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            // `"…"` string literal, `\`-escaped.
            '"' => {
                i += 1;
                while i < n {
                    match chars[i] {
                        '\\' => i += 2,
                        '"' => {
                            i += 1;
                            break;
                        }
                        '\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            // `'…'` char literal, `\`-escaped.
            '\'' => {
                i += 1;
                while i < n {
                    match chars[i] {
                        '\\' => i += 2,
                        '\'' => {
                            i += 1;
                            break;
                        }
                        '\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            // `` `…` `` verbatim/interpolated string — skip to the closing backtick.
            '`' => {
                i += 1;
                while i < n {
                    if chars[i] == '`' {
                        i += 1;
                        break;
                    }
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            // `#"…"#` raw string (any number of hashes). NOT `#{ … }` object maps.
            '#' => {
                let mut h = 0usize;
                while i + h < n && chars[i + h] == '#' {
                    h += 1;
                }
                if i + h < n && chars[i + h] == '"' {
                    // Body runs until a `"` followed by exactly `h` `#`s.
                    let mut j = i + h + 1;
                    loop {
                        if j >= n {
                            break;
                        }
                        if chars[j] == '\n' {
                            line += 1;
                            j += 1;
                        } else if chars[j] == '"'
                            && (0..h).all(|k| j + 1 + k < n && chars[j + 1 + k] == '#')
                        {
                            j = j + 1 + h;
                            break;
                        } else {
                            j += 1;
                        }
                    }
                    i = j;
                } else {
                    // A lone `#` (e.g. the `#` of a `#{ … }` map) — a separator.
                    tokens.push(Token {
                        kind: Tok::Other,
                        line,
                    });
                    i += 1;
                }
            }
            // Identifier.
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                tokens.push(Token {
                    kind: Tok::Ident(chars[start..i].iter().collect()),
                    line,
                });
            }
            '.' => {
                tokens.push(Token {
                    kind: Tok::Dot,
                    line,
                });
                i += 1;
            }
            '[' => {
                tokens.push(Token {
                    kind: Tok::LBracket,
                    line,
                });
                i += 1;
            }
            ']' => {
                tokens.push(Token {
                    kind: Tok::RBracket,
                    line,
                });
                i += 1;
            }
            // Any other operator/punctuation: a compound assignment, or a
            // separator (`Other`) consumed one char at a time.
            _ => {
                if let Some(len) = opassign_len(&chars, i) {
                    tokens.push(Token {
                        kind: Tok::OpAssign,
                        line,
                    });
                    i += len;
                } else {
                    tokens.push(Token {
                        kind: Tok::Other,
                        line,
                    });
                    i += 1;
                }
            }
        }
    }

    tokens
}

/// Scan one script source for compound assignments on the `flags` accessor.
/// Returns `(reference, line)` for each hit, where `reference` is the offending
/// target as authored (`flags.<name>` or `flags[…]`).
fn scan_flag_opassign(source: &str) -> Vec<(String, usize)> {
    let tokens = lex_significant(source);
    let mut hits = Vec::new();

    for i in 0..tokens.len() {
        let Tok::Ident(name) = &tokens[i].kind else {
            continue;
        };
        if name != "flags" {
            continue;
        }
        // Dot form: `flags . <name> <op=>` (also matches the mid-chain `flags`
        // segment of `ctx.flags.<name> <op=>`).
        if i + 3 < tokens.len()
            && tokens[i + 1].kind == Tok::Dot
            && tokens[i + 3].kind == Tok::OpAssign
        {
            if let Tok::Ident(flag) = &tokens[i + 2].kind {
                hits.push((format!("flags.{flag}"), tokens[i].line));
                continue;
            }
        }
        // Index form: `flags [ … ] <op=>` with balanced brackets.
        if i + 1 < tokens.len() && tokens[i + 1].kind == Tok::LBracket {
            let mut depth = 0usize;
            let mut close = None;
            for (j, tok) in tokens.iter().enumerate().skip(i + 1) {
                match tok.kind {
                    Tok::LBracket => depth += 1,
                    Tok::RBracket => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(cl) = close {
                if cl + 1 < tokens.len() && tokens[cl + 1].kind == Tok::OpAssign {
                    hits.push(("flags[…]".to_string(), tokens[i].line));
                }
            }
        }
    }

    hits
}

/// Reject a compound assignment (`+=`, `-=`, `*=`, …) on the script `flags`
/// accessor across every source (issue #994).
///
/// `flags.x += n` still *parses and runs*, but Rhai desugars it to an absolute
/// get-then-set on the indexer, so it drains as `FlagMutation::SetValue` and
/// silently re-introduces the flag-clobber hazard M3 removed (issue #981). This
/// pass turns each such spelling into a blocking [`FLAG_OPASSIGN_NOT_COMPOSABLE`]
/// error finding located at the offending script + line, pointing the author at
/// the composable verb `flags.increment(name, n)`. It runs on the
/// same authoring-validation channel as the cross-reference checks, so the atomic
/// activation gate (`world::validate::has_error`) blocks the world.
pub fn validate_flag_opassign(sources: &[ScriptSource]) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for src in sources {
        for (reference, line) in scan_flag_opassign(&src.source) {
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: FLAG_OPASSIGN_NOT_COMPOSABLE,
                message: format!(
                    "compound assignment on the script `flags` accessor `{reference}` is not a \
                     composable increment: Rhai desugars `+=`/`-=`/… on the flags indexer to an \
                     absolute get-then-set, so it drains as SetValue and re-introduces the \
                     flag-clobber hazard. Use `flags.increment(\"name\", n)` for a counter, or \
                     `flags.name = v` for an absolute set."
                ),
                source: SourceLocation {
                    file: src.path.clone(),
                    line: Some(line),
                    reference,
                },
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(event: &str, handler: &str, path: &str) -> Registration {
        Registration {
            event: event.to_string(),
            handler: handler.to_string(),
            source_path: path.to_string(),
        }
    }

    fn defined(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_resolved_handler_produces_no_finding() {
        let regs = vec![reg("flag_set:armed", "handle_armed", "w.toml#script.s")];
        let findings = validate_registrations(&regs, &defined(&["handle_armed"]));
        assert!(findings.is_empty());
    }

    #[test]
    fn an_unresolved_handler_is_an_error_finding() {
        let regs = vec![reg("flag_set:armed", "handle_armed", "w.toml#script.s")];
        let findings = validate_registrations(&regs, &defined(&["something_else"]));
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert!(f.is_error());
        assert_eq!(f.category, UNRESOLVED_SCRIPT_FN);
        assert_eq!(f.source.file, "w.toml#script.s");
        assert_eq!(f.source.reference, "handle_armed");
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn each_unresolved_handler_reports_separately() {
        let regs = vec![
            reg("a", "handler_a", "p1"),
            reg("b", "handler_b", "p2"),
            reg("c", "handler_c", "p3"),
        ];
        // Only `handler_b` is defined.
        let findings = validate_registrations(&regs, &defined(&["handler_b"]));
        assert_eq!(findings.len(), 2);
    }

    // ── Rhai trigger front-end handler resolution (issue #980) ────────────────

    fn script_trigger(handler: &str, path: &str) -> ScriptTrigger {
        ScriptTrigger {
            trigger: crate::world::config::scripted_trigger(
                crate::world::config::TriggerCondition::OnWorldLoaded,
            ),
            handler: handler.to_string(),
            source_path: path.to_string(),
        }
    }

    #[test]
    fn a_resolved_scripted_trigger_handler_is_clean() {
        let sts = vec![script_trigger("on_loaded", "w.toml#script.s")];
        assert!(validate_script_triggers(&sts, &defined(&["on_loaded"])).is_empty());
    }

    #[test]
    fn an_unresolved_scripted_trigger_handler_blocks_activation() {
        let sts = vec![script_trigger("missing", "w.toml#script.s")];
        let findings = validate_script_triggers(&sts, &defined(&["something_else"]));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, UNRESOLVED_SCRIPT_FN);
        assert_eq!(findings[0].source.reference, "missing");
        assert!(crate::world::validate::has_error(&findings));
    }

    // ── TOML `[[trigger]] script = "fn"` front-end (issue #980) ───────────────

    fn world(toml_str: &str) -> toml::Value {
        toml::from_str(toml_str).expect("valid toml")
    }

    #[test]
    fn a_resolved_toml_script_trigger_is_clean() {
        let w = world(
            r#"
            [[trigger]]
            condition = "on_destroyed"
            entity = "raider"
            script = "on_raider_dead"
            "#,
        );
        let findings = validate_toml_script_triggers("w.toml", &w, &defined(&["on_raider_dead"]));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn an_unresolved_toml_script_trigger_is_an_error() {
        let w = world(
            r#"
            [[trigger]]
            condition = "on_world_loaded"
            script = "never_defined"
            "#,
        );
        let findings = validate_toml_script_triggers("w.toml", &w, &defined(&["other"]));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, UNRESOLVED_SCRIPT_FN);
        assert_eq!(findings[0].source.reference, "never_defined");
        assert_eq!(findings[0].source.file, "w.toml");
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_trigger_with_both_script_and_actions_is_rejected() {
        // The handler IS defined, so the ONLY finding is the both-front-ends one.
        let w = world(
            r#"
            [[trigger]]
            condition = "on_destroyed"
            entity = "raider"
            script = "on_raider_dead"

            [[trigger.action]]
            type = "complete_objective"
            id = "obj-x"
            "#,
        );
        let findings = validate_toml_script_triggers("w.toml", &w, &defined(&["on_raider_dead"]));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, TRIGGER_SCRIPT_AND_ACTION);
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_declarative_trigger_yields_no_script_findings() {
        // No `script` key: this front-end must not touch it at all.
        let w = world(
            r#"
            [[trigger]]
            condition = "on_destroyed"
            entity = "raider"

            [[trigger.action]]
            type = "complete_objective"
            id = "obj-x"
            "#,
        );
        assert!(validate_toml_script_triggers("w.toml", &w, &defined(&[])).is_empty());
    }

    // ── TOML `[[comms]] script = "fn"` front-end (issue #982, M4) ─────────────

    #[test]
    fn a_resolved_toml_script_comms_is_clean() {
        let w = world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            script = "hail_axiom"
            "#,
        );
        let findings = validate_toml_script_comms("w.toml", &w, &defined(&["hail_axiom"]));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn an_unresolved_toml_script_comms_is_an_error() {
        let w = world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            script = "never_defined"
            "#,
        );
        let findings = validate_toml_script_comms("w.toml", &w, &defined(&["other"]));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, UNRESOLVED_SCRIPT_FN);
        assert_eq!(findings[0].source.reference, "never_defined");
        assert_eq!(findings[0].source.file, "w.toml");
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_comms_with_both_script_and_responses_is_rejected() {
        // The root fn IS defined, so the ONLY finding is the both-front-ends one.
        let w = world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            script = "hail_axiom"

            [[comms.response]]
            text = "Acknowledge"
            [[comms.response.action]]
            type = "complete_objective"
            id = "obj-x"
            "#,
        );
        let findings = validate_toml_script_comms("w.toml", &w, &defined(&["hail_axiom"]));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, COMMS_SCRIPT_AND_RESPONSE);
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_declarative_comms_yields_no_script_findings() {
        // No `script` key: this front-end must not touch a plain TOML comms block.
        let w = world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            message = "Go ahead."

            [[comms.response]]
            text = "Acknowledge"
            [[comms.response.action]]
            type = "complete_objective"
            id = "obj-x"
            "#,
        );
        assert!(validate_toml_script_comms("w.toml", &w, &defined(&[])).is_empty());
    }

    // ── `flags` compound-assignment lint (issue #994) ─────────────────────────

    fn src(path: &str, source: &str) -> ScriptSource {
        ScriptSource {
            path: path.to_string(),
            source: source.to_string(),
        }
    }

    #[test]
    fn a_flag_plus_equals_is_a_blocking_finding() {
        let sources = vec![src(
            "w.toml#script.s",
            r#"fn on_x(ctx) { flags.score += 50; }"#,
        )];
        let findings = validate_flag_opassign(&sources);
        assert_eq!(findings.len(), 1, "{findings:?}");
        let f = &findings[0];
        assert!(f.is_error());
        assert_eq!(f.category, FLAG_OPASSIGN_NOT_COMPOSABLE);
        assert_eq!(f.source.file, "w.toml#script.s");
        assert_eq!(f.source.reference, "flags.score");
        assert_eq!(f.source.line, Some(1));
        assert!(f.message.contains("flags.increment"));
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn the_real_ctx_flags_idiom_is_also_caught() {
        // The shipped spelling is `ctx.flags.x += n`, where `flags` sits mid-chain.
        let sources = vec![src(
            "w.toml#script.s",
            r#"fn on_x(ctx) { ctx.flags.score += 50; }"#,
        )];
        let findings = validate_flag_opassign(&sources);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, FLAG_OPASSIGN_NOT_COMPOSABLE);
        assert_eq!(findings[0].source.reference, "flags.score");
    }

    #[test]
    fn every_compound_operator_is_rejected() {
        // Each of the compound-assignment operators degrades identically.
        for op in [
            "+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", "&=", "|=", "^=",
        ] {
            let source = format!("fn on_x(ctx) {{ ctx.flags.score {op} 1; }}");
            let findings = validate_flag_opassign(&[src("s", &source)]);
            assert_eq!(findings.len(), 1, "operator {op} should fire: {findings:?}");
            assert_eq!(findings[0].category, FLAG_OPASSIGN_NOT_COMPOSABLE);
        }
    }

    #[test]
    fn the_flag_index_form_is_rejected() {
        let sources = vec![src(
            "w.toml#script.s",
            r#"fn on_x(ctx) { ctx.flags["kills"] += 1; }"#,
        )];
        let findings = validate_flag_opassign(&sources);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, FLAG_OPASSIGN_NOT_COMPOSABLE);
        assert_eq!(findings[0].source.reference, "flags[…]");
    }

    #[test]
    fn the_increment_verb_is_clean() {
        // The composable verb — the whole point of M3 — must not be flagged.
        let sources = vec![src(
            "s",
            r#"fn on_x(ctx) { ctx.flags.increment("score", 50); }"#,
        )];
        assert!(validate_flag_opassign(&sources).is_empty());
    }

    #[test]
    fn a_plain_absolute_assign_is_clean() {
        // `flags.x = v` (and the index form) stay absolute and are allowed.
        let sources = vec![src(
            "s",
            r#"fn on_x(ctx) { ctx.flags.armed = 1; ctx.flags["kills"] = 3; }"#,
        )];
        assert!(validate_flag_opassign(&sources).is_empty());
    }

    #[test]
    fn a_non_flags_opassign_is_not_flagged() {
        // A `+=` on a plain local must never be flagged (no false positives).
        let sources = vec![src(
            "s",
            r#"fn on_x(ctx) { let x = 0; x += 1; let score = 0; score += 5; }"#,
        )];
        assert!(validate_flag_opassign(&sources).is_empty());
    }

    #[test]
    fn a_flag_opassign_in_a_comment_or_string_is_ignored() {
        // The stripping pass must not see `+=` inside a comment or a string —
        // otherwise even the module docs (which spell out `flags.x += 50`) would
        // trip the lint.
        let sources = vec![src(
            "s",
            r#"fn on_x(ctx) {
                // do NOT write flags.score += 50 here
                /* nor flags.score += 1 in a block comment */
                let note = "flags.score += 99";
                ctx.flags.increment("score", 1);
            }"#,
        )];
        assert!(validate_flag_opassign(&sources).is_empty());
    }

    #[test]
    fn a_flag_read_is_not_a_write() {
        // Reading a flag (`let s = flags.score;`) is not an assignment.
        let sources = vec![src("s", r#"fn on_x(ctx) { let s = ctx.flags.score; }"#)];
        assert!(validate_flag_opassign(&sources).is_empty());
    }

    #[test]
    fn each_offending_source_reports_independently() {
        let sources = vec![
            src("a", r#"fn on_a(ctx) { ctx.flags.a += 1; }"#),
            src("b", r#"fn on_b(ctx) { ctx.flags.increment("b", 1); }"#),
            src("c", r#"fn on_c(ctx) { ctx.flags["c"] += 1; }"#),
        ];
        let findings = validate_flag_opassign(&sources);
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert_eq!(findings[0].source.file, "a");
        assert_eq!(findings[1].source.file, "c");
    }
}
