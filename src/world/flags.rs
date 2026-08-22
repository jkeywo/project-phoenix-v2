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
//             | "history" "(" REDUCER "," NAME "," WINDOW ")" CMP OPERAND
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

use crate::bounded_history::BoundedHistory;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Flag store ────────────────────────────────────────────────────────────

/// Per-world boolean / integer counter namespace.
///
/// Booleans and counters share the same key: `flag(name)` reads `true` iff the
/// stored counter is non-zero. `set_flag` writes `1`, `clear_flag` writes
/// `0`.
///
/// serde for the #862 snapshot payload; the payload boundary is the #894 record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

    /// Iterate the SET `(name, counter)` pairs.
    ///
    /// Order is unspecified — the store is a `HashMap` — so a caller that lets
    /// this reach a payload, a fold or a rendered list sorts it. The two that
    /// exist do (`campaign::projection`, issue #867).
    ///
    /// "Set" is the whole vocabulary here: `set_flag_value(name, 0)` REMOVES the
    /// entry rather than storing a zero, so an unset name and a cleared one are
    /// the same thing to every reader, this one included. That is what makes
    /// #1043's exclusivity invariant legible — a family's answer is the member
    /// that is present.
    pub fn iter(&self) -> impl Iterator<Item = (&str, i64)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), *value))
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

    /// Float comparison, used by the typed-fact atoms (issue #775). Facts and
    /// authored parameters are real-valued (durations, margins, weights), so
    /// they compare as `f64` rather than the integer counter view.
    fn apply_f64(self, lhs: f64, rhs: f64) -> bool {
        match self {
            CmpOp::Ge => lhs >= rhs,
            CmpOp::Gt => lhs > rhs,
            CmpOp::Eq => lhs == rhs,
            CmpOp::Ne => lhs != rhs,
            CmpOp::Le => lhs <= rhs,
            CmpOp::Lt => lhs < rhs,
        }
    }

    /// The comparison as an author writes it, for diagnostics (issue #1152).
    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Ge => ">=",
            CmpOp::Gt => ">",
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Le => "<=",
            CmpOp::Lt => "<",
        }
    }
}

// ── Typed AI facts + named parameters (issue #775) ─────────────────────────
//
// The world-trigger predicate grammar reads scenario flags and counters. AI
// fine-system policies additionally read *typed facts* — real-valued readings
// snapshotted from the immutable per-tick world state (e.g. seconds since the
// ship was last in combat) — and *named parameters* — authored tunables
// (thresholds, durations, margins, weights) referenced by name from the
// expression so no gameplay value is hardcoded in the predicate.
//
// Both are read-only. Flags and counters remain read-only too: a policy never
// writes world state through an expression.

/// The registry name of one typed AI fact (issue #1210).
///
/// A `&'static str` newtype whose only values are the catalogue constants in
/// [`crate::entities::ai_flag_hosts`]. Production seeders record a reading
/// through [`AiFacts::set_fact`] so the seeded name is a registry constant
/// rather than a bare literal that a typo could silently diverge from — the
/// developer-facing half of closing PRD #774 §11's unvalidated-`fact()` hole.
/// The author-facing half is
/// [`crate::entities::ai_flag_hosts::AiHost::check_facts`], which rejects a
/// `fact(...)` name no host declares it seeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FactId(pub &'static str);

impl FactId {
    /// The fact's name, as it appears in a `fact(...)` atom.
    pub const fn name(self) -> &'static str {
        self.0
    }
}

/// Immutable snapshot of typed facts for one policy evaluation.
///
/// A fact that is *absent* (no reading available — e.g. the ship has never
/// been in combat) makes every comparison against it evaluate `false`, so an
/// author writing `fact(secs_since_combat) < param(window)` gets the intuitive
/// "not recently in combat" answer when there is no combat history at all.
///
/// serde for the #862 snapshot payload; the payload boundary is the #894 record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AiFacts {
    values: HashMap<String, f64>,
}

/// Which of the three evaluation contexts a `fact(...)` atom reads (issue #776).
///
/// The bare `fact(name)` keyword resolves to [`FactContext::SelfCtx`] for
/// #775 back-compat (e.g. `fact(secs_since_combat)` is a self reading). The
/// per-system target selector adds `candidate_fact(name)` and `target_fact(name)`
/// so an eligibility/score expression can compare the candidate under
/// consideration and the currently-retained target against self.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactContext {
    /// The operating ship itself (`self_fact(...)` or bare `fact(...)`).
    SelfCtx,
    /// The candidate contact being scored (`candidate_fact(...)`).
    Candidate,
    /// The currently-retained selected target (`target_fact(...)`).
    Target,
    /// The owning fine system's *typed private memory* (`memory(name)`, issue
    /// #882). Read from the [`AiPolicyMemory`] bag the OWNING system seeds for its
    /// own evaluation and nothing else: a sibling fine system's seeding call
    /// never populates this bag, so a policy physically cannot read another
    /// system's memory. Absent name → comparison `false`, the same contract
    /// every other context carries.
    Memory,
    /// The owning fine system's *state time* (`state_time`, issue #882): how
    /// long, in shared-AI-tick-derived seconds, the policy has been in its
    /// current state. Carried on the same [`AiPolicyMemory`] bag as `memory(...)`
    /// but in its own field, so no authored memory name can collide with it.
    /// The atom takes no argument; the `name` on the parsed
    /// [`Predicate::Fact`] is the fixed literal `"state_time"` and is used for
    /// diagnostics only.
    StateTime,
}

