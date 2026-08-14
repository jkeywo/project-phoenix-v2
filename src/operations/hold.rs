//! Pure, Bevy-free **external operations**: eligibility and the timed hold
//! (issue #1026, Falling Skyway foundation).
//!
//! An *operation* is a verb a crewed ship performs on something outside its own
//! hull — stabilise a failing skyhook, tow a crippled freighter, escort a
//! convoy. Per the PRD there are **no minigames**: an operation is
//!
//! * **eligibility** — is the ship close enough, does the hull carry the
//!   capability, is there power for it — and
//! * a **timed hold** the crew maintain through the consoles they already have.
//!   Helm holds station inside the authored range, engineering keeps the power
//!   group allocated. Nothing new is asked of the crew; the operation simply
//!   counts the ticks during which they managed it.
//!
//! This module owns both halves and nothing else. It reaches for no ECS, no
//! wire type and no world flag store: eligibility is a function of a plain
//! [`OperationConditions`] struct the adapter gathers, and the hold is an
//! integer tick counter. Its Bevy adapter is the sibling
//! [`crate::operations::server`].
//!
//! # Stall versus fail (AC3)
//!
//! Losing eligibility mid-hold does not end the operation. Progress **stalls**:
//! it stops advancing and it does not decay, so a helm that drifts out of range
//! and comes back resumes where it left off rather than starting again. Two
//! things end a stalled hold instead:
//!
//! * an **unrecoverable** loss — the target is gone, or the hull never had the
//!   capability — which fails it immediately, because no amount of waiting
//!   fixes it ([`Ineligibility::recoverable`]);
//! * an authored **stall budget** (`stall_limit_secs`), counted **cumulatively**
//!   across the whole hold rather than per stalled run. A crew that keeps
//!   drifting out of range spends the same budget as one that drifts out once
//!   and stays there, which is the honest reading of "the skyhook was left
//!   unattended for two minutes". Authoring no budget lets a hold stall
//!   indefinitely.
//!
//! The wider interruption vocabulary — hazard bands, being fired upon — is the
//! next slice's; everything here follows from eligibility alone, which is what
//! the issue scopes this slice to.

use serde::{Deserialize, Serialize};

// ── The verb vocabulary ──────────────────────────────────────────────────────

/// One external operation verb.
///
/// Ops slices split by **verb, not by layer**: `stabilise` ships through every
/// layer first so the verbs after it — `tow`, `escort`, `transfer`,
/// `field_repair` — are authoring rather than architecture. Each new one is a
/// variant here plus a capability block in a hull's TOML.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationVerb {
    /// Hold station on a degrading structure and arrest its decline. Completing
    /// one raises the target's infrastructure condition by the authored
    /// `condition_on_complete`.
    #[default]
    Stabilise,
}

impl OperationVerb {
    /// Every verb, in declaration order. The authoring lint and the console's
    /// capability list walk this rather than restating the set.
    pub const ALL: &'static [OperationVerb] = &[OperationVerb::Stabilise];

    /// The authored/wire spelling — the same string the TOML `verb` field and
    /// the script effect use.
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationVerb::Stabilise => "stabilise",
        }
    }

    /// Parse the authored spelling. `None` for anything not in [`Self::ALL`],
    /// so a typo in a script effect is a named refusal rather than a silent
    /// no-op.
    pub fn parse(text: &str) -> Option<OperationVerb> {
        OperationVerb::ALL
            .iter()
            .copied()
            .find(|verb| verb.as_str() == text)
    }
}

// ── Authored TOML shape ──────────────────────────────────────────────────────

/// Default operating range, in world units, for a capability that does not
/// author one.
///
/// A TOML-parse fallback, the only kind of hardcoded gameplay value AGENTS.md
/// #11 sanctions. Comfortably outside a station's collider and comfortably
/// inside sensor range, so an unauthored capability is station-keeping work
/// rather than a docking manoeuvre.
fn default_range() -> f32 {
    400.0
}

/// Default hold duration in whole seconds.
fn default_duration_secs() -> i64 {
    20
}

/// Default power group an operation draws on.
///
/// `helm`, because every operation in the vocabulary is the ship holding a
/// position it would otherwise not hold. A verb that draws on something else
/// authors it.
fn default_power_group() -> String {
    "helm".to_string()
}

/// Default minimum allocation level for the operation's power group.
///
/// `2` — one step above the idle floor, so an engineering team that has stripped
/// helm to keep the shields up has visibly taken the operation with it.
fn default_min_power_level() -> u8 {
    2
}

/// The `[operations]` table on a hull's entity TOML.
///
/// Absent for every hull that performs no external operations, which is every
/// hull shipped before this existed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationsConfig {
    /// The verbs this hull can perform, in authored order.
    #[serde(default, rename = "capability", skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<CapabilityConfig>,
}

