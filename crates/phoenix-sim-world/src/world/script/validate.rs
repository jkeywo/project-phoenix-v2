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

use std::collections::{BTreeMap, BTreeSet};

use vellum_script::ScriptSource;

use crate::world::script::engine::{Registration, ScriptTrigger};
use crate::world::validate::{Severity, SourceLocation, WorldFinding};

/// Category slug for a handler that resolves to no defined function.
pub const UNRESOLVED_SCRIPT_FN: &str = "unresolved-script-fn";

/// Category slug for a malformed or duplicated GM-operable event id
/// (issue #1301).
pub const INVALID_GM_EVENT: &str = "invalid-gm-event";

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

/// Category slug for a `[[deadline]]` block and an `on_deadline(…)` declaration
/// that do not pair up (issue #1024).
///
/// A deadline is authored in TOML but *named* by script, so the two halves can
/// disagree in two directions: an `on_deadline("typo", …)` naming a block that
/// does not exist, and a `[[deadline]]` block no `on_deadline` ever claims. Both
/// produce a deadline that can never fire, which shows the crew a countdown
/// running to zero with nothing behind it — a failure no runtime check can
/// report, because nothing goes wrong until the moment nothing happens.
pub const DEADLINE_NOT_PAIRED: &str = "deadline-not-paired";

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

/// Prove every GM-operable event's authored identity is valid and unique
/// (issue #1301).
///
/// Two error findings, both on the same authoring-validation channel every
/// other pass here uses, so the atomic activation gate
/// (`world::validate::has_error`) blocks the world rather than shipping a
/// mission panel that cannot address what it lists:
///
/// 1. a malformed id or label — the exact shape
///    [`GmEventControls::validate_authored`] defines, checked again here so a
///    control set that reaches this pass from anywhere other than the
///    `gm_event` host fn (issue #1302's `gm_controls` on an ordinary trigger)
///    meets the same rule;
/// 2. two events authored with the SAME id in one compiled set;
/// 3. a Skip lever declared on a `TriggerCondition::Manual` event (issue
///    #1304) — the exact rule
///    [`GmEventControls::validate_skip_condition`] defines, checked again here
///    for (1)'s reason. A manual event has no automatic occurrence, so its
///    Skip could never be consumed: it would publish a mission-panel button an
///    operator can press for ever with no possible effect;
/// 4. an invented GM-attention band (issue #1434) — the rule
///    [`GmEventControls::validate_attention_band`] defines, checked again here
///    for (1)'s reason.
///
/// (2) is an error rather than a tolerated duplicate, and this is deliberately
/// stricter than `ResetTrigger`'s `Trigger::id` lookup, which re-arms EVERY
/// trigger sharing an id on purpose. A GM action names exactly one event and
/// must get exactly one handler run; "fire whichever of these two the table
/// happens to hold first" is not a contract an operator can act on. Ids are
/// qualified by their origin layer at read time, so this pass only has to make
/// them unique within one compiled set — which is precisely the set it sees.
fn gm_event_finding(source_path: &str, id: &str, message: String) -> WorldFinding {
    WorldFinding {
        severity: Severity::Error,
        category: INVALID_GM_EVENT,
        message,
        source: SourceLocation {
            file: source_path.to_string(),
            line: None,
            reference: id.to_string(),
        },
    }
}

pub fn validate_gm_events(script_triggers: &[ScriptTrigger]) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for st in script_triggers {
        let Some(controls) = st.trigger.gm_controls.as_ref() else {
            continue;
        };
        if let Err(message) =
            crate::world::config::GmEventControls::validate_authored(&controls.id, &controls.label)
        {
            findings.push(gm_event_finding(&st.source_path, &controls.id, message));
            continue;
        }
        let skip_placement = if controls.skip {
            crate::world::config::GmEventControls::validate_skip_condition(&st.trigger.condition)
        } else {
            Ok(())
        };
        if let Err(message) = skip_placement {
            findings.push(gm_event_finding(&st.source_path, &controls.id, message));
            continue;
        }
        // (4) an invented GM-attention band (issue #1434) — the exact rule
        // `GmEventControls::validate_attention_band` defines, checked again
        // here for (1)'s reason. A beat whose band nobody can parse would land
        // silently in the default one, which is a priority the author did not
        // choose turning up on a live facilitator's desk.
        if let Some(band) = controls.attention_band.as_deref() {
            if let Err(message) =
                crate::world::config::GmEventControls::validate_attention_band(band)
            {
                findings.push(gm_event_finding(&st.source_path, &controls.id, message));
                continue;
            }
        }
        if !seen.insert(controls.id.as_str()) {
            findings.push(gm_event_finding(
                &st.source_path,
                &controls.id,
                format!(
                    "duplicate GM event id '{}': a GM action names exactly one event",
                    controls.id
                ),
            ));
        }
    }
    findings
}