/// The three typed-fact sets one selector evaluation reads (issue #776).
///
/// `self_facts` describe the operating ship (position-derived readings,
/// faction, authored `power_rating`), `candidate_facts` the contact being
/// scored, and `target_facts` the currently-retained selection. Any set may be
/// empty; an absent fact in any context evaluates a comparison `false` (never a
/// panic), the same absent-fact contract [`AiFacts`] already carries.
///
/// serde for the #862 snapshot payload; the payload boundary is the #894 record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AiFactSet {
    pub self_facts: AiFacts,
    pub candidate_facts: AiFacts,
    pub target_facts: AiFacts,
}

impl AiFacts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a present fact reading under a bare name.
    ///
    /// Kept for tests, and for the two dynamically-named fact FAMILIES a
    /// registry constant cannot express — `power_<group>` and
    /// `recent_damage_<facing>`, whose suffix is a data-driven id. Every
    /// statically-named production seed goes through [`set_fact`](Self::set_fact)
    /// instead (issue #1210).
    pub fn set(&mut self, name: &str, value: f64) {
        self.values.insert(name.to_string(), value);
    }

    /// Record a present fact reading under a registry [`FactId`] (issue #1210).
    ///
    /// The typed-name path every statically-named production seeder uses, so a
    /// seeded fact name is a catalogue constant that
    /// [`crate::entities::ai_flag_hosts`]'s drift test can pin against the
    /// per-host descriptor registry.
    pub fn set_fact(&mut self, id: FactId, value: f64) {
        self.set(id.name(), value);
    }

    /// Read a fact reading; `None` when the fact is absent this tick.
    pub fn get(&self, name: &str) -> Option<f64> {
        self.values.get(name).copied()
    }

    /// Iterate the present `(name, value)` readings. Used by the target
    /// selector to fold multiple sources' candidate facts into one entry
    /// (issue #776). Order is unspecified (backed by a `HashMap`).
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> {
        self.values.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

/// One fine system's *typed private memory* plus its state time (issue #882).
///
/// This is the only readable surface behind the `memory(name)` and `state_time`
/// atoms. It is deliberately NOT part of [`AiFacts`]: facts are a shared,
/// host-seeded snapshot of world readings that several systems may derive from
/// the same surfaces, whereas this bag belongs to exactly one fine system's
/// policy runtime. AC3 scoping is therefore structural — a system seeds its own
/// bag from its own state component, so there is no path by which one fine
/// system's evaluation observes another's memory, and no ship-wide state
/// machine can form out of it.
///
/// Values are typed as `f64` for the same reason authored parameters are: the
/// predicate grammar compares real-valued quantities. An absent name makes
/// every comparison against it evaluate `false` (never a panic), matching
/// [`AiFacts`]'s absent-fact contract.
///
/// Named `AiPolicyMemory`, NOT `AiMemory`: `AiMemory` is the per-ship
/// private-reasoning blob issue #702 DELETED (see the notes in `ai::server` and
/// `ai::core`), and reusing that name would make a history grep conflate the
/// thing that was removed with the thing #882 introduced. They are not the same
/// idea — the deleted one was ship-wide and mutated by coarse AI, this one is
/// owned by exactly one fine system's policy runtime.
///
/// serde for the #862 snapshot payload; the payload boundary is the #894 record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AiPolicyMemory {
    values: HashMap<String, f64>,
    state_time_secs: f64,
    history: AiHistory,
}

impl AiPolicyMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// This system's bounded history windows, the surface behind the
    /// `history(...)` atom (issue #890).
    ///
    /// Carried on this bag rather than as a fourth argument threaded through
    /// every evaluation entry point, because it is the same KIND of thing the
    /// bag already holds: per-fine-system retained state that the host writes
    /// and the policy only reads, scoped so that no sibling system can observe
    /// it and rebuilt from scratch by [`crate::ai::policy::AiPolicyRuntimeState::reset`]
    /// when AI (re)gains control. Riding here is also what makes a history atom
    /// readable in BOTH authorable positions on a stateful host — the
    /// transition guards the state tick resolves, and the per-state rule guards
    /// the per-axis actuator systems resolve later in the same tick — since both
    /// already receive this bag.
    pub fn history(&self) -> &AiHistory {
        &self.history
    }

    /// Advance this system's history windows by exactly one sample.
    ///
    /// The ONE mutating entry point, deliberately named so a source scan can
    /// find every fold site in the crate — see [`AiHistory::fold_history`] for
    /// why there must only ever be one per shared AI tick.
    pub fn fold_history(&mut self, specs: &[HistorySpec], facts: &AiFacts) {
        self.history.fold_history(specs, facts);
    }

    /// Write one named private memory slot.
    pub fn set(&mut self, name: &str, value: f64) {
        self.values.insert(name.to_string(), value);
    }

    /// Read one named private memory slot; `None` when undeclared/unwritten.
    pub fn get(&self, name: &str) -> Option<f64> {
        self.values.get(name).copied()
    }

    /// Seconds spent in the current policy state. Advanced from the shared AI
    /// tick cadence by the policy state runtime, never from a per-frame clock
    /// (issue #882 AC4).
    pub fn state_time_secs(&self) -> f64 {
        self.state_time_secs
    }

    /// Set the state time. Called only by the policy state runtime.
    pub fn set_state_time_secs(&mut self, secs: f64) {
        self.state_time_secs = secs;
    }

    /// Iterate the present `(name, value)` slots. Order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> {
        self.values.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

/// Authored named parameters referenced by policy expressions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiParams {
    values: HashMap<String, f64>,
}

impl AiParams {
    pub fn new() -> Self {
        Self::default()
    }

    /// Author a named parameter value.
    pub fn set(&mut self, name: &str, value: f64) {
        self.values.insert(name.to_string(), value);
    }

