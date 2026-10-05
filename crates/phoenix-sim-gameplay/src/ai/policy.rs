//! Pure, Bevy-free AI fine-system policy runtime (issues #775, #882).
//!
//! An AI-capable fine system declares an *inline policy*: a set of prioritised
//! reactive rules, each bound to a named *output channel*, guarded by a `when`
//! predicate over typed facts, read-only world flags/counters, and authored
//! named parameters. For each output channel the runtime resolves the single
//! highest-priority rule whose guard fires and emits that channel's typed verb.
//!
//! ## Two paths, one spine
//!
//! **Stateless (issue #775) — the default, and what all twelve Group A hosts
//! use.** No private memory, no lifecycle state: the decision is a pure
//! function of the immutable per-tick snapshot handed in.
//! [`AiPolicy::resolve_channel`] is that path and its behaviour is frozen.
//!
//! **Stateful (issue #882) — strictly opt-in.** A policy MAY additionally
//! declare an [`AiPolicyMachine`]: an initial state and a set of named
//! [`AiPolicyState`]s, each with its own continuous rules and its own
//! explicitly prioritised [`AiPolicyTransition`]s. Inside a state the
//! resolution rules are *identical* — same channel scan, same
//! strictly-greater-priority/earliest-authored tie-break — because both paths
//! call the same [`best_in`] helper. At most ONE transition fires per eligible
//! tick: [`AiPolicy::resolve_transition`] returns an `Option`, so that is a
//! property of the evaluator rather than of host discipline.
//!
//! A policy whose `machine` is `None` never touches any of this: no state, no
//! memory, no transition scan. The stateless twelve are byte-identical.
//!
//! ## What the machine deliberately is NOT
//!
//! Private memory and state time are read through the `memory(name)` /
//! `state_time` atoms off an [`AiPolicyMemory`] bag the OWNING fine system seeds from
//! its OWN per-fine-system state component. There is no ship-wide state
//! machine and no shared memory: a sibling system's evaluation call simply
//! never populates this bag, so cross-system reads are structurally impossible
//! rather than merely discouraged.
//!
//! State time is advanced by the host from the shared AI tick cadence, never a
//! per-frame clock or a policy-owned `Timer` (AGENTS.md #7).
//!
//! This module owns the *typed* policy (already parsed + validated); the TOML
//! schema and content validation live in `entities::config`, and the predicate
//! grammar lives in `world::flags`.

use crate::world::flags::{
    AiFacts, AiParams, AiPolicyMemory, FlagStore, HistoryRef, HistorySpec, Predicate,
};