/// One `[[operations.capability]]` block: a verb this hull can perform, and the
/// terms it performs it on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityConfig {
    /// The verb this block authorises.
    pub verb: OperationVerb,
    /// How far from the target the ship may be, in world units, and still
    /// count the tick. Measured centre to centre, the same way comms range is.
    #[serde(default = "default_range")]
    pub range: f32,
    /// How long the hold runs for, in whole seconds of *eligible* time. Stalled
    /// ticks do not count towards it — that is the whole point of the hold.
    ///
    /// Whole seconds rather than a float because every authored delay in this
    /// codebase is (`no_float`); the conversion to ticks happens once, at
    /// [`OperationHold::start`], against the world's own `sim_tick_hz`.
    #[serde(default = "default_duration_secs")]
    pub duration_secs: i64,
    /// The power group the operation draws on.
    #[serde(default = "default_power_group")]
    pub power_group: String,
    /// The minimum allocation level that group must hold for the tick to count.
    /// A whole level rather than a fraction, because that is the unit the power
    /// grid and its console are authored in.
    #[serde(default = "default_min_power_level")]
    pub min_power_level: u8,
    /// Infrastructure condition points the target gains when the hold completes.
    ///
    /// Paid **once, on completion**, not sliced per tick: a stabilise either
    /// held long enough to arrest the decline or it did not, and a half-finished
    /// one leaving half the points behind would make abandoning the op a viable
    /// strategy. (The per-tick slice is `field_repair`'s shape, in its own
    /// slice.) `0.0` authors an operation whose payoff is entirely scripted.
    #[serde(default)]
    pub condition_on_complete: f32,
    /// Whole seconds of **cumulative** stalled time this hold tolerates before
    /// it fails. `None` (the default) lets it stall indefinitely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stall_limit_secs: Option<i64>,
}

impl Default for CapabilityConfig {
    /// Hand-written so it calls the same `default_*` fns serde does — two copies
    /// of these numbers could only ever drift apart.
    fn default() -> Self {
        Self {
            verb: OperationVerb::default(),
            range: default_range(),
            duration_secs: default_duration_secs(),
            power_group: default_power_group(),
            min_power_level: default_min_power_level(),
            condition_on_complete: 0.0,
            stall_limit_secs: None,
        }
    }
}

impl OperationsConfig {
    /// The authored terms for `verb`, or `None` when this hull cannot perform
    /// it. The whole of "which ships can stabilise" is this lookup returning
    /// `Some`.
    pub fn capability(&self, verb: OperationVerb) -> Option<&CapabilityConfig> {
        self.capabilities.iter().find(|c| c.verb == verb)
    }

    /// Reject an `[operations]` table that cannot mean anything.
    ///
    /// Called at entity-config parse time so a typo is a load error naming the
    /// field, not a hull whose operations silently never complete.
    pub fn validate(&self) -> Result<(), String> {
        for (index, capability) in self.capabilities.iter().enumerate() {
            let verb = capability.verb.as_str();
            if !capability.range.is_finite() || capability.range <= 0.0 {
                return Err(format!(
                    "[[operations.capability]] {verb} range must be a positive finite number of \
                     world units, got {}",
                    capability.range
                ));
            }
            if capability.duration_secs <= 0 {
                return Err(format!(
                    "[[operations.capability]] {verb} duration_secs must be a positive whole \
                     number of seconds, got {} — an operation that takes no time is not an \
                     operation",
                    capability.duration_secs
                ));
            }
            if capability.power_group.trim().is_empty() {
                return Err(format!(
                    "[[operations.capability]] {verb} needs a non-empty power_group"
                ));
            }
            if !capability.condition_on_complete.is_finite()
                || capability.condition_on_complete < 0.0
            {
                return Err(format!(
                    "[[operations.capability]] {verb} condition_on_complete must be a \
                     non-negative finite number of condition points, got {} — an operation that \
                     degrades its target is authored as a hazard, not as a capability",
                    capability.condition_on_complete
                ));
            }
            if let Some(limit) = capability.stall_limit_secs {
                if limit < 0 {
                    return Err(format!(
                        "[[operations.capability]] {verb} stall_limit_secs must be a \
                         non-negative whole number of seconds, got {limit} — omit it to allow an \
                         indefinite stall"
                    ));
                }
            }
            if self.capabilities[..index]
                .iter()
                .any(|c| c.verb == capability.verb)
            {
                return Err(format!(
                    "[[operations.capability]] verb {verb} is declared twice on one hull — a \
                     start would get whichever came first"
                ));
            }
        }
        Ok(())
    }
}

// ── Eligibility ──────────────────────────────────────────────────────────────

/// Everything the eligibility test reads, gathered by the adapter each tick.
///
/// Plain data with no ECS in it, so the whole verdict is unit-testable without
/// booting an app — the adapter's only job is to fill this in honestly and
/// apply what comes back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OperationConditions {
    /// Whether the target entity still exists.
    pub target_present: bool,
    /// Whether the target is a thing this verb can be performed *on* — for
    /// `stabilise`, whether it carries an infrastructure condition track. A
    /// pristine asteroid is present and in range and still not stabilisable.
    pub target_applicable: bool,
    /// Centre-to-centre distance from the ship to the target, in world units.
    pub distance: f32,
    /// The ship's current allocation level for the capability's power group.
    pub power_level: u8,
    /// Whether the ship's power grid is under an exhaustion lock. A locked grid
    /// reads as insufficient however the levels are set, because the levels are
    /// not what the grid is delivering.
    pub power_locked: bool,
}