    /// Resolve a named parameter; `None` when the name is unknown.
    pub fn get(&self, name: &str) -> Option<f64> {
        self.values.get(name).copied()
    }

    /// The set of authored parameter names (used by content validation).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }
}

/// Right-hand side of a `fact(name) CMP …` comparison: a numeric literal or a
/// reference to an authored named parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Number(f64),
    Param(String),
}

impl Operand {
    fn resolve(&self, params: &AiParams) -> Option<f64> {
        match self {
            Operand::Number(n) => Some(*n),
            Operand::Param(name) => params.get(name),
        }
    }

    /// Render the operand back the way an author typed it, for diagnostics.
    fn render(&self) -> String {
        match self {
            Operand::Number(n) => {
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Operand::Param(name) => format!("param({name})"),
        }
    }
}

// ── Authored history operators (issue #890) ───────────────────────────────
//
// Every other atom in this grammar reads ONE tick. A decision like "has this
// ship HELD its distance" or "is the gap actually opening" cannot be taken from
// one tick and cannot be taken from a running aggregate either (a running
// minimum never recovers from one bad sample). It needs the last N readings and
// nothing older — [`crate::bounded_history::BoundedHistory`].
//
// Before #890 that shape existed only as host-side Rust: #788 and #789 each
// invented a bespoke fact (`safe_distance_held`, `separation_progress`) with its
// own hand-rolled window, capacity param and fold site, which is exactly the
// per-question-Rust pattern PRD #774 §5.2 forbids. These types make the window
// AUTHORABLE:
//
//     history(min, range_to_target, param(standoff_ticks)) >= param(safe_range)
//     history(net_change, range_to_target, param(escape_ticks)) > param(min_progress)
//
// # Why a REDUCER and a comparison, rather than a `held(...)` predicate
//
// The grammar everywhere else is "one atom CMP one operand", and keeping that
// shape means a history atom composes with `and`/`or`/`not` and with `param(...)`
// operands for free. `min` over a full window compared `>=` IS
// `BoundedHistory::all_at_least` (pinned by a test), `max ... <=` is its mirror,
// and `net_change` is the trend question — one spelling covers both shipped
// shapes and the four comparison directions, where a bespoke `held(...)` would
// have needed a new primitive per direction.
//
// # Why every reducer is gated on a FULL window
//
// A partly-filled window measures a SHORTER span than the one the designer
// authored. `min` over two samples of an authored eight would answer a question
// nobody asked, and would do so most confidently right after a clear — exactly
// when the window knows least. An unfilled window reduces to `None`, and an
// absent reading makes the comparison `false`, the same contract `fact(...)`
// carries.

/// The names of the reducers a `history(...)` atom may use, for diagnostics.
pub const HISTORY_REDUCERS: &[&str] = &["min", "max", "net_change"];

/// Which scalar a `history(...)` atom reduces its bounded window to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HistoryReducer {
    /// The smallest retained sample. `history(min, x, n) >= t` is "every one of
    /// the last `n` readings of `x` was at least `t`".
    Min,
    /// The largest retained sample — the mirror of [`Self::Min`].
    Max,
    /// Newest sample minus oldest: which way, and how far, the reading has moved
    /// across the authored span.
    NetChange,
}

impl HistoryReducer {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "min" => Some(HistoryReducer::Min),
            "max" => Some(HistoryReducer::Max),
            "net_change" => Some(HistoryReducer::NetChange),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            HistoryReducer::Min => "min",
            HistoryReducer::Max => "max",
            HistoryReducer::NetChange => "net_change",
        }
    }

    /// Reduce a window to one reading, or `None` while it is not yet full.
    fn reduce(self, window: &BoundedHistory) -> Option<f64> {
        if !window.is_full() {
            return None;
        }
        match self {
            HistoryReducer::Min => window.min(),
            HistoryReducer::Max => window.max(),
            HistoryReducer::NetChange => window.net_change(),
        }
    }
}

/// A RESOLVED window: which fact is sampled, and over how many shared AI ticks.
///
/// The identity of a window, and therefore the key the host folds under. The
/// capacity is part of that identity on purpose: #789 needed two windows over
/// the SAME reading with independent authored lengths (a level question and a
/// trend question, tuned for different things), and keying on the fact alone
/// would have silently coupled them.
///
/// Serialisable because it is the `BTreeMap` key in [`AiHistory`], which is
/// itself a field of [`AiPolicyMemory`] — serde for the #862 snapshot payload;
/// the payload boundary is the #894 record.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HistorySpec {
    pub fact: String,
    pub ticks: usize,
}

/// An UNRESOLVED window as authored: the fact name plus the window length,
/// which may be a literal or a `param(...)` reference.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryWindow {
    pub fact: String,
    pub ticks: Operand,
}

impl HistoryWindow {
    /// Resolve the authored length against the policy's parameters.
    ///
    /// `None` when the parameter is unknown, or the length is not a positive
    /// whole number of ticks. Content validation rejects both at load, so a
    /// live policy never takes this arm; evaluating it as "no window" rather
    /// than panicking is the same no-panic contract every other atom keeps.
    pub fn resolve(&self, params: &AiParams) -> Option<HistorySpec> {
        let ticks = self.ticks.resolve(params)?;
        if !ticks.is_finite() || ticks.fract() != 0.0 || ticks < 1.0 {
            return None;
        }
        Some(HistorySpec {
            fact: self.fact.clone(),
            ticks: ticks as usize,
        })
    }
}