/// Prove every named deadline and its handler pair up (issue #1024).
///
/// Three error findings, all on the existing authoring-validation channel so the
/// atomic activation gate (`world::validate::has_error`) blocks the world:
///
/// 1. an `on_deadline(id, fn)` whose `fn` is not defined anywhere in the
///    compiled set — the same check every other handler name gets;
/// 2. an `on_deadline(id, …)` naming an `id` no `[[deadline]]` block declares;
/// 3. a `[[deadline]]` block no `on_deadline` claims.
///
/// (3) is an error rather than a shrug because a deadline without a handler is
/// not "a deadline that does nothing" — it is a deadline that cannot be *armed*,
/// since arming it means queuing the call it runs. An author who genuinely wants
/// a pure countdown writes an empty handler, which says so.
///
/// Reads the authored ids straight out of `world_toml` rather than taking a
/// parsed `WorldConfig`, because this pass runs inside the script loader, which
/// is handed the raw document and no config. `parse_world` has already refused a
/// duplicate id by the time a world reaches activation, so a repeated id here
/// cannot silently satisfy two registrations.
pub fn validate_deadline_handlers(
    world_path: &str,
    world_toml: &toml::Value,
    handlers: &[crate::world::deadlines::DeadlineHandler],
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let authored: Vec<String> = world_toml
        .get("deadline")
        .and_then(|v| v.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("id").and_then(|v| v.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut findings = Vec::new();
    for handler in handlers {
        if !defined_fns.contains(&handler.handler) {
            findings.push(unresolved_finding(
                &handler.handler,
                &handler.source_path,
                &format!("on_deadline(\"{}\", …)", handler.deadline_id),
            ));
        }
        if !authored.contains(&handler.deadline_id) {
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: DEADLINE_NOT_PAIRED,
                message: format!(
                    "on_deadline(\"{}\", \"{}\") names a deadline no [[deadline]] block                      declares in '{world_path}'",
                    handler.deadline_id, handler.handler
                ),
                source: SourceLocation {
                    file: handler.source_path.clone(),
                    line: None,
                    reference: handler.deadline_id.clone(),
                },
            });
        }
    }
    for id in &authored {
        if !handlers.iter().any(|h| &h.deadline_id == id) {
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: DEADLINE_NOT_PAIRED,
                message: format!(
                    "[[deadline]] '{id}' has no on_deadline(\"{id}\", \"fn\") registration, so                      nothing can be armed for it; a deadline with no effect still needs an                      (empty) handler"
                ),
                source: SourceLocation {
                    file: world_path.to_string(),
                    line: None,
                    reference: id.clone(),
                },
            });
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

/// A significant lexical token for the script source scans. Whitespace and
/// comments are dropped before these are produced, and a literal's *contents*
/// never become code tokens — so a `+=` inside a comment or string can never be
/// mistaken for code, while an authored string like an `on_pick` fn name is
/// still readable as data ([`Tok::Str`]).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Tok {
    /// An identifier run (`flags`, `score`, `ctx`, `increment`, …).
    Ident(String),
    /// A string or char literal, carrying its contents. Whatever is inside is
    /// data, never code: `"a += b"` is one `Str`, not an `OpAssign`.
    Str(String),
    /// `.` member access.
    Dot,
    /// `:` — the map-entry separator (`#{ on_pick: "fn" }`).
    Colon,
    /// `[` index open.
    LBracket,
    /// `]` index close.
    RBracket,
    /// One of the compound-assignment operators (`+=`, `-=`, `*=`, `/=`, `%=`,
    /// `**=`, `<<=`, `>>=`, `&=`, `|=`, `^=`). Plain `=` and `==` are `Other`.
    OpAssign,
    /// Any other token we do not distinguish (plain `=`, `==`, numbers, `(`, `,`,
    /// `;`, braces, other operators), carrying its FIRST character. A separator
    /// for the op-assign scan; the character is what lets the `on_pick` scan tell
    /// a value that ends (`, } ] ) ;`) from one that continues into an expression
    /// (`"on_" + kind`).
    Other(char),
}

