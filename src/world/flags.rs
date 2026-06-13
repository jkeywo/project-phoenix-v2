// Flag store, predicate DSL parser, and evaluation primitives for the world
// engine (issue #412, foundation for PRD #397).
//
// Pure Rust module — no Bevy. The Bevy layer (`world::server`) wraps a
// `FlagStore` as a resource, dispatches the new flag-mutation trigger actions,
// and emits `FlagTransition` events that the reactive `on_flag_set` /
// `on_flag_cleared` trigger conditions react to.
//
// # Data model
//
// `FlagStore` holds a single namespace of named integer counters. The boolean
// view of a counter is `counter != 0`. `set_flag` writes `1`, `clear_flag`
// writes `0`. `increment_flag(name, by)` adds `by`. `set_flag_value(name,
// value)` assigns directly. All mutations return `(before, after)` integer
// pairs so callers can detect transitions (false→true, true→false) and emit
// events for the reactive trigger conditions.
//
// Unreferenced names read as `0` (counter view) / `false` (boolean view).
//
// # Parent chain
//
// World content can be additively layered (root world + sub-worlds via
// `LoadWorld`). A predicate may reference a flag in a parent layer via
// `parent:name`. `parent:parent:name` walks up two levels. Walking past the
// root resolves as "not found" (counter 0 / boolean false), matching the
// default for unreferenced names.
//
// To keep this module Bevy-free, evaluation accepts a slice of `&FlagStore`
// references representing the layer chain, ordered **innermost first**
// (`chain[0]` is the current layer, `chain[1]` its parent, etc.). The Bevy
// layer builds this slice from `WorldLayerMap` / `WorldContentRuntime` and
// hands it in. In the current single-runtime architecture the slice is
// usually a single element, and `parent:` references simply resolve as
// "not found".
//
// # Predicate DSL
//
// Infix grammar:
//
//     expr   := or
//     or     := and ("or" and)*
//     and    := unary ("and" unary)*
//     unary  := "not" unary | atom
//     atom   := "(" expr ")"
//             | "flag" "(" NAME ")"
//             | "counter" "(" NAME ")" CMP INT
//
// Precedence: `not` > `and` > `or`. Parens override. Standard math
// precedence; equivalent to most boolean expression languages.
//
// `NAME` is a sequence of `[A-Za-z0-9_:-]+` (so `parent:foo` and
// `parent:parent:foo-bar` are single tokens). `CMP` is one of `>=`, `<=`,
// `==`, `!=`, `>`, `<`. `INT` is a (possibly signed) decimal integer.
//
// Predicates are parsed eagerly at world-load time. Parse errors include the
// offending token and a position hint; no panics.

use std::collections::HashMap;

// ── Flag store ────────────────────────────────────────────────────────────

/// Per-world boolean / integer counter namespace.
///
/// Booleans and counters share the same key: `flag(name)` reads `true` iff the
/// stored counter is non-zero. `set_flag` writes `1`, `clear_flag` writes
/// `0`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlagStore {
    values: HashMap<String, i64>,
}

impl FlagStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the integer (counter) view of a name.
    ///
    /// Unset names default to `0`.
    pub fn counter(&self, name: &str) -> i64 {
        self.values.get(name).copied().unwrap_or(0)
    }

    /// Read the boolean view of a name (`counter != 0`).
    pub fn flag(&self, name: &str) -> bool {
        self.counter(name) != 0
    }

    /// Set the boolean to `true` (counter = 1). Returns `(before, after)`.
    pub fn set_flag(&mut self, name: &str) -> (i64, i64) {
        self.set_flag_value(name, 1)
    }

    /// Set the boolean to `false` (counter = 0). Returns `(before, after)`.
    pub fn clear_flag(&mut self, name: &str) -> (i64, i64) {
        self.set_flag_value(name, 0)
    }

    /// Assign the counter to `value`. Returns `(before, after)`.
    pub fn set_flag_value(&mut self, name: &str, value: i64) -> (i64, i64) {
        let before = self.counter(name);
        if value == 0 {
            // Keep the store compact for unset semantics.
            self.values.remove(name);
        } else {
            self.values.insert(name.to_string(), value);
        }
        (before, value)
    }

    /// Add `by` to the counter (can be negative). Returns `(before, after)`.
    pub fn increment_flag(&mut self, name: &str, by: i64) -> (i64, i64) {
        let before = self.counter(name);
        let after = before.saturating_add(by);
        self.set_flag_value(name, after);
        (before, after)
    }
}