/// The typed output a fired rule applies to its channel.
///
/// Verbs are restricted to a closed set per system kind so AI output stays on
/// the same admitted-command path a human uses (AGENTS.md rule #6). The
/// stateless Captain Red Alert slice needs exactly one verb.
///
/// ## Mode verbs for continuous actuators (issue #779)
///
/// The Helm's Engines and Steering are *continuous* actuators: the scalar
/// thrust/yaw comes from geometry (`DesiredMotion.desired_velocity_local` /
/// `desired_facing_local`, computed by the planner), not from a fixed payload a
/// designer can author. Putting that scalar in the verb would duplicate the
/// planner and smuggle geometry into this Bevy-free policy module. Instead the
/// policy selects a reactive *mode* per channel — a value-less verb that says
/// *whether* to actuate this tick — and the host reads the continuous magnitude
/// from the planner fact when the mode says "actuate". `resolve_channel`
/// returning `None` (no rule fired, or an explicit idle) means "hold": the host
/// emits nothing and the actuator coasts on its last input.
#[derive(Clone, Debug, PartialEq)]
pub enum AiPolicyVerb {
    /// Drive the ship's Red Alert to `active` (the `red_alert` channel).
    SetRedAlert(bool),
    /// Actuate the planner's desired longitudinal travel this tick (the
    /// `longitudinal` channel of the Engines fine system). A mode verb: the
    /// continuous forward/reverse magnitude is decoded from the shared
    /// `DesiredMotion.desired_velocity_local`, not carried here.
    ActuateDesiredTravel,
    /// Actuate the planner's desired facing this tick (the `yaw` channel of the
    /// Steering fine system). A mode verb: the continuous yaw magnitude is
    /// decoded from the shared `DesiredMotion.desired_facing_local`, not carried
    /// here.
    ActuateDesiredFacing,
    /// Fly the heading the host FROZE at the last committed transition (the
    /// `yaw` channel's second mode verb, issue #883).
    ///
    /// The sibling of [`AiPolicyVerb::ActuateDesiredFacing`], and the difference
    /// between them is the whole point of the fly-through doctrine.
    /// `actuate_desired_facing` means "solve the facing against the world this
    /// tick" — a moving target keeps being tracked. `hold_committed_heading`
    /// means "the facing solution is CLOSED: fly the heading recorded in this
    /// system's own `memory(escape_heading_rad)`".
    ///
    /// Note this is emphatically NOT the same as resolving to `None` ("hold").
    /// Holding the channel holds the last admitted *steering command*, and a
    /// non-zero yaw at the merge instant would keep the ship turning for ever.
    /// This verb holds the *heading*, which is what "commits to the current
    /// outward heading" actually requires.
    ///
    /// Like every other mode verb it carries no magnitude: the frozen heading is
    /// host-written private memory, and the host still feeds it through the
    /// shared motion planner, so hazard avoidance composes onto the escape
    /// exactly as it composes onto any other travel solution.
    HoldCommittedHeading,
    /// Fly the shield-recovery standoff orbit (the `yaw` channel's THIRD mode
    /// verb, issue #788).
    ///
    /// Says the facing solution is neither "track the target" nor "fly a frozen
    /// heading" but "hold a ring around the target": the host solves a tangent
    /// of the authored safe radius, bent toward or away from it in proportion to
    /// the current radial error, so the ship spirals onto the ring instead of
    /// stopping short or running away for ever.
    ///
    /// Value-less like every other mode verb. The ring's radius is derived
    /// host-side from the TARGET's own direct-fire reach plus the hull's
    /// authored margin — a number that cannot be authored as a constant because
    /// it depends on which ship is being fought — and the direction of travel is
    /// host-written private memory, drawn once per recovery from a seeded
    /// composite key.
    HoldRecoveryOrbit,
    /// Turn back onto the target to begin another pass (the `yaw` channel's
    /// FOURTH mode verb, issue #788).
    ///
    /// The re-entry pivot. Steering tracks the target exactly as
    /// [`AiPolicyVerb::ActuateDesiredFacing`] does; what differs is the
    /// *throttle* the host pairs with it — the authored re-engage fraction
    /// rather than the approach fraction — so a hull can author a pivot flown on
    /// cut thrust before the next run's acceleration begins.
    ///
    /// It is a distinct verb rather than a reuse of `actuate_desired_facing`
    /// precisely so the host can tell the two apart: the leg is read off the
    /// authored verb and never off a state name, so a designer stays free to
    /// call the states whatever they like.
    PivotToReengage,
    /// Fly the continuous COMBAT broadside orbit (the `yaw` channel's FIFTH mode
    /// verb, issue #790).
    ///
    /// The same ring geometry [`AiPolicyVerb::HoldRecoveryOrbit`] flies — a
    /// tangent of a ring around the target, bent toward or away from it in
    /// proportion to the fractional radial error, so the hull spirals onto the
    /// ring from either side — asked for a different reason, and that difference
    /// is why it is its own verb rather than a reuse.
    ///
    /// `hold_recovery_orbit` is a BREAK-OFF: the host only publishes it when the
    /// hull authors a complete shield-recovery parameter set, and the ring's
    /// radius is derived from the TARGET's own direct-fire reach plus a margin —
    /// a standoff distance, chosen to sit outside the other ship's guns while
    /// shields come back. This verb is the opposite intent: it is how the hull
    /// fights, at a range the DESIGNER authored precisely because the whole point
    /// is to keep the enemy inside this hull's own weapon envelope. Deriving that
    /// range from the enemy's reach, or gating it on a shield doctrine the hull
    /// need not have, would both be wrong.
    ///
    /// Value-less like every other mode verb: the ring radius, the throttle and
    /// the spiral gain are authored `param`s the host reads off the Steering
    /// policy, and the circulation direction is host-written private memory drawn
    /// once per engagement from a seeded composite key.
    HoldCombatOrbit,
    /// Hold the bow on the target for a torpedo opportunity (the `yaw` channel's
    /// SIXTH mode verb, issue #791).
    ///
    /// Tracks the target's LIVE position every tick, exactly as
    /// [`AiPolicyVerb::ActuateDesiredFacing`] and
    /// [`AiPolicyVerb::PivotToReengage`] do; what differs is the *throttle* the
    /// host pairs with it — the authored `torpedo_bearing_speed` — so a hull can
    /// author `0.0` and cut thrust while it lines a fixed forward tube up on a
    /// shield that has just gone down.
    ///
    /// It is its own verb rather than a reuse of `pivot_to_reengage` because the
    /// two are gated on different authoring. The re-engage pivot is one leg of
    /// the shield-RECOVERY doctrine and the host only publishes it when the hull
    /// authors the complete recovery parameter set — six scalars all describing a
    /// standoff ring derived from the *target's* reach, none of which a
    /// torpedo-opportunity hull has any business inventing (AGENTS.md #11). This
    /// verb carries one authored scalar of its own and nothing else.
    ///
    /// Value-less like every other mode verb: which shield is down, which arc the
    /// tubes cover and whether a salvo is still in flight are all host readings,
    /// never authored constants.
    HoldTorpedoBearing,
    /// Hold the artillery firing position (the `yaw` channel's SEVENTH mode verb,
    /// issue #792).
    ///
    /// The battleship leg. Says the facing solution is neither "track the target"
    /// nor "hold a ring" nor "point at where the target IS" but "point at where
    /// the target WILL BE when my bolt arrives" — a predictive intercept
    /// solution, re-solved every tick from the target's reconstructed velocity
    /// and the artillery bolt's own authored flight speed — while the
    /// TRANSLATIONAL axes hold station on the authored hold throttle.
    ///
    /// It is its own verb rather than a reuse of [`AiPolicyVerb::PivotToReengage`]
    /// or [`AiPolicyVerb::HoldTorpedoBearing`], and the reason is different for
    /// each. The re-engage pivot's host gate is the six shield-RECOVERY scalars —
    /// a standoff ring derived from the *target's* reach, which an artillery
    /// platform has no business inventing to borrow a turn (AGENTS.md #11). The
    /// bow hold tracks the target's LIVE position with no lead at all, which is
    /// the right answer for a fixed tube settling on a shield gap a few units
    /// away and the wrong one for a slow bolt with several seconds of flight
    /// time: at artillery range "where it is" and "where it will be" are
    /// different bearings, and firing at the first is firing at nothing.
    ///
    /// Value-less like every other mode verb: the hold throttle and the range
    /// band are authored Steering `param`s, and the lead speed is a host reading
    /// of the hull's own longest-reaching bolt — never an authored duplicate of
    /// it.
    HoldArtilleryPosition,
    /// Actuate the lateral-thrust axis this tick (the `lateral` channel of the
    /// Lateral Thrust fine system, issue #780). A mode verb: the continuous
    /// starboard/port magnitude comes from the shared hazard assessment weighted
    /// by the hull's authored `lateral_hazard_sensitivity` (or the docking
    /// translation), never from the verb.
    ActuateLateralThrust,
    /// Actuate the bounded/full-3D vertical-thrust axis this tick (the `vertical`
    /// channel of the Vertical Thrust fine system, issue #780). A mode verb: the
    /// continuous climb/return magnitude comes from the shared moving-hazard
    /// threat and the authored `VerticalMovementMode` ceiling / return rate, never
    /// from the verb.
    ActuateVerticalThrust,
    /// Engage the impulse drive this tick (the `impulse` channel of the Impulse
    /// fine system, issue #780). A mode verb: whether the host actually emits
    /// `StartImpulseCharge`/`CancelImpulse` still follows the authored doctrine
    /// `use_impulse` and the `decide_impulse` geometry — the verb only says the
    /// policy permits impulse manoeuvres this tick.
    EngageImpulse,
    /// Engage the boost drive this tick (the `boost` channel of the Boost fine
    /// system, issue #780). A mode verb: its presence tells the host to drive the
    /// ship's boost active via the same admitted `SetBoost` a human uses; its
    /// absence ("hold"/idle) leaves boost as it is.
    EngageBoost,
    /// Open fire from this phaser bank this tick (the `phaser_fire` channel of a
    /// phaser bank fine system, issue #781). A value-less action verb: the target
    /// (the ship's authoritative combat lock), the firing bank, and the beam
    /// frequency all come from the host context, not the verb. Its presence tells
    /// the host to emit the same admitted `FirePhaser` a human does; its absence
    /// ("hold"/idle) holds this bank's fire.
    FirePhaser,
    /// Open fire from this blaster bank this tick (the `blaster_fire` channel of a
    /// blaster bank fine system, issue #781). A value-less action verb, the
    /// blaster twin of [`AiPolicyVerb::FirePhaser`]: the target and bank come from
    /// the host context. Its presence tells the host to emit the same admitted
    /// `ChargeBlasterStart` a human does; its absence ("hold"/idle) holds this
    /// bank's volley.
    FireBlaster,
    /// Load a round into this torpedo tube this tick (the `torpedo_load` channel
    /// of a torpedo tube fine system, issue #782). A value-less action verb: the
    /// tube, its authored volley target, and the shared magazine all come from the
    /// host context, never the verb. Its presence tells the host to emit the same
    /// admitted `SetTorpedoVolleyTarget` a Tactical player does (which the auto-load
    /// path turns into a `ClaimTorpedoRound` against the shared magazine); its
    /// absence ("hold"/idle) leaves the tube's volley target where it is.
    LoadTorpedo,
    /// Launch an already-loaded round from this torpedo tube this tick (the
    /// `torpedo_launch` channel of a torpedo tube fine system, issue #782). A
    /// value-less action verb, the launch twin of [`AiPolicyVerb::LoadTorpedo`]:
    /// the tube and the ship's authoritative combat lock come from the host
    /// context. Its presence tells the host to emit the same admitted `FireTorpedo`
    /// a human does; its absence ("hold"/idle) holds this tube's launch. Launch
    /// consumes ammunition reserved earlier at load time — it never touches the
    /// magazine counter itself.
    LaunchTorpedo,
    /// Grant a pending magazine round claim this tick (the `torpedo_magazine_grant`
    /// channel of the shared torpedo magazine fine system, issue #782). A
    /// value-less action verb resolved inside the single magazine consumer right
    /// before the authoritative `claim_magazine_round`: its presence permits the
    /// reservation to proceed, its absence ("hold"/idle) refuses the claim without
    /// decrementing the counter. The offline gate remains the hard authority; this
    /// policy is a data-authored arbiter layered on top, and never becomes a second
    /// writer of `torpedoes_remaining`.
    GrantTorpedoRound,
    /// Spend an already-authorised round on THIS launch rather than holding it
    /// for later in the mission (the `torpedo_conservation` channel of the
    /// shared torpedo magazine fine system, issue #943).
    ///
    /// A value-less action verb, and the only one resolved on BOTH origins'
    /// behalf: its channel is read inside `handle_fire_torpedo`, the single
    /// consumer of an admitted `FireTorpedo`, so a human Tactical operator and
    /// an AI backfill meet the same guard and the resolve cannot see which one
    /// asked (AGENTS.md #6). Its absence ("hold"/idle) drops this launch without
    /// touching the magazine or unloading the tube — the round stays where it is
    /// and the decision is offered again next tick.
    ///
    /// Every quantity it reasons about is a host reading (magazine level, the
    /// world's remaining mission threat, the ship's own objective count); the
    /// reserve those readings are measured against is an authored `param`.
    ReleaseTorpedo,
    /// Present the PHASER banks: name that family as the one this ship turns to
    /// bring to bear (an `arc_bearing_*` channel of the Weapons doctrine fine
    /// system, issue #956).
    ///
    /// ## Why the channel is a RANK and the verb is a family
    ///
    /// Weapons asks Helm to turn (a channel-3 `ArcBearingRequest`) when the
    /// target is in range of a family but outside every one of that family's
    /// arcs. Exactly one request may be active, so *which* family is worth
    /// turning for has to be decided — and until #956 that decision was a Rust
    /// array, `[Phasers, Blasters, Torpedoes]`, with a doc comment calling it
    /// "structural, not a gameplay value". Which gun a ship manoeuvres to
    /// present is a tactical choice, so it is authored now.
    ///
    /// It is an ORDER rather than a single choice because a family may not
    /// qualify (every bank shot offline, every tube empty, the target already
    /// bearing), and the ship should then turn for the next one rather than fly
    /// on with nothing to say. The three `arc_bearing_first` /
    /// `arc_bearing_second` / `arc_bearing_third` channels are that order, one
    /// rank per channel: each is a single decision in the ordinary sense — "what
    /// do I present FIRST?" — with its own guards, so a doctrine can reorder
    /// itself as the fight changes. The host resolves the three in rank order and
    /// drops repeats, so a doctrine that names the same family twice is
    /// harmless, and one that leaves a rank unauthored simply has a shorter
    /// order.
    ///
    /// Value-less like the fire verbs: the arcs, ranges and geometry the request
    /// carries are host readings of the ship's own emitters.
    BringPhasersToBear,
    /// Present the BLASTER banks. The blaster twin of
    /// [`AiPolicyVerb::BringPhasersToBear`]; see it for the whole model.
    BringBlastersToBear,
    /// Present the TORPEDO tubes. The torpedo twin of
    /// [`AiPolicyVerb::BringPhasersToBear`]; see it for the whole model.
    ///
    /// This is the verb the issue's worked example is authored with: a hull that
    /// fights with fixed bow tubes ranks torpedoes first *while the target's
    /// striking arc is down* and falls back to its beams otherwise, which is a
    /// doctrine rather than a fixed ordering.
    BringTorpedoesToBear,
    /// Focus a shield arc this tick (the `shield_focus` channel of the Shields
    /// fine system, issue #783). A value-less action verb, the shields twin of
    /// [`AiPolicyVerb::FirePhaser`]: *which* of the four arcs is focused is NOT
    /// carried here — it comes from the host's retained arc-ranking kernel
    /// (`tick_shield_focus_ai`, damage-concentration primary with health-imbalance
    /// fallback) reading the authored windows/thresholds from this policy's
    /// `param` map. Its presence tells the host to run that kernel and emit the
    /// same admitted `SetShieldArcFocus` a human Shields operator does; its
    /// absence ("hold"/idle) leaves the current focus where it is.
    FocusShieldArc,
    /// Set one power group's allocation to an ABSOLUTE target level (the power
    /// group's own channel of the Power reactor fine system, issue #784). This
    /// is the FIRST verb that carries a magnitude in its payload: every prior
    /// verb was either value-less or the boolean `SetRedAlert`. The channel is
    /// the power group id, so a per-group rule emits the level *its* group
    /// should hold. The host emits the same admitted `SetPowerGroupAllocation`
    /// a human Power operator does; the applier re-clamps to the per-group
    /// `[1, max]` range and the ship-wide `total <= 8` cap, so an absolute
    /// level is safe and idempotent (the host skips the emit when
    /// `level == current`). Its absence ("hold"/idle) leaves the group where it
    /// is — the brownout-avoidance reserve guard is authored in the rule's
    /// `when`, not here.
    SetPowerGroupAllocation(u8),
    /// Answer the Comms dialogue currently being resolved with the authored
    /// response INDEX (the `comms_respond` channel of the Comms fine system,
    /// issue #786). The SECOND value-carrying verb, after
    /// [`AiPolicyVerb::SetPowerGroupAllocation`] — and value-carrying for the
    /// same reason: the choice is a position in a fixed, small, index-addressed
    /// set (`ActiveDialogue.current_node.responses`), so it is authored data,
    /// not host geometry.
    ///
    /// WHICH message is being answered is NOT carried here: the host resolves
    /// this channel once per open dialogue awaiting a response and supplies the
    /// message id from its own context, exactly as the phaser verbs take their
    /// bank from the host. The host emits the same admitted `RespondToMessage`
    /// a human Comms officer sends, so the shared `handle_respond_to_message`
    /// router fires the response's trigger actions and advances its follow-up
    /// identically for both. Its absence ("hold"/idle) leaves the dialogue open
    /// this tick — there is no "decline" index.
    RespondToMessage(u8),
}