/// Why an operation may not run right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ineligibility {
    /// This hull authored no capability for the verb.
    NotCapable,
    /// The target entity is gone.
    TargetGone,
    /// The target exists but the verb means nothing on it.
    TargetNotApplicable,
    /// The ship is further from the target than the capability's range.
    OutOfRange,
    /// The capability's power group is below its authored level, or the grid is
    /// exhaustion-locked.
    InsufficientPower,
}

impl Ineligibility {
    /// The `strings.csv` id the console displays. No English crosses the wire
    /// (AGENTS.md rule 11) — the client resolves this through `t()`.
    pub fn string_id(&self) -> &'static str {
        match self {
            Ineligibility::NotCapable => "operation.refused.not_capable",
            Ineligibility::TargetGone => "operation.refused.target_gone",
            Ineligibility::TargetNotApplicable => "operation.refused.target_not_applicable",
            Ineligibility::OutOfRange => "operation.refused.out_of_range",
            Ineligibility::InsufficientPower => "operation.refused.insufficient_power",
        }
    }

    /// The stable wire spelling, for the blackboard's reason field.
    pub fn as_str(&self) -> &'static str {
        match self {
            Ineligibility::NotCapable => "not_capable",
            Ineligibility::TargetGone => "target_gone",
            Ineligibility::TargetNotApplicable => "target_not_applicable",
            Ineligibility::OutOfRange => "out_of_range",
            Ineligibility::InsufficientPower => "insufficient_power",
        }
    }

    /// Whether a hold that hit this can go on to recover from it.
    ///
    /// Range and power are the two the crew are *for*: helm flies back,
    /// engineering reallocates, and the hold resumes. The other three are facts
    /// about the hull or about the target that no console can change, so a hold
    /// that meets one is over rather than waiting.
    pub fn recoverable(&self) -> bool {
        matches!(
            self,
            Ineligibility::OutOfRange | Ineligibility::InsufficientPower
        )
    }
}

/// Decide whether an operation may run this tick.
///
/// The order the checks run in is the order a console should report them: the
/// hull's capability first (a fact about the ship, true or false before the
/// mission started), then the target, then the two things the crew can actually
/// do something about. A ship that is out of range *and* under-powered reports
/// the range, because that is the one helm can fix without asking anyone.
///
/// A non-finite distance fails as [`Ineligibility::OutOfRange`] rather than
/// passing: the comparison is written so that `NaN` takes the failing arm.
pub fn eligibility(
    capability: Option<&CapabilityConfig>,
    conditions: &OperationConditions,
) -> Result<(), Ineligibility> {
    let Some(capability) = capability else {
        return Err(Ineligibility::NotCapable);
    };
    if !conditions.target_present {
        return Err(Ineligibility::TargetGone);
    }
    if !conditions.target_applicable {
        return Err(Ineligibility::TargetNotApplicable);
    }
    // Fail-closed on an incomparable distance: a NaN reading refuses rather
    // than passes, which is why this is spelled via partial_cmp instead of
    // `distance > range` (that flips the NaN outcome to "in range").
    if !matches!(
        conditions.distance.partial_cmp(&capability.range),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    ) {
        return Err(Ineligibility::OutOfRange);
    }
    if conditions.power_locked || conditions.power_level < capability.min_power_level {
        return Err(Ineligibility::InsufficientPower);
    }
    Ok(())
}

// ── The timed hold ───────────────────────────────────────────────────────────

/// What a hold is doing right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldState {
    /// Eligibility holds; progress advanced this tick.
    #[default]
    Holding,
    /// Eligibility lapsed for a recoverable reason; progress is frozen where it
    /// stood, and the stall budget is being spent.
    Stalled(Ineligibility),
    /// The hold ran to term. Terminal.
    Completed,
    /// A console called it off. Terminal.
    Aborted,
    /// Eligibility was lost unrecoverably, or the stall budget ran out.
    /// Terminal.
    Failed(Ineligibility),
}

impl HoldState {
    /// The stable wire spelling of the state itself, without its reason.
    pub fn as_str(&self) -> &'static str {
        match self {
            HoldState::Holding => "holding",
            HoldState::Stalled(_) => "stalled",
            HoldState::Completed => "completed",
            HoldState::Aborted => "aborted",
            HoldState::Failed(_) => "failed",
        }
    }

    /// The reason attached to this state, when it has one.
    pub fn reason(&self) -> Option<Ineligibility> {
        match self {
            HoldState::Stalled(reason) | HoldState::Failed(reason) => Some(*reason),
            _ => None,
        }
    }

    /// Whether the hold is over. A settled hold ignores every further tick.
    pub fn is_settled(&self) -> bool {
        matches!(
            self,
            HoldState::Completed | HoldState::Aborted | HoldState::Failed(_)
        )
    }
}

