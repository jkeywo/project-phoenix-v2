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
//! # The shared interrupt vocabulary (issue #1027)
//!
//! Eligibility answers "may this operation run at all". *Interrupts* answer
//! "and at what rate" — they are the conditions that arrive from outside the
//! operation's own terms: the crew are under fire, or the ship has drifted into
//! a hazard band. They are authored per capability as
//! [`InterruptRule`]s and evaluated **once, for every verb**
//! ([`interrupt_outcome`]) rather than per verb, which is the whole point of
//! putting them here.
//!
//! Each rule names a cause and a **response**, and the response is authored
//! rather than inferred:
//!
//! * `pause` — freeze progress, exactly as a recoverable ineligibility does,
//!   and spend the stall budget while it lasts;
//! * `fail` — end the hold on the tick the cause is seen;
//! * `slow` — keep holding at an authored fraction of the normal rate. This is
//!   the storm's shape: a radiation band stretches an operation rather than
//!   cancelling it, and the crew watch the bar crawl.
//!
//! Two rules that both fire take the stricter response, so authoring a `slow`
//! and a `fail` over the same band cannot accidentally produce the gentler one.

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
    /// Take a crippled craft under tow: while the hold runs, the target's
    /// position is the operator's, offset by the authored `tow_offset`.
    Tow,
    /// Keep station on something that is *moving*. Mechanically this is the
    /// ordinary proximity hold — the distance is recomputed every tick against
    /// wherever the escortee has got to — plus the authored `separation_limit`
    /// past which the relationship is over rather than merely stalled.
    Escort,
    /// Move an authored quantity of a named infrastructure capacity between the
    /// operator and the target, on the terms in `transfer`.
    Transfer,
    /// Work on a structure's condition track continuously, paying
    /// `condition_per_second` for every second held rather than a lump on
    /// completion — and committing `repair_teams` of the operator's own teams
    /// for the duration.
    FieldRepair,
}

impl OperationVerb {
    /// Every verb, in declaration order. The authoring lint and the console's
    /// capability list walk this rather than restating the set.
    pub const ALL: &'static [OperationVerb] = &[
        OperationVerb::Stabilise,
        OperationVerb::Tow,
        OperationVerb::Escort,
        OperationVerb::Transfer,
        OperationVerb::FieldRepair,
    ];

    /// The authored/wire spelling — the same string the TOML `verb` field and
    /// the script effect use.
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationVerb::Stabilise => "stabilise",
            OperationVerb::Tow => "tow",
            OperationVerb::Escort => "escort",
            OperationVerb::Transfer => "transfer",
            OperationVerb::FieldRepair => "field_repair",
        }
    }

    /// What a target must be for this verb to mean anything, when a capability
    /// block does not say.
    ///
    /// **The one place a verb decides behaviour.** Everything else that
    /// separates `tow` from `field_repair` is a field on the capability block —
    /// what it pays, what it consumes, how far it may stray — and is authored,
    /// not matched on. This match is the verb's *meaning*, not its behaviour:
    /// "stabilise" is a word about condition tracks in the same way "transfer"
    /// is a word about capacities, and an author who writes neither should get
    /// the obvious reading rather than a load error. A capability that wants
    /// something else says so with `target_requirement`.
    pub fn default_requirement(&self) -> TargetRequirement {
        match self {
            OperationVerb::Stabilise | OperationVerb::FieldRepair => {
                TargetRequirement::ConditionTrack
            }
            OperationVerb::Transfer => TargetRequirement::Capacity,
            OperationVerb::Tow | OperationVerb::Escort => TargetRequirement::Present,
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

/// Default rate for an authored `slow` interrupt, as a percentage of normal.
///
/// `50` — half speed. A `slow` rule that names no rate is asking for "this
/// takes noticeably longer", and half is the reading that cannot be mistaken
/// for either "no effect" or "stopped".
fn default_slow_rate() -> u16 {
    50
}

// ── Progress rate ────────────────────────────────────────────────────────────

/// How fast a hold banks time this tick, as a percentage of normal.
///
/// Whole percent rather than a float for the reason every authored delay in
/// this codebase is whole seconds: it is a designer-facing quantity, it is
/// folded into the authoritative digest, and integer arithmetic gives two hosts
/// the same answer without an ordering argument about floating point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProgressRate(u16);

impl ProgressRate {
    /// One tick of hold per tick of clock — what an uninterrupted operation
    /// runs at, and the scale everything else is a fraction of.
    pub const FULL: ProgressRate = ProgressRate(100);

    /// A rate as whole percent, clamped to `0..=100`. A rate above normal is
    /// refused rather than honoured: an interrupt makes an operation *harder*,
    /// and a hazard band that sped one up would be a different feature.
    pub fn percent(value: u16) -> Self {
        ProgressRate(value.min(Self::FULL.0))
    }

    /// The rate as whole percent.
    pub fn as_percent(&self) -> u16 {
        self.0
    }

    /// Whether this is the normal, uninterrupted rate — what the console uses
    /// to decide whether the crew need telling.
    pub fn is_full(&self) -> bool {
        *self == Self::FULL
    }
}

impl Default for ProgressRate {
    fn default() -> Self {
        Self::FULL
    }
}

// ── The interrupt vocabulary ─────────────────────────────────────────────────

/// What can interrupt an operation from outside its own terms.
///
/// Power loss is deliberately **not** here: it is already an eligibility
/// condition ([`Ineligibility::InsufficientPower`]), tested every tick against
/// the live grid, and giving it a second spelling would let a capability author
/// two different answers to the same question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptCause {
    /// The operator has taken fire recently — the same decaying window the
    /// doctrine gates read, not a latch.
    Attack,
    /// The operator is inside a region carrying the rule's `region_effect`.
    /// Membership is the whole test: an operation does not care *why* a band is
    /// dangerous, only that the ship is in it.
    Region,
    /// The people who staff the **target** are out (issue #1035).
    ///
    /// The first cause that is a fact about the far end rather than about the
    /// operator, and it is a cause rather than an eligibility condition for the
    /// reason the other two are: what a stoppage means depends entirely on the
    /// work. A transfer nobody will authorise cannot happen at any speed
    /// (`response = "fail"`); a repair the local crews have walked away from is
    /// still a repair, done the hard way (`response = "slow"`). Making it an
    /// eligibility condition would have forced one answer on both.
    WorkStoppage,
}

impl InterruptCause {
    /// The authored spelling, for the load errors that quote a rule back to
    /// the designer who wrote it.
    pub fn as_str(&self) -> &'static str {
        match self {
            InterruptCause::Attack => "attack",
            InterruptCause::Region => "region",
            InterruptCause::WorkStoppage => "work_stoppage",
        }
    }
}

/// The authored region effect an [`InterruptCause::Region`] rule watches for.
///
/// The spellings mirror `crate::regions::effects::RegionEffectKind`'s variants,
/// and the adapter maps one to the other. It is restated here rather than
/// imported so this module stays a leaf — and it is an enum rather than a raw
/// string so a misspelt band is a load error instead of a rule that silently
/// never fires. A test in the adapter proves every region effect kind has a
/// name here, so a new hazard cannot ship unauthorable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionEffectName {
    DamageZone,
    SlowZone,
    BlocksImpulse,
    RadarDampening,
    CommsJam,
    SensorBlind,
    NebulaFog,
}

impl RegionEffectName {
    /// Every effect name, in declaration order.
    pub const ALL: &'static [RegionEffectName] = &[
        RegionEffectName::DamageZone,
        RegionEffectName::SlowZone,
        RegionEffectName::BlocksImpulse,
        RegionEffectName::RadarDampening,
        RegionEffectName::CommsJam,
        RegionEffectName::SensorBlind,
        RegionEffectName::NebulaFog,
    ];

    /// The authored spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            RegionEffectName::DamageZone => "damage_zone",
            RegionEffectName::SlowZone => "slow_zone",
            RegionEffectName::BlocksImpulse => "blocks_impulse",
            RegionEffectName::RadarDampening => "radar_dampening",
            RegionEffectName::CommsJam => "comms_jam",
            RegionEffectName::SensorBlind => "sensor_blind",
            RegionEffectName::NebulaFog => "nebula_fog",
        }
    }
}

/// What an interrupt does to a hold that meets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptResponse {
    /// Keep holding, at `rate_percent` of the normal rate. The operation is
    /// stretched, not stopped — a storm band makes the work take longer.
    ///
    /// Ordered first because two rules that both fire take the **stricter**
    /// response, and this is the gentlest.
    Slow,
    /// Freeze progress and spend the stall budget, the same way a recoverable
    /// ineligibility does. Resumable the moment the cause clears.
    Pause,
    /// End the hold on the tick the cause is seen.
    Fail,
}