/// One inline stateless policy rule.
#[derive(Clone, Debug, PartialEq)]
pub struct AiPolicyRule {
    /// Higher wins within a channel. Ties resolve to the earliest-authored
    /// rule so evaluation is deterministic regardless of container order.
    pub priority: i32,
    /// The output channel this rule contributes to (e.g. `"red_alert"`).
    pub channel: String,
    /// Guard predicate; the rule "fires" when this evaluates `true`.
    pub when: Predicate,
    /// The typed output applied when this rule wins its channel.
    pub verb: AiPolicyVerb,
}

/// One explicitly prioritised transition out of the enclosing state (issue
/// #882).
///
/// `from` is not a field: a transition is authored *inside* the state it leaves,
/// so the source is the enclosing [`AiPolicyState`] and cannot drift out of
/// sync with it. Priority uses the same strictly-greater /
/// earliest-authored-wins tie-break as channel rules, so "which of two eligible
/// transitions fires" has exactly one answer and it does not depend on
/// container iteration order.
#[derive(Clone, Debug, PartialEq)]
pub struct AiPolicyTransition {
    /// Higher wins; ties resolve to the earliest-authored transition.
    pub priority: i32,
    /// The state id this transition enters.
    pub to: String,
    /// Guard predicate; the transition is eligible when this evaluates `true`.
    pub when: Predicate,
}