/// What a tick's [`OperationHold::advance`] settled, reported **exactly once**.
///
/// The adapter pays a completed operation's condition points off this, so
/// returning it twice would pay twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settlement {
    /// The hold ran to term.
    Completed,
    /// The hold ended without completing, for this reason.
    Failed(Ineligibility),
    /// A console called the hold off.
    Aborted,
}

/// One operation a ship is running.
///
/// Every field is private: progress is only correct when it moves through
/// [`Self::advance`], and a caller that could write `elapsed_ticks` directly
/// could complete an operation the crew never held.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationHold {
    id: u64,
    verb: OperationVerb,
    target_uuid: String,
    required_ticks: u64,
    stall_limit_ticks: Option<u64>,
    elapsed_ticks: u64,
    stalled_ticks: u64,
    condition_on_complete: f32,
    state: HoldState,
}

impl OperationHold {
    /// Open a hold against `capability`, converting its authored whole seconds
    /// into logical ticks once, here, at the boundary — the same rule
    /// `ctx.schedule.in_seconds` follows. Ticks are what is persisted and
    /// compared thereafter.
    ///
    /// A duration that rounds to zero ticks (a sub-tick `sim_tick_hz`) still
    /// takes one tick: an operation that completes on the tick it started would
    /// never be visible to the crew, and `validate` has already refused a
    /// non-positive `duration_secs`.
    pub fn start(
        id: u64,
        target_uuid: impl Into<String>,
        capability: &CapabilityConfig,
        tick_hz: f32,
    ) -> Self {
        let required_ticks =
            crate::world::script::schedule::seconds_to_ticks(capability.duration_secs, tick_hz)
                .max(1);
        Self {
            id,
            verb: capability.verb,
            target_uuid: target_uuid.into(),
            required_ticks,
            stall_limit_ticks: capability
                .stall_limit_secs
                .map(|secs| crate::world::script::schedule::seconds_to_ticks(secs, tick_hz)),
            elapsed_ticks: 0,
            stalled_ticks: 0,
            condition_on_complete: capability.condition_on_complete,
            state: HoldState::Holding,
        }
    }

    /// Advance the hold by one logical tick against this tick's verdict.
    ///
    /// Returns `Some` on the tick the hold settles and `None` on every other,
    /// including every tick after it settled — so an adapter that pays a
    /// completion off the return value pays it once.
    pub fn advance(&mut self, verdict: Result<(), Ineligibility>) -> Option<Settlement> {
        if self.state.is_settled() {
            return None;
        }
        match verdict {
            Ok(()) => {
                self.state = HoldState::Holding;
                self.elapsed_ticks = self.elapsed_ticks.saturating_add(1);
                if self.elapsed_ticks >= self.required_ticks {
                    self.elapsed_ticks = self.required_ticks;
                    self.state = HoldState::Completed;
                    return Some(Settlement::Completed);
                }
                None
            }
            Err(reason) if !reason.recoverable() => {
                self.state = HoldState::Failed(reason);
                Some(Settlement::Failed(reason))
            }
            Err(reason) => {
                // Progress freezes rather than decaying: the crew that flies
                // back gets the ticks they already earned.
                self.state = HoldState::Stalled(reason);
                self.stalled_ticks = self.stalled_ticks.saturating_add(1);
                match self.stall_limit_ticks {
                    Some(limit) if self.stalled_ticks > limit => {
                        self.state = HoldState::Failed(reason);
                        Some(Settlement::Failed(reason))
                    }
                    _ => None,
                }
            }
        }
    }

    /// Call the hold off. Returns `Some(Settlement::Aborted)` only when it was
    /// still live, so a second abort of the same operation is a no-op rather
    /// than a second wire event.
    pub fn abort(&mut self) -> Option<Settlement> {
        if self.state.is_settled() {
            return None;
        }
        self.state = HoldState::Aborted;
        Some(Settlement::Aborted)
    }

    /// Progress through the hold, `0.0..=1.0`.
    ///
    /// Reported off eligible ticks only, so a stalled operation's bar sits
    /// still — which is the readout the crew need in order to notice.
    pub fn progress(&self) -> f32 {
        if self.required_ticks == 0 {
            return 1.0;
        }
        (self.elapsed_ticks as f32 / self.required_ticks as f32).clamp(0.0, 1.0)
    }

    /// This operation's id, unique within the ship's run.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The verb being performed.
    pub fn verb(&self) -> OperationVerb {
        self.verb
    }

    /// The target entity's UUID.
    pub fn target_uuid(&self) -> &str {
        &self.target_uuid
    }

    /// What the hold is doing right now.
    pub fn state(&self) -> HoldState {
        self.state
    }

    /// Whether the hold is over.
    pub fn is_settled(&self) -> bool {
        self.state.is_settled()
    }

    /// Condition points owed to the target on completion.
    pub fn condition_on_complete(&self) -> f32 {
        self.condition_on_complete
    }

    /// Eligible ticks banked so far.
    pub fn elapsed_ticks(&self) -> u64 {
        self.elapsed_ticks
    }

    /// Eligible ticks the hold needs in total.
    pub fn required_ticks(&self) -> u64 {
        self.required_ticks
    }