/// One authored `[[operations.capability.interrupt]]` block.
///
/// The point of authoring these rather than hardcoding them is that the same
/// cause means different things to different work. A tow through a radiation
/// band is slow but survivable; a field-repair with the hull open to it is not
/// something a crew should be able to keep doing. That judgement is a
/// designer's, and it belongs in the TOML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterruptRule {
    /// What this rule watches for.
    pub cause: InterruptCause,
    /// Which region effect, for a [`InterruptCause::Region`] rule. Ignored —
    /// and refused at load — for any other cause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_effect: Option<RegionEffectName>,
    /// What happens when it fires.
    pub response: InterruptResponse,
    /// The rate to hold at, for a [`InterruptResponse::Slow`] rule. Ignored for
    /// the other two, which do not have a rate.
    #[serde(default = "default_slow_rate")]
    pub rate_percent: u16,
}

impl InterruptRule {
    /// Whether this rule's cause is true right now.
    fn fires(&self, conditions: &OperationConditions) -> bool {
        match self.cause {
            InterruptCause::Attack => conditions.under_attack,
            InterruptCause::Region => self
                .region_effect
                .is_some_and(|effect| conditions.region_effects.contains(&effect)),
            InterruptCause::WorkStoppage => conditions.target_work_stopped,
        }
    }

    /// The reason a fired rule reports to the crew.
    fn reason(&self) -> Ineligibility {
        match self.cause {
            InterruptCause::Attack => Ineligibility::UnderAttack,
            InterruptCause::Region => Ineligibility::HazardBand,
            InterruptCause::WorkStoppage => Ineligibility::WorkStopped,
        }
    }
}

/// The terms of a `transfer`: what moves, how much of it, and which way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferTerms {
    /// The `[[infrastructure.capacity]]` id being moved. Both ends must carry
    /// one under this id — that is what makes them the two infrastructure
    /// entities the transfer is between.
    pub capacity: String,
    /// How much of it moves, in the capacity's own authored units. Paid once,
    /// on completion: a half-finished transfer that moved half the load would
    /// make abandoning one a strategy, exactly as it would for `stabilise`.
    pub amount: i64,
    /// Which way it moves.
    pub direction: TransferDirection,
}

/// Which way a [`TransferTerms`] moves its load.
///
/// Named from the **operator's** point of view, because the operator is who the
/// crew are: a tender *delivers* to a depot and *collects* from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    /// Operator → target.
    Deliver,
    /// Target → operator.
    Collect,
}

/// What a target has to be for a capability to mean anything on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRequirement {
    /// It merely has to exist. What `tow` and `escort` ask: a derelict freighter
    /// carries no condition track and no capacities, and is exactly the thing
    /// you tow.
    Present,
    /// It has to carry an `[infrastructure]` condition track. A pristine
    /// asteroid is present, in range, and still not a thing you can stabilise.
    ConditionTrack,
    /// It has to carry the capability's `transfer.capacity` id.
    Capacity,
}

/// One end of a transfer, as the adapter reads it off an entity's
/// `[infrastructure]` capacities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapacityReading {
    /// How much is there now.
    pub level: i64,
    /// How much more the authored ceiling would still admit.
    pub headroom: i64,
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
    /// Infrastructure condition points the target gains for every **second**
    /// the hold runs (issue #1027).
    ///
    /// `field_repair`'s shape, and the deliberate opposite of
    /// `condition_on_complete` above: a repair party working a skyhook's spine
    /// is doing good the whole time they are there, and a crew pulled off after
    /// two of three minutes should keep two minutes of it. That is the honest
    /// reading for *repair*, and the dishonest one for *stabilise*, which is why
    /// the two are separate fields rather than one with a flag.
    ///
    /// Scaled by the tick's [`ProgressRate`], so a hazard band that halves the
    /// rate halves the repair as well — the work and its payoff cannot come
    /// apart. `0.0` (the default) pays nothing per tick.
    #[serde(default)]
    pub condition_per_second: f32,
    /// How many of the operator's own repair teams this operation commits while
    /// it is held (issue #1027).
    ///
    /// **Capacity as cost.** The teams never leave the hull — they are not
    /// modelled as crossing to the target and they cannot be shot at over
    /// there — but they are unavailable to the ship's own internal repair sweep
    /// for the duration, and released the moment the hold settles. A captain
    /// field-repairing a skyhook with every team committed is a captain who
    /// cannot also fix their own shields, and that trade is the mechanic.
    #[serde(default)]
    pub repair_teams: u8,
    /// Where a towed target sits relative to the operator, in the operator's own
    /// local frame: `[starboard, up, forward]` in world units, so `[0, 0, -150]`
    /// rides 150 units astern (issue #1027).
    ///
    /// Authored rather than captured from wherever the target happened to be,
    /// which keeps the tow rig stateless: the towed craft's position is a pure
    /// function of the operator's position and yaw, so it needs nothing in the
    /// save and nothing in the digest beyond the hold that is already there.
    #[serde(default, skip_serializing_if = "is_zero_offset")]
    pub tow_offset: [f32; 3],
    /// How far the target may get before the relationship is **over** rather
    /// than merely stalled (issue #1027).
    ///
    /// `escort`'s shape. Past `range` an operation stalls and the crew close
    /// back up; past this it has failed, because an escortee that far away is
    /// not an escortee any more. Must be at least `range` — a limit inside the
    /// operating range would fail every hold before it could stall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separation_limit: Option<f32>,
    /// What this operation moves between the operator and the target, for a
    /// `transfer` (issue #1027).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<TransferTerms>,
    /// What the target has to be. `None` takes the verb's own reading —
    /// [`OperationVerb::default_requirement`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_requirement: Option<TargetRequirement>,
    /// The authored interrupt rules (issue #1027), in authored order. Empty is
    /// the #1026 behaviour exactly: only eligibility can stop the hold.
    #[serde(default, rename = "interrupt", skip_serializing_if = "Vec::is_empty")]
    pub interrupts: Vec<InterruptRule>,
    /// Whole seconds of **cumulative** stalled time this hold tolerates before
    /// it fails. `None` (the default) lets it stall indefinitely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stall_limit_secs: Option<i64>,
}

/// Serde skip predicate for an unauthored [`CapabilityConfig::tow_offset`].
fn is_zero_offset(offset: &[f32; 3]) -> bool {
    offset.iter().all(|component| *component == 0.0)
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
            condition_per_second: 0.0,
            repair_teams: 0,
            tow_offset: [0.0; 3],
            separation_limit: None,
            transfer: None,
            target_requirement: None,
            interrupts: Vec::new(),
            stall_limit_secs: None,
        }
    }
}