/// One authored `history(...)` atom, as collected out of an expression by
/// [`Predicate::referenced_history`].
///
/// Carries the reducer as well as the window so a rejection can quote the atom
/// back verbatim — with three reducers and two window spellings, "which one did
/// I write?" is the author's immediate next question.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryRef {
    pub reducer: HistoryReducer,
    pub window: HistoryWindow,
}

impl HistoryRef {
    /// The atom as the author typed it.
    pub fn render(&self) -> String {
        format!(
            "history({}, {}, {})",
            self.reducer.name(),
            self.window.fact,
            self.window.ticks.render()
        )
    }
}

/// One fine system's BOUNDED history windows (issue #890).
///
/// Bounded twice over, which is the whole reason this is not a `Vec` of
/// readings. The SET of windows is fixed by the authored expression — the host
/// folds exactly the specs its policy asks for and drops any other — and each
/// window retains at most its authored capacity. Memory is therefore constant
/// for the life of a run, however long the scenario lasts, and "recently" keeps
/// meaning the same span from the first tick to the last.
///
/// Serialisable because it is a field of [`AiPolicyMemory`] — serde for the
/// #862 snapshot payload; the payload boundary is the #894 record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AiHistory {
    windows: std::collections::BTreeMap<HistorySpec, BoundedHistory>,
}

impl AiHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance every declared window by EXACTLY ONE sample.
    ///
    /// # Call this once per shared AI tick, from one host, and nowhere else
    ///
    /// This is the sharp edge issue #789 documented and #890 exists to make
    /// safe. The four per-axis helm actuator systems all resolve guards from the
    /// same ship in the same tick; a window folded from each of them would
    /// advance four times per shared tick, so an authored
    /// `history(net_change, x, 30)` would silently measure a quarter of the span
    /// the file says. The fold therefore belongs where the shared per-tick state
    /// advance already happens, and `entities::ai_flag_hosts` records which host
    /// that is for every AI policy surface — with a rejection at load for the
    /// hosts where nothing folds, and a source scan that fails if a second fold
    /// site appears.
    ///
    /// An ABSENT reading clears the window rather than skipping the tick. A
    /// window that closed over a hole would span more real time than its
    /// authored length while claiming not to — the reading either exists every
    /// tick of the span or the span has not happened yet.
    pub fn fold_history(&mut self, specs: &[HistorySpec], facts: &AiFacts) {
        // Windows nobody asks for any more are dropped, so the map is exactly
        // the authored set and cannot accumulate.
        self.windows.retain(|spec, _| specs.contains(spec));
        for spec in specs {
            let window = self.windows.entry(spec.clone()).or_default();
            // Re-applied every fold because the capacity comes from authored
            // data a `default()` bag cannot see; `set_capacity` is a no-op when
            // unchanged, so this can never reset the window.
            window.set_capacity(spec.ticks);
            match facts.get(&spec.fact) {
                Some(value) => window.push(value),
                None => window.clear(),
            }
        }
    }

    /// Reduce one window to a single reading; `None` when the window is unknown
    /// or not yet full.
    pub fn reduce(&self, spec: &HistorySpec, reducer: HistoryReducer) -> Option<f64> {
        reducer.reduce(self.windows.get(spec)?)
    }

    /// The window folded under `spec`, if any. Read-only: only
    /// [`Self::fold_history`] may advance one.
    pub fn window(&self, spec: &HistorySpec) -> Option<&BoundedHistory> {
        self.windows.get(spec)
    }

    /// How many distinct windows are being folded.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
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
    /// `fact(name) CMP operand` — typed AI fact compared to a literal or a
    /// named parameter (issue #775). The `context` selects which of the three
    /// selector fact sets the reading comes from (issue #776); bare `fact(...)`
    /// parses as [`FactContext::SelfCtx`] for #775 back-compat.
    Fact {
        context: FactContext,
        name: String,
        op: CmpOp,
        rhs: Operand,
    },
    /// `history(REDUCER, fact_name, window) CMP operand` — a bounded window
    /// over one fact's recent readings, reduced to a single scalar and compared
    /// like any other atom (issue #890).
    ///
    /// Reads the OWNING fine system's own history bag, folded once per shared AI
    /// tick by its host. Absent (window unknown, or not yet full) makes the
    /// comparison `false`, never a panic.
    History {
        reducer: HistoryReducer,
        window: HistoryWindow,
        op: CmpOp,
        rhs: Operand,
    },
    /// `true` / `false` literal (issue #775) — a default rule guard.
    Bool(bool),
    Not(Box<Predicate>),
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
}

impl Predicate {
    /// Evaluate the predicate against a flag-store chain only.
    ///
    /// World triggers use this; typed-fact atoms have no readings here and
    /// resolve `false`, and named-parameter operands have no bindings.
    pub fn evaluate(&self, chain: &[&FlagStore]) -> bool {
        self.evaluate_with(&AiFacts::default(), &AiParams::default(), chain)
    }

    /// Evaluate against typed facts, named parameters, and a read-only flag
    /// chain (issue #775). This is the AI fine-system policy entry point.
    ///
    /// Flags and counters stay read-only; facts and parameters are read-only
    /// too. An absent fact, or a parameter operand that resolves to no value,
    /// makes the comparison `false` — never a panic (the diagnostic-Err /
    /// no-panic contract that content validation depends on).
    pub fn evaluate_with(&self, facts: &AiFacts, params: &AiParams, chain: &[&FlagStore]) -> bool {
        // A single-context (#775) evaluation is a selector evaluation whose
        // candidate/target sets are empty: bare `fact(...)` reads SELF, and any
        // stray candidate/target atom simply finds no reading → false.
        let empty = AiFacts::default();
        self.evaluate_ctx(
            facts,
            &empty,
            &empty,
            &AiPolicyMemory::default(),
            params,
            chain,
        )
    }