/// Resolve `name` against a layer chain, walking up `parent:` prefixes.
///
/// `chain[0]` is the innermost (current) layer; subsequent entries are
/// successively outer parents. Each leading `parent:` token advances one
/// step up the chain. Walking past the end resolves as not-found (returns
/// `None`, evaluated as `0` / `false`).
fn resolve_chain<'a>(chain: &'a [&'a FlagStore], name: &str) -> Option<(&'a FlagStore, String)> {
    let mut depth = 0usize;
    let mut rest = name;
    while let Some(stripped) = rest.strip_prefix("parent:") {
        depth += 1;
        rest = stripped;
    }
    chain
        .get(depth)
        .copied()
        .map(|store| (store, rest.to_string()))
}

/// Read the counter at `name` from a layer chain, honouring `parent:` prefixes.
pub fn counter_in_chain(chain: &[&FlagStore], name: &str) -> i64 {
    match resolve_chain(chain, name) {
        Some((store, key)) => store.counter(&key),
        None => 0,
    }
}

/// Read the boolean at `name` from a layer chain, honouring `parent:` prefixes.
pub fn flag_in_chain(chain: &[&FlagStore], name: &str) -> bool {
    counter_in_chain(chain, name) != 0
}

// ── Predicate AST ─────────────────────────────────────────────────────────

/// Comparison operator for `counter(name) CMP n`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Ge,
    Gt,
    Eq,
    Ne,
    Le,
    Lt,
}

impl CmpOp {
    fn apply(self, lhs: i64, rhs: i64) -> bool {
        match self {
            CmpOp::Ge => lhs >= rhs,
            CmpOp::Gt => lhs > rhs,
            CmpOp::Eq => lhs == rhs,
            CmpOp::Ne => lhs != rhs,
            CmpOp::Le => lhs <= rhs,
            CmpOp::Lt => lhs < rhs,
        }
    }
}

/// Parsed predicate expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Predicate {
    /// `flag(name)`
    Flag {
        name: String,
    },
    /// `counter(name) CMP n`
    Counter {
        name: String,
        op: CmpOp,
        rhs: i64,
    },
    Not(Box<Predicate>),
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
}

impl Predicate {
    /// Evaluate the predicate against a flag-store chain.
    pub fn evaluate(&self, chain: &[&FlagStore]) -> bool {
        match self {
            Predicate::Flag { name } => flag_in_chain(chain, name),
            Predicate::Counter { name, op, rhs } => op.apply(counter_in_chain(chain, name), *rhs),
            Predicate::Not(inner) => !inner.evaluate(chain),
            Predicate::And(a, b) => a.evaluate(chain) && b.evaluate(chain),
            Predicate::Or(a, b) => a.evaluate(chain) || b.evaluate(chain),
        }
    }
}

// ── Tokeniser ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Token {
    LParen,
    RParen,
    Comma,
    And,
    Or,
    Not,
    Flag,
    Counter,
    Cmp(CmpOp),
    Ident(String),
    Int(i64),
}