/// One named state of an [`AiPolicyMachine`] (issue #882).
///
/// A state carries its own *continuous* rules — resolved exactly like the
/// stateless top-level rule set, on the same channels, with the same
/// priority semantics — and its own outgoing transitions.
#[derive(Clone, Debug, PartialEq)]
pub struct AiPolicyState {
    /// Unique id within the machine, referenced by `initial` and by every
    /// transition's `to`.
    pub id: String,
    /// Continuous per-channel rules that apply while this state is current.
    pub rules: Vec<AiPolicyRule>,
    /// Outgoing transitions, evaluated once per eligible tick.
    pub transitions: Vec<AiPolicyTransition>,
    /// Whether the leg this state flies yields its solved facing to a
    /// channel-3 `ArcBearingRequest` (issue #918).
    ///
    /// `true` — the authored default, and the value every leg carried before
    /// #918 — means a weapon family that cannot bear may take the facing: the
    /// request replaces the leg's solved steering with a bow-on tracking
    /// solution, which is the #673-#684 behaviour and the right answer for a
    /// leg that is merely travelling.
    ///
    /// `false` says this leg has CHOSEN a heading and the choice is the whole
    /// point of it — a ring tangent, a frozen escape heading — so a request
    /// that arrives while it is flown is declined rather than obeyed.
    ///
    /// It lives on the STATE rather than on the rule because the unit a
    /// designer commits to a heading in is the leg, and a state is what a leg
    /// is. It follows that a STATELESS policy has no legs and therefore always
    /// yields, which is what [`AiPolicy::leg_yields_to_arc_requests`] answers
    /// for a helm with no authored doctrine at all.
    pub yields_to_arc_requests: bool,
}

