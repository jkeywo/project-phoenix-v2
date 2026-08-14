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
enum Tok {
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

    // ── dialogue `on_pick` resolution (issue #984) ────────────────────────────

    const TREE: &str = r#"
        fn hail_axiom(ctx) {
            #{ message: "Go ahead.", responses: [
                #{ text: "Acknowledge", on_pick: "on_ack" },
                #{ text: "Decline",     on_pick: "on_declien", important: true },
            ] }
        }
        fn on_ack(ctx)     { ctx.effects.complete_objective("reach_axiom"); }
        fn on_decline(ctx) { ctx.effects.fail_objective("reach_axiom"); }
    "#;

    #[test]
    fn a_resolved_on_pick_is_clean() {
        let sources = vec![src(
            "w.toml#script.s",
            r#"
            fn root(ctx) {
                #{ message: "Go ahead.", responses: [
                    #{ text: "Yes", on_pick: "on_yes" },
                    #{ text: "No",  on_pick: "on_no" },
                ] }
            }
            fn on_yes(ctx) { }
            fn on_no(ctx) { }
            "#,
        )];
        assert!(validate_on_pick_fns(&sources, &defined(&["root", "on_yes", "on_no"])).is_empty());
    }

    #[test]
    fn a_typoed_on_pick_blocks_activation_naming_the_fn_and_file() {
        let sources = vec![src("w.toml#script.axiom", TREE)];
        let findings =
            validate_on_pick_fns(&sources, &defined(&["hail_axiom", "on_ack", "on_decline"]));
        assert_eq!(findings.len(), 1, "{findings:?}");
        let f = &findings[0];
        assert_eq!(f.category, UNRESOLVED_ON_PICK_FN);
        assert_eq!(f.source.reference, "on_declien", "the finding names the fn");
        assert_eq!(
            f.source.file, "w.toml#script.axiom",
            "and the file it is authored in"
        );
        assert!(f.source.line.is_some(), "and the line");
        assert!(f.message.contains("on_declien"));
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_dynamically_built_on_pick_is_not_flagged() {
        // The documented limitation of a lexical pass: only a literal is
        // visible. A computed name is left to the runtime's `EnterError::
        // Unresolved` rather than guessed at — a false positive here would block
        // a legitimate world at load, which is strictly worse.
        let sources = vec![src(
            "s",
            r#"
            fn root(ctx) {
                let kind = "ack";
                #{ message: "Go ahead.", responses: [
                    #{ text: "Yes", on_pick: pick_for(kind) },
                    #{ text: "No",  on_pick: "on_" + kind },
                ] }
            }
            "#,
        )];
        assert!(validate_on_pick_fns(&sources, &defined(&["root"])).is_empty());
    }

    #[test]
    fn an_on_pick_in_a_comment_or_string_is_ignored() {
        // The tokenizer lexes a literal's contents as DATA, so an `on_pick:` that
        // is itself inside a comment or a string is not a response.
        let sources = vec![src(
            "s",
            r#"
            fn root(ctx) {
                // author responses as on_pick: "handler_name"
                let doc = "on_pick: \"never_defined\"";
                #{ message: "x", responses: [] }
            }
            "#,
        )];
        assert!(validate_on_pick_fns(&sources, &defined(&["root"])).is_empty());
    }

    #[test]
    fn every_unresolved_on_pick_across_every_source_reports() {
        let sources = vec![
            src(
                "a",
                r#"fn a(ctx) { #{ responses: [ #{ on_pick: "gone_a" } ] } }"#,
            ),
            src(
                "b",
                r#"fn b(ctx) { #{ responses: [ #{ on_pick: "here_b" } ] } }"#,
            ),
            src(
                "c",
                r#"fn c(ctx) { #{ responses: [ #{ on_pick: "gone_c" } ] } }"#,
            ),
        ];
        let findings = validate_on_pick_fns(&sources, &defined(&["a", "b", "c", "here_b"]));
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert_eq!(findings[0].source.reference, "gone_a");
        assert_eq!(findings[1].source.reference, "gone_c");
    }
}