struct Tokeniser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Tokeniser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.rest().chars().next() {
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<(Token, usize)>, String> {
        self.skip_ws();
        let start = self.pos;
        let mut chars = self.rest().chars();
        let Some(c) = chars.next() else {
            return Ok(None);
        };
        // Single-char punctuation
        match c {
            '(' => {
                self.pos += 1;
                return Ok(Some((Token::LParen, start)));
            }
            ')' => {
                self.pos += 1;
                return Ok(Some((Token::RParen, start)));
            }
            ',' => {
                self.pos += 1;
                return Ok(Some((Token::Comma, start)));
            }
            _ => {}
        }
        // Comparison operators
        if c == '>' || c == '<' || c == '=' || c == '!' {
            let two: String = self.rest().chars().take(2).collect();
            let (op, n) = match two.as_str() {
                ">=" => (CmpOp::Ge, 2),
                "<=" => (CmpOp::Le, 2),
                "==" => (CmpOp::Eq, 2),
                "!=" => (CmpOp::Ne, 2),
                _ => match c {
                    '>' => (CmpOp::Gt, 1),
                    '<' => (CmpOp::Lt, 1),
                    '=' => {
                        return Err(format!(
                            "Unexpected token '=' at position {start}; did you mean '=='?"
                        ))
                    }
                    '!' => {
                        return Err(format!(
                            "Unexpected token '!' at position {start}; did you mean '!='?"
                        ))
                    }
                    _ => unreachable!(),
                },
            };
            self.pos += n;
            return Ok(Some((Token::Cmp(op), start)));
        }
        // Integer (with optional leading -)
        if c.is_ascii_digit()
            || (c == '-'
                && self
                    .rest()
                    .chars()
                    .nth(1)
                    .map(|d| d.is_ascii_digit())
                    .unwrap_or(false))
        {
            let mut end = self.pos + c.len_utf8();
            for ch in self.rest().chars().skip(1) {
                if ch.is_ascii_digit() {
                    end += ch.len_utf8();
                } else {
                    break;
                }
            }
            let slice = &self.src[self.pos..end];
            let value: i64 = slice
                .parse()
                .map_err(|_| format!("Invalid integer '{slice}' at position {start}"))?;
            self.pos = end;
            return Ok(Some((Token::Int(value), start)));
        }
        // Identifier-ish: letters/digits/underscore/colon/hyphen
        if is_ident_start(c) {
            let mut end = self.pos + c.len_utf8();
            for ch in self.rest().chars().skip(1) {
                if is_ident_continue(ch) {
                    end += ch.len_utf8();
                } else {
                    break;
                }
            }
            let slice = &self.src[self.pos..end];
            self.pos = end;
            let tok = match slice {
                "and" => Token::And,
                "or" => Token::Or,
                "not" => Token::Not,
                "flag" => Token::Flag,
                "counter" => Token::Counter,
                _ => Token::Ident(slice.to_string()),
            };
            return Ok(Some((tok, start)));
        }
        Err(format!("Unexpected character '{c}' at position {start}"))
    }

    fn tokenise(mut self) -> Result<Vec<(Token, usize)>, String> {
        let mut out = Vec::new();
        while let Some(t) = self.next_token()? {
            out.push(t);
        }
        Ok(out)
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-'
}

// ── Parser ────────────────────────────────────────────────────────────────

struct Parser {
    tokens: Vec<(Token, usize)>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<(Token, usize)>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(t, _)| t)
    }

    fn peek_pos(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map(|(_, p)| *p)
            .unwrap_or(usize::MAX)
    }

    fn bump(&mut self) -> Option<(Token, usize)> {
        let v = self.tokens.get(self.pos).cloned();
        if v.is_some() {
            self.pos += 1;
        }
        v
    }

    fn expect(&mut self, expected: &Token, ctx: &str) -> Result<(), String> {
        match self.bump() {
            Some((t, _)) if &t == expected => Ok(()),
            Some((t, p)) => Err(format!("Expected {ctx} but got {t:?} at position {p}")),
            None => Err(format!("Expected {ctx} but reached end of predicate")),
        }
    }

    fn parse_expr(&mut self) -> Result<Predicate, String> {
        let lhs = self.parse_and()?;
        let mut acc = lhs;
        while matches!(self.peek(), Some(Token::Or)) {
            self.bump();
            let rhs = self.parse_and()?;
            acc = Predicate::Or(Box::new(acc), Box::new(rhs));
        }
        Ok(acc)
    }

    fn parse_and(&mut self) -> Result<Predicate, String> {
        let lhs = self.parse_unary()?;
        let mut acc = lhs;
        while matches!(self.peek(), Some(Token::And)) {
            self.bump();
            let rhs = self.parse_unary()?;
            acc = Predicate::And(Box::new(acc), Box::new(rhs));
        }
        Ok(acc)
    }

    fn parse_unary(&mut self) -> Result<Predicate, String> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.bump();
            let inner = self.parse_unary()?;
            return Ok(Predicate::Not(Box::new(inner)));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Predicate, String> {
        let pos = self.peek_pos();
        match self.bump() {
            Some((Token::LParen, _)) => {
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen, "')'")?;
                Ok(inner)
            }
            Some((Token::Flag, _)) => {
                self.expect(&Token::LParen, "'(' after 'flag'")?;
                let name = self.expect_name("name inside flag(...)")?;
                self.expect(&Token::RParen, "')' to close flag(...)")?;
                Ok(Predicate::Flag { name })
            }
            Some((Token::Counter, _)) => {
                self.expect(&Token::LParen, "'(' after 'counter'")?;
                let name = self.expect_name("name inside counter(...)")?;
                self.expect(&Token::RParen, "')' to close counter(...)")?;
                let op = match self.bump() {
                    Some((Token::Cmp(op), _)) => op,
                    Some((t, p)) => return Err(format!(
                        "Expected comparison operator after counter(...) but got {t:?} at position {p}"
                    )),
                    None => return Err(
                        "Expected comparison operator after counter(...) but reached end of predicate".into()
                    ),
                };
                let rhs = match self.bump() {
                    Some((Token::Int(n), _)) => n,
                    Some((t, p)) => {
                        return Err(format!(
                        "Expected integer after comparison operator but got {t:?} at position {p}"
                    ))
                    }
                    None => return Err(
                        "Expected integer after comparison operator but reached end of predicate"
                            .into(),
                    ),
                };
                Ok(Predicate::Counter { name, op, rhs })
            }
            Some((t, p)) => Err(format!("Unexpected token {t:?} at position {p}")),
            None => Err(format!(
                "Unexpected end of predicate at position {pos}; expected an atom"
            )),
        }
    }

    fn expect_name(&mut self, ctx: &str) -> Result<String, String> {
        match self.bump() {
            Some((Token::Ident(s), _)) => Ok(s),
            // Allow keywords as names when used inside flag()/counter()
            // (rare, but lets users name a flag "and" without quoting).
            Some((Token::And, _)) => Ok("and".to_string()),
            Some((Token::Or, _)) => Ok("or".to_string()),
            Some((Token::Not, _)) => Ok("not".to_string()),
            Some((Token::Flag, _)) => Ok("flag".to_string()),
            Some((Token::Counter, _)) => Ok("counter".to_string()),
            Some((t, p)) => Err(format!("Expected {ctx} but got {t:?} at position {p}")),
            None => Err(format!("Expected {ctx} but reached end of predicate")),
        }
    }
}