    /// Stalled ticks spent so far, cumulatively across the whole hold.
    pub fn stalled_ticks(&self) -> u64 {
        self.stalled_ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 60 Hz, the default `sim_tick_hz`.
    const HZ: f32 = 60.0;

    /// A stabilise capability: 400 m, five seconds, helm at level 2 or better,
    /// paying 30 condition points.
    fn stabilise() -> CapabilityConfig {
        CapabilityConfig {
            verb: OperationVerb::Stabilise,
            range: 400.0,
            duration_secs: 5,
            power_group: "helm".to_string(),
            min_power_level: 2,
            condition_on_complete: 30.0,
            stall_limit_secs: None,
        }
    }

    /// Conditions under which the operation is eligible.
    fn eligible() -> OperationConditions {
        OperationConditions {
            target_present: true,
            target_applicable: true,
            distance: 100.0,
            power_level: 3,
            power_locked: false,
        }
    }

    fn fresh_hold() -> OperationHold {
        OperationHold::start(1, "depot-1", &stabilise(), HZ)
    }

    /// Run `ticks` eligible ticks, returning the settlement if one happened.
    fn run_eligible(hold: &mut OperationHold, ticks: u64) -> Option<Settlement> {
        let mut settled = None;
        for _ in 0..ticks {
            settled = settled.or(hold.advance(Ok(())));
        }
        settled
    }

    // ── AC1: capability is authored per hull, and its absence has a reason ──

    #[test]
    fn a_hull_that_authored_no_capability_for_the_verb_is_refused_by_name() {
        let empty = OperationsConfig::default();
        assert!(
            empty.capability(OperationVerb::Stabilise).is_none(),
            "a hull with no [operations] table can perform nothing"
        );
        assert_eq!(
            eligibility(empty.capability(OperationVerb::Stabilise), &eligible()),
            Err(Ineligibility::NotCapable),
            "…and the refusal names the reason rather than reading as a generic failure — the \
             console has to be able to tell the crew WHY"
        );
        assert_eq!(
            Ineligibility::NotCapable.string_id(),
            "operation.refused.not_capable",
            "and that reason is a strings.csv id, because no English crosses the wire"
        );
    }

    #[test]
    fn an_authored_capability_is_found_by_its_verb_and_carries_its_own_terms() {
        let config = OperationsConfig {
            capabilities: vec![stabilise()],
        };
        let found = config
            .capability(OperationVerb::Stabilise)
            .expect("the authored verb resolves");
        assert_eq!(found.range, 400.0);
        assert_eq!(
            found.duration_secs, 5,
            "the terms come off the hull's own block, so two hulls can stabilise at different \
             ranges and speeds without either number living in Rust"
        );
    }

    #[test]
    fn every_verb_round_trips_through_its_authored_spelling() {
        for verb in OperationVerb::ALL {
            assert_eq!(
                OperationVerb::parse(verb.as_str()),
                Some(*verb),
                "{} must parse back to itself — the TOML `verb` field, the script effect and \
                 the wire all use this one spelling",
                verb.as_str()
            );
        }
        assert_eq!(
            OperationVerb::parse("stabilize"),
            None,
            "a near-miss spelling is a refusal, not a silent no-op"
        );
    }

    // ── AC2: eligibility is pure — proximity, capability, power ──

    #[test]
    fn a_capable_ship_in_range_with_power_is_eligible() {
        assert_eq!(
            eligibility(Some(&stabilise()), &eligible()),
            Ok(()),
            "the three conditions the issue names, all met"
        );
    }

    #[test]
    fn eligibility_ends_exactly_at_the_authored_range_and_not_a_metre_past_it() {
        let capability = stabilise();
        for (distance, expected) in [
            (399.9_f32, Ok(())),
            (400.0, Ok(())),
            (400.1, Err(Ineligibility::OutOfRange)),
        ] {
            let conditions = OperationConditions {
                distance,
                ..eligible()
            };
            assert_eq!(
                eligibility(Some(&capability), &conditions),
                expected,
                "at {distance} m against an authored 400 m range: the boundary is inclusive, so \
                 a ship parked exactly on it is working"
            );
        }
    }

    #[test]
    fn a_non_finite_distance_is_out_of_range_rather_than_silently_eligible() {
        for distance in [f32::NAN, f32::INFINITY] {
            let conditions = OperationConditions {
                distance,
                ..eligible()
            };
            assert_eq!(
                eligibility(Some(&stabilise()), &conditions),
                Err(Ineligibility::OutOfRange),
                "a {distance} distance must take the failing arm — a naive `>` comparison lets \
                 NaN through, which would run an operation against a target nobody can locate"
            );
        }
    }

    #[test]
    fn power_below_the_authored_level_or_a_locked_grid_both_read_as_insufficient() {
        let capability = stabilise();
        let under = OperationConditions {
            power_level: 1,
            ..eligible()
        };
        assert_eq!(
            eligibility(Some(&capability), &under),
            Err(Ineligibility::InsufficientPower),
            "one level below the authored minimum is not enough"
        );
        let locked = OperationConditions {
            power_locked: true,
            ..eligible()
        };
        assert_eq!(
            eligibility(Some(&capability), &locked),
            Err(Ineligibility::InsufficientPower),
            "…and an exhaustion-locked grid is insufficient however the levels are SET, because \
             the levels are not what the grid is delivering"
        );
    }

    #[test]
    fn a_missing_target_and_an_inapplicable_one_are_different_refusals() {
        let gone = OperationConditions {
            target_present: false,
            ..eligible()
        };
        assert_eq!(
            eligibility(Some(&stabilise()), &gone),
            Err(Ineligibility::TargetGone)
        );
        let rock = OperationConditions {
            target_applicable: false,
            ..eligible()
        };
        assert_eq!(
            eligibility(Some(&stabilise()), &rock),
            Err(Ineligibility::TargetNotApplicable),
            "a target that is present, in range and simply not a thing you can stabilise gets \
             its own reason — 'target gone' would send helm looking for it"
        );
    }

    #[test]
    fn the_reported_reason_is_the_one_the_crew_can_act_on_first() {
        // Out of range AND under-powered AND the hull is capable: helm can fix
        // the range without asking anyone, so that is the reason shown.
        let conditions = OperationConditions {
            distance: 5_000.0,
            power_level: 0,
            ..eligible()
        };
        assert_eq!(
            eligibility(Some(&stabilise()), &conditions),
            Err(Ineligibility::OutOfRange),
            "with several conditions unmet the console shows the most actionable one, not \
             whichever the implementation happened to check last"
        );
    }

    // ── AC3: the timed hold — progress, stall, fail ──

    #[test]
    fn progress_advances_one_eligible_tick_at_a_time_and_completes_on_the_authored_duration() {
        let mut hold = fresh_hold();
        assert_eq!(
            hold.required_ticks(),
            300,
            "five authored seconds at 60 Hz is 300 logical ticks — the conversion happens once, \
             at the boundary, and ticks are what is compared thereafter"
        );
        assert_eq!(hold.progress(), 0.0, "a fresh hold has banked nothing");

        assert_eq!(
            run_eligible(&mut hold, 150),
            None,
            "half way is not settled"
        );
        assert!(
            (hold.progress() - 0.5).abs() < 1e-6,
            "…and reads as half done: {}",
            hold.progress()
        );

        assert_eq!(
            run_eligible(&mut hold, 150),
            Some(Settlement::Completed),
            "the 300th eligible tick completes it"
        );
        assert_eq!(hold.state(), HoldState::Completed);
        assert_eq!(hold.progress(), 1.0);
    }

    #[test]
    fn a_completion_settles_exactly_once_however_many_ticks_follow_it() {
        // The adapter pays the target's condition points off this return value,
        // so a second Some would pay a second time.
        let mut hold = fresh_hold();
        assert_eq!(run_eligible(&mut hold, 300), Some(Settlement::Completed));
        for _ in 0..10 {
            assert_eq!(
                hold.advance(Ok(())),
                None,
                "a settled hold reports nothing further — an adapter paying a completion off \
                 this return value must not pay twice"
            );
        }
        assert_eq!(hold.progress(), 1.0, "and does not run past its own term");
    }

    #[test]
    fn a_recoverable_loss_stalls_the_hold_and_freezes_progress_where_it_stood() {
        let mut hold = fresh_hold();
        run_eligible(&mut hold, 100);
        let banked = hold.progress();

        for _ in 0..500 {
            assert_eq!(
                hold.advance(Err(Ineligibility::OutOfRange)),
                None,
                "drifting out of range does not end the operation — it is exactly the thing helm \
                 is there to fix"
            );
        }
        assert_eq!(hold.state(), HoldState::Stalled(Ineligibility::OutOfRange));
        assert_eq!(
            hold.progress(),
            banked,
            "…and the ticks already held are not lost. Progress that decayed would make a brief \
             drift as expensive as never having started."
        );
        assert_eq!(hold.elapsed_ticks(), 100);
        assert_eq!(
            hold.stalled_ticks(),
            500,
            "the stall is counted even where no budget is authored, so the readout can show it"
        );
    }

    #[test]
    fn a_stalled_hold_resumes_from_where_it_stopped_and_still_completes() {
        let mut hold = fresh_hold();
        run_eligible(&mut hold, 200);
        for _ in 0..1_000 {
            hold.advance(Err(Ineligibility::InsufficientPower));
        }
        assert_eq!(
            run_eligible(&mut hold, 100),
            Some(Settlement::Completed),
            "100 more eligible ticks after a long stall is all it takes: the hold needs 300 \
             ELIGIBLE ticks, not 300 ticks of wall clock"
        );
    }

    #[test]
    fn an_unrecoverable_loss_fails_the_hold_immediately() {
        for reason in [
            Ineligibility::TargetGone,
            Ineligibility::TargetNotApplicable,
            Ineligibility::NotCapable,
        ] {
            let mut hold = fresh_hold();
            run_eligible(&mut hold, 100);
            assert_eq!(
                hold.advance(Err(reason)),
                Some(Settlement::Failed(reason)),
                "{reason:?} cannot be waited out, so the hold ends on the tick it is seen rather \
                 than stalling forever against a target that is not coming back"
            );
            assert_eq!(hold.state(), HoldState::Failed(reason));
            assert_eq!(
                hold.advance(Err(reason)),
                None,
                "…and settles once, like every other terminal state"
            );
        }
    }

    #[test]
    fn an_authored_stall_budget_fails_the_hold_once_it_is_spent() {
        let capability = CapabilityConfig {
            stall_limit_secs: Some(2),
            ..stabilise()
        };
        let mut hold = OperationHold::start(1, "depot-1", &capability, HZ);
        run_eligible(&mut hold, 10);
        for tick in 0..120 {
            assert_eq!(
                hold.advance(Err(Ineligibility::OutOfRange)),
                None,
                "stalled tick {tick} is inside the two-second budget"
            );
        }
        assert_eq!(
            hold.advance(Err(Ineligibility::OutOfRange)),
            Some(Settlement::Failed(Ineligibility::OutOfRange)),
            "the 121st stalled tick is one past two seconds at 60 Hz, and the hold fails with \
             the reason it was stalled for — not with a generic timeout the crew cannot act on"
        );
    }

    #[test]
    fn the_stall_budget_is_cumulative_rather_than_per_stall() {
        // Two separate 1.5-second drifts spend the same budget as one 3-second
        // drift. A crew that keeps wandering off has left the skyhook alone for
        // three seconds either way.
        let capability = CapabilityConfig {
            stall_limit_secs: Some(2),
            ..stabilise()
        };
        let mut hold = OperationHold::start(1, "depot-1", &capability, HZ);
        for _ in 0..90 {
            hold.advance(Err(Ineligibility::OutOfRange));
        }
        run_eligible(&mut hold, 60);
        assert!(
            !hold.is_settled(),
            "precondition: 90 stalled ticks is inside the 120-tick budget, and recovering did \
             not settle it"
        );
        let mut settlement = None;
        for _ in 0..90 {
            settlement = settlement.or(hold.advance(Err(Ineligibility::OutOfRange)));
        }
        assert_eq!(
            settlement,
            Some(Settlement::Failed(Ineligibility::OutOfRange)),
            "the second drift spends the REST of the budget rather than a fresh one — a counter \
             that reset on recovery would let a crew stall indefinitely in 1.9-second bursts"
        );
    }

    #[test]
    fn a_zero_second_stall_budget_fails_on_the_first_stalled_tick() {
        let capability = CapabilityConfig {
            stall_limit_secs: Some(0),
            ..stabilise()
        };
        let mut hold = OperationHold::start(1, "depot-1", &capability, HZ);
        run_eligible(&mut hold, 10);
        assert_eq!(
            hold.advance(Err(Ineligibility::OutOfRange)),
            Some(Settlement::Failed(Ineligibility::OutOfRange)),
            "an operation authored to tolerate no interruption at all is legal, and means what \
             it says"
        );
    }

    // ── AC4: abort ──

    #[test]
    fn an_abort_settles_a_live_hold_once_and_leaves_a_settled_one_alone() {
        let mut hold = fresh_hold();
        run_eligible(&mut hold, 100);
        assert_eq!(
            hold.abort(),
            Some(Settlement::Aborted),
            "a console calling the operation off settles it"
        );
        assert_eq!(hold.state(), HoldState::Aborted);
        assert_eq!(
            hold.abort(),
            None,
            "…and a second abort is a no-op rather than a second wire event"
        );

        let mut completed = fresh_hold();
        run_eligible(&mut completed, 300);
        assert_eq!(
            completed.abort(),
            None,
            "aborting a finished operation cannot un-finish it"
        );
        assert_eq!(completed.state(), HoldState::Completed);
    }

    #[test]
    fn an_aborted_hold_stops_advancing() {
        let mut hold = fresh_hold();
        run_eligible(&mut hold, 100);
        hold.abort();
        assert_eq!(run_eligible(&mut hold, 1_000), None);
        assert_eq!(
            hold.elapsed_ticks(),
            100,
            "an aborted operation banks nothing further, whatever the ship does afterwards"
        );
    }

    // ── The wire spellings the blackboard carries ──

    #[test]
    fn every_state_and_reason_has_a_stable_wire_spelling() {
        assert_eq!(HoldState::Holding.as_str(), "holding");
        assert_eq!(
            HoldState::Stalled(Ineligibility::OutOfRange).as_str(),
            "stalled",
            "the state and its reason are separate fields on the wire, so the state string \
             carries no reason in it"
        );
        assert_eq!(
            HoldState::Stalled(Ineligibility::OutOfRange).reason(),
            Some(Ineligibility::OutOfRange)
        );
        assert_eq!(HoldState::Completed.as_str(), "completed");
        assert_eq!(HoldState::Aborted.as_str(), "aborted");
        assert_eq!(
            HoldState::Failed(Ineligibility::TargetGone).as_str(),
            "failed"
        );
        assert_eq!(
            HoldState::Completed.reason(),
            None,
            "a completed hold has no failure reason to report"
        );
    }

    #[test]
    fn only_the_terminal_states_settle() {
        assert!(!HoldState::Holding.is_settled());
        assert!(!HoldState::Stalled(Ineligibility::OutOfRange).is_settled());
        assert!(HoldState::Completed.is_settled());
        assert!(HoldState::Aborted.is_settled());
        assert!(HoldState::Failed(Ineligibility::TargetGone).is_settled());
    }

    // ── Authoring validation ──

    #[test]
    fn a_capability_that_cannot_mean_anything_is_refused_at_load_by_name() {
        let cases: Vec<(CapabilityConfig, &str)> = vec![
            (
                CapabilityConfig {
                    range: 0.0,
                    ..stabilise()
                },
                "range",
            ),
            (
                CapabilityConfig {
                    duration_secs: 0,
                    ..stabilise()
                },
                "duration_secs",
            ),
            (
                CapabilityConfig {
                    power_group: "  ".to_string(),
                    ..stabilise()
                },
                "power_group",
            ),
            (
                CapabilityConfig {
                    condition_on_complete: -5.0,
                    ..stabilise()
                },
                "condition_on_complete",
            ),
            (
                CapabilityConfig {
                    stall_limit_secs: Some(-1),
                    ..stabilise()
                },
                "stall_limit_secs",
            ),
        ];
        for (capability, field) in cases {
            let config = OperationsConfig {
                capabilities: vec![capability],
            };
            let err = config
                .validate()
                .expect_err("this table cannot describe an operation that runs");
            assert!(
                err.contains(field),
                "the load error must name the field the author got wrong; for {field} it said: \
                 {err}"
            );
        }
    }

    #[test]
    fn one_hull_declaring_the_same_verb_twice_is_refused() {
        let config = OperationsConfig {
            capabilities: vec![stabilise(), stabilise()],
        };
        let err = config.validate().expect_err("a duplicate verb is refused");
        assert!(
            err.contains("stabilise") && err.contains("twice"),
            "two blocks for one verb means a start silently gets whichever came first, so the \
             error names the verb: {err}"
        );
    }

    #[test]
    fn a_hull_that_authors_no_operations_validates_and_can_do_nothing() {
        let config = OperationsConfig::default();
        assert_eq!(
            config.validate(),
            Ok(()),
            "every hull shipped before this existed is in this arm"
        );
        assert!(config.capability(OperationVerb::Stabilise).is_none());
    }

    #[test]
    fn the_authored_defaults_come_from_one_place_each() {
        // Hand-written `Default` and serde's `default = "…"` must call the same
        // fns; two copies of these numbers could only ever drift apart.
        let defaulted = CapabilityConfig::default();
        let parsed: CapabilityConfig =
            toml::from_str("verb = \"stabilise\"").expect("a bare capability block parses");
        assert_eq!(
            parsed, defaulted,
            "a capability block authoring nothing but its verb must equal the hand-written \
             Default in every field"
        );
    }

    #[test]
    fn the_authored_toml_shape_is_the_one_a_designer_writes() {
        let config: OperationsConfig = toml::from_str(
            r#"
[[capability]]
verb = "stabilise"
range = 250.0
duration_secs = 30
power_group = "helm"
min_power_level = 3
condition_on_complete = 40.0
stall_limit_secs = 12
"#,
        )
        .expect("the `[[operations.capability]]` shape parses");
        config.validate().expect("and validates");
        let capability = config
            .capability(OperationVerb::Stabilise)
            .expect("the verb resolves");
        assert_eq!(capability.range, 250.0);
        assert_eq!(capability.duration_secs, 30);
        assert_eq!(capability.min_power_level, 3);
        assert_eq!(capability.condition_on_complete, 40.0);
        assert_eq!(capability.stall_limit_secs, Some(12));
    }

    #[test]
    fn an_unknown_field_in_a_capability_block_is_a_load_error_not_a_shrug() {
        let err = toml::from_str::<OperationsConfig>(
            "[[capability]]\nverb = \"stabilise\"\nrange_metres = 250.0\n",
        )
        .expect_err("a misspelled field is refused");
        assert!(
            err.to_string().contains("range_metres"),
            "deny_unknown_fields is what stops a typo becoming a capability that quietly uses \
             the default: {err}"
        );
    }

    // ── Snapshot/resume: the hold is serialisable whole ──

    #[test]
    fn a_hold_round_trips_through_serde_with_its_progress_and_its_state() {
        let mut hold = fresh_hold();
        run_eligible(&mut hold, 137);
        hold.advance(Err(Ineligibility::OutOfRange));

        let encoded = serde_json::to_string(&hold).expect("a hold serialises");
        let decoded: OperationHold = serde_json::from_str(&encoded).expect("and comes back");
        assert_eq!(
            decoded, hold,
            "a resumed operation has to come back mid-hold — its banked ticks, its stall budget \
             spend and its current state together. Restoring the progress alone would resume a \
             stalled operation as if the crew were still on station."
        );
        assert_eq!(
            decoded.state(),
            HoldState::Stalled(Ineligibility::OutOfRange)
        );
        assert_eq!(decoded.elapsed_ticks(), 137);
    }
}