    /// Evaluate against typed facts, the owning fine system's private memory +
    /// state time, named parameters, and a read-only flag chain (issue #882).
    ///
    /// This is the *stateful* fine-system policy entry point. It differs from
    /// [`evaluate_with`] only in that `memory(...)` / `state_time` atoms find
    /// readings; every other atom behaves identically, so a stateless
    /// expression evaluated here gives the same answer it always did.
    pub fn evaluate_stateful(
        &self,
        facts: &AiFacts,
        memory: &AiPolicyMemory,
        params: &AiParams,
        chain: &[&FlagStore],
    ) -> bool {
        let empty = AiFacts::default();
        self.evaluate_ctx(facts, &empty, &empty, memory, params, chain)
    }

    /// Render the predicate back to a readable, source-like guard expression for
    /// diagnostics (issue #1152) — e.g. `fact(range_to_target) < param(orbit)`.
    ///
    /// Reconstructed from the parsed tree, so it reads the way the author wrote
    /// it. A composed `and`/`or` sub-expression is parenthesised so a nested
    /// guard is unambiguous; every atom (`flag`, `counter`, `fact`, `memory`,
    /// `state_time`, `history`) renders through the same per-context vocabulary
    /// the parser accepts. Used only by the read-only AI policy-state debug
    /// surface (`crate::debug::ai_state`); it never changes an evaluation.
    pub fn render(&self) -> String {
        match self {
            Predicate::Flag { name } => format!("flag({name})"),
            Predicate::Counter { name, op, rhs } => {
                format!("counter({name}) {} {rhs}", op.symbol())
            }
            Predicate::Fact {
                context,
                name,
                op,
                rhs,
            } => {
                let lhs = match context {
                    FactContext::SelfCtx => format!("fact({name})"),
                    FactContext::Candidate => format!("candidate_fact({name})"),
                    FactContext::Target => format!("target_fact({name})"),
                    FactContext::Memory => format!("memory({name})"),
                    // The `state_time` atom takes no argument; `name` is the
                    // fixed literal, kept for diagnostics only (see FactContext).
                    FactContext::StateTime => "state_time".to_string(),
                };
                format!("{lhs} {} {}", op.symbol(), rhs.render())
            }
            Predicate::History {
                reducer,
                window,
                op,
                rhs,
            } => format!(
                "history({}, {}, {}) {} {}",
                reducer.name(),
                window.fact,
                window.ticks.render(),
                op.symbol(),
                rhs.render()
            ),
            Predicate::Bool(b) => b.to_string(),
            Predicate::Not(inner) => format!("not {}", inner.render_grouped()),
            Predicate::And(a, b) => {
                format!("{} and {}", a.render_grouped(), b.render_grouped())
            }
            Predicate::Or(a, b) => format!("{} or {}", a.render_grouped(), b.render_grouped()),
        }
    }

    /// [`render`](Self::render), parenthesised when the node is itself a
    /// composed `and`/`or`, so a nested guard reads unambiguously.
    fn render_grouped(&self) -> String {
        match self {
            Predicate::And(..) | Predicate::Or(..) => format!("({})", self.render()),
            _ => self.render(),
        }
    }

    /// Evaluate against the three selector fact contexts (self / candidate /
    /// target), named parameters, and a read-only flag chain (issue #776).
    ///
    /// This is the per-system target-selector entry point. Every context obeys
    /// the same absent-fact-→-false, no-panic contract as [`evaluate_with`].
    pub fn evaluate_selector(
        &self,
        facts: &AiFactSet,
        params: &AiParams,
        chain: &[&FlagStore],
    ) -> bool {
        self.evaluate_ctx(
            &facts.self_facts,
            &facts.candidate_facts,
            &facts.target_facts,
            &AiPolicyMemory::default(),
            params,
            chain,
        )
    }

    fn evaluate_ctx(
        &self,
        self_facts: &AiFacts,
        candidate_facts: &AiFacts,
        target_facts: &AiFacts,
        memory: &AiPolicyMemory,
        params: &AiParams,
        chain: &[&FlagStore],
    ) -> bool {
        match self {
            Predicate::Flag { name } => flag_in_chain(chain, name),
            Predicate::Counter { name, op, rhs } => op.apply(counter_in_chain(chain, name), *rhs),
            Predicate::Fact {
                context,
                name,
                op,
                rhs,
            } => {
                // The private contexts (issue #882) read the owning system's
                // own memory bag; the three world contexts read their fact set.
                let lhs = match context {
                    FactContext::SelfCtx => self_facts.get(name),
                    FactContext::Candidate => candidate_facts.get(name),
                    FactContext::Target => target_facts.get(name),
                    FactContext::Memory => memory.get(name),
                    FactContext::StateTime => Some(memory.state_time_secs()),
                };
                match (lhs, rhs.resolve(params)) {
                    (Some(lhs), Some(rhs)) => op.apply_f64(lhs, rhs),
                    // Absent fact or unresolved parameter → false, never panic.
                    _ => false,
                }
            }
            Predicate::History {
                reducer,
                window,
                op,
                rhs,
            } => {
                // An unresolvable window (unknown param, or a length content
                // validation would have rejected) reads as no window at all,
                // which makes the comparison false — the same absent-reading
                // contract every other atom keeps.
                let Some(spec) = window.resolve(params) else {
                    return false;
                };
                match (
                    memory.history().reduce(&spec, *reducer),
                    rhs.resolve(params),
                ) {
                    (Some(lhs), Some(rhs)) => op.apply_f64(lhs, rhs),
                    _ => false,
                }
            }
            Predicate::Bool(b) => *b,
            Predicate::Not(inner) => !inner.evaluate_ctx(
                self_facts,
                candidate_facts,
                target_facts,
                memory,
                params,
                chain,
            ),
            Predicate::And(a, b) => {
                a.evaluate_ctx(
                    self_facts,
                    candidate_facts,
                    target_facts,
                    memory,
                    params,
                    chain,
                ) && b.evaluate_ctx(
                    self_facts,
                    candidate_facts,
                    target_facts,
                    memory,
                    params,
                    chain,
                )
            }
            Predicate::Or(a, b) => {
                a.evaluate_ctx(
                    self_facts,
                    candidate_facts,
                    target_facts,
                    memory,
                    params,
                    chain,
                ) || b.evaluate_ctx(
                    self_facts,
                    candidate_facts,
                    target_facts,
                    memory,
                    params,
                    chain,
                )
            }
        }
    }