/// Parse a predicate string into an AST.
///
/// Returns a diagnostic `Err` (mentioning the offending token / position) on
/// malformed input. Never panics.
pub fn parse_predicate(src: &str) -> Result<Predicate, String> {
    let tokens = Tokeniser::new(src).tokenise()?;
    if tokens.is_empty() {
        return Err("Predicate is empty".into());
    }
    let mut parser = Parser::new(tokens);
    let pred = parser.parse_expr()?;
    if parser.pos != parser.tokens.len() {
        let (t, p) = parser.tokens[parser.pos].clone();
        return Err(format!("Unexpected trailing token {t:?} at position {p}"));
    }
    Ok(pred)
}

// ── Unit Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- FlagStore basics --------------------------------------------------

    #[test]
    fn unset_flag_reads_false_and_zero() {
        let store = FlagStore::new();
        assert!(!store.flag("missing"));
        assert_eq!(store.counter("missing"), 0);
    }

    #[test]
    fn set_flag_then_read_true() {
        let mut store = FlagStore::new();
        let (before, after) = store.set_flag("a");
        assert_eq!(before, 0);
        assert_eq!(after, 1);
        assert!(store.flag("a"));
        assert_eq!(store.counter("a"), 1);
    }

    #[test]
    fn clear_flag_resets_to_false() {
        let mut store = FlagStore::new();
        store.set_flag("a");
        let (before, after) = store.clear_flag("a");
        assert_eq!(before, 1);
        assert_eq!(after, 0);
        assert!(!store.flag("a"));
    }

    #[test]
    fn increment_flag_accumulates() {
        let mut store = FlagStore::new();
        let (_, a1) = store.increment_flag("k", 3);
        let (b2, a2) = store.increment_flag("k", 4);
        assert_eq!(a1, 3);
        assert_eq!(b2, 3);
        assert_eq!(a2, 7);
        assert_eq!(store.counter("k"), 7);
        // Boolean view: any non-zero counter is true.
        assert!(store.flag("k"));
    }

    #[test]
    fn nonzero_counter_reads_true_via_flag() {
        let mut store = FlagStore::new();
        store.set_flag_value("c", 5);
        assert!(store.flag("c"));
        assert_eq!(store.counter("c"), 5);
    }

    #[test]
    fn set_flag_value_zero_clears_storage() {
        let mut store = FlagStore::new();
        store.set_flag_value("c", 5);
        let (before, after) = store.set_flag_value("c", 0);
        assert_eq!(before, 5);
        assert_eq!(after, 0);
        assert!(!store.flag("c"));
    }

    // --- Parent chain ------------------------------------------------------

    #[test]
    fn parent_prefix_walks_one_layer_up() {
        let mut parent = FlagStore::new();
        parent.set_flag("a");
        let child = FlagStore::new();
        assert!(flag_in_chain(&[&child, &parent], "parent:a"));
        // No prefix → looks in current (child), which is unset.
        assert!(!flag_in_chain(&[&child, &parent], "a"));
    }

    #[test]
    fn parent_prefix_walks_multiple_layers_up() {
        let mut root = FlagStore::new();
        root.set_flag_value("k", 7);
        let mid = FlagStore::new();
        let child = FlagStore::new();
        let chain: &[&FlagStore] = &[&child, &mid, &root];
        assert_eq!(counter_in_chain(chain, "parent:parent:k"), 7);
        assert!(flag_in_chain(chain, "parent:parent:k"));
    }

    #[test]
    fn parent_past_root_resolves_as_unset() {
        let store = FlagStore::new();
        let chain: &[&FlagStore] = &[&store];
        assert!(!flag_in_chain(chain, "parent:parent:anything"));
        assert_eq!(counter_in_chain(chain, "parent:parent:anything"), 0);
    }

    // --- Predicate parsing & evaluation ------------------------------------

    fn store_with(pairs: &[(&str, i64)]) -> FlagStore {
        let mut s = FlagStore::new();
        for (k, v) in pairs {
            s.set_flag_value(k, *v);
        }
        s
    }

    #[test]
    fn parse_flag_atom() {
        let pred = parse_predicate("flag(a)").unwrap();
        assert_eq!(pred, Predicate::Flag { name: "a".into() });
    }

    #[test]
    fn parse_counter_with_ge() {
        let pred = parse_predicate("counter(x) >= 5").unwrap();
        assert_eq!(
            pred,
            Predicate::Counter {
                name: "x".into(),
                op: CmpOp::Ge,
                rhs: 5
            }
        );
    }

    #[test]
    fn parse_all_comparison_operators() {
        for (src, op) in [
            ("counter(x) >= 1", CmpOp::Ge),
            ("counter(x) > 1", CmpOp::Gt),
            ("counter(x) == 1", CmpOp::Eq),
            ("counter(x) != 1", CmpOp::Ne),
            ("counter(x) <= 1", CmpOp::Le),
            ("counter(x) < 1", CmpOp::Lt),
        ] {
            let p = parse_predicate(src).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert_eq!(
                p,
                Predicate::Counter {
                    name: "x".into(),
                    op,
                    rhs: 1
                }
            );
        }
    }

    #[test]
    fn parse_negative_integer_in_counter_comparison() {
        let pred = parse_predicate("counter(x) >= -3").unwrap();
        assert_eq!(
            pred,
            Predicate::Counter {
                name: "x".into(),
                op: CmpOp::Ge,
                rhs: -3
            }
        );
    }

    #[test]
    fn parse_parent_prefix_in_name() {
        let pred = parse_predicate("flag(parent:parent:goal)").unwrap();
        assert_eq!(
            pred,
            Predicate::Flag {
                name: "parent:parent:goal".into()
            }
        );
    }

    #[test]
    fn parse_and_precedence_higher_than_or() {
        // a or b and c → a or (b and c)
        let pred = parse_predicate("flag(a) or flag(b) and flag(c)").unwrap();
        match pred {
            Predicate::Or(lhs, rhs) => {
                assert_eq!(*lhs, Predicate::Flag { name: "a".into() });
                assert!(matches!(*rhs, Predicate::And(_, _)));
            }
            other => panic!("expected Or at root, got {other:?}"),
        }
    }

    #[test]
    fn parse_not_binds_tightest() {
        // not a and b → (not a) and b
        let pred = parse_predicate("not flag(a) and flag(b)").unwrap();
        match pred {
            Predicate::And(lhs, _) => assert!(matches!(*lhs, Predicate::Not(_))),
            other => panic!("expected And at root, got {other:?}"),
        }
    }

    #[test]
    fn parens_override_precedence() {
        // (a or b) and c
        let pred = parse_predicate("(flag(a) or flag(b)) and flag(c)").unwrap();
        match pred {
            Predicate::And(lhs, _) => assert!(matches!(*lhs, Predicate::Or(_, _))),
            other => panic!("expected And at root, got {other:?}"),
        }
    }

    #[test]
    fn evaluate_flag_atom_against_store() {
        let s = store_with(&[("a", 1)]);
        let pred = parse_predicate("flag(a)").unwrap();
        assert!(pred.evaluate(&[&s]));
        let s2 = FlagStore::new();
        assert!(!pred.evaluate(&[&s2]));
    }

    #[test]
    fn evaluate_counter_comparison() {
        let s = store_with(&[("kills", 4)]);
        assert!(parse_predicate("counter(kills) >= 4")
            .unwrap()
            .evaluate(&[&s]));
        assert!(!parse_predicate("counter(kills) > 4")
            .unwrap()
            .evaluate(&[&s]));
        assert!(parse_predicate("counter(kills) == 4")
            .unwrap()
            .evaluate(&[&s]));
        assert!(parse_predicate("counter(missing) == 0")
            .unwrap()
            .evaluate(&[&s]));
    }

    #[test]
    fn evaluate_compound_expression() {
        let s = store_with(&[("a", 1), ("b", 0), ("c", 1)]);
        // (a and not b) or c
        let p = parse_predicate("(flag(a) and not flag(b)) or flag(c)").unwrap();
        assert!(p.evaluate(&[&s]));
    }

    #[test]
    fn evaluate_compound_short_circuit_or_with_false_lhs() {
        let s = store_with(&[("b", 1)]);
        let p = parse_predicate("flag(a) or flag(b)").unwrap();
        assert!(p.evaluate(&[&s]));
    }

    #[test]
    fn evaluate_parent_reference_in_predicate() {
        let mut parent = FlagStore::new();
        parent.set_flag("phase_done");
        let child = FlagStore::new();
        let p = parse_predicate("flag(parent:phase_done)").unwrap();
        assert!(p.evaluate(&[&child, &parent]));
    }

    #[test]
    fn round_trip_representative_expressions() {
        for src in [
            "flag(a)",
            "not flag(a)",
            "flag(a) and flag(b)",
            "flag(a) or flag(b)",
            "(flag(a) or flag(b)) and not flag(c)",
            "counter(x) >= 5 and counter(y) < 10",
            "flag(parent:phase) or counter(parent:parent:kills) > 0",
        ] {
            parse_predicate(src).unwrap_or_else(|e| panic!("'{src}' parse failed: {e}"));
        }
    }

    // --- Parser error diagnostics -----------------------------------------

    #[test]
    fn parse_empty_predicate_errors() {
        let err = parse_predicate("").unwrap_err();
        assert!(err.to_lowercase().contains("empty"), "got: {err}");
    }

    #[test]
    fn parse_unbalanced_paren_errors_without_panic() {
        let err = parse_predicate("(flag(a)").unwrap_err();
        assert!(
            err.contains("')'") || err.contains("end of predicate"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_missing_atom_errors() {
        let err = parse_predicate("flag(a) and").unwrap_err();
        assert!(err.contains("end of predicate"), "got: {err}");
    }

    #[test]
    fn parse_unknown_character_reports_position() {
        let err = parse_predicate("flag(a) & flag(b)").unwrap_err();
        assert!(err.contains("position"), "got: {err}");
        assert!(
            err.contains('&'),
            "error must mention offending char: {err}"
        );
    }

    #[test]
    fn parse_trailing_garbage_errors() {
        let err = parse_predicate("flag(a) flag(b)").unwrap_err();
        assert!(err.to_lowercase().contains("trailing"), "got: {err}");
    }

    #[test]
    fn parse_counter_without_operator_errors() {
        let err = parse_predicate("counter(a)").unwrap_err();
        assert!(err.contains("comparison operator"), "got: {err}");
    }

    #[test]
    fn parse_counter_with_non_integer_rhs_errors() {
        let err = parse_predicate("counter(a) >= foo").unwrap_err();
        assert!(err.contains("integer"), "got: {err}");
    }

    #[test]
    fn parse_bare_equals_errors_with_hint() {
        let err = parse_predicate("counter(a) = 1").unwrap_err();
        assert!(err.contains("=="), "should hint at ==, got: {err}");
    }
}
