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

    // --- Typed AI facts + named parameters (issue #775) --------------------

    fn facts_with(pairs: &[(&str, f64)]) -> AiFacts {
        let mut f = AiFacts::new();
        for (k, v) in pairs {
            f.set(k, *v);
        }
        f
    }

    fn params_with(pairs: &[(&str, f64)]) -> AiParams {
        let mut p = AiParams::new();
        for (k, v) in pairs {
            p.set(k, *v);
        }
        p
    }

    #[test]
    fn parse_fact_atom_with_numeric_rhs() {
        let pred = parse_predicate("fact(secs_since_combat) < 10.0").unwrap();
        assert_eq!(
            pred,
            Predicate::Fact {
                context: FactContext::SelfCtx,
                name: "secs_since_combat".into(),
                op: CmpOp::Lt,
                rhs: Operand::Number(10.0),
            }
        );
    }

    #[test]
    fn parse_fact_atom_with_param_rhs() {
        let pred = parse_predicate("fact(secs_since_combat) < param(combat_window_secs)").unwrap();
        assert_eq!(
            pred,
            Predicate::Fact {
                context: FactContext::SelfCtx,
                name: "secs_since_combat".into(),
                op: CmpOp::Lt,
                rhs: Operand::Param("combat_window_secs".into()),
            }
        );
    }

    #[test]
    fn parse_true_and_false_literals() {
        assert_eq!(parse_predicate("true").unwrap(), Predicate::Bool(true));
        assert_eq!(parse_predicate("false").unwrap(), Predicate::Bool(false));
    }

    #[test]
    fn evaluate_fact_against_named_parameter_table() {
        let p = parse_predicate("fact(secs_since_combat) < param(combat_window_secs)").unwrap();
        let params = params_with(&[("combat_window_secs", 10.0)]);
        let no_flags: &[&FlagStore] = &[];
        // In combat 3s ago → within the 10s window → true.
        assert!(p.evaluate_with(
            &facts_with(&[("secs_since_combat", 3.0)]),
            &params,
            no_flags
        ));
        // 12s ago → outside the window → false.
        assert!(!p.evaluate_with(
            &facts_with(&[("secs_since_combat", 12.0)]),
            &params,
            no_flags
        ));
        // Exactly at the boundary → strict `<` is false.
        assert!(!p.evaluate_with(
            &facts_with(&[("secs_since_combat", 10.0)]),
            &params,
            no_flags
        ));
    }

    #[test]
    fn absent_fact_evaluates_false_never_panics() {
        let p = parse_predicate("fact(secs_since_combat) < param(w)").unwrap();
        let params = params_with(&[("w", 10.0)]);
        // No `secs_since_combat` reading at all → false (not in combat).
        assert!(!p.evaluate_with(&AiFacts::new(), &params, &[]));
    }

    #[test]
    fn unresolved_parameter_evaluates_false_never_panics() {
        let p = parse_predicate("fact(x) < param(missing)").unwrap();
        // Parameter `missing` is not authored → comparison is false.
        assert!(!p.evaluate_with(&facts_with(&[("x", 1.0)]), &AiParams::new(), &[]));
    }

    #[test]
    fn bool_literal_evaluates_to_itself() {
        assert!(parse_predicate("true").unwrap().evaluate_with(
            &AiFacts::new(),
            &AiParams::new(),
            &[]
        ));
        assert!(!parse_predicate("false").unwrap().evaluate_with(
            &AiFacts::new(),
            &AiParams::new(),
            &[]
        ));
    }

    #[test]
    fn facts_compose_with_flags_and_boolean_operators() {
        let s = store_with(&[("shooting", 1)]);
        let p = parse_predicate("flag(shooting) or fact(secs_since_combat) < param(w)").unwrap();
        let params = params_with(&[("w", 10.0)]);
        // flag(shooting) is true → whole OR true even with no fact.
        assert!(p.evaluate_with(&AiFacts::new(), &params, &[&s]));
    }

    #[test]
    fn world_flag_evaluate_ignores_facts() {
        // The world-trigger `evaluate` entry point still works and reads flags.
        let s = store_with(&[("a", 1)]);
        assert!(parse_predicate("flag(a)").unwrap().evaluate(&[&s]));
    }

    #[test]
    fn parse_fact_without_operand_errors_without_panic() {
        let err = parse_predicate("fact(x) <").unwrap_err();
        assert!(err.contains("end of predicate"), "got: {err}");
    }

    #[test]
    fn parse_fact_with_bad_operand_errors_without_panic() {
        let err = parse_predicate("fact(x) < flag(y)").unwrap_err();
        assert!(err.contains("number or param"), "got: {err}");
    }

    #[test]
    fn parse_integer_rhs_for_fact_is_widened() {
        // A fact comparison may use a bare integer; it widens to f64.
        let p = parse_predicate("fact(x) >= 5").unwrap();
        assert_eq!(
            p,
            Predicate::Fact {
                context: FactContext::SelfCtx,
                name: "x".into(),
                op: CmpOp::Ge,
                rhs: Operand::Number(5.0),
            }
        );
        assert!(p.evaluate_with(&facts_with(&[("x", 5.0)]), &AiParams::new(), &[]));
    }

    // --- Three-context selector grammar (issue #776) -----------------------

    fn factset(
        self_pairs: &[(&str, f64)],
        candidate_pairs: &[(&str, f64)],
        target_pairs: &[(&str, f64)],
    ) -> AiFactSet {
        AiFactSet {
            self_facts: facts_with(self_pairs),
            candidate_facts: facts_with(candidate_pairs),
            target_facts: facts_with(target_pairs),
        }
    }

    #[test]
    fn parse_bare_fact_is_self_context() {
        // #775 back-compat: bare `fact(...)` still parses as the SELF context.
        let p = parse_predicate("fact(secs_since_combat) < 10").unwrap();
        assert!(matches!(
            p,
            Predicate::Fact {
                context: FactContext::SelfCtx,
                ..
            }
        ));
    }

    #[test]
    fn parse_three_context_fact_keywords() {
        for (src, ctx) in [
            ("self_fact(power_rating) >= 5", FactContext::SelfCtx),
            ("candidate_fact(hostile) > 0", FactContext::Candidate),
            ("target_fact(distance) < 100", FactContext::Target),
        ] {
            let p = parse_predicate(src).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert!(
                matches!(p, Predicate::Fact { context, .. } if context == ctx),
                "{src} should parse in context {ctx:?}"
            );
        }
    }

    #[test]
    fn selector_reads_each_context_independently() {
        // The same fact name resolves to a different value per context.
        let p = parse_predicate("candidate_fact(dist) < self_fact(dist)");
        // `self_fact(dist)` is not a valid operand rhs — operands are numbers or
        // param(...). So compare candidate vs a param instead.
        assert!(p.is_err(), "a fact atom is not a valid comparison operand");

        let p = parse_predicate(
            "candidate_fact(hostile) > 0 and candidate_fact(dist) < target_fact(dist)",
        );
        assert!(p.is_err(), "target_fact is likewise not a valid operand");

        // The supported shape: each context compared to a literal / param.
        let p = parse_predicate(
            "self_fact(power_rating) >= param(min_rating) and candidate_fact(hostile) > 0 and target_fact(locked) > 0",
        )
        .unwrap();
        let params = params_with(&[("min_rating", 4.0)]);
        // All three contexts satisfy their clause.
        assert!(p.evaluate_selector(
            &factset(
                &[("power_rating", 5.0)],
                &[("hostile", 1.0)],
                &[("locked", 1.0)]
            ),
            &params,
            &[],
        ));
        // Candidate not hostile → whole conjunction false.
        assert!(!p.evaluate_selector(
            &factset(
                &[("power_rating", 5.0)],
                &[("hostile", 0.0)],
                &[("locked", 1.0)]
            ),
            &params,
            &[],
        ));
        // Self power rating below the authored floor → false.
        assert!(!p.evaluate_selector(
            &factset(
                &[("power_rating", 3.0)],
                &[("hostile", 1.0)],
                &[("locked", 1.0)]
            ),
            &params,
            &[],
        ));
    }

    #[test]
    fn absent_candidate_and_target_facts_evaluate_false() {
        // A candidate/target atom with no reading in that context is false,
        // never a panic — the same contract SELF facts already carry.
        let p = parse_predicate("candidate_fact(hidden) > 0").unwrap();
        assert!(!p.evaluate_selector(&AiFactSet::default(), &AiParams::new(), &[]));
        let p = parse_predicate("target_fact(anything) >= 1").unwrap();
        assert!(!p.evaluate_selector(&AiFactSet::default(), &AiParams::new(), &[]));
    }

    #[test]
    fn evaluate_with_treats_bare_fact_as_self_and_ignores_other_contexts() {
        // The #775 entry point still evaluates bare `fact(...)` as SELF and a
        // stray candidate atom (no candidate context supplied) reads false.
        let p = parse_predicate("fact(x) >= 1 and not candidate_fact(y) > 0").unwrap();
        assert!(p.evaluate_with(&facts_with(&[("x", 2.0)]), &AiParams::new(), &[]));
    }

    #[test]
    fn context_fact_keywords_usable_as_flag_names() {
        // The new keywords remain legal flag/counter names (unquoted).
        let s = store_with(&[("candidate_fact", 1)]);
        assert!(parse_predicate("flag(candidate_fact)")
            .unwrap()
            .evaluate(&[&s]));
    }

    // ── Private memory + state time atoms (issue #882) ───────────────────────

    fn memory_with(pairs: &[(&str, f64)], state_time: f64) -> AiPolicyMemory {
        let mut m = AiPolicyMemory::new();
        for (k, v) in pairs {
            m.set(k, *v);
        }
        m.set_state_time_secs(state_time);
        m
    }

    #[test]
    fn memory_atom_reads_the_owning_systems_bag() {
        let p = parse_predicate("memory(engagements) >= param(limit)").unwrap();
        let mut params = AiParams::new();
        params.set("limit", 2.0);
        assert!(p.evaluate_stateful(
            &AiFacts::new(),
            &memory_with(&[("engagements", 3.0)], 0.0),
            &params,
            &[]
        ));
        assert!(!p.evaluate_stateful(
            &AiFacts::new(),
            &memory_with(&[("engagements", 1.0)], 0.0),
            &params,
            &[]
        ));
    }

    #[test]
    fn state_time_atom_reads_the_state_clock() {
        let p = parse_predicate("state_time >= 3.0").unwrap();
        assert!(!p.evaluate_stateful(
            &AiFacts::new(),
            &memory_with(&[], 2.9),
            &AiParams::new(),
            &[]
        ));
        assert!(p.evaluate_stateful(
            &AiFacts::new(),
            &memory_with(&[], 3.0),
            &AiParams::new(),
            &[]
        ));
    }

    #[test]
    fn absent_memory_evaluates_false_and_never_panics() {
        // The absent-→-false / no-panic contract, on the private atoms too.
        let p = parse_predicate("memory(never_written) > 0").unwrap();
        assert!(!p.evaluate_stateful(
            &AiFacts::new(),
            &AiPolicyMemory::new(),
            &AiParams::new(),
            &[]
        ));
        // And through the STATELESS entry point, where no bag exists at all:
        // a stateless policy can never read private state (content validation
        // rejects that authoring outright — this is the belt to that braces).
        assert!(!p.evaluate_with(&AiFacts::new(), &AiParams::new(), &[]));
        assert!(!parse_predicate("state_time > 0").unwrap().evaluate_with(
            &AiFacts::new(),
            &AiParams::new(),
            &[]
        ));
    }

    #[test]
    fn memory_is_not_a_fact_and_a_fact_is_not_memory() {
        // The two namespaces are disjoint: seeding `x` as a world fact does not
        // satisfy `memory(x)`, and vice versa.
        let mem_pred = parse_predicate("memory(x) > 0").unwrap();
        let fact_pred = parse_predicate("fact(x) > 0").unwrap();
        let facts = facts_with(&[("x", 5.0)]);
        let memory = memory_with(&[("x", 5.0)], 0.0);
        assert!(!mem_pred.evaluate_stateful(&facts, &AiPolicyMemory::new(), &AiParams::new(), &[]));
        assert!(!fact_pred.evaluate_stateful(&AiFacts::new(), &memory, &AiParams::new(), &[]));
        assert!(mem_pred.evaluate_stateful(&AiFacts::new(), &memory, &AiParams::new(), &[]));
        assert!(fact_pred.evaluate_stateful(&facts, &AiPolicyMemory::new(), &AiParams::new(), &[]));
    }

    #[test]
    fn private_atom_references_are_reportable_for_content_validation() {
        let p = parse_predicate("memory(a) > 0 and (state_time < 5 or memory(b) == 1)").unwrap();
        let mut refs = Vec::new();
        p.referenced_memory(&mut refs);
        refs.sort();
        assert_eq!(refs, vec!["a".to_string(), "b".to_string()]);
        assert!(p.references_state_time());

        let stateless = parse_predicate("fact(x) > 0 and flag(y)").unwrap();
        let mut refs = Vec::new();
        stateless.referenced_memory(&mut refs);
        assert!(refs.is_empty());
        assert!(!stateless.references_state_time());
    }

    #[test]
    fn private_keywords_remain_usable_as_flag_and_fact_names() {
        // `memory` / `state_time` were legal identifiers before #882; a world
        // trigger or fact that used them must keep parsing.
        let s = store_with(&[("memory", 1), ("state_time", 1)]);
        assert!(parse_predicate("flag(memory)").unwrap().evaluate(&[&s]));
        assert!(parse_predicate("flag(state_time)").unwrap().evaluate(&[&s]));
        assert!(parse_predicate("fact(state_time) > 0")
            .unwrap()
            .evaluate_with(&facts_with(&[("state_time", 4.0)]), &AiParams::new(), &[]));
    }

    // ── Authored history operators (issue #890) ─────────────────────────────

    /// Drive one policy's history bag with a series of readings for one fact,
    /// one reading per shared tick, and return the bag.
    fn folded(fact: &str, ticks: usize, samples: &[Option<f64>]) -> AiPolicyMemory {
        let spec = HistorySpec {
            fact: fact.to_string(),
            ticks,
        };
        let mut memory = AiPolicyMemory::new();
        for sample in samples {
            let mut facts = AiFacts::new();
            if let Some(v) = sample {
                facts.set(fact, *v);
            }
            memory.fold_history(std::slice::from_ref(&spec), &facts);
        }
        memory
    }

    fn history_says(src: &str, memory: &AiPolicyMemory, params: &AiParams) -> bool {
        parse_predicate(src)
            .expect("history expression parses")
            .evaluate_stateful(&AiFacts::new(), memory, params, &[])
    }

    #[test]
    fn a_history_atom_parses_with_a_literal_and_with_a_param_window() {
        let literal = parse_predicate("history(min, range_to_target, 8) >= 40").unwrap();
        let Predicate::History {
            reducer,
            window,
            op,
            rhs,
        } = literal
        else {
            panic!("a history atom must parse to Predicate::History");
        };
        assert_eq!(reducer, HistoryReducer::Min);
        assert_eq!(window.fact, "range_to_target");
        assert_eq!(window.ticks, Operand::Number(8.0));
        assert_eq!(op, CmpOp::Ge);
        assert_eq!(rhs, Operand::Number(40.0));

        let authored =
            parse_predicate("history(net_change, range_to_target, param(w)) > param(gain)")
                .unwrap();
        let Predicate::History { window, rhs, .. } = authored else {
            panic!("a history atom must parse to Predicate::History");
        };
        assert_eq!(window.ticks, Operand::Param("w".into()));
        assert_eq!(rhs, Operand::Param("gain".into()));
    }

    /// AC: the THRESHOLD-OVER-WINDOW shape — "every sample in the window
    /// satisfies a predicate" — is authorable, and reads false until the window
    /// has actually been held for the authored span.
    #[test]
    fn threshold_over_window_answers_held_only_once_the_whole_window_qualifies() {
        let params = params_with(&[("standoff_ticks", 3.0), ("safe_range", 40.0)]);
        let guard = "history(min, range_to_target, param(standoff_ticks)) >= param(safe_range)";

        // Two good samples of an authored three: not held yet.
        let two = folded("range_to_target", 3, &[Some(50.0), Some(50.0)]);
        assert!(
            !history_says(guard, &two, &params),
            "a partly-filled window is not a maintained distance"
        );

        let three = folded("range_to_target", 3, &[Some(50.0); 3]);
        assert!(history_says(guard, &three, &params));

        // One breach anywhere in the window answers no...
        let breached = folded("range_to_target", 3, &[Some(50.0), Some(10.0), Some(50.0)]);
        assert!(!history_says(guard, &breached, &params));
        // ...and stops mattering once it ages out, which is the property a
        // running minimum could never give.
        let recovered = folded(
            "range_to_target",
            3,
            &[Some(50.0), Some(10.0), Some(50.0), Some(50.0), Some(50.0)],
        );
        assert!(
            history_says(guard, &recovered, &params),
            "a stale breach must age out of a bounded window"
        );
    }

    /// The equivalence that justifies expressing "has held" as a reducer plus a
    /// comparison rather than inventing a `held(...)` predicate: `min` over a
    /// full window compared `>=` IS `BoundedHistory::all_at_least`.
    #[test]
    fn min_over_a_full_window_is_exactly_all_at_least() {
        let params = params_with(&[("w", 4.0), ("t", 12.5)]);
        let guard = "history(min, x, param(w)) >= param(t)";
        for series in [
            vec![20.0, 30.0, 40.0, 50.0],
            vec![12.5, 12.5, 12.5, 12.5],
            vec![12.4, 99.0, 99.0, 99.0],
            vec![99.0, 99.0, 99.0, 12.4],
        ] {
            let mut window = BoundedHistory::new(4);
            for v in &series {
                window.push(*v);
            }
            let bag = folded("x", 4, &series.iter().map(|v| Some(*v)).collect::<Vec<_>>());
            assert_eq!(
                history_says(guard, &bag, &params),
                window.all_at_least(12.5),
                "history(min, …) >= t must agree with all_at_least(t) for {series:?}"
            );
        }
    }

    /// AC: the NET-CHANGE-OVER-WINDOW shape — "is the quantity trending" — is
    /// authorable, keeps its sign, and is absent (so `false`) until the window
    /// spans the authored length.
    #[test]
    fn net_change_over_window_answers_the_trend_with_its_sign() {
        let params = params_with(&[("w", 3.0), ("min_progress", 5.0)]);
        let opening = "history(net_change, range_to_target, param(w)) > param(min_progress)";
        let closing = "history(net_change, range_to_target, param(w)) < 0";

        let short = folded("range_to_target", 3, &[Some(10.0), Some(40.0)]);
        assert!(
            !history_says(opening, &short, &params),
            "a window shorter than the authored span measures a span nobody authored"
        );

        let opened = folded("range_to_target", 3, &[Some(10.0), Some(20.0), Some(40.0)]);
        assert!(history_says(opening, &opened, &params));
        assert!(!history_says(closing, &opened, &params));

        let closed = folded("range_to_target", 3, &[Some(40.0), Some(20.0), Some(10.0)]);
        assert!(!history_says(opening, &closed, &params));
        assert!(
            history_says(closing, &closed, &params),
            "a shrinking gap is a NEGATIVE net change, not an absolute distance"
        );
    }

    /// The shape #789 needed and could not express: two windows over the SAME
    /// reading with independent authored lengths, tuned for different questions.
    #[test]
    fn two_windows_over_one_fact_keep_independent_authored_lengths() {
        let specs = vec![
            HistorySpec {
                fact: "range".into(),
                ticks: 2,
            },
            HistorySpec {
                fact: "range".into(),
                ticks: 5,
            },
        ];
        let mut memory = AiPolicyMemory::new();
        for v in [10.0, 20.0, 30.0] {
            let mut facts = AiFacts::new();
            facts.set("range", v);
            memory.fold_history(&specs, &facts);
        }
        assert_eq!(memory.history().len(), 2, "two windows, not one shared one");
        assert_eq!(
            memory
                .history()
                .reduce(&specs[0], HistoryReducer::NetChange),
            Some(10.0),
            "the short window spans the last two readings"
        );
        assert_eq!(
            memory
                .history()
                .reduce(&specs[1], HistoryReducer::NetChange),
            None,
            "the long window is not full yet, so it has no span to measure"
        );
    }

    /// An absent reading is a HOLE, not a skipped tick: a window that closed
    /// over one would span more real time than its authored length while
    /// claiming not to.
    #[test]
    fn an_absent_reading_clears_the_window_instead_of_spanning_the_gap() {
        let params = params_with(&[("w", 3.0)]);
        let guard = "history(min, x, param(w)) >= 0";
        let spanning = folded("x", 3, &[Some(1.0), Some(1.0), None, Some(1.0)]);
        assert!(
            !history_says(guard, &spanning, &params),
            "the window must restart after a tick with no reading"
        );
        let refilled = folded("x", 3, &[Some(1.0), None, Some(1.0), Some(1.0), Some(1.0)]);
        assert!(history_says(guard, &refilled, &params));
    }

    /// No unbounded buffers: the window keeps exactly its authored capacity
    /// however long the scenario runs, and the SET of windows is the authored
    /// set rather than everything ever asked for.
    #[test]
    fn a_folded_window_never_grows_past_its_authored_capacity() {
        let spec = HistorySpec {
            fact: "x".into(),
            ticks: 4,
        };
        let mut memory = AiPolicyMemory::new();
        for n in 0..10_000 {
            let mut facts = AiFacts::new();
            facts.set("x", n as f64);
            memory.fold_history(std::slice::from_ref(&spec), &facts);
            assert!(memory.history().window(&spec).unwrap().len() <= 4);
        }
        assert_eq!(memory.history().len(), 1);

        // Re-authoring to a different set drops what is no longer asked for.
        let other = HistorySpec {
            fact: "y".into(),
            ticks: 2,
        };
        memory.fold_history(std::slice::from_ref(&other), &AiFacts::new());
        assert_eq!(memory.history().len(), 1);
        assert!(memory.history().window(&spec).is_none());
    }

    /// A history atom outside a bag that anything folds is `false`, never a
    /// panic — the same absent-reading contract `fact(...)` carries. The load
    /// error that stops content getting here lives in `entities::config` /
    /// `entities::ai_flag_hosts`.
    #[test]
    fn an_unfolded_history_atom_is_false_and_never_panics() {
        let params = params_with(&[("w", 3.0)]);
        let p = parse_predicate("history(min, x, param(w)) >= 0").unwrap();
        assert!(!p.evaluate(&[]));
        assert!(!p.evaluate_with(&AiFacts::new(), &params, &[]));
        assert!(!p.evaluate_stateful(&AiFacts::new(), &AiPolicyMemory::new(), &params, &[]));
        // An undeclared window param resolves to no window at all.
        let bag = folded("x", 3, &[Some(1.0); 3]);
        assert!(!history_says(
            "history(min, x, param(missing)) >= 0",
            &bag,
            &AiParams::new()
        ));
    }

    #[test]
    fn history_atoms_compose_with_the_rest_of_the_grammar() {
        let params = params_with(&[("w", 2.0)]);
        let bag = folded("x", 2, &[Some(9.0), Some(9.0)]);
        assert!(history_says(
            "history(min, x, param(w)) > 5 and not history(max, x, param(w)) > 100",
            &bag,
            &params
        ));
        assert!(history_says(
            "flag(nope) or history(min, x, param(w)) > 5",
            &bag,
            &params
        ));
    }

    // ── Malformed history expressions are load errors that name the problem ──

    #[test]
    fn an_unknown_reducer_is_a_parse_error_naming_the_valid_ones() {
        let err = parse_predicate("history(average, x, 4) > 0").unwrap_err();
        assert!(err.contains("average"), "{err}");
        assert!(err.contains("min") && err.contains("net_change"), "{err}");
    }

    #[test]
    fn a_history_window_must_be_a_positive_whole_number_of_ticks() {
        let fractional = parse_predicate("history(min, x, 2.5) > 0").unwrap_err();
        assert!(fractional.contains("WHOLE number"), "{fractional}");
        assert!(fractional.contains("2.5"), "{fractional}");

        for src in ["history(min, x, 0) > 0", "history(min, x, -4) > 0"] {
            let err = parse_predicate(src).unwrap_err();
            assert!(err.contains("positive whole number"), "{src}: {err}");
        }
    }

    #[test]
    fn a_malformed_history_atom_names_what_was_missing() {
        // Missing the window argument entirely.
        let err = parse_predicate("history(min, x) > 0").unwrap_err();
        assert!(err.contains("window length"), "{err}");
        // Missing the separator between reducer and fact.
        let err = parse_predicate("history(min x, 4) > 0").unwrap_err();
        assert!(err.contains("','"), "{err}");
        // No comparison at all: a window reduces to a scalar, it is not itself
        // a boolean.
        let err = parse_predicate("history(min, x, 4)").unwrap_err();
        assert!(err.contains("comparison operator"), "{err}");
        // Nothing to compare against.
        let err = parse_predicate("history(min, x, 4) >").unwrap_err();
        assert!(err.contains("reached end of predicate"), "{err}");
        // A window that is neither a literal nor a param.
        let err = parse_predicate("history(min, x, fact(y)) > 0").unwrap_err();
        assert!(err.contains("window length"), "{err}");
    }

    #[test]
    fn history_remains_usable_as_a_flag_and_fact_name() {
        // `history` was a legal identifier before #890; existing content that
        // used it as a name must keep parsing.
        let s = store_with(&[("history", 1)]);
        assert!(parse_predicate("flag(history)").unwrap().evaluate(&[&s]));
        assert!(parse_predicate("fact(history) > 0").unwrap().evaluate_with(
            &facts_with(&[("history", 1.0)]),
            &AiParams::new(),
            &[]
        ));
    }

    #[test]
    fn history_references_are_reportable_for_content_validation() {
        let p = parse_predicate(
            "history(min, a, param(w)) >= 1 and not history(net_change, b, 7) < param(t)",
        )
        .unwrap();
        let mut refs = Vec::new();
        p.referenced_history(&mut refs);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].render(), "history(min, a, param(w))");
        assert_eq!(refs[1].render(), "history(net_change, b, 7)");
        assert_eq!(
            p.history_atom().unwrap().render(),
            "history(min, a, param(w))"
        );

        // BOTH operands are declaration-checked references.
        let mut params = Vec::new();
        p.referenced_params(&mut params);
        params.sort();
        assert_eq!(params, vec!["t".to_string(), "w".to_string()]);

        // And an expression with no window reports none.
        let plain = parse_predicate("fact(x) > 0").unwrap();
        assert!(plain.history_atom().is_none());
    }

    // --- serde round-trips (issue #862) -------------------------------------
    //
    // Round-tripped through RON specifically, because that is the text format
    // `vellum-save`'s browser backend stores strings as — a value that only
    // round-trips through `serde_json` or in-memory would prove nothing about
    // the snapshot path these types actually travel.

    #[test]
    fn flag_store_round_trips_through_ron() {
        let mut store = FlagStore::new();
        store.set_flag("alpha");
        store.set_flag_value("beta", -7);
        store.increment_flag("gamma", 42);

        let text = ron::to_string(&store).expect("FlagStore should serialise");
        let restored: FlagStore = ron::from_str(&text).expect("FlagStore should parse");

        assert_eq!(restored, store);
        assert!(restored.flag("alpha"));
        assert_eq!(restored.counter("beta"), -7);
        assert_eq!(restored.counter("gamma"), 42);
    }

    #[test]
    fn ai_facts_round_trip_through_ron() {
        let mut facts = AiFacts::new();
        facts.set("secs_since_combat", 12.5);
        facts.set("power_rating", 0.75);

        let text = ron::to_string(&facts).expect("AiFacts should serialise");
        let restored: AiFacts = ron::from_str(&text).expect("AiFacts should parse");

        assert_eq!(restored, facts);
        assert_eq!(restored.get("secs_since_combat"), Some(12.5));
        assert_eq!(restored.get("power_rating"), Some(0.75));
    }

    #[test]
    fn ai_fact_set_round_trips_through_ron() {
        let mut set = AiFactSet::default();
        set.self_facts.set("hull_pct", 0.4);
        set.candidate_facts.set("range", 1200.0);
        set.target_facts.set("shield_pct", 0.9);

        let text = ron::to_string(&set).expect("AiFactSet should serialise");
        let restored: AiFactSet = ron::from_str(&text).expect("AiFactSet should parse");

        assert_eq!(restored, set);
        assert_eq!(restored.self_facts.get("hull_pct"), Some(0.4));
        assert_eq!(restored.candidate_facts.get("range"), Some(1200.0));
        assert_eq!(restored.target_facts.get("shield_pct"), Some(0.9));
    }

    /// Exercises every field, including the nested [`AiHistory`] /
    /// [`BoundedHistory`] windows the memory bag carries (issue #890), since
    /// those are the transitive serde dependency that makes `AiPolicyMemory`
    /// derivable at all.
    #[test]
    fn ai_policy_memory_round_trips_through_ron_including_history_windows() {
        let mut memory = AiPolicyMemory::new();
        memory.set("threat_level", 3.0);
        memory.set_state_time_secs(9.5);
        let specs = vec![HistorySpec {
            fact: "range_to_target".to_string(),
            ticks: 4,
        }];
        let mut facts = AiFacts::new();
        for reading in [500.0, 480.0, 460.0, 440.0] {
            facts.set("range_to_target", reading);
            memory.fold_history(&specs, &facts);
        }

        let text = ron::to_string(&memory).expect("AiPolicyMemory should serialise");
        let restored: AiPolicyMemory = ron::from_str(&text).expect("AiPolicyMemory should parse");

        assert_eq!(restored, memory);
        assert_eq!(restored.get("threat_level"), Some(3.0));
        assert_eq!(restored.state_time_secs(), 9.5);
        assert_eq!(
            restored.history().reduce(&specs[0], HistoryReducer::Min),
            Some(440.0)
        );
        assert_eq!(
            restored
                .history()
                .reduce(&specs[0], HistoryReducer::NetChange),
            Some(-60.0)
        );
    }
}