/// A token plus the 1-based source line it starts on (for locating a finding).
pub(super) struct Token {
    pub(super) kind: Tok,
    pub(super) line: usize,
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
    lex_significant_raw(source)
        .into_iter()
        .map(|(token, _)| token)
        .collect()
}

/// Preserve exact literal spelling for source-aware recipient validation.
/// Existing callers continue to consume the same significant token stream.
pub(super) fn lex_significant_raw(source: &str) -> Vec<(Token, String)> {
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut tokens = Vec::new();
    let mut output = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;

    while i < n {
        let token_start = i;
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
            // `"…"` string literal, `\`-escaped, and `'…'` char literal.
            // Contents are captured as data (a fn name an `on_pick` names), never
            // lexed as code; a `\`-escape contributes the escaped character.
            '"' | '\'' => {
                let quote = c;
                let start_line = line;
                let mut body = String::new();
                i += 1;
                while i < n {
                    match chars[i] {
                        '\\' => {
                            if i + 1 < n {
                                body.push(chars[i + 1]);
                            }
                            i += 2;
                        }
                        ch if ch == quote => {
                            i += 1;
                            break;
                        }
                        '\n' => {
                            line += 1;
                            body.push('\n');
                            i += 1;
                        }
                        ch => {
                            body.push(ch);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token {
                    kind: Tok::Str(body),
                    line: start_line,
                });
            }
            // `` `…` `` verbatim/interpolated string — contents captured verbatim.
            '`' => {
                let start_line = line;
                let mut body = String::new();
                i += 1;
                while i < n {
                    if chars[i] == '`' {
                        i += 1;
                        break;
                    }
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    body.push(chars[i]);
                    i += 1;
                }
                tokens.push(Token {
                    kind: Tok::Str(body),
                    line: start_line,
                });
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
                        kind: Tok::Other('#'),
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
            ':' => {
                tokens.push(Token {
                    kind: Tok::Colon,
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
                        kind: Tok::Other(c),
                        line,
                    });
                    i += 1;
                }
            }
        }
        for token in tokens.drain(..) {
            output.push((token, chars[token_start..i].iter().collect()));
        }
    }
    output
}

/// Best-effort 1-based definition lines for named Rhai functions.
///
/// This shares the validation lexer so comments and every supported string
/// form cannot masquerade as code. Anonymous callbacks intentionally have no
/// definition line; callers still retain their exact script path.
pub fn named_function_lines(source: &str) -> BTreeMap<String, usize> {
    let tokens = lex_significant(source);
    let mut lines = BTreeMap::new();
    for window in tokens.windows(3) {
        if let [Token {
            kind: Tok::Ident(keyword),
            ..
        }, Token {
            kind: Tok::Ident(name),
            line,
        }, Token {
            kind: Tok::Other('('),
            ..
        }] = window
        {
            if keyword == "fn" {
                lines.entry(name.clone()).or_insert(*line);
            }
        }
    }
    lines
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

// ── dialogue `on_pick` resolution lint (issue #984) ──────────────────────────
//
// The comms front-end's branching lives in STRING LITERALS inside the maps a
// dialogue node fn returns — `#{ text: "Acknowledge", on_pick: "on_ack" }` — and
// nothing else in the load path can see them. `validate_toml_script_comms`
// reaches only the ROOT fn a `[[comms]] script = "fn"` block names; every node
// past the root is named by a literal the loader never reads. A typo therefore
// survived load and surfaced mid-mission as an unresolvable call the moment a
// player picked that response.
//
// Lexical, not an AST walk, for the reason `validate_flag_opassign` documents
// above: a true `AST::walk` over `Stmt`/`Expr` needs Rhai's `internals` feature,
// which this build does not enable, and `vellum_script` exposes no walk helper.
// So this reuses that pass's tokenizer — which already strips comments and lexes
// literals as data rather than code — and matches the token shape
// `on_pick : "<name>"`.

/// Category slug for a dialogue response whose `on_pick` string literal names no
/// defined function (issue #984).
pub const UNRESOLVED_ON_PICK_FN: &str = "unresolved-on-pick-fn";

/// Scan one script source for `on_pick: "<name>"` literals, returning
/// `(name, line)` for each.
///
/// Only a literal that is the WHOLE value is a hit — the token after it must
/// END the map entry (`,`, `}`, `]`, `)`, `;`, or the source). `on_pick: "on_" +
/// kind` opens with a `Str` too, and reporting `"on_"` as an undefined function
/// would block a legitimate world for a name the pass cannot compute. An
/// `on_pick` built any other way (`pick_fn_for(i)`, a ternary, a variable) never
/// produces a `Str` in that position at all. See [`validate_on_pick_fns`].
fn scan_on_pick_literals(source: &str) -> Vec<(String, usize)> {
    /// Characters that terminate a map-entry value.
    fn ends_the_value(kind: &Tok) -> bool {
        matches!(kind, Tok::Other(',' | '}' | ']' | ')' | ';'))
    }

    let tokens = lex_significant(source);
    let mut hits = Vec::new();
    for i in 0..tokens.len() {
        let Tok::Ident(name) = &tokens[i].kind else {
            continue;
        };
        if name != "on_pick" {
            continue;
        }
        if i + 2 >= tokens.len() || tokens[i + 1].kind != Tok::Colon {
            continue;
        }
        let Tok::Str(fn_name) = &tokens[i + 2].kind else {
            continue;
        };
        let whole_value = tokens
            .get(i + 3)
            .is_none_or(|next| ends_the_value(&next.kind));
        if whole_value {
            hits.push((fn_name.clone(), tokens[i + 2].line));
        }
    }
    hits
}

/// Prove every literal `on_pick` in every script resolves against `defined_fns`
/// (issue #984).
///
/// A scripted comms response's `on_pick` names the fn that runs when the player
/// picks it. Unlike a registration or a `[[comms]] script = "fn"`, that name is
/// authored *inside* a node fn's returned map, so no cross-reference pass could
/// see it and a typo reached the player instead of the loader: picking the
/// response called a function that does not exist, which the host answers by
/// refusing the pick (`EnterError::Unresolved`) — visible, but a dead branch in a
/// shipped mission all the same. Each unresolved name is a blocking
/// [`UNRESOLVED_ON_PICK_FN`] error on the same authoring-validation channel as
/// every other script check, so `world::validate::has_error` refuses to activate
/// the world.
///
/// # What this pass cannot see (deliberate)
///
/// The scan is lexical, so it reports only `on_pick: "<literal>"`. A dynamically
/// built name — `on_pick: pick_for(kind)`, `on_pick: "on_" + verb` — is left
/// alone rather than guessed at: the alternative is a false positive that blocks
/// a legitimate world, which for a *load-time* gate is strictly worse than the
/// missed catch. Those names are still answered at runtime by
/// [`EnterError::Unresolved`](crate::world::script::comms::EnterError::Unresolved),
/// which refuses the pick rather than killing the thread.
///
/// `defined_fns` is the WHOLE content set's function list, matching every other
/// cross-reference pass here; a name defined in a different unit from the one
/// that references it therefore passes this lint, and is caught at runtime
/// instead (`call_fn` resolves against one unit's AST).
pub fn validate_on_pick_fns(
    sources: &[ScriptSource],
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for src in sources {
        for (fn_name, line) in scan_on_pick_literals(&src.source) {
            if defined_fns.contains(&fn_name) {
                continue;
            }
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: UNRESOLVED_ON_PICK_FN,
                message: format!(
                    "dialogue response `on_pick: \"{fn_name}\"` in '{}' names no defined \
                     function; a scripted comms response's on_pick must name a node fn, or \
                     picking it refuses the response mid-mission",
                    src.path
                ),
                source: SourceLocation {
                    file: src.path.clone(),
                    line: Some(line),
                    reference: fn_name,
                },
            });
        }
    }
    findings
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