/// The opt-in state machine half of a policy (issue #882).
///
/// Held as a single `Option` on [`AiPolicy`] rather than as several sibling
/// fields, so a stateless policy is exactly "`machine: None`" — one condition
/// to test, one line to author at the many exhaustive construction sites, and
/// no way to end up with half a machine.
#[derive(Clone, Debug, PartialEq)]
pub struct AiPolicyMachine {
    /// The state entered on reset (AI gains control, or an unavailable system
    /// recovers). Validated at content load to name a declared state.
    pub initial: String,
    /// The authored initial values of this policy's typed private memory. Held
    /// on the MACHINE rather than on [`AiPolicy`] because private memory only
    /// exists for a stateful policy — content validation rejects a
    /// `memory(...)` reference in a stateless one — and because it makes
    /// [`AiPolicyRuntimeState::reset`] self-contained: the host can rebuild a
    /// clean runtime state from the policy alone, with no second lookup into
    /// authored TOML at reset time.
    pub initial_memory: AiPolicyMemory,
    /// The declared states, in authored order.
    pub states: Vec<AiPolicyState>,
}

impl AiPolicyMachine {
    /// Look up a state by id.
    pub fn state(&self, id: &str) -> Option<&AiPolicyState> {
        self.states.iter().find(|s| s.id == id)
    }
}

/// An inline fine-system policy: authored parameters plus the rule set (or an
/// explicit idle declaration), and OPTIONALLY an [`AiPolicyMachine`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiPolicy {
    /// Authored named parameters referenced by rule guards.
    pub params: AiParams,
    /// Prioritised reactive rules across all channels.
    pub rules: Vec<AiPolicyRule>,
    /// Explicit idle: the system declares it takes no AI action. Distinct from
    /// an empty rule set so "silence" (an ambiguous declaration) is rejected by
    /// content validation while a deliberate idle is accepted.
    pub idle: bool,
    /// The OPTIONAL state machine (issue #882). `None` — the shape every
    /// shipped policy has — means this is a stateless policy and none of the
    /// state/transition/memory code is ever entered.
    pub machine: Option<AiPolicyMachine>,
}