    /// Collect every `param(name)` referenced anywhere in the expression.
    ///
    /// Content validation uses this to reject a policy expression that
    /// references a named parameter the author never declared (issue #775).
    pub fn referenced_params(&self, out: &mut Vec<String>) {
        match self {
            Predicate::Fact { rhs, .. } => {
                if let Operand::Param(name) = rhs {
                    out.push(name.clone());
                }
            }
            // BOTH operands of a history atom are references an author can get
            // wrong: the window length as often as the threshold.
            Predicate::History { window, rhs, .. } => {
                if let Operand::Param(name) = &window.ticks {
                    out.push(name.clone());
                }
                if let Operand::Param(name) = rhs {
                    out.push(name.clone());
                }
            }
            Predicate::Flag { .. } | Predicate::Counter { .. } | Predicate::Bool(_) => {}
            Predicate::Not(inner) => inner.referenced_params(out),
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                a.referenced_params(out);
                b.referenced_params(out);
            }
        }
    }

    /// Collect every `history(...)` atom in the expression (issue #890).
    ///
    /// Two callers, and they need the same answer for opposite reasons.
    /// [`crate::ai::policy::AiPolicy::history_windows`] uses it to work out
    /// which windows the HOST must fold this tick; content validation uses it to
    /// reject an atom whose window is not a positive whole number of ticks, and
    /// one authored on a host or a policy shape where nothing folds it.
    pub fn referenced_history(&self, out: &mut Vec<HistoryRef>) {
        match self {
            Predicate::History {
                reducer, window, ..
            } => out.push(HistoryRef {
                reducer: *reducer,
                window: window.clone(),
            }),
            Predicate::Fact { .. }
            | Predicate::Flag { .. }
            | Predicate::Counter { .. }
            | Predicate::Bool(_) => {}
            Predicate::Not(inner) => inner.referenced_history(out),
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                a.referenced_history(out);
                b.referenced_history(out);
            }
        }
    }

    /// The first `history(...)` atom in the expression, if any (issue #890).
    ///
    /// The shorthand every rejection wants: an expression evaluated somewhere no
    /// window is folded is rejected on its first history atom, quoted verbatim.
    pub fn history_atom(&self) -> Option<HistoryRef> {
        let mut refs = Vec::new();
        self.referenced_history(&mut refs);
        refs.into_iter().next()
    }

    /// Collect every `memory(name)` referenced anywhere in the expression
    /// (issue #882). Content validation uses this to reject a memory reference
    /// the author never declared, and — together with
    /// [`references_state_time`](Self::references_state_time) — to reject any
    /// private reference inside a *stateless* policy (AC6).
    pub fn referenced_memory(&self, out: &mut Vec<String>) {
        match self {
            Predicate::Fact { context, name, .. } if *context == FactContext::Memory => {
                out.push(name.clone())
            }
            Predicate::Fact { .. }
            | Predicate::History { .. }
            | Predicate::Flag { .. }
            | Predicate::Counter { .. }
            | Predicate::Bool(_) => {}
            Predicate::Not(inner) => inner.referenced_memory(out),
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                a.referenced_memory(out);
                b.referenced_memory(out);
            }
        }
    }

    /// Collect every world-context typed fact atom referenced anywhere in the
    /// expression, as `(context, name)` pairs (issue #1210).
    ///
    /// The three world contexts — `fact(...)` / `self_fact(...)` (both
    /// [`FactContext::SelfCtx`]), `candidate_fact(...)`
    /// ([`FactContext::Candidate`]) and `target_fact(...)`
    /// ([`FactContext::Target`]) — are collected; the two PRIVATE contexts are
    /// not. `memory(...)` is validated against the policy's declared slots by
    /// [`referenced_memory`](Self::referenced_memory), and `state_time` by
    /// [`references_state_time`](Self::references_state_time); neither is a
    /// host-seeded fact. This walker exists so a host can reject a typed
    /// `fact(...)` NAME it never seeds — the unvalidated-`fact()` hole PRD #774
    /// §11 leaves open, and the sibling of
    /// [`referenced_world_state`](Self::referenced_world_state).
    pub fn referenced_facts(&self, out: &mut Vec<(FactContext, String)>) {
        match self {
            Predicate::Fact { context, name, .. }
                if matches!(
                    context,
                    FactContext::SelfCtx | FactContext::Candidate | FactContext::Target
                ) =>
            {
                out.push((*context, name.clone()))
            }
            Predicate::Fact { .. }
            | Predicate::History { .. }
            | Predicate::Flag { .. }
            | Predicate::Counter { .. }
            | Predicate::Bool(_) => {}
            Predicate::Not(inner) => inner.referenced_facts(out),
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                a.referenced_facts(out);
                b.referenced_facts(out);
            }
        }
    }

    /// Collect every world-state atom — `flag(name)` and `counter(name)` —
    /// referenced anywhere in the expression, rendered the way an author typed
    /// it (issue #891).
    ///
    /// Unlike [`referenced_params`](Self::referenced_params) and
    /// [`referenced_memory`](Self::referenced_memory), which exist to check a
    /// name against a declaration, this exists to check the atom against its
    /// HOST: `flag(...)` and `counter(...)` only ever read true where the host
    /// passes a populated flag-store chain into evaluation, and most fine-system
    /// hosts pass `&[]`. The rendered form (not the bare name) is collected so a
    /// rejection can quote the offending atom back verbatim.
    pub fn referenced_world_state(&self, out: &mut Vec<String>) {
        match self {
            Predicate::Flag { name } => out.push(format!("flag({name})")),
            Predicate::Counter { name, .. } => out.push(format!("counter({name})")),
            Predicate::Fact { .. } | Predicate::History { .. } | Predicate::Bool(_) => {}
            Predicate::Not(inner) => inner.referenced_world_state(out),
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                a.referenced_world_state(out);
                b.referenced_world_state(out);
            }
        }
    }

    /// True when the expression reads `state_time` anywhere (issue #882).
    pub fn references_state_time(&self) -> bool {
        match self {
            Predicate::Fact { context, .. } => *context == FactContext::StateTime,
            Predicate::History { .. }
            | Predicate::Flag { .. }
            | Predicate::Counter { .. }
            | Predicate::Bool(_) => false,
            Predicate::Not(inner) => inner.references_state_time(),
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                a.references_state_time() || b.references_state_time()
            }
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
    Fact,
    SelfFact,
    CandidateFact,
    TargetFact,
    /// `memory` — the owning fine system's private memory atom (issue #882).
    Memory,
    /// `state_time` — the owning fine system's state clock (issue #882). Unlike
    /// every other fact keyword this one takes NO argument list.
    StateTime,
    /// `history` — the bounded-window atom (issue #890). Takes THREE arguments
    /// (reducer, fact name, window length) where every other fact keyword takes
    /// one.
    History,
    Param,
    Bool(bool),
    Cmp(CmpOp),
    Ident(String),
    Int(i64),
    Num(f64),
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
        // Number (with optional leading - and optional fractional part). A
        // number with a decimal point tokenises as `Num(f64)`; otherwise as
        // `Int(i64)` so integer counter comparisons keep their exact typing.
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
            let mut seen_dot = false;
            for ch in self.rest().chars().skip(1) {
                if ch.is_ascii_digit() {
                    end += ch.len_utf8();
                } else if ch == '.' && !seen_dot {
                    seen_dot = true;
                    end += ch.len_utf8();
                } else {
                    break;
                }
            }
            let slice = &self.src[self.pos..end];
            self.pos = end;
            if seen_dot {
                let value: f64 = slice
                    .parse()
                    .map_err(|_| format!("Invalid number '{slice}' at position {start}"))?;
                return Ok(Some((Token::Num(value), start)));
            }
            let value: i64 = slice
                .parse()
                .map_err(|_| format!("Invalid integer '{slice}' at position {start}"))?;
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
                "fact" => Token::Fact,
                "self_fact" => Token::SelfFact,
                "candidate_fact" => Token::CandidateFact,
                "target_fact" => Token::TargetFact,
                "memory" => Token::Memory,
                "state_time" => Token::StateTime,
                "history" => Token::History,
                "param" => Token::Param,
                "true" => Token::Bool(true),
                "false" => Token::Bool(false),
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
            Some((Token::Fact, _)) => self.parse_fact_atom(FactContext::SelfCtx, "fact"),
            Some((Token::SelfFact, _)) => self.parse_fact_atom(FactContext::SelfCtx, "self_fact"),
            Some((Token::CandidateFact, _)) => {
                self.parse_fact_atom(FactContext::Candidate, "candidate_fact")
            }
            Some((Token::TargetFact, _)) => {
                self.parse_fact_atom(FactContext::Target, "target_fact")
            }
            // Private (issue #882): `memory(name) CMP operand` takes an
            // argument like the world contexts; `state_time CMP operand` is a
            // bare atom, so it skips the `(name)` step entirely.
            Some((Token::Memory, _)) => self.parse_fact_atom(FactContext::Memory, "memory"),
            Some((Token::StateTime, _)) => {
                let op = match self.bump() {
                    Some((Token::Cmp(op), _)) => op,
                    Some((t, p)) => {
                        return Err(format!(
                            "Expected comparison operator after state_time but got {t:?} at position {p}"
                        ))
                    }
                    None => {
                        return Err(
                            "Expected comparison operator after state_time but reached end of predicate"
                                .into(),
                        )
                    }
                };
                let rhs = self.parse_operand()?;
                Ok(Predicate::Fact {
                    context: FactContext::StateTime,
                    name: "state_time".to_string(),
                    op,
                    rhs,
                })
            }
            // The bounded window (issue #890): three arguments, then the same
            // `CMP operand` tail every fact atom carries.
            Some((Token::History, p)) => self.parse_history_atom(p),
            Some((Token::Bool(b), _)) => Ok(Predicate::Bool(b)),
            Some((t, p)) => Err(format!("Unexpected token {t:?} at position {p}")),
            None => Err(format!(
                "Unexpected end of predicate at position {pos}; expected an atom"
            )),
        }
    }

    /// Parse a `<kw>(name) CMP operand` atom for one of the three fact
    /// contexts (issue #776). `kw` is the keyword already consumed, used only
    /// for diagnostics so `candidate_fact(...)` errors read naturally.
    fn parse_fact_atom(&mut self, context: FactContext, kw: &str) -> Result<Predicate, String> {
        self.expect(&Token::LParen, "'(' after fact keyword")?;
        let name = self.expect_name("name inside fact(...)")?;
        self.expect(&Token::RParen, "')' to close fact(...)")?;
        let op = match self.bump() {
            Some((Token::Cmp(op), _)) => op,
            Some((t, p)) => {
                return Err(format!(
                    "Expected comparison operator after {kw}(...) but got {t:?} at position {p}"
                ))
            }
            None => {
                return Err(format!(
                    "Expected comparison operator after {kw}(...) but reached end of predicate"
                ))
            }
        };
        let rhs = self.parse_operand()?;
        Ok(Predicate::Fact {
            context,
            name,
            op,
            rhs,
        })
    }

    /// Parse a `history(REDUCER, fact_name, window) CMP operand` atom
    /// (issue #890).
    ///
    /// `at` is the position of the `history` keyword, so an unknown reducer is
    /// reported against the atom rather than against whatever token happened to
    /// follow it.
    fn parse_history_atom(&mut self, at: usize) -> Result<Predicate, String> {
        self.expect(&Token::LParen, "'(' after 'history'")?;
        let reducer_name = self.expect_name("reducer inside history(...)")?;
        let Some(reducer) = HistoryReducer::parse(&reducer_name) else {
            return Err(format!(
                "Unknown history reducer '{reducer_name}' at position {at}; valid \
                 reducers are {}",
                HISTORY_REDUCERS.join(", ")
            ));
        };
        self.expect(&Token::Comma, "',' after the history reducer")?;
        let fact = self.expect_name("fact name inside history(...)")?;
        self.expect(
            &Token::Comma,
            "',' after the history fact name (history(...) takes a reducer, a fact \
             name and a window length)",
        )?;
        let ticks = self.parse_window_length()?;
        self.expect(&Token::RParen, "')' to close history(...)")?;
        let op =
            match self.bump() {
                Some((Token::Cmp(op), _)) => op,
                Some((t, p)) => {
                    return Err(format!(
                        "Expected comparison operator after history(...) but got {t:?} at \
                     position {p}"
                    ))
                }
                None => return Err(
                    "Expected comparison operator after history(...) but reached end of predicate"
                        .into(),
                ),
            };
        let rhs = self.parse_operand()?;
        Ok(Predicate::History {
            reducer,
            window: HistoryWindow { fact, ticks },
            op,
            rhs,
        })
    }

    /// Parse the WINDOW LENGTH argument of a `history(...)` atom: a positive
    /// whole number of shared AI ticks, or a `param(name)` naming one.
    ///
    /// A literal is checked here because the parser is the only place that sees
    /// it; a `param(...)` is checked against its declared value by content
    /// validation, which is the only place that sees THAT. Between them no
    /// authored window can be fractional, zero or negative.
    fn parse_window_length(&mut self) -> Result<Operand, String> {
        match self.bump() {
            Some((Token::Int(n), p)) => {
                if n < 1 {
                    return Err(format!(
                        "history window length must be a positive whole number of shared \
                         AI ticks, got {n} at position {p}"
                    ));
                }
                Ok(Operand::Number(n as f64))
            }
            Some((Token::Num(n), p)) => Err(format!(
                "history window length must be a WHOLE number of shared AI ticks, got \
                 {n} at position {p}: the window counts ticks, it is not a duration"
            )),
            Some((Token::Param, _)) => {
                self.expect(&Token::LParen, "'(' after 'param'")?;
                let name = self.expect_name("name inside param(...)")?;
                self.expect(&Token::RParen, "')' to close param(...)")?;
                Ok(Operand::Param(name))
            }
            Some((t, p)) => Err(format!(
                "Expected a positive whole number or param(...) as the history window \
                 length but got {t:?} at position {p}"
            )),
            None => Err(
                "Expected a positive whole number or param(...) as the history window \
                 length but reached end of predicate"
                    .into(),
            ),
        }
    }

    /// Parse the right-hand side of a `fact(...) CMP` comparison: a numeric
    /// literal (`Int` or `Num`) or a `param(name)` reference.
    fn parse_operand(&mut self) -> Result<Operand, String> {
        match self.bump() {
            Some((Token::Int(n), _)) => Ok(Operand::Number(n as f64)),
            Some((Token::Num(n), _)) => Ok(Operand::Number(n)),
            Some((Token::Param, _)) => {
                self.expect(&Token::LParen, "'(' after 'param'")?;
                let name = self.expect_name("name inside param(...)")?;
                self.expect(&Token::RParen, "')' to close param(...)")?;
                Ok(Operand::Param(name))
            }
            Some((t, p)) => Err(format!(
                "Expected a number or param(...) after comparison but got {t:?} at position {p}"
            )),
            None => Err(
                "Expected a number or param(...) after comparison but reached end of predicate"
                    .into(),
            ),
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
            Some((Token::Fact, _)) => Ok("fact".to_string()),
            Some((Token::SelfFact, _)) => Ok("self_fact".to_string()),
            Some((Token::CandidateFact, _)) => Ok("candidate_fact".to_string()),
            Some((Token::TargetFact, _)) => Ok("target_fact".to_string()),
            Some((Token::Memory, _)) => Ok("memory".to_string()),
            Some((Token::StateTime, _)) => Ok("state_time".to_string()),
            Some((Token::History, _)) => Ok("history".to_string()),
            Some((Token::Param, _)) => Ok("param".to_string()),
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
#[path = "flags_tests.rs"]
mod tests;