impl CapabilityConfig {
    /// What the target has to be, as authored or as the verb reads it.
    pub fn target_requirement(&self) -> TargetRequirement {
        self.target_requirement
            .unwrap_or_else(|| self.verb.default_requirement())
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
            if !capability.condition_per_second.is_finite() || capability.condition_per_second < 0.0
            {
                return Err(format!(
                    "[[operations.capability]] {verb} condition_per_second must be a non-negative \
                     finite number of condition points, got {} — an operation that degrades its \
                     target while it works is authored as a hazard, not as a capability",
                    capability.condition_per_second
                ));
            }
            for (axis, component) in capability.tow_offset.iter().enumerate() {
                if !component.is_finite() {
                    return Err(format!(
                        "[[operations.capability]] {verb} tow_offset component {axis} must be \
                         finite, got {component} — a non-finite offset puts the towed craft \
                         nowhere"
                    ));
                }
            }
            if let Some(limit) = capability.separation_limit {
                if !limit.is_finite() || limit < capability.range {
                    return Err(format!(
                        "[[operations.capability]] {verb} separation_limit must be a finite \
                         distance at or beyond range ({}), got {limit} — a limit inside the \
                         operating range fails every hold before it can stall, which is not a \
                         thing any crew could fly out of",
                        capability.range
                    ));
                }
            }
            if let Some(transfer) = &capability.transfer {
                if transfer.capacity.trim().is_empty() {
                    return Err(format!(
                        "[[operations.capability]] {verb} transfer needs a non-empty capacity id — \
                         it names the [[infrastructure.capacity]] block at both ends"
                    ));
                }
                if transfer.amount <= 0 {
                    return Err(format!(
                        "[[operations.capability]] {verb} transfer amount must be a positive \
                         quantity, got {} — reverse the direction rather than authoring a negative \
                         load",
                        transfer.amount
                    ));
                }
            }
            if capability.target_requirement() == TargetRequirement::Capacity
                && capability.transfer.is_none()
            {
                return Err(format!(
                    "[[operations.capability]] {verb} requires a target capacity but authors no \
                     transfer block, so nothing names which capacity — add [.transfer] or set \
                     target_requirement"
                ));
            }
            for rule in &capability.interrupts {
                match rule.cause {
                    InterruptCause::Region if rule.region_effect.is_none() => {
                        return Err(format!(
                            "[[operations.capability.interrupt]] on {verb} has cause = \"region\" \
                             but names no region_effect, so it could never fire"
                        ));
                    }
                    InterruptCause::Region => {}
                    _ if rule.region_effect.is_some() => {
                        return Err(format!(
                            "[[operations.capability.interrupt]] on {verb} names a region_effect \
                             with cause = \"{}\", which does not read one — the rule would not \
                             mean what it says",
                            rule.cause.as_str()
                        ));
                    }
                    _ => {}
                }
                if rule.response == InterruptResponse::Slow
                    && (rule.rate_percent == 0 || rule.rate_percent > 100)
                {
                    return Err(format!(
                        "[[operations.capability.interrupt]] on {verb} has response = \"slow\" \
                         with rate_percent {} — a slow rate is 1..=100 percent of normal. Author \
                         response = \"pause\" for a full stop.",
                        rule.rate_percent
                    ));
                }
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
    /// Whether the target carries an `[infrastructure]` condition track.
    pub target_has_condition_track: bool,
    /// The target's reading of the capability's `transfer.capacity`, when it
    /// carries one. `None` means the target has no such capacity at all, which
    /// is what makes a `transfer` inapplicable rather than merely blocked.
    pub target_capacity: Option<CapacityReading>,
    /// The **operator's** reading of the same capacity — the other end of the
    /// transfer. `None` for an operator that authors no `[infrastructure]`.
    pub operator_capacity: Option<CapacityReading>,
    /// Centre-to-centre distance from the ship to the target, in world units.
    pub distance: f32,
    /// The ship's current allocation level for the capability's power group.
    pub power_level: u8,
    /// Whether the ship's power grid is under an exhaustion lock. A locked grid
    /// reads as insufficient however the levels are set, because the levels are
    /// not what the grid is delivering.
    pub power_locked: bool,
    /// How many of the operator's repair teams are free to be committed to this
    /// operation. `u8::MAX` for a hull that carries no repair teams at all,
    /// which is the absence of the constraint rather than a failure of it —
    /// the same reading `power_level` takes for a hull with no grid.
    pub repair_teams_available: u8,
    /// Whether the operator has taken fire recently (issue #1027). Read off the
    /// same decaying window the doctrine gates use, so "recently" means one
    /// authored thing across the whole simulation.
    pub under_attack: bool,
    /// Which authored region effects the operator is currently inside
    /// (issue #1027), in a deterministic order.
    pub region_effects: Vec<RegionEffectName>,
    /// Whether the people who staff the **target** are out (issue #1035).
    ///
    /// Read by the adapter off the target's own
    /// `[infrastructure] workforce` and the world's live workforce register.
    /// `false` for a target that names no workforce, for a world that declares
    /// no dispute, and for a side that is at work — three different facts that
    /// all mean the same thing to an operation: the local crews are there.
    pub target_work_stopped: bool,
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
    /// An escortee has got further away than the capability's
    /// `separation_limit`. Terminal, unlike [`Self::OutOfRange`]: past this the
    /// relationship is over rather than stretched.
    Separated,
    /// One end of a `transfer` cannot take part right now — the source is short
    /// of the authored amount, or the destination has no room for it.
    CapacityUnavailable,
    /// The operator has fewer free repair teams than the capability commits.
    TeamsUnavailable,
    /// The operator is under fire and an authored interrupt rule says that
    /// matters (issue #1027).
    UnderAttack,
    /// The operator is inside a hazard band an authored interrupt rule watches
    /// for (issue #1027).
    HazardBand,
    /// The people who staff the target are out, and an authored interrupt rule
    /// says that matters (issue #1035).
    ///
    /// The one refusal in this list that names a decision somebody else made
    /// rather than a physical fact, which is why it is the one the crew can do
    /// something about by talking.
    WorkStopped,
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
            Ineligibility::Separated => "operation.refused.separated",
            Ineligibility::CapacityUnavailable => "operation.refused.capacity_unavailable",
            Ineligibility::TeamsUnavailable => "operation.refused.teams_unavailable",
            Ineligibility::UnderAttack => "operation.refused.under_attack",
            Ineligibility::HazardBand => "operation.refused.hazard_band",
            Ineligibility::WorkStopped => "operation.refused.work_stopped",
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
            Ineligibility::Separated => "separated",
            Ineligibility::CapacityUnavailable => "capacity_unavailable",
            Ineligibility::TeamsUnavailable => "teams_unavailable",
            Ineligibility::UnderAttack => "under_attack",
            Ineligibility::HazardBand => "hazard_band",
            Ineligibility::WorkStopped => "work_stopped",
        }
    }

    /// Whether a hold that hit this can go on to recover from it.
    ///
    /// Range, power, a blocked capacity and a busy repair roster are the ones
    /// the crew are *for*: helm flies back, engineering reallocates, a depot
    /// drains, a repair party finishes. The rest are facts about the hull or
    /// about the target that no console can change, so a hold that meets one is
    /// over rather than waiting.
    ///
    /// The three **interrupt** reasons — [`Self::UnderAttack`],
    /// [`Self::HazardBand`] and [`Self::WorkStopped`] — read as recoverable
    /// here, but that is only the fallback: an interrupt carries its own
    /// authored terminality on [`Interruption::terminal`], because whether a
    /// crew may keep working through fire is a designer's call and not a
    /// property of the word "attack". A stoppage is the clearest case of the
    /// three: the reason it reads recoverable is that a negotiation can end it,
    /// and the reason a shipped transfer capability nonetheless authors
    /// `response = "fail"` against it is that a refused transfer is a refusal,
    /// not a queue.
    pub fn recoverable(&self) -> bool {
        matches!(
            self,
            Ineligibility::OutOfRange
                | Ineligibility::InsufficientPower
                | Ineligibility::CapacityUnavailable
                | Ineligibility::TeamsUnavailable
                | Ineligibility::UnderAttack
                | Ineligibility::HazardBand
                | Ineligibility::WorkStopped
        )
    }
}

// ── One tick's whole verdict ─────────────────────────────────────────────────

/// A loss of eligibility, with whether it ends the hold.
///
/// Terminality is carried rather than derived because #1027's interrupts made
/// it **authored**: the same `under_attack` cause pauses a tow and fails a
/// field-repair, on the same hull, in the same mission. Everything that comes
/// out of [`eligibility`] takes its terminality from
/// [`Ineligibility::recoverable`]; everything that comes out of an authored
/// [`InterruptRule`] takes it from the rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interruption {
    /// Why the hold is not advancing.
    pub reason: Ineligibility,
    /// Whether this ends the hold now, rather than stalling it.
    pub terminal: bool,
}

/// Everything one tick decides: whether the hold advances, how fast, or why not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickVerdict {
    /// `Ok` with the rate to bank at, or the interruption that stopped it.
    pub outcome: Result<ProgressRate, Interruption>,
}

impl TickVerdict {
    /// A full-rate, uninterrupted tick.
    pub const HOLDING: TickVerdict = TickVerdict {
        outcome: Ok(ProgressRate::FULL),
    };

    /// The rate this tick banks at, or `None` when it banks nothing.
    pub fn rate(&self) -> Option<ProgressRate> {
        self.outcome.ok()
    }
}

impl From<Result<(), Ineligibility>> for TickVerdict {
    /// The #1026 verdict shape, lifted: an eligible tick runs at full rate, and
    /// a lost one takes its terminality from the reason itself.
    fn from(verdict: Result<(), Ineligibility>) -> Self {
        TickVerdict {
            outcome: match verdict {
                Ok(()) => Ok(ProgressRate::FULL),
                Err(reason) => Err(Interruption {
                    reason,
                    terminal: !reason.recoverable(),
                }),
            },
        }
    }
}

/// Apply the authored interrupt rules to this tick's conditions.
///
/// Implemented **once for every verb**, which is the acceptance criterion: a
/// verb does not get to interpret "under attack" its own way, it gets to author
/// what happens. Rules are walked in authored order and the **strictest**
/// response among those that fire wins, so a capability carrying both a `slow`
/// and a `fail` over the same band cannot accidentally get the gentler one from
/// an ordering accident. Among two `slow` rules the lower rate wins, for the
/// same reason.
pub fn interrupt_outcome(
    rules: &[InterruptRule],
    conditions: &OperationConditions,
) -> Result<ProgressRate, Interruption> {
    let mut worst: Option<(&InterruptRule, InterruptResponse)> = None;
    for rule in rules.iter().filter(|rule| rule.fires(conditions)) {
        let strictly_worse = match worst {
            None => true,
            Some((held, response)) => match rule.response.cmp(&response) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                // Two rules of the same severity: for a slow, the lower rate is
                // the harsher reading of "both of these are happening at once".
                std::cmp::Ordering::Equal => {
                    rule.response == InterruptResponse::Slow
                        && rule.rate_percent < held.rate_percent
                }
            },
        };
        if strictly_worse {
            worst = Some((rule, rule.response));
        }
    }
    match worst {
        None => Ok(ProgressRate::FULL),
        Some((rule, InterruptResponse::Slow)) => Ok(ProgressRate::percent(rule.rate_percent)),
        Some((rule, response)) => Err(Interruption {
            reason: rule.reason(),
            terminal: response == InterruptResponse::Fail,
        }),
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
    if !target_satisfies(capability, conditions) {
        return Err(Ineligibility::TargetNotApplicable);
    }
    // The separation limit is tested BEFORE the range, even though it is the
    // larger distance, because it is the TERMINAL one: checking range first
    // would report a recoverable `OutOfRange` for an escortee that is already
    // gone for good, and the hold would stall forever waiting for it.
    if let Some(limit) = capability.separation_limit {
        if !within(conditions.distance, limit) {
            return Err(Ineligibility::Separated);
        }
    }
    // Fail-closed on an incomparable distance: a NaN reading refuses rather
    // than passes, which is why this is spelled via partial_cmp instead of
    // `distance > range` (that flips the NaN outcome to "in range").
    if !within(conditions.distance, capability.range) {
        return Err(Ineligibility::OutOfRange);
    }
    if conditions.power_locked || conditions.power_level < capability.min_power_level {
        return Err(Ineligibility::InsufficientPower);
    }
    if capability.repair_teams > conditions.repair_teams_available {
        return Err(Ineligibility::TeamsUnavailable);
    }
    if let Some(transfer) = &capability.transfer {
        if !transfer_possible(transfer, conditions) {
            return Err(Ineligibility::CapacityUnavailable);
        }
    }
    Ok(())
}