/// Scan one rule slice for the winning verb on `channel` (issue #882).
///
/// Extracted verbatim from [`AiPolicy::resolve_channel`]'s original loop so the
/// stateless path and the per-state path resolve channels through ONE
/// implementation: identical guard evaluation, identical strictly-greater
/// priority comparison, identical earliest-authored tie-break. The stateless
/// path passes an empty [`AiPolicyMemory`] (its policies may not reference private
/// state at all — content validation rejects that), which makes this call
/// behaviourally indistinguishable from the pre-#882 loop.
fn best_in<'a>(
    rules: &'a [AiPolicyRule],
    channel: &str,
    facts: &AiFacts,
    memory: &AiPolicyMemory,
    params: &AiParams,
    flags: &[&FlagStore],
) -> Option<&'a AiPolicyVerb> {
    best_rule_in(rules, channel, facts, memory, params, flags).map(|r| &r.verb)
}

/// The winning RULE on `channel`, not just its verb.
///
/// [`best_in`]'s body, one level down, so the two cannot diverge. A caller that
/// needs the authored `priority` alongside the verb — issue #959's power-budget
/// planner, which ranks the groups competing for the reactor's points by the
/// priority their own hull file gave them — reads it here instead of
/// re-implementing the scan.
fn best_rule_in<'a>(
    rules: &'a [AiPolicyRule],
    channel: &str,
    facts: &AiFacts,
    memory: &AiPolicyMemory,
    params: &AiParams,
    flags: &[&FlagStore],
) -> Option<&'a AiPolicyRule> {
    let mut best: Option<&AiPolicyRule> = None;
    for rule in rules {
        if rule.channel != channel {
            continue;
        }
        if !rule.when.evaluate_stateful(facts, memory, params, flags) {
            continue;
        }
        // Strictly-greater keeps the earliest-authored rule on ties.
        match best {
            Some(b) if rule.priority <= b.priority => {}
            _ => best = Some(rule),
        }
    }
    best
}

impl AiPolicy {
    /// Resolve the winning verb for one output channel against an immutable
    /// snapshot of typed facts and read-only world flags.
    ///
    /// Evaluates every rule bound to `channel`, keeps those whose guard fires,
    /// and returns the highest-priority survivor's verb (earliest-authored on
    /// a tie). `None` when no rule fires (or the policy is idle) — the caller
    /// then applies no output to that channel.
    ///
    /// This is the STATELESS path and its behaviour is frozen (issue #882): it
    /// scans `self.rules` only, never consults `self.machine`, and cannot read
    /// private memory or state time.
    pub fn resolve_channel(
        &self,
        channel: &str,
        facts: &AiFacts,
        flags: &[&FlagStore],
    ) -> Option<&AiPolicyVerb> {
        if self.idle {
            return None;
        }
        best_in(
            &self.rules,
            channel,
            facts,
            &AiPolicyMemory::default(),
            &self.params,
            flags,
        )
    }

    /// [`Self::resolve_channel`], plus the authored `priority` of the rule that
    /// won (issue #959).
    ///
    /// Same scan, same guards, same tie-break — it IS the same scan — so this
    /// can never disagree with `resolve_channel` about which verb wins. The
    /// extra `i32` is for a caller that has to rank several channels' outputs
    /// against each other rather than apply each in isolation: the power-budget
    /// planner spends a fixed pool of reactor points across every power group,
    /// and "which group gets the last point" has to be the hull's decision, so
    /// it comes out of the hull's own `[[power.ai_policy.rule]] priority`.
    pub fn resolve_channel_ranked(
        &self,
        channel: &str,
        facts: &AiFacts,
        flags: &[&FlagStore],
    ) -> Option<(i32, &AiPolicyVerb)> {
        if self.idle {
            return None;
        }
        best_rule_in(
            &self.rules,
            channel,
            facts,
            &AiPolicyMemory::default(),
            &self.params,
            flags,
        )
        .map(|r| (r.priority, &r.verb))
    }

    /// The opt-in state machine, when this policy declares one (issue #882).
    pub fn machine(&self) -> Option<&AiPolicyMachine> {
        self.machine.as_ref()
    }

    /// Does the leg this policy is flying RIGHT NOW yield its solved facing to
    /// a channel-3 `ArcBearingRequest` (issue #918)?
    ///
    /// `current_state` is the runtime state id the host is holding — `None`, or
    /// a name no state answers to, for a policy that has no machine to be in a
    /// leg of.
    ///
    /// Answers `true` for everything except an authored doctrine leg that says
    /// otherwise, and that default is the whole of the #673-#684 guarantee: a
    /// helm with no doctrine, a stateless policy, a hull whose machine has not
    /// entered anything yet — none of them have chosen a heading, so all of
    /// them still turn to bring a family that cannot bear onto its target.
    ///
    /// The question is asked of the HELM's own current leg and of nothing else.
    /// It cannot see who raised the request, and there is deliberately no
    /// parameter through which it could (AGENTS.md #6).
    pub fn leg_yields_to_arc_requests(&self, current_state: Option<&str>) -> bool {
        let Some(machine) = self.machine() else {
            return true;
        };
        current_state
            .and_then(|id| machine.state(id))
            .map(|leg| leg.yields_to_arc_requests)
            .unwrap_or(true)
    }