/// `distance <= limit`, written so a `NaN` distance takes the failing arm.
///
/// A naive `distance > limit` lets `NaN` through as "inside", which would run
/// an operation against a target nobody can locate.
fn within(distance: f32, limit: f32) -> bool {
    matches!(
        distance.partial_cmp(&limit),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    )
}

/// Whether the target is a thing this capability can be performed *on*.
fn target_satisfies(capability: &CapabilityConfig, conditions: &OperationConditions) -> bool {
    match capability.target_requirement() {
        TargetRequirement::Present => true,
        TargetRequirement::ConditionTrack => conditions.target_has_condition_track,
        TargetRequirement::Capacity => conditions.target_capacity.is_some(),
    }
}

/// Whether both ends of a transfer can take part right now.
///
/// Both are asked, always, whichever way the load is going: the source must
/// hold the authored amount and the destination must have room for it. A
/// transfer that only checked the source would overfill a depot; one that only
/// checked the destination would move goods that were never there.
fn transfer_possible(transfer: &TransferTerms, conditions: &OperationConditions) -> bool {
    let (source, destination) = match transfer.direction {
        TransferDirection::Deliver => (conditions.operator_capacity, conditions.target_capacity),
        TransferDirection::Collect => (conditions.target_capacity, conditions.operator_capacity),
    };
    let (Some(source), Some(destination)) = (source, destination) else {
        return false;
    };
    source.level >= transfer.amount && destination.headroom >= transfer.amount
}

/// The whole of one tick's decision: eligibility first, then the authored
/// interrupts.
///
/// The order is the one the crew can act on. Eligibility is about the operation
/// itself — is the ship capable, is the target there, is it close enough, is
/// there power — and an operation that fails those is not running at any rate.
/// Interrupts are about the world happening *to* an operation that is otherwise
/// fine, so they only get asked once it is.
pub fn verdict(
    capability: Option<&CapabilityConfig>,
    conditions: &OperationConditions,
) -> TickVerdict {
    match eligibility(capability, conditions) {
        Err(reason) => TickVerdict::from(Err(reason)),
        Ok(()) => TickVerdict {
            outcome: interrupt_outcome(
                capability.map(|c| c.interrupts.as_slice()).unwrap_or(&[]),
                conditions,
            ),
        },
    }
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
    /// Part of a tick banked at a reduced rate, in hundredths of a tick
    /// (issue #1027). Always `0` for a hold that has never been slowed, which is
    /// why every #1026 assertion about `elapsed_ticks` still reads the same
    /// number: at full rate this fills and empties within one tick.
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    rate_remainder: u16,
    /// Condition points owed to the target for every **whole** tick held at
    /// full rate — the authored `condition_per_second` converted once, at
    /// [`Self::start`], against the world's own tick rate.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    condition_per_tick: f32,
    /// The rate the last advanced tick banked at (issue #1027). Published so
    /// the console can show a crew that the storm is stretching their work, and
    /// carried on the hold rather than recomputed because the console reads the
    /// hold and not the conditions that produced it.
    #[serde(default)]
    rate: ProgressRate,
}

/// Serde skip predicate for a hold that has never been slowed.
fn is_zero_u16(value: &u16) -> bool {
    *value == 0
}