    /// Every `history(...)` atom this policy's guards contain, wherever they
    /// sit — top-level rules, per-state rules and transitions (issue #890).
    ///
    /// All three positions, not merely the transitions: a state's continuous
    /// rules are resolved by a DIFFERENT host later in the same tick, off the
    /// same bag, so a window an author put in a rule guard has to be folded
    /// exactly like one in a transition guard or it would read absent for ever
    /// — the trap #779/#788/#789 kept re-opening.
    pub fn history_refs(&self) -> Vec<HistoryRef> {
        let mut out = Vec::new();
        for rule in &self.rules {
            rule.when.referenced_history(&mut out);
        }
        if let Some(machine) = &self.machine {
            for state in &machine.states {
                for rule in &state.rules {
                    rule.when.referenced_history(&mut out);
                }
                for transition in &state.transitions {
                    transition.when.referenced_history(&mut out);
                }
            }
        }
        out
    }

    /// The bounded history windows the HOST must fold for this policy, resolved
    /// against its authored parameters (issue #890).
    ///
    /// Deduplicated, so two atoms asking the same question of the same span
    /// share one window — and, equally, two atoms over the same fact with
    /// DIFFERENT authored lengths get two, which is the shape #789 needed and
    /// could not express.
    ///
    /// A window whose length does not resolve to a positive whole number is
    /// dropped rather than guessed at; content validation rejects that at load,
    /// so a live policy never has one.
    pub fn history_windows(&self) -> Vec<HistorySpec> {
        let mut specs: Vec<HistorySpec> = self
            .history_refs()
            .iter()
            .filter_map(|h| h.window.resolve(&self.params))
            .collect();
        specs.sort();
        specs.dedup();
        specs
    }

    /// The id of the state a fresh (or reset) runtime enters (issue #882).
    /// `None` for a stateless policy.
    pub fn initial_state(&self) -> Option<&str> {
        self.machine.as_ref().map(|m| m.initial.as_str())
    }

    /// Resolve the winning verb for one output channel *within a named state*
    /// (issue #882).
    ///
    /// Scans that state's continuous rules through the same [`best_in`] helper
    /// the stateless path uses, so priority and tie-break semantics are
    /// uniform across both paths. `None` when the policy is idle, declares no
    /// machine, names no such state, or no rule in that state fires.
    pub fn resolve_channel_in_state(
        &self,
        state: &str,
        channel: &str,
        facts: &AiFacts,
        memory: &AiPolicyMemory,
        flags: &[&FlagStore],
    ) -> Option<&AiPolicyVerb> {
        if self.idle {
            return None;
        }
        let state = self.machine.as_ref()?.state(state)?;
        best_in(&state.rules, channel, facts, memory, &self.params, flags)
    }

    /// Resolve AT MOST ONE outgoing transition from `state` this tick
    /// (issue #882, AC2).
    ///
    /// Returning an `Option` is *how* one-transition-per-tick is enforced:
    /// the evaluator picks the single highest-priority eligible transition
    /// (earliest-authored on a tie, strictly-greater comparison — the same
    /// rule channel resolution uses) and the host commits that one. A host
    /// cannot accidentally chain transitions within a tick, because there is
    /// no API that returns more than one.
    ///
    /// `None` when the policy is idle, declares no machine, names no such
    /// state, or no transition guard fires (the machine simply stays put).
    pub fn resolve_transition(
        &self,
        state: &str,
        facts: &AiFacts,
        memory: &AiPolicyMemory,
        flags: &[&FlagStore],
    ) -> Option<&AiPolicyTransition> {
        if self.idle {
            return None;
        }
        let state = self.machine.as_ref()?.state(state)?;
        let mut best: Option<&AiPolicyTransition> = None;
        for t in &state.transitions {
            if !t.when.evaluate_stateful(facts, memory, &self.params, flags) {
                continue;
            }
            match best {
                Some(b) if t.priority <= b.priority => {}
                _ => best = Some(t),
            }
        }
        best
    }

    /// The highest-priority outgoing transition from `state` whose guard does
    /// NOT fire this tick — the transition the machine *considered and rejected*,
    /// and the guard that is holding it (issue #1152).
    ///
    /// The read-only mirror of [`resolve_transition`](Self::resolve_transition):
    /// same scan, same strictly-greater / earliest-authored tie-break, inverted
    /// eligibility. It exists ONLY to make the stateful policy machine visible in
    /// the AI policy-state debug surface — it never influences which transition
    /// commits (that is `resolve_transition`'s answer alone), and evaluating a
    /// guard is side-effect free, so calling it cannot perturb the run or its
    /// digest. `None` when the policy is idle, declares no machine, names no such
    /// state, or every outgoing guard is currently satisfied.
    pub fn blocking_transition(
        &self,
        state: &str,
        facts: &AiFacts,
        memory: &AiPolicyMemory,
        flags: &[&FlagStore],
    ) -> Option<&AiPolicyTransition> {
        if self.idle {
            return None;
        }
        let state = self.machine.as_ref()?.state(state)?;
        let mut best: Option<&AiPolicyTransition> = None;
        for t in &state.transitions {
            if t.when.evaluate_stateful(facts, memory, &self.params, flags) {
                continue;
            }
            match best {
                Some(b) if t.priority <= b.priority => {}
                _ => best = Some(t),
            }
        }
        best
    }
}

/// A transition the machine COMMITTED, retained for the read-only AI
/// policy-state debug surface (issue #1152).
///
/// Diagnostic only: it records what the machine *did* so a tuner can see it, and
/// is never read by the machine's own decision. It is not folded into the #894
/// authoritative-state digest (`sim_digest::world_digest` does not fold policy
/// runtime state at all) and is not carried in the #862 snapshot
/// (`snapshot::policy_state` copies only `current`/`entered_at_secs`/`memory`) —
/// it is re-derived on the next machine tick, so its absence after a resume is
/// inert.
#[derive(Clone, Debug, PartialEq)]
pub struct CommittedTransition {
    /// The state the machine left.
    pub from: String,
    /// The state it entered.
    pub to: String,
    /// The guard that fired, rendered as authored (`Predicate::render`).
    pub guard: String,
    /// The tick-derived clock reading at which it committed.
    pub at_secs: f64,
}

/// The outgoing transition the machine CONSIDERED and did not take on the most
/// recent tick, and the guard blocking it (issue #1152).
///
/// Diagnostic only, with the same digest/snapshot exclusion as
/// [`CommittedTransition`]: the highest-priority outgoing transition of the
/// current state whose guard was not satisfied
/// ([`AiPolicy::blocking_transition`]). This is what turns "the machine is
/// stuck" from a black box into "it is waiting on *this* guard".
#[derive(Clone, Debug, PartialEq)]
pub struct BlockedTransition {
    /// The current state the machine would leave.
    pub from: String,
    /// The state the blocked transition would enter.
    pub to: String,
    /// The guard that is not yet satisfied, rendered as authored.
    pub guard: String,
}

/// The per-fine-system runtime state of a stateful policy (issue #882).
///
/// Deliberately a SEPARATE value from [`AiPolicy`]: the policy is immutable
/// authored data shared for the life of the entity, while this mutates every
/// tick. Hosts hold it in a per-fine-system Bevy component sibling to the
/// policy component — never inside the policy component (which would dirty
/// change detection on the authored data every tick) and never in one
/// ship-wide aggregate (which is exactly the hidden ship-wide state machine
/// AC3 forbids).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiPolicyRuntimeState {
    /// The currently-entered state id.
    pub current: String,
    /// The tick-derived clock reading at which `current` was entered. State
    /// time is `now - entered_at_secs`, where `now` comes from the shared AI
    /// tick cadence, never `Time::delta` (AC4).
    pub entered_at_secs: f64,
    /// This fine system's typed private memory. Seeded into evaluation as the
    /// `memory(...)` / `state_time` bag and readable by nothing else.
    pub memory: AiPolicyMemory,
    /// The most recent transition this machine COMMITTED, for the read-only AI
    /// policy-state debug surface (issue #1152). `None` until the first
    /// transition fires; cleared on [`reset`](Self::reset). Diagnostic only —
    /// never read by the machine, never folded into the digest, never
    /// snapshotted (see [`CommittedTransition`]).
    pub last_transition: Option<CommittedTransition>,
    /// The outgoing transition the machine considered and did NOT take on the
    /// most recent tick (issue #1152), with the same diagnostic-only status as
    /// `last_transition`. `None` when every outgoing guard is satisfied, the
    /// state has no transitions, or the machine has not ticked since a reset.
    pub blocked_transition: Option<BlockedTransition>,
}

impl AiPolicyRuntimeState {
    /// A runtime state freshly entered into `policy`'s initial state at `now`,
    /// with memory reset to the policy's authored declarations.
    ///
    /// This is the AC5 reset: the host calls it when AI gains control of the
    /// system and when an unavailable system recovers, so a stateful policy
    /// never resumes mid-manoeuvre on stale state.
    pub fn reset(policy: &AiPolicy, now_secs: f64) -> Self {
        Self {
            current: policy.initial_state().unwrap_or_default().to_string(),
            entered_at_secs: now_secs,
            memory: policy
                .machine()
                .map(|m| m.initial_memory.clone())
                .unwrap_or_default(),
            // A fresh (or reset) machine has taken no transition and considered
            // none — the diagnostic history starts empty (issue #1152).
            last_transition: None,
            blocked_transition: None,
        }
    }

    /// The evaluation bag for this tick: private memory with `state_time`
    /// filled in from the tick-derived clock.
    pub fn memory_at(&self, now_secs: f64) -> AiPolicyMemory {
        let mut m = self.memory.clone();
        m.set_state_time_secs((now_secs - self.entered_at_secs).max(0.0));
        m
    }

    /// Commit an entered state, restarting the state clock. Called by the
    /// host's single state-tick system BEFORE any output resolves this tick,
    /// so every fine system observes the committed state (AC2).
    pub fn enter(&mut self, state: &str, now_secs: f64) {
        self.current = state.to_string();
        self.entered_at_secs = now_secs;
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