/// Serde skip predicate for a hold that pays nothing per tick.
fn is_zero_f32(value: &f32) -> bool {
    *value == 0.0
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
            rate_remainder: 0,
            // The authored per-second rate converted once, here, at the same
            // boundary the duration crosses. `tick_hz` is validated positive at
            // world load; the guard keeps a bare fixture from dividing by zero.
            condition_per_tick: if tick_hz > 0.0 {
                capability.condition_per_second / tick_hz
            } else {
                0.0
            },
            rate: ProgressRate::FULL,
        }
    }

    /// Advance the hold by one logical tick against this tick's verdict.
    ///
    /// Returns `Some` on the tick the hold settles and `None` on every other,
    /// including every tick after it settled — so an adapter that pays a
    /// completion off the return value pays it once.
    ///
    /// Takes anything a verdict can be spelled as, so the #1026 call shape
    /// (`advance(Ok(()))`, `advance(Err(reason))`) still reads the same and a
    /// caller that has a rate to pass says so with a whole [`TickVerdict`].
    pub fn advance(&mut self, verdict: impl Into<TickVerdict>) -> Option<Settlement> {
        if self.state.is_settled() {
            return None;
        }
        match verdict.into().outcome {
            Ok(rate) => {
                self.state = HoldState::Holding;
                self.rate = rate;
                // Sub-tick banking. At the full rate this fills and empties
                // within the tick, so `elapsed_ticks` counts exactly the ticks
                // held — which is why every #1026 assertion about it is
                // untouched by slowing existing at all. A slowed tick banks its
                // fraction and carries the rest forward, so three ticks at 40 %
                // are worth one whole tick and a fifth, not one.
                let banked = u32::from(self.rate_remainder) + u32::from(rate.as_percent());
                self.rate_remainder = (banked % u32::from(ProgressRate::FULL.0)) as u16;
                self.elapsed_ticks = self
                    .elapsed_ticks
                    .saturating_add(u64::from(banked / u32::from(ProgressRate::FULL.0)));
                if self.elapsed_ticks >= self.required_ticks {
                    self.elapsed_ticks = self.required_ticks;
                    self.rate_remainder = 0;
                    self.state = HoldState::Completed;
                    return Some(Settlement::Completed);
                }
                None
            }
            Err(Interruption { reason, terminal }) if terminal => {
                self.state = HoldState::Failed(reason);
                Some(Settlement::Failed(reason))
            }
            Err(Interruption { reason, .. }) => {
                // Progress freezes rather than decaying: the crew that flies
                // back gets the ticks they already earned. The sub-tick
                // remainder freezes with it — a crew interrupted four fifths of
                // the way through a slowed tick keeps those four fifths.
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
        let banked =
            self.elapsed_ticks * u64::from(ProgressRate::FULL.0) + u64::from(self.rate_remainder);
        let required = self.required_ticks * u64::from(ProgressRate::FULL.0);
        (banked as f32 / required as f32).clamp(0.0, 1.0)
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

    /// Sub-tick progress banked at a reduced rate, in hundredths of a tick.
    pub fn rate_remainder(&self) -> u16 {
        self.rate_remainder
    }

    /// The rate the last advanced tick banked at.
    pub fn rate(&self) -> ProgressRate {
        self.rate
    }

    /// Condition points owed to the target for one whole tick at full rate.
    ///
    /// `field_repair`'s per-tick slice, as distinct from
    /// [`Self::condition_on_complete`]'s lump. The caller scales it by the
    /// tick's rate — a hazard band that halves the work halves the repair.
    pub fn condition_per_tick(&self) -> f32 {
        self.condition_per_tick
    }

    /// What this tick's repair is worth at `rate`, or `0.0` for a hold that
    /// pays nothing per tick.
    ///
    /// Here rather than in the adapter so the scaling rule — the payout tracks
    /// the progress exactly — is stated once, next to the arithmetic it has to
    /// agree with.
    pub fn condition_payout(&self, rate: ProgressRate) -> f32 {
        if self.condition_per_tick == 0.0 {
            return 0.0;
        }
        self.condition_per_tick * f32::from(rate.as_percent()) / f32::from(ProgressRate::FULL.0)
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
            ..Default::default()
        }
    }

    /// Conditions under which the operation is eligible.
    fn eligible() -> OperationConditions {
        OperationConditions {
            target_present: true,
            target_has_condition_track: true,
            target_capacity: None,
            operator_capacity: None,
            distance: 100.0,
            power_level: 3,
            power_locked: false,
            repair_teams_available: u8::MAX,
            under_attack: false,
            region_effects: Vec::new(),
            target_work_stopped: false,
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
            target_has_condition_track: false,
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

    // ══ Issue #1027: the remaining verbs ═════════════════════════════════════

    fn capability(verb: OperationVerb) -> CapabilityConfig {
        CapabilityConfig {
            verb,
            range: 400.0,
            duration_secs: 5,
            ..Default::default()
        }
    }

    // ── AC1: every verb goes through the same pure module ──

    #[test]
    fn all_five_verbs_round_trip_through_their_authored_spelling() {
        assert_eq!(
            OperationVerb::ALL.len(),
            5,
            "stabilise, tow, escort, transfer and field_repair — the vocabulary the PRD names"
        );
        for verb in OperationVerb::ALL {
            assert_eq!(OperationVerb::parse(verb.as_str()), Some(*verb));
        }
        assert_eq!(
            OperationVerb::parse("field-repair"),
            None,
            "the authored spelling is snake_case throughout; a hyphen is a refusal rather than a \
             silent no-op"
        );
    }

    #[test]
    fn each_verb_reads_its_target_requirement_from_the_verb_unless_the_author_overrides_it() {
        // The ONE place a verb decides anything. Everything else that separates
        // the verbs is an authored field.
        for (verb, expected) in [
            (OperationVerb::Stabilise, TargetRequirement::ConditionTrack),
            (
                OperationVerb::FieldRepair,
                TargetRequirement::ConditionTrack,
            ),
            (OperationVerb::Transfer, TargetRequirement::Capacity),
            (OperationVerb::Tow, TargetRequirement::Present),
            (OperationVerb::Escort, TargetRequirement::Present),
        ] {
            assert_eq!(capability(verb).target_requirement(), expected);
        }
        let overridden = CapabilityConfig {
            target_requirement: Some(TargetRequirement::ConditionTrack),
            ..capability(OperationVerb::Tow)
        };
        assert_eq!(
            overridden.target_requirement(),
            TargetRequirement::ConditionTrack,
            "a scenario that only wants a damaged hulk towed says so in TOML, without a new verb"
        );
    }

    #[test]
    fn a_tow_or_escort_target_needs_only_to_exist() {
        // A derelict freighter carries no condition track and no capacities,
        // and is exactly the thing you tow.
        let bare = OperationConditions {
            target_has_condition_track: false,
            ..eligible()
        };
        for verb in [OperationVerb::Tow, OperationVerb::Escort] {
            assert_eq!(
                eligibility(Some(&capability(verb)), &bare),
                Ok(()),
                "{} must not inherit stabilise's condition-track requirement",
                verb.as_str()
            );
        }
        assert_eq!(
            eligibility(Some(&capability(OperationVerb::Stabilise)), &bare),
            Err(Ineligibility::TargetNotApplicable),
            "…while the verb that IS about condition tracks still says so"
        );
    }

    // ── AC6: escort separates rather than stalling forever ──

    fn escort() -> CapabilityConfig {
        CapabilityConfig {
            separation_limit: Some(2_000.0),
            ..capability(OperationVerb::Escort)
        }
    }

    #[test]
    fn an_escortee_that_drifts_stalls_and_one_that_is_lost_ends_the_relationship() {
        for (distance, expected) in [
            (399.0_f32, Ok(())),
            // Past the operating range but still in company: the crew close up.
            (1_500.0, Err(Ineligibility::OutOfRange)),
            (2_000.0, Err(Ineligibility::OutOfRange)),
            // Past the separation limit: it is not an escortee any more.
            (2_000.1, Err(Ineligibility::Separated)),
            (60_000.0, Err(Ineligibility::Separated)),
        ] {
            let conditions = OperationConditions {
                distance,
                ..eligible()
            };
            assert_eq!(
                eligibility(Some(&escort()), &conditions),
                expected,
                "at {distance} m against a 400 m station and a 2000 m separation limit"
            );
        }
        assert!(
            !Ineligibility::Separated.recoverable(),
            "separation is TERMINAL — a hold that stalled on it would sit there forever waiting \
             for a convoy that has gone"
        );
    }

    #[test]
    fn the_separation_limit_is_tested_before_the_range_so_the_terminal_reason_wins() {
        // Both are breached at 60 km. Checking range first would report a
        // recoverable OutOfRange and the hold would stall forever.
        let mut hold = OperationHold::start(1, "convoy", &escort(), HZ);
        let far = OperationConditions {
            distance: 60_000.0,
            ..eligible()
        };
        assert_eq!(
            hold.advance(eligibility(Some(&escort()), &far)),
            Some(Settlement::Failed(Ineligibility::Separated)),
            "the escort ends on the tick the escortee is lost, not on the tick a stall budget \
             nobody authored happens to run out"
        );
    }

    #[test]
    fn a_separation_limit_inside_the_operating_range_is_refused_at_load() {
        let config = OperationsConfig {
            capabilities: vec![CapabilityConfig {
                range: 400.0,
                separation_limit: Some(100.0),
                ..capability(OperationVerb::Escort)
            }],
        };
        let err = config
            .validate()
            .expect_err("this cannot describe an escort");
        assert!(
            err.contains("separation_limit"),
            "a limit inside the range fails every hold before it can stall, which no crew could \
             fly out of: {err}"
        );
    }

    // ── AC7: transfer respects BOTH ends' capacities ──

    fn transfer(direction: TransferDirection, amount: i64) -> CapabilityConfig {
        CapabilityConfig {
            transfer: Some(TransferTerms {
                capacity: "berths".to_string(),
                amount,
                direction,
            }),
            ..capability(OperationVerb::Transfer)
        }
    }

    fn with_capacities(
        operator: Option<(i64, i64)>,
        target: Option<(i64, i64)>,
    ) -> OperationConditions {
        OperationConditions {
            operator_capacity: operator
                .map(|(level, headroom)| CapacityReading { level, headroom }),
            target_capacity: target.map(|(level, headroom)| CapacityReading { level, headroom }),
            ..eligible()
        }
    }

    #[test]
    fn a_transfer_needs_the_named_capacity_at_the_target_at_all() {
        assert_eq!(
            eligibility(
                Some(&transfer(TransferDirection::Deliver, 10)),
                &with_capacities(Some((50, 50)), None)
            ),
            Err(Ineligibility::TargetNotApplicable),
            "a target that carries no such capacity is not a thing you can transfer to — that is \
             a different refusal from one that is simply full right now, and the crew act on it \
             differently"
        );
    }

    #[test]
    fn a_delivery_asks_the_source_for_stock_and_the_destination_for_room() {
        let terms = transfer(TransferDirection::Deliver, 10);
        assert_eq!(
            eligibility(Some(&terms), &with_capacities(Some((10, 0)), Some((0, 10)))).map(|_| ()),
            Ok(()),
            "exactly enough at both ends is enough"
        );
        assert_eq!(
            eligibility(Some(&terms), &with_capacities(Some((9, 0)), Some((0, 999)))),
            Err(Ineligibility::CapacityUnavailable),
            "one short at the SOURCE blocks it — a transfer that only checked the destination \
             would move goods that were never there"
        );
        assert_eq!(
            eligibility(Some(&terms), &with_capacities(Some((999, 0)), Some((0, 9)))),
            Err(Ineligibility::CapacityUnavailable),
            "one short at the DESTINATION blocks it too — a transfer that only checked the source \
             would overfill the depot"
        );
    }

    #[test]
    fn collecting_swaps_which_end_is_the_source() {
        let terms = transfer(TransferDirection::Collect, 10);
        assert_eq!(
            eligibility(Some(&terms), &with_capacities(Some((0, 10)), Some((10, 0)))).map(|_| ()),
            Ok(()),
            "collecting takes from the TARGET and puts it aboard the operator"
        );
        assert_eq!(
            eligibility(Some(&terms), &with_capacities(Some((0, 10)), Some((9, 0)))),
            Err(Ineligibility::CapacityUnavailable),
            "…so a target one short is what blocks a collection, where it would not block a \
             delivery"
        );
    }

    #[test]
    fn a_blocked_capacity_stalls_rather_than_failing() {
        assert!(
            Ineligibility::CapacityUnavailable.recoverable(),
            "a depot that is full now may not be full in a minute — a crew waiting at the airlock \
             is playing the game, not stuck in it"
        );
    }

    #[test]
    fn a_transfer_that_names_no_capacity_id_or_a_non_positive_load_is_refused_at_load() {
        for (terms, field) in [
            (
                TransferTerms {
                    capacity: "  ".to_string(),
                    amount: 5,
                    direction: TransferDirection::Deliver,
                },
                "capacity",
            ),
            (
                TransferTerms {
                    capacity: "berths".to_string(),
                    amount: 0,
                    direction: TransferDirection::Deliver,
                },
                "amount",
            ),
        ] {
            let config = OperationsConfig {
                capabilities: vec![CapabilityConfig {
                    transfer: Some(terms),
                    ..capability(OperationVerb::Transfer)
                }],
            };
            let err = config
                .validate()
                .expect_err("this cannot describe a transfer");
            assert!(err.contains(field), "the error must name {field}: {err}");
        }
    }

    #[test]
    fn a_transfer_verb_with_no_transfer_block_is_refused_at_load() {
        let config = OperationsConfig {
            capabilities: vec![capability(OperationVerb::Transfer)],
        };
        let err = config
            .validate()
            .expect_err("a transfer with nothing to transfer is an author mistake");
        assert!(
            err.contains("transfer"),
            "nothing names which capacity moves, so the operation could only ever stall: {err}"
        );
    }

    // ── AC4: field-repair pays per tick, and commits teams ──

    fn field_repair(points_per_sec: f32, teams: u8) -> CapabilityConfig {
        CapabilityConfig {
            condition_per_second: points_per_sec,
            repair_teams: teams,
            ..capability(OperationVerb::FieldRepair)
        }
    }

    #[test]
    fn field_repair_pays_per_tick_where_stabilise_pays_on_completion() {
        let hold = OperationHold::start(1, "skyhook", &field_repair(6.0, 0), HZ);
        assert!(
            (hold.condition_per_tick() - 0.1).abs() < 1e-6,
            "six points a second at 60 Hz is a tenth of a point a tick — the conversion happens \
             once, at the boundary, like the duration's: {}",
            hold.condition_per_tick()
        );
        assert_eq!(
            hold.condition_on_complete(),
            0.0,
            "and it owes nothing on completion; the two payout shapes are separate fields so an \
             author cannot get both by accident"
        );

        let stabilise_hold = fresh_hold();
        assert_eq!(
            stabilise_hold.condition_per_tick(),
            0.0,
            "…and stabilise owes nothing per tick, which is what makes abandoning one worthless"
        );
        assert_eq!(stabilise_hold.condition_on_complete(), 30.0);
    }

    #[test]
    fn a_slowed_field_repair_pays_at_the_slowed_rate() {
        let hold = OperationHold::start(1, "skyhook", &field_repair(6.0, 0), HZ);
        assert!(
            (hold.condition_payout(ProgressRate::FULL) - 0.1).abs() < 1e-6,
            "full rate pays the whole tick's worth"
        );
        assert!(
            (hold.condition_payout(ProgressRate::percent(50)) - 0.05).abs() < 1e-6,
            "half rate pays half: the work and its payoff cannot come apart, or a crew could farm \
             condition points by parking in a storm"
        );
        assert_eq!(
            fresh_hold().condition_payout(ProgressRate::FULL),
            0.0,
            "and an operation authored to pay nothing per tick pays nothing at any rate"
        );
    }

    #[test]
    fn a_hull_with_fewer_free_teams_than_the_capability_commits_is_refused_and_can_recover() {
        let capability = field_repair(2.0, 3);
        let busy = OperationConditions {
            repair_teams_available: 2,
            ..eligible()
        };
        assert_eq!(
            eligibility(Some(&capability), &busy),
            Err(Ineligibility::TeamsUnavailable),
            "an operation that commits three teams cannot run on two"
        );
        assert!(
            Ineligibility::TeamsUnavailable.recoverable(),
            "…and it STALLS rather than failing: a team finishing its internal job frees the \
             operation to start banking, which is exactly the trade the mechanic is for"
        );
        let free = OperationConditions {
            repair_teams_available: 3,
            ..eligible()
        };
        assert_eq!(eligibility(Some(&capability), &free), Ok(()));
    }

    #[test]
    fn a_hull_with_no_repair_teams_at_all_is_not_gated_on_them() {
        // The same reading `power_level` takes for a hull with no grid: the
        // constraint is absent, not failed.
        let bare = OperationConditions {
            repair_teams_available: u8::MAX,
            ..eligible()
        };
        assert_eq!(eligibility(Some(&field_repair(2.0, 0)), &bare), Ok(()));
    }

    // ── AC2/AC3: the shared interrupts ──

    fn rule(cause: InterruptCause, response: InterruptResponse) -> InterruptRule {
        InterruptRule {
            cause,
            region_effect: matches!(cause, InterruptCause::Region)
                .then_some(RegionEffectName::SlowZone),
            response,
            rate_percent: default_slow_rate(),
        }
    }

    #[test]
    fn a_capability_authoring_no_interrupts_behaves_exactly_as_it_did_before_they_existed() {
        let under_fire_in_a_storm = OperationConditions {
            under_attack: true,
            region_effects: vec![RegionEffectName::SlowZone, RegionEffectName::DamageZone],
            ..eligible()
        };
        assert_eq!(
            verdict(Some(&stabilise()), &under_fire_in_a_storm),
            TickVerdict::HOLDING,
            "every capability that shipped before #1027 authors no interrupt rules, and must go \
             on holding at full rate through a fight in a nebula exactly as it used to"
        );
    }

    #[test]
    fn an_authored_attack_rule_pauses_or_fails_as_the_author_said_and_not_by_nature() {
        // The same cause, the same conditions, two different authored answers.
        let under_fire = OperationConditions {
            under_attack: true,
            ..eligible()
        };
        let paused = CapabilityConfig {
            interrupts: vec![rule(InterruptCause::Attack, InterruptResponse::Pause)],
            ..capability(OperationVerb::Tow)
        };
        assert_eq!(
            verdict(Some(&paused), &under_fire).outcome,
            Err(Interruption {
                reason: Ineligibility::UnderAttack,
                terminal: false,
            }),
            "a tow under fire is interrupted, and resumes when the shooting stops"
        );

        let failed = CapabilityConfig {
            interrupts: vec![rule(InterruptCause::Attack, InterruptResponse::Fail)],
            ..capability(OperationVerb::FieldRepair)
        };
        assert_eq!(
            verdict(Some(&failed), &under_fire).outcome,
            Err(Interruption {
                reason: Ineligibility::UnderAttack,
                terminal: true,
            }),
            "…while a repair party working an open hull under fire is called off for good. \
             Terminality is AUTHORED — it is not a property of the word 'attack'"
        );

        let quiet = eligible();
        assert_eq!(
            verdict(Some(&failed), &quiet),
            TickVerdict::HOLDING,
            "and neither rule fires when nobody is shooting"
        );
    }

    #[test]
    fn a_slow_zone_stretches_the_operation_rather_than_cancelling_it() {
        let capability = CapabilityConfig {
            interrupts: vec![InterruptRule {
                rate_percent: 25,
                ..rule(InterruptCause::Region, InterruptResponse::Slow)
            }],
            ..capability(OperationVerb::Tow)
        };
        let in_the_band = OperationConditions {
            region_effects: vec![RegionEffectName::SlowZone],
            ..eligible()
        };
        assert_eq!(
            verdict(Some(&capability), &in_the_band).rate(),
            Some(ProgressRate::percent(25)),
            "a hazard band SLOWS the work — the crew watch the bar crawl rather than watching the \
             operation die, which is the whole storm mechanic"
        );

        let mut hold = OperationHold::start(1, "hulk", &capability, HZ);
        for _ in 0..4 {
            hold.advance(verdict(Some(&capability), &in_the_band));
        }
        assert_eq!(
            hold.elapsed_ticks(),
            1,
            "four ticks at a quarter rate are worth one whole tick of hold, banked exactly — the \
             fractions carry rather than rounding away"
        );
        assert_eq!(
            hold.stalled_ticks(),
            0,
            "and a slowed tick is not a stalled one"
        );
        assert_eq!(
            hold.state(),
            HoldState::Holding,
            "…so it never spends the stall budget, however long the storm lasts"
        );
    }

    #[test]
    fn a_slowed_hold_completes_after_the_stretched_time_and_not_before() {
        let capability = CapabilityConfig {
            duration_secs: 1,
            interrupts: vec![InterruptRule {
                rate_percent: 50,
                ..rule(InterruptCause::Region, InterruptResponse::Slow)
            }],
            ..capability(OperationVerb::Tow)
        };
        let in_the_band = OperationConditions {
            region_effects: vec![RegionEffectName::SlowZone],
            ..eligible()
        };
        let mut hold = OperationHold::start(1, "hulk", &capability, HZ);
        let mut settled = None;
        for _ in 0..119 {
            settled = settled.or(hold.advance(verdict(Some(&capability), &in_the_band)));
        }
        assert_eq!(
            settled, None,
            "119 ticks at half rate is 59.5 ticks of hold against the 60 it needs"
        );
        assert_eq!(
            hold.advance(verdict(Some(&capability), &in_the_band)),
            Some(Settlement::Completed),
            "the 120th finishes it — exactly twice the authored second, which is what 'the storm \
             stretches an operation' has to mean if it means anything"
        );
        assert_eq!(hold.progress(), 1.0);
    }

    // ── Issue #1035: the work stoppage ───────────────────────────────────────
    //
    // The cause is one flag on the conditions and one arm in `fires`; what
    // matters — and what these tests hold the design to — is that the SAME
    // cause produces a refusal for one verb and a slower job for another, out of
    // authored data alone.

    #[test]
    fn a_stoppage_refuses_the_verb_that_authors_fail_and_stretches_the_one_that_authors_slow() {
        // The transfer: nobody will authorise it, so it is over. Both ends can
        // take part — the cargo is aboard and the depot has room — which is
        // what makes the refusal a fact about the PEOPLE rather than about the
        // goods.
        let refused = CapabilityConfig {
            interrupts: vec![rule(InterruptCause::WorkStoppage, InterruptResponse::Fail)],
            ..transfer(TransferDirection::Deliver, 10)
        };
        let struck = OperationConditions {
            target_work_stopped: true,
            ..with_capacities(Some((40, 0)), Some((0, 40)))
        };
        let mut hold = OperationHold::start(1, "depot-b", &refused, HZ);
        assert_eq!(
            hold.advance(verdict(Some(&refused), &struck)),
            Some(Settlement::Failed(Ineligibility::WorkStopped)),
            "the refusal lands on the FIRST tick — a transfer nobody is signing off does not \
             sit there timing out"
        );
        assert_eq!(hold.state(), HoldState::Failed(Ineligibility::WorkStopped));
        assert_eq!(
            hold.state().reason().map(|r| r.string_id()),
            Some("operation.refused.work_stopped"),
            "and it carries a strings.csv id, so the crew are told in words rather than \
             watching a bar that never moves"
        );

        // The field repair: the crews are gone, so the ship's own teams do it
        // the hard way.
        let unassisted = CapabilityConfig {
            duration_secs: 1,
            condition_per_second: 2.0,
            interrupts: vec![InterruptRule {
                rate_percent: 40,
                ..rule(InterruptCause::WorkStoppage, InterruptResponse::Slow)
            }],
            ..capability(OperationVerb::FieldRepair)
        };
        let mut hold = OperationHold::start(2, "skyhook", &unassisted, HZ);
        let struck_structure = OperationConditions {
            target_work_stopped: true,
            ..eligible()
        };
        assert_eq!(
            hold.advance(verdict(Some(&unassisted), &struck_structure)),
            None,
            "the same stoppage, the same tick, and this one is still working"
        );
        assert_eq!(hold.state(), HoldState::Holding);
        assert_eq!(
            hold.rate().as_percent(),
            40,
            "at the rate the CAPABILITY authored — the stoppage carries no number of its own, \
             which is what stops a hard-coded multiplier appearing at the call site"
        );
    }

    #[test]
    fn a_stoppage_that_ends_restores_the_rate_and_the_next_operation_runs() {
        let capability = CapabilityConfig {
            duration_secs: 1,
            condition_per_second: 2.0,
            interrupts: vec![InterruptRule {
                rate_percent: 40,
                ..rule(InterruptCause::WorkStoppage, InterruptResponse::Slow)
            }],
            ..capability(OperationVerb::FieldRepair)
        };
        let struck = OperationConditions {
            target_work_stopped: true,
            ..eligible()
        };
        let mut hold = OperationHold::start(1, "skyhook", &capability, HZ);
        hold.advance(verdict(Some(&capability), &struck));
        assert_eq!(hold.rate().as_percent(), 40);

        // The negotiation lands. Nothing is un-latched, because nothing latched.
        hold.advance(verdict(Some(&capability), &eligible()));
        assert_eq!(
            hold.rate(),
            ProgressRate::FULL,
            "settling the strike restores the assisted rate on the very next tick, with no \
             restoration path to run — the rule simply stops firing"
        );
        assert!(
            hold.condition_payout(ProgressRate::FULL) > hold.condition_payout(ProgressRate::percent(40)),
            "and the repair pays out faster again, because the payout is scaled by the same rate"
        );
    }

    #[test]
    fn a_target_nobody_has_walked_out_on_is_worked_normally() {
        let capability = CapabilityConfig {
            interrupts: vec![rule(InterruptCause::WorkStoppage, InterruptResponse::Fail)],
            ..transfer(TransferDirection::Deliver, 10)
        };
        assert_eq!(
            verdict(
                Some(&capability),
                &with_capacities(Some((40, 0)), Some((0, 40)))
            )
            .outcome,
            Ok(ProgressRate::FULL),
            "a hull that authors the rule and a world with no dispute is the pre-#1035 \
             behaviour exactly"
        );
    }

    #[test]
    fn a_stoppage_rule_may_not_name_a_region_effect() {
        let config = OperationsConfig {
            capabilities: vec![CapabilityConfig {
                interrupts: vec![InterruptRule {
                    cause: InterruptCause::WorkStoppage,
                    region_effect: Some(RegionEffectName::SlowZone),
                    response: InterruptResponse::Fail,
                    rate_percent: default_slow_rate(),
                }],
                ..transfer(TransferDirection::Deliver, 10)
            }],
        };
        let err = config.validate().expect_err("a stoppage reads no band");
        assert!(
            err.contains("work_stoppage"),
            "the load error quotes the cause back to the author: {err}"
        );
    }

    #[test]
    fn a_region_rule_only_fires_for_the_band_it_names() {
        let capability = CapabilityConfig {
            interrupts: vec![rule(InterruptCause::Region, InterruptResponse::Pause)],
            ..capability(OperationVerb::Tow)
        };
        let elsewhere = OperationConditions {
            region_effects: vec![RegionEffectName::CommsJam, RegionEffectName::NebulaFog],
            ..eligible()
        };
        assert_eq!(
            verdict(Some(&capability), &elsewhere),
            TickVerdict::HOLDING,
            "a rule watching for a slow zone must not fire inside a comms jammer — an operation \
             does not care that a band is dangerous, only that it is the band it was told about"
        );
    }

    #[test]
    fn two_rules_that_both_fire_take_the_stricter_one() {
        let attacked_in_a_band = OperationConditions {
            under_attack: true,
            region_effects: vec![RegionEffectName::SlowZone],
            ..eligible()
        };
        // Authored gentlest-first, so an implementation that took the first
        // match would return the slow.
        let capability = CapabilityConfig {
            interrupts: vec![
                rule(InterruptCause::Region, InterruptResponse::Slow),
                rule(InterruptCause::Attack, InterruptResponse::Fail),
            ],
            ..capability(OperationVerb::Tow)
        };
        assert_eq!(
            verdict(Some(&capability), &attacked_in_a_band).outcome,
            Err(Interruption {
                reason: Ineligibility::UnderAttack,
                terminal: true,
            }),
            "the strictest response wins whatever order the rules were authored in — otherwise a \
             capability carrying both would get a different answer depending on which line the \
             designer typed first"
        );
    }

    #[test]
    fn two_slow_rules_that_both_fire_take_the_lower_rate() {
        let both = OperationConditions {
            under_attack: true,
            region_effects: vec![RegionEffectName::DamageZone],
            ..eligible()
        };
        let capability = CapabilityConfig {
            interrupts: vec![
                InterruptRule {
                    rate_percent: 20,
                    ..rule(InterruptCause::Attack, InterruptResponse::Slow)
                },
                InterruptRule {
                    region_effect: Some(RegionEffectName::DamageZone),
                    rate_percent: 60,
                    ..rule(InterruptCause::Region, InterruptResponse::Slow)
                },
            ],
            ..capability(OperationVerb::Tow)
        };
        assert_eq!(
            verdict(Some(&capability), &both).rate(),
            Some(ProgressRate::percent(20)),
            "two things going wrong at once is the harsher reading, not the average of them"
        );
    }

    #[test]
    fn eligibility_is_asked_before_the_interrupts_are() {
        // Out of range AND under fire: the range is the reason, because an
        // operation that is not running is not running at a reduced rate.
        let capability = CapabilityConfig {
            interrupts: vec![rule(InterruptCause::Attack, InterruptResponse::Fail)],
            ..stabilise()
        };
        let conditions = OperationConditions {
            distance: 5_000.0,
            under_attack: true,
            ..eligible()
        };
        assert_eq!(
            verdict(Some(&capability), &conditions).outcome,
            Err(Interruption {
                reason: Ineligibility::OutOfRange,
                terminal: false,
            }),
            "an out-of-range operation must not be FAILED by an interrupt rule it was never close \
             enough to meet — the crew fly back and carry on"
        );
    }

    #[test]
    fn power_loss_stays_an_eligibility_condition_rather_than_becoming_a_second_interrupt() {
        // The issue names power loss as a shared interrupt. It already was one,
        // and giving it a rule as well would let a capability author two
        // different answers to the same question.
        assert!(
            !InterruptRule::fires(
                &rule(InterruptCause::Attack, InterruptResponse::Fail),
                &OperationConditions {
                    power_level: 0,
                    ..eligible()
                }
            ),
            "no interrupt cause reads the power grid"
        );
        let brownout = OperationConditions {
            power_level: 0,
            ..eligible()
        };
        assert_eq!(
            verdict(Some(&stabilise()), &brownout).outcome,
            Err(Interruption {
                reason: Ineligibility::InsufficientPower,
                terminal: false,
            }),
            "…and losing the allocation pauses every verb, with no rule needed, because it is \
             tested against the live grid every tick"
        );
    }

    #[test]
    fn a_paused_interrupt_spends_the_stall_budget_and_a_slowed_one_does_not() {
        let paused = CapabilityConfig {
            stall_limit_secs: Some(1),
            interrupts: vec![rule(InterruptCause::Attack, InterruptResponse::Pause)],
            ..capability(OperationVerb::Tow)
        };
        let under_fire = OperationConditions {
            under_attack: true,
            ..eligible()
        };
        let mut hold = OperationHold::start(1, "hulk", &paused, HZ);
        let mut settled = None;
        for _ in 0..61 {
            settled = settled.or(hold.advance(verdict(Some(&paused), &under_fire)));
        }
        assert_eq!(
            settled,
            Some(Settlement::Failed(Ineligibility::UnderAttack)),
            "a PAUSE is a stall, so an authored stall budget ends a hold that is paused too long \
             — the two interrupt mechanisms compose rather than sitting side by side"
        );

        let slowed = CapabilityConfig {
            stall_limit_secs: Some(1),
            interrupts: vec![InterruptRule {
                rate_percent: 10,
                ..rule(InterruptCause::Attack, InterruptResponse::Slow)
            }],
            ..capability(OperationVerb::Tow)
        };
        let mut crawling = OperationHold::start(1, "hulk", &slowed, HZ);
        for _ in 0..600 {
            crawling.advance(verdict(Some(&slowed), &under_fire));
        }
        assert!(
            !crawling.is_settled(),
            "…while a SLOW never spends it, however long the band lasts: the operation is being \
             held, just badly"
        );
        assert_eq!(crawling.stalled_ticks(), 0);
    }

    #[test]
    fn an_interrupt_rule_that_could_never_fire_or_never_mean_anything_is_refused_at_load() {
        let cases: Vec<(InterruptRule, &str)> = vec![
            (
                InterruptRule {
                    cause: InterruptCause::Region,
                    region_effect: None,
                    response: InterruptResponse::Pause,
                    rate_percent: 50,
                },
                "region_effect",
            ),
            (
                InterruptRule {
                    cause: InterruptCause::Attack,
                    region_effect: Some(RegionEffectName::SlowZone),
                    response: InterruptResponse::Pause,
                    rate_percent: 50,
                },
                "region_effect",
            ),
            (
                InterruptRule {
                    cause: InterruptCause::Attack,
                    region_effect: None,
                    response: InterruptResponse::Slow,
                    rate_percent: 0,
                },
                "rate_percent",
            ),
            (
                InterruptRule {
                    cause: InterruptCause::Attack,
                    region_effect: None,
                    response: InterruptResponse::Slow,
                    rate_percent: 250,
                },
                "rate_percent",
            ),
        ];
        for (interrupt, field) in cases {
            let config = OperationsConfig {
                capabilities: vec![CapabilityConfig {
                    interrupts: vec![interrupt],
                    ..capability(OperationVerb::Tow)
                }],
            };
            let err = config
                .validate()
                .expect_err("this rule cannot mean what it says");
            assert!(err.contains(field), "the error must name {field}: {err}");
        }
    }

    #[test]
    fn a_rate_above_normal_is_clamped_rather_than_honoured() {
        assert_eq!(
            ProgressRate::percent(400),
            ProgressRate::FULL,
            "an interrupt makes an operation harder; a hazard band that sped one up would be a \
             different feature, and validate() refuses the authoring anyway"
        );
        assert!(ProgressRate::FULL.is_full());
        assert!(!ProgressRate::percent(99).is_full());
    }

    #[test]
    fn every_region_effect_has_a_name_an_interrupt_rule_can_author() {
        assert_eq!(
            RegionEffectName::ALL.len(),
            7,
            "the whole region vocabulary is authorable as an interrupt cause"
        );
        for name in RegionEffectName::ALL {
            let authored = format!(
                "[[capability]]\nverb = \"tow\"\n\n[[capability.interrupt]]\ncause = \
                 \"region\"\nregion_effect = \"{}\"\nresponse = \"pause\"\n",
                name.as_str()
            );
            let config: OperationsConfig = toml::from_str(&authored)
                .expect("every name round-trips through its authored spelling");
            assert_eq!(
                config.capabilities[0].interrupts[0].region_effect,
                Some(*name)
            );
        }
    }

    #[test]
    fn every_refusal_reason_carries_a_wire_spelling_and_a_strings_id() {
        for reason in [
            Ineligibility::NotCapable,
            Ineligibility::TargetGone,
            Ineligibility::TargetNotApplicable,
            Ineligibility::OutOfRange,
            Ineligibility::InsufficientPower,
            Ineligibility::Separated,
            Ineligibility::CapacityUnavailable,
            Ineligibility::TeamsUnavailable,
            Ineligibility::UnderAttack,
            Ineligibility::HazardBand,
        ] {
            assert!(
                reason.string_id().starts_with("operation.refused."),
                "{reason:?} must resolve through strings.csv, not read as English on the wire"
            );
            assert!(!reason.as_str().is_empty());
        }
    }

    // ── The authored shape a designer writes ──

    #[test]
    fn the_full_authored_capability_shape_parses_with_its_interrupts() {
        let config: OperationsConfig = toml::from_str(
            r#"
[[capability]]
verb = "field_repair"
range = 300.0
duration_secs = 60
condition_per_second = 2.5
repair_teams = 2
stall_limit_secs = 20

[[capability.interrupt]]
cause = "attack"
response = "fail"

[[capability.interrupt]]
cause = "region"
region_effect = "damage_zone"
response = "slow"
rate_percent = 30

[[capability]]
verb = "tow"
range = 150.0
duration_secs = 10
tow_offset = [0.0, 0.0, -120.0]

[[capability]]
verb = "escort"
range = 800.0
duration_secs = 90
separation_limit = 4000.0

[[capability]]
verb = "transfer"
range = 250.0
duration_secs = 45

[capability.transfer]
capacity = "depot_transfer_throughput"
amount = 12
direction = "deliver"
"#,
        )
        .expect("the authored shape parses");
        config.validate().expect("and validates");

        let repair = config
            .capability(OperationVerb::FieldRepair)
            .expect("field_repair resolves");
        assert_eq!(repair.condition_per_second, 2.5);
        assert_eq!(repair.repair_teams, 2);
        assert_eq!(repair.interrupts.len(), 2);
        assert_eq!(repair.interrupts[0].response, InterruptResponse::Fail);
        assert_eq!(
            repair.interrupts[1].region_effect,
            Some(RegionEffectName::DamageZone)
        );
        assert_eq!(repair.interrupts[1].rate_percent, 30);

        assert_eq!(
            config.capability(OperationVerb::Tow).unwrap().tow_offset,
            [0.0, 0.0, -120.0]
        );
        assert_eq!(
            config
                .capability(OperationVerb::Escort)
                .unwrap()
                .separation_limit,
            Some(4_000.0)
        );
        let transfer = config
            .capability(OperationVerb::Transfer)
            .unwrap()
            .transfer
            .as_ref()
            .expect("the transfer terms parse");
        assert_eq!(transfer.capacity, "depot_transfer_throughput");
        assert_eq!(transfer.amount, 12);
        assert_eq!(transfer.direction, TransferDirection::Deliver);
    }

    #[test]
    fn one_hull_can_author_all_five_verbs_at_once() {
        let config = OperationsConfig {
            capabilities: OperationVerb::ALL
                .iter()
                .map(|verb| match verb {
                    OperationVerb::Transfer => transfer(TransferDirection::Deliver, 5),
                    other => capability(*other),
                })
                .collect(),
        };
        config
            .validate()
            .expect("a tender that can do everything is legal");
        for verb in OperationVerb::ALL {
            assert!(
                config.capability(*verb).is_some(),
                "{} resolves off the one table",
                verb.as_str()
            );
        }
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

    /// **Issue #1027.** A hold caught mid-slow comes back mid-slow — including
    /// the fraction of a tick it had banked.
    #[test]
    fn a_slowed_hold_round_trips_with_its_sub_tick_progress_and_its_payout_rate() {
        let capability = CapabilityConfig {
            condition_per_second: 6.0,
            interrupts: vec![InterruptRule {
                rate_percent: 30,
                ..rule(InterruptCause::Region, InterruptResponse::Slow)
            }],
            ..capability(OperationVerb::FieldRepair)
        };
        let in_the_band = OperationConditions {
            region_effects: vec![RegionEffectName::SlowZone],
            ..eligible()
        };
        let mut hold = OperationHold::start(7, "skyhook", &capability, HZ);
        for _ in 0..7 {
            hold.advance(verdict(Some(&capability), &in_the_band));
        }
        assert_eq!(
            (hold.elapsed_ticks(), hold.rate_remainder()),
            (2, 10),
            "precondition: seven ticks at 30 % is two whole ticks and a tenth"
        );

        let encoded = serde_json::to_string(&hold).expect("a slowed hold serialises");
        let decoded: OperationHold = serde_json::from_str(&encoded).expect("and comes back");
        assert_eq!(
            decoded, hold,
            "the sub-tick remainder, the per-tick payout and the current rate all have to come \
             back with the banked ticks. Restoring the whole ticks alone would quietly discard \
             part of a second of the crew's work on every save, and a resumed field-repair would \
             pay at a rate the storm is no longer imposing."
        );
        assert_eq!(decoded.rate(), ProgressRate::percent(30));
        assert!((decoded.condition_per_tick() - 0.1).abs() < 1e-6);
    }

    /// **Issue #1027.** A hold that has never been slowed encodes exactly the
    /// bytes it did before slowing existed.
    #[test]
    fn an_unslowed_hold_carries_no_extra_fields_into_a_save() {
        let mut hold = fresh_hold();
        run_eligible(&mut hold, 50);
        let encoded = serde_json::to_string(&hold).expect("a hold serialises");
        assert!(
            !encoded.contains("rate_remainder") && !encoded.contains("condition_per_tick"),
            "the sub-tick fields skip when they are zero, so a save from a mission that met no \
             hazard band is byte-identical to one written before #1027: {encoded}"
        );
        let decoded: OperationHold =
            serde_json::from_str(&encoded).expect("and an older save decodes");
        assert_eq!(decoded, hold);
    }
}
