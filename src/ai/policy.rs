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

use crate::world::flags::{AiFacts, AiParams, AiPolicyMemory, FlagStore, Predicate};

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
    best.map(|r| &r.verb)
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

    /// The opt-in state machine, when this policy declares one (issue #882).
    pub fn machine(&self) -> Option<&AiPolicyMachine> {
        self.machine.as_ref()
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
mod tests {
    use super::*;
    use crate::world::flags::parse_predicate;

    fn facts_since_combat(secs: f64) -> AiFacts {
        let mut f = AiFacts::new();
        f.set("secs_since_combat", secs);
        f
    }

    fn combat_window_policy() -> AiPolicy {
        let mut params = AiParams::new();
        params.set("combat_window_secs", 10.0);
        AiPolicy {
            params,
            rules: vec![
                AiPolicyRule {
                    priority: 10,
                    channel: "red_alert".into(),
                    when: parse_predicate("fact(secs_since_combat) < param(combat_window_secs)")
                        .unwrap(),
                    verb: AiPolicyVerb::SetRedAlert(true),
                },
                AiPolicyRule {
                    priority: 0,
                    channel: "red_alert".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::SetRedAlert(false),
                },
            ],
            idle: false,
            machine: None,
        }
    }

    #[test]
    fn high_priority_rule_wins_when_in_combat() {
        let p = combat_window_policy();
        assert_eq!(
            p.resolve_channel("red_alert", &facts_since_combat(3.0), &[]),
            Some(&AiPolicyVerb::SetRedAlert(true))
        );
    }

    #[test]
    fn fallback_rule_wins_when_not_in_combat() {
        let p = combat_window_policy();
        // No combat fact at all → high-priority guard is false → fallback.
        assert_eq!(
            p.resolve_channel("red_alert", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::SetRedAlert(false))
        );
        // Stale combat (outside window) → fallback too.
        assert_eq!(
            p.resolve_channel("red_alert", &facts_since_combat(30.0), &[]),
            Some(&AiPolicyVerb::SetRedAlert(false))
        );
    }

    #[test]
    fn idle_policy_resolves_no_output() {
        let p = AiPolicy {
            idle: true,
            ..Default::default()
        };
        assert_eq!(p.resolve_channel("red_alert", &AiFacts::new(), &[]), None);
    }

    #[test]
    fn unknown_channel_resolves_none() {
        let p = combat_window_policy();
        assert_eq!(
            p.resolve_channel("shields", &facts_since_combat(1.0), &[]),
            None
        );
    }

    // ── Helm continuous-actuator mode verbs (issue #779) ─────────────────────

    /// A ship-idle helm policy: an authored guard can hold the actuator by not
    /// firing on its channel, distinct from an unconditional "always actuate".
    fn engines_hold_when_arrived_policy() -> AiPolicy {
        let mut params = AiParams::new();
        params.set("arrival_radius", 5.0);
        AiPolicy {
            params,
            rules: vec![AiPolicyRule {
                // Actuate only while farther than the arrival radius; inside it
                // no rule fires, so the channel resolves to None ("hold").
                priority: 10,
                channel: "longitudinal".into(),
                when: parse_predicate("fact(distance_to_dest) > param(arrival_radius)").unwrap(),
                verb: AiPolicyVerb::ActuateDesiredTravel,
            }],
            idle: false,
            machine: None,
        }
    }

    #[test]
    fn longitudinal_mode_verb_resolves_when_guard_fires() {
        let p = engines_hold_when_arrived_policy();
        let mut facts = AiFacts::new();
        facts.set("distance_to_dest", 100.0);
        assert_eq!(
            p.resolve_channel("longitudinal", &facts, &[]),
            Some(&AiPolicyVerb::ActuateDesiredTravel),
        );
    }

    #[test]
    fn longitudinal_channel_holds_when_no_rule_fires() {
        let p = engines_hold_when_arrived_policy();
        let mut facts = AiFacts::new();
        facts.set("distance_to_dest", 1.0);
        // Inside the arrival radius nothing fires → hold (None), NOT an idle
        // policy and NOT a scalar-zero verb.
        assert_eq!(p.resolve_channel("longitudinal", &facts, &[]), None);
    }

    #[test]
    fn yaw_channel_resolves_its_own_mode_verb_independently() {
        // Engines and Steering are independent systems: a policy authored for
        // one channel resolves nothing on the other.
        let p = AiPolicy {
            params: AiParams::new(),
            rules: vec![AiPolicyRule {
                priority: 0,
                channel: "yaw".into(),
                when: parse_predicate("true").unwrap(),
                verb: AiPolicyVerb::ActuateDesiredFacing,
            }],
            idle: false,
            machine: None,
        };
        assert_eq!(
            p.resolve_channel("yaw", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::ActuateDesiredFacing),
        );
        assert_eq!(
            p.resolve_channel("longitudinal", &AiFacts::new(), &[]),
            None
        );
    }

    // ── Helm secondary-actuator mode verbs (issue #780) ──────────────────────

    /// The four secondary helm channels each resolve their own value-less mode
    /// verb independently, and a guard that references a seeded fact actually
    /// fires — proving the #779 empty-facts edge is closed once the host seeds
    /// facts (issue #780 populates them; here we prove the runtime honours them).
    #[test]
    fn secondary_actuator_channels_resolve_their_own_mode_verbs() {
        let p = AiPolicy {
            params: AiParams::new(),
            rules: vec![
                AiPolicyRule {
                    priority: 0,
                    channel: "lateral".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::ActuateLateralThrust,
                },
                AiPolicyRule {
                    priority: 0,
                    channel: "vertical".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::ActuateVerticalThrust,
                },
                AiPolicyRule {
                    priority: 0,
                    channel: "impulse".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::EngageImpulse,
                },
                AiPolicyRule {
                    priority: 0,
                    channel: "boost".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::EngageBoost,
                },
            ],
            idle: false,
            machine: None,
        };
        assert_eq!(
            p.resolve_channel("lateral", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::ActuateLateralThrust)
        );
        assert_eq!(
            p.resolve_channel("vertical", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::ActuateVerticalThrust)
        );
        assert_eq!(
            p.resolve_channel("impulse", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::EngageImpulse)
        );
        assert_eq!(
            p.resolve_channel("boost", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::EngageBoost)
        );
    }

    /// A fact-referencing guard on a secondary channel fires only when the seeded
    /// fact crosses the authored threshold — the concrete behaviour #780 needs
    /// from the runtime once the host populates hazard/availability facts.
    #[test]
    fn boost_guard_fires_only_when_seeded_fact_crosses_threshold() {
        let mut params = AiParams::new();
        params.set("boost_urgency", 0.5);
        let p = AiPolicy {
            params,
            rules: vec![AiPolicyRule {
                priority: 10,
                channel: "boost".into(),
                when: parse_predicate(
                    "fact(hazard_urgency) > param(boost_urgency) and fact(boost_available) > 0",
                )
                .unwrap(),
                verb: AiPolicyVerb::EngageBoost,
            }],
            idle: false,
            machine: None,
        };
        // Available but calm → no fire (hold).
        let mut calm = AiFacts::new();
        calm.set("hazard_urgency", 0.1);
        calm.set("boost_available", 1.0);
        assert_eq!(p.resolve_channel("boost", &calm, &[]), None);
        // Available and urgent → fire.
        let mut urgent = AiFacts::new();
        urgent.set("hazard_urgency", 0.9);
        urgent.set("boost_available", 1.0);
        assert_eq!(
            p.resolve_channel("boost", &urgent, &[]),
            Some(&AiPolicyVerb::EngageBoost)
        );
        // Urgent but unavailable → no fire.
        let mut unavailable = AiFacts::new();
        unavailable.set("hazard_urgency", 0.9);
        unavailable.set("boost_available", 0.0);
        assert_eq!(p.resolve_channel("boost", &unavailable, &[]), None);
    }

    // ── Weapon-bank action verbs (issue #781) ────────────────────────────────

    /// A per-bank phaser policy whose fire guard references seeded facts fires
    /// only when the target is valid, in range, in arc, AND the bank is
    /// off-cooldown — the concrete behaviour #781 needs from the runtime once the
    /// host seeds the per-bank readiness snapshot. Mirrors the boost-guard fact
    /// test: proves a `fact(...)` guard actually evaluates (the #779 empty-facts
    /// edge is closed once the host seeds facts).
    #[test]
    fn phaser_fire_guard_fires_only_when_seeded_readiness_facts_pass() {
        let p = AiPolicy {
            params: AiParams::new(),
            rules: vec![AiPolicyRule {
                priority: 10,
                channel: "phaser_fire".into(),
                when: parse_predicate(
                    "fact(target_valid) > 0 and fact(in_range) > 0 \
                     and fact(in_arc) > 0 and fact(on_cooldown) < 1",
                )
                .unwrap(),
                verb: AiPolicyVerb::FirePhaser,
            }],
            idle: false,
            machine: None,
        };
        // Ready in every dimension → fire.
        let mut ready = AiFacts::new();
        ready.set("target_valid", 1.0);
        ready.set("in_range", 1.0);
        ready.set("in_arc", 1.0);
        ready.set("on_cooldown", 0.0);
        assert_eq!(
            p.resolve_channel("phaser_fire", &ready, &[]),
            Some(&AiPolicyVerb::FirePhaser)
        );
        // On cooldown → hold.
        let mut cooling = ready.clone();
        cooling.set("on_cooldown", 1.0);
        assert_eq!(p.resolve_channel("phaser_fire", &cooling, &[]), None);
        // Out of arc → hold.
        let mut out_of_arc = ready.clone();
        out_of_arc.set("in_arc", 0.0);
        assert_eq!(p.resolve_channel("phaser_fire", &out_of_arc, &[]), None);
        // Empty facts (no seeding) → guard is false → hold, never a spurious fire.
        assert_eq!(p.resolve_channel("phaser_fire", &AiFacts::new(), &[]), None);
    }

    /// The blaster action verb resolves on its own `blaster_fire` channel and is
    /// held by an explicit idle declaration — the per-bank opt-out (AC1).
    #[test]
    fn blaster_fire_verb_resolves_and_idle_holds() {
        let firing = AiPolicy {
            params: AiParams::new(),
            rules: vec![AiPolicyRule {
                priority: 0,
                channel: "blaster_fire".into(),
                when: parse_predicate("true").unwrap(),
                verb: AiPolicyVerb::FireBlaster,
            }],
            idle: false,
            machine: None,
        };
        assert_eq!(
            firing.resolve_channel("blaster_fire", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::FireBlaster)
        );
        // A phaser-channel query never picks up the blaster verb.
        assert_eq!(
            firing.resolve_channel("phaser_fire", &AiFacts::new(), &[]),
            None
        );
        // An explicit idle bank holds fire regardless of readiness.
        let idle = AiPolicy {
            idle: true,
            ..Default::default()
        };
        assert_eq!(
            idle.resolve_channel("blaster_fire", &AiFacts::new(), &[]),
            None
        );
    }

    #[test]
    fn equal_priority_ties_break_to_earliest_authored() {
        let p = AiPolicy {
            params: AiParams::new(),
            rules: vec![
                AiPolicyRule {
                    priority: 5,
                    channel: "red_alert".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::SetRedAlert(true),
                },
                AiPolicyRule {
                    priority: 5,
                    channel: "red_alert".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::SetRedAlert(false),
                },
            ],
            idle: false,
            machine: None,
        };
        // Both fire at equal priority; the earliest-authored rule wins.
        assert_eq!(
            p.resolve_channel("red_alert", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::SetRedAlert(true))
        );
    }

    // ── Torpedo tube load + launch channels (issue #782) ─────────────────────

    /// A tube policy carrying a load rule and a launch rule: the two channels
    /// resolve independently, and a fact guard on one channel does not affect the
    /// other. Mirrors the two-stage tube pipeline (LOAD then LAUNCH).
    fn torpedo_tube_policy() -> AiPolicy {
        AiPolicy {
            params: AiParams::new(),
            rules: vec![
                AiPolicyRule {
                    priority: 0,
                    channel: "torpedo_load".into(),
                    when: parse_predicate("fact(magazine) > 0").unwrap(),
                    verb: AiPolicyVerb::LoadTorpedo,
                },
                AiPolicyRule {
                    priority: 0,
                    channel: "torpedo_launch".into(),
                    when: parse_predicate("fact(target_facing_shields) <= 0").unwrap(),
                    verb: AiPolicyVerb::LaunchTorpedo,
                },
            ],
            idle: false,
            machine: None,
        }
    }

    #[test]
    fn torpedo_load_and_launch_channels_resolve_independently() {
        let p = torpedo_tube_policy();

        // Magazine has stock → load fires; shields down → launch fires.
        let mut f = AiFacts::new();
        f.set("magazine", 4.0);
        f.set("target_facing_shields", 0.0);
        assert_eq!(
            p.resolve_channel("torpedo_load", &f, &[]),
            Some(&AiPolicyVerb::LoadTorpedo)
        );
        assert_eq!(
            p.resolve_channel("torpedo_launch", &f, &[]),
            Some(&AiPolicyVerb::LaunchTorpedo)
        );

        // Empty magazine holds LOAD but a downed striking arc still fires LAUNCH:
        // the two channels are independent.
        let mut f = AiFacts::new();
        f.set("magazine", 0.0);
        f.set("target_facing_shields", 0.0);
        assert_eq!(p.resolve_channel("torpedo_load", &f, &[]), None);
        assert_eq!(
            p.resolve_channel("torpedo_launch", &f, &[]),
            Some(&AiPolicyVerb::LaunchTorpedo)
        );

        // A healthy striking arc holds LAUNCH while a stocked magazine still loads.
        let mut f = AiFacts::new();
        f.set("magazine", 4.0);
        f.set("target_facing_shields", 25.0);
        assert_eq!(
            p.resolve_channel("torpedo_load", &f, &[]),
            Some(&AiPolicyVerb::LoadTorpedo)
        );
        assert_eq!(p.resolve_channel("torpedo_launch", &f, &[]), None);
    }

    #[test]
    fn idle_torpedo_tube_holds_both_channels() {
        let p = AiPolicy {
            idle: true,
            ..Default::default()
        };
        let mut f = AiFacts::new();
        f.set("magazine", 4.0);
        f.set("target_facing_shields", 0.0);
        assert_eq!(p.resolve_channel("torpedo_load", &f, &[]), None);
        assert_eq!(p.resolve_channel("torpedo_launch", &f, &[]), None);
    }

    #[test]
    fn torpedo_magazine_grant_channel_resolves() {
        let p = AiPolicy {
            params: AiParams::new(),
            rules: vec![AiPolicyRule {
                priority: 0,
                channel: "torpedo_magazine_grant".into(),
                when: parse_predicate("fact(in_flight) < 3").unwrap(),
                verb: AiPolicyVerb::GrantTorpedoRound,
            }],
            idle: false,
            machine: None,
        };
        // Few in flight → grant.
        let mut f = AiFacts::new();
        f.set("in_flight", 1.0);
        assert_eq!(
            p.resolve_channel("torpedo_magazine_grant", &f, &[]),
            Some(&AiPolicyVerb::GrantTorpedoRound)
        );
        // Saturated in flight → hold (refuse the claim).
        let mut f = AiFacts::new();
        f.set("in_flight", 5.0);
        assert_eq!(p.resolve_channel("torpedo_magazine_grant", &f, &[]), None);
    }

    // ── Shields focus channel (issue #783) ───────────────────────────────────

    /// The value-less `focus_shield_arc` verb resolves on its own `shield_focus`
    /// channel when a concentration guard fires, and an explicit idle holds it —
    /// the gate half of #783 (WHICH arc is the retained kernel's call, not the
    /// verb's). Mirrors the weapon-bank fire-verb tests.
    #[test]
    fn shield_focus_verb_resolves_when_guard_fires_and_idle_holds() {
        let mut params = AiParams::new();
        params.set("damage_pct_threshold", 50.0);
        let p = AiPolicy {
            params,
            rules: vec![AiPolicyRule {
                priority: 10,
                channel: "shield_focus".into(),
                when: parse_predicate("fact(recent_damage_pct_max) >= param(damage_pct_threshold)")
                    .unwrap(),
                verb: AiPolicyVerb::FocusShieldArc,
            }],
            idle: false,
            machine: None,
        };
        // Concentrated recent damage (80% on one arc) → act.
        let mut concentrated = AiFacts::new();
        concentrated.set("recent_damage_pct_max", 80.0);
        assert_eq!(
            p.resolve_channel("shield_focus", &concentrated, &[]),
            Some(&AiPolicyVerb::FocusShieldArc)
        );
        // Diffuse damage (below the threshold) → hold.
        let mut diffuse = AiFacts::new();
        diffuse.set("recent_damage_pct_max", 20.0);
        assert_eq!(p.resolve_channel("shield_focus", &diffuse, &[]), None);
        // Empty facts (no seeding) → guard is false → hold, never a spurious act.
        assert_eq!(
            p.resolve_channel("shield_focus", &AiFacts::new(), &[]),
            None
        );
        // An explicit idle policy holds the channel regardless of damage.
        let idle = AiPolicy {
            idle: true,
            ..Default::default()
        };
        assert_eq!(
            idle.resolve_channel("shield_focus", &concentrated, &[]),
            None
        );
    }

    // ── Power group allocation channels (issue #784) ─────────────────────────

    /// The power channel is the power GROUP id (not a fixed name), and the
    /// winning rule carries an ABSOLUTE target level in its verb. Two groups on
    /// one policy resolve independently, and the highest-priority matching rule
    /// for a group wins (AC4).
    #[test]
    fn power_group_channels_resolve_absolute_levels_independently() {
        let mut params = AiParams::new();
        params.set("thrust_threshold", 0.7);
        params.set("min_reserve_helm", 50.0);
        let p = AiPolicy {
            params,
            rules: vec![
                // helm: elevate to 3 while thrusting hard with a healthy
                // battery, else fall back to the baseline 2.
                AiPolicyRule {
                    priority: 10,
                    channel: "helm".into(),
                    when: parse_predicate(
                        "fact(thrust) >= param(thrust_threshold) \
                         and fact(battery_pct) >= param(min_reserve_helm)",
                    )
                    .unwrap(),
                    verb: AiPolicyVerb::SetPowerGroupAllocation(3),
                },
                AiPolicyRule {
                    priority: 0,
                    channel: "helm".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::SetPowerGroupAllocation(2),
                },
                // A ship-authored extra group ("ops") on its own channel.
                AiPolicyRule {
                    priority: 0,
                    channel: "ops".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::SetPowerGroupAllocation(1),
                },
            ],
            idle: false,
            machine: None,
        };

        // Thrusting hard, battery healthy → helm elevates to 3.
        let mut hot = AiFacts::new();
        hot.set("thrust", 0.9);
        hot.set("battery_pct", 80.0);
        assert_eq!(
            p.resolve_channel("helm", &hot, &[]),
            Some(&AiPolicyVerb::SetPowerGroupAllocation(3))
        );
        // Same thrust but battery below the reserve → the elevate guard fails,
        // the baseline wins: allocation never rises when the battery can't
        // sustain it (AC5 brownout avoidance).
        let mut drained = AiFacts::new();
        drained.set("thrust", 0.9);
        drained.set("battery_pct", 40.0);
        assert_eq!(
            p.resolve_channel("helm", &drained, &[]),
            Some(&AiPolicyVerb::SetPowerGroupAllocation(2))
        );
        // The ops channel resolves its own level, independent of helm.
        assert_eq!(
            p.resolve_channel("ops", &hot, &[]),
            Some(&AiPolicyVerb::SetPowerGroupAllocation(1))
        );
        // A group with no authored rule holds (None).
        assert_eq!(p.resolve_channel("weapons", &hot, &[]), None);
    }

    // ── Comms dialogue-response channel (issue #786) ────────────────────────

    /// The `comms_respond` channel resolves the SECOND value-carrying verb, and
    /// the authored index rides the verb (not the host).
    #[test]
    fn comms_respond_channel_resolves_the_authored_response_index() {
        let mut params = AiParams::new();
        params.set("urgent_threshold", 1.0);
        let p = AiPolicy {
            params,
            rules: vec![
                // An urgent message gets the second (index 1) response...
                AiPolicyRule {
                    priority: 10,
                    channel: "comms_respond".into(),
                    when: parse_predicate("fact(is_urgent) >= param(urgent_threshold)").unwrap(),
                    verb: AiPolicyVerb::RespondToMessage(1),
                },
                // ...everything else takes the first, reproducing the retired
                // channel-2 auto-response stub's decision.
                AiPolicyRule {
                    priority: 0,
                    channel: "comms_respond".into(),
                    when: parse_predicate("true").unwrap(),
                    verb: AiPolicyVerb::RespondToMessage(0),
                },
            ],
            idle: false,
            machine: None,
        };

        let mut routine = AiFacts::new();
        routine.set("is_urgent", 0.0);
        routine.set("response_count", 3.0);
        assert_eq!(
            p.resolve_channel("comms_respond", &routine, &[]),
            Some(&AiPolicyVerb::RespondToMessage(0)),
            "the baseline rule answers with index 0"
        );

        let mut urgent = AiFacts::new();
        urgent.set("is_urgent", 1.0);
        urgent.set("response_count", 3.0);
        assert_eq!(
            p.resolve_channel("comms_respond", &urgent, &[]),
            Some(&AiPolicyVerb::RespondToMessage(1)),
            "the higher-priority urgent rule answers with its own authored index"
        );

        // No rule on another channel: the Comms policy drives only its own axis.
        assert_eq!(p.resolve_channel("red_alert", &urgent, &[]), None);
    }

    /// An idle Comms policy answers nothing — the explicit "the AI does not
    /// speak for me" declaration.
    #[test]
    fn idle_comms_policy_never_responds() {
        let p = AiPolicy {
            params: AiParams::new(),
            rules: vec![AiPolicyRule {
                priority: 0,
                channel: "comms_respond".into(),
                when: parse_predicate("true").unwrap(),
                verb: AiPolicyVerb::RespondToMessage(0),
            }],
            idle: true,
            machine: None,
        };
        assert_eq!(
            p.resolve_channel("comms_respond", &AiFacts::new(), &[]),
            None
        );
    }

    /// A `comms_respond` guard may read scenario flags; the flag chain is a
    /// shared slice, so the policy can never write one back (AC4).
    #[test]
    fn comms_respond_guard_reads_the_scenario_flag_chain() {
        let p = AiPolicy {
            params: AiParams::new(),
            rules: vec![AiPolicyRule {
                priority: 0,
                channel: "comms_respond".into(),
                when: parse_predicate("flag(cleared_to_answer)").unwrap(),
                verb: AiPolicyVerb::RespondToMessage(2),
            }],
            idle: false,
            machine: None,
        };
        let empty = FlagStore::new();
        assert_eq!(
            p.resolve_channel("comms_respond", &AiFacts::new(), &[&empty]),
            None,
            "an unset flag holds the response"
        );
        let mut set = FlagStore::new();
        set.set_flag("cleared_to_answer");
        assert_eq!(
            p.resolve_channel("comms_respond", &AiFacts::new(), &[&set]),
            Some(&AiPolicyVerb::RespondToMessage(2))
        );
        // The store is untouched by evaluation.
        let mut expected = FlagStore::new();
        expected.set_flag("cleared_to_answer");
        assert_eq!(set, expected);
    }

    // ── Optional stateful path (issue #882) ──────────────────────────────────

    /// A two-state boost machine: `cruise` holds, `surge` engages. It leaves
    /// `cruise` when the hazard fact crosses an authored threshold, and returns
    /// once it has been surging for the authored dwell (the `state_time` atom).
    /// Both transitions out of `cruise` are authored so priority is exercised.
    fn two_state_boost_machine() -> AiPolicy {
        let mut params = AiParams::new();
        params.set("surge_urgency", 0.5);
        params.set("surge_dwell_secs", 3.0);
        AiPolicy {
            params,
            rules: Vec::new(),
            idle: false,
            machine: Some(AiPolicyMachine {
                initial: "cruise".into(),
                initial_memory: AiPolicyMemory::new(),
                states: vec![
                    AiPolicyState {
                        id: "cruise".into(),
                        // No rule fires here: cruising holds boost.
                        rules: Vec::new(),
                        transitions: vec![AiPolicyTransition {
                            priority: 10,
                            to: "surge".into(),
                            when: parse_predicate(
                                "fact(hazard_urgency) > param(surge_urgency) \
                                 and fact(boost_available) > 0",
                            )
                            .unwrap(),
                        }],
                    },
                    AiPolicyState {
                        id: "surge".into(),
                        rules: vec![AiPolicyRule {
                            priority: 0,
                            channel: "boost".into(),
                            when: parse_predicate("true").unwrap(),
                            verb: AiPolicyVerb::EngageBoost,
                        }],
                        transitions: vec![AiPolicyTransition {
                            priority: 0,
                            to: "cruise".into(),
                            when: parse_predicate("state_time >= param(surge_dwell_secs)").unwrap(),
                        }],
                    },
                ],
            }),
        }
    }

    fn urgent_facts() -> AiFacts {
        let mut f = AiFacts::new();
        f.set("hazard_urgency", 0.9);
        f.set("boost_available", 1.0);
        f
    }

    fn calm_facts() -> AiFacts {
        let mut f = AiFacts::new();
        f.set("hazard_urgency", 0.0);
        f.set("boost_available", 1.0);
        f
    }

    /// AC7 — THE invariant. A stateless policy resolves EXACTLY as it did
    /// before #882 existed: `machine: None` means the transition/state/memory
    /// code is never entered, and every channel answer is unchanged. This is
    /// the regression guard the twelve shipped Group A hosts rest on.
    #[test]
    fn stateless_policy_resolution_is_unchanged_by_the_optional_machine() {
        let p = combat_window_policy();
        assert!(
            p.machine().is_none(),
            "a #775-shaped policy declares no machine"
        );
        assert_eq!(p.initial_state(), None);

        // Every stateless answer this module already pins, re-asserted through
        // the post-#882 `best_in` helper.
        assert_eq!(
            p.resolve_channel("red_alert", &facts_since_combat(3.0), &[]),
            Some(&AiPolicyVerb::SetRedAlert(true))
        );
        assert_eq!(
            p.resolve_channel("red_alert", &facts_since_combat(30.0), &[]),
            Some(&AiPolicyVerb::SetRedAlert(false))
        );
        assert_eq!(
            p.resolve_channel("red_alert", &AiFacts::new(), &[]),
            Some(&AiPolicyVerb::SetRedAlert(false))
        );
        assert_eq!(p.resolve_channel("shields", &AiFacts::new(), &[]), None);

        // The stateful entry points are inert on a stateless policy: they
        // cannot resolve anything, whatever state id or memory is offered.
        let mut memory = AiPolicyMemory::new();
        memory.set("anything", 99.0);
        memory.set_state_time_secs(1000.0);
        assert_eq!(
            p.resolve_channel_in_state(
                "red_alert",
                "red_alert",
                &facts_since_combat(3.0),
                &memory,
                &[]
            ),
            None,
            "a stateless policy has no states to resolve in"
        );
        assert_eq!(
            p.resolve_transition("red_alert", &facts_since_combat(3.0), &memory, &[]),
            None,
            "a stateless policy has no transitions to fire"
        );
    }

    /// AC1 + AC9 (initial state): a fresh runtime state enters the authored
    /// initial state with the authored memory, and that state's own continuous
    /// rules are what resolve.
    #[test]
    fn runtime_state_starts_in_the_authored_initial_state() {
        let p = two_state_boost_machine();
        assert_eq!(p.initial_state(), Some("cruise"));
        let st = AiPolicyRuntimeState::reset(&p, 100.0);
        assert_eq!(st.current, "cruise");
        assert_eq!(st.entered_at_secs, 100.0);
        // `cruise` authors no boost rule → the channel holds even while urgent.
        assert_eq!(
            p.resolve_channel_in_state("cruise", "boost", &urgent_facts(), &st.memory, &[]),
            None
        );
        // `surge` authors an unconditional one.
        assert_eq!(
            p.resolve_channel_in_state("surge", "boost", &calm_facts(), &st.memory, &[]),
            Some(&AiPolicyVerb::EngageBoost)
        );
        // An undeclared state resolves nothing rather than panicking.
        assert_eq!(
            p.resolve_channel_in_state("nowhere", "boost", &urgent_facts(), &st.memory, &[]),
            None
        );
    }

    /// AC2 (one transition per tick) + same-tick outputs: `resolve_transition`
    /// returns AT MOST ONE transition — that is the enforcement, not host
    /// discipline — and the entered state's continuous rules answer
    /// immediately, in the very same tick, with no intervening evaluation.
    #[test]
    fn one_transition_per_tick_and_the_entered_state_outputs_immediately() {
        let p = two_state_boost_machine();
        let mut st = AiPolicyRuntimeState::reset(&p, 0.0);
        let facts = urgent_facts();

        // Tick 1: cruise holds boost until the transition is resolved...
        assert_eq!(
            p.resolve_channel_in_state(&st.current, "boost", &facts, &st.memory_at(0.0), &[]),
            None
        );
        let t = p
            .resolve_transition(&st.current, &facts, &st.memory_at(0.0), &[])
            .expect("the urgency guard fires");
        assert_eq!(t.to, "surge");
        st.enter(&t.to, 0.0);
        // ...and the moment it is committed, THIS SAME TICK, the new state's
        // continuous output is live (AC2).
        assert_eq!(
            p.resolve_channel_in_state(&st.current, "boost", &facts, &st.memory_at(0.0), &[]),
            Some(&AiPolicyVerb::EngageBoost)
        );

        // Still the same tick: the `surge → cruise` guard is `state_time >= 3`,
        // and state time is 0 here, so nothing chains. Even if it did fire, the
        // API returns ONE transition — a host cannot walk two in a tick.
        assert_eq!(
            p.resolve_transition(&st.current, &facts, &st.memory_at(0.0), &[]),
            None
        );
    }

    /// AC9 (deterministic priority ties): two eligible transitions out of one
    /// state resolve by strictly-greater priority, and an exact tie resolves to
    /// the EARLIEST-AUTHORED transition — the same rule channel resolution
    /// uses, so the two halves of the spine can never disagree.
    #[test]
    fn transition_priority_and_ties_are_deterministic() {
        let machine = |a: i32, b: i32| AiPolicy {
            params: AiParams::new(),
            rules: Vec::new(),
            idle: false,
            machine: Some(AiPolicyMachine {
                initial: "start".into(),
                initial_memory: AiPolicyMemory::new(),
                states: vec![
                    AiPolicyState {
                        id: "start".into(),
                        rules: Vec::new(),
                        transitions: vec![
                            AiPolicyTransition {
                                priority: a,
                                to: "first".into(),
                                when: parse_predicate("true").unwrap(),
                            },
                            AiPolicyTransition {
                                priority: b,
                                to: "second".into(),
                                when: parse_predicate("true").unwrap(),
                            },
                        ],
                    },
                    AiPolicyState {
                        id: "first".into(),
                        rules: Vec::new(),
                        transitions: Vec::new(),
                    },
                    AiPolicyState {
                        id: "second".into(),
                        rules: Vec::new(),
                        transitions: Vec::new(),
                    },
                ],
            }),
        };
        let m = AiPolicyMemory::new();
        // Higher priority wins regardless of authored order.
        assert_eq!(
            machine(0, 10)
                .resolve_transition("start", &AiFacts::new(), &m, &[])
                .map(|t| t.to.as_str()),
            Some("second")
        );
        assert_eq!(
            machine(10, 0)
                .resolve_transition("start", &AiFacts::new(), &m, &[])
                .map(|t| t.to.as_str()),
            Some("first")
        );
        // An exact tie goes to the earliest-authored transition.
        assert_eq!(
            machine(5, 5)
                .resolve_transition("start", &AiFacts::new(), &m, &[])
                .map(|t| t.to.as_str()),
            Some("first")
        );
    }

    /// AC9 (monotonic state time): state time is `now - entered_at`, it grows
    /// monotonically while the state is held, it RESTARTS on entering a state,
    /// and it is never negative even if a clock reading arrives out of order.
    #[test]
    fn state_time_is_monotonic_within_a_state_and_restarts_on_entry() {
        let p = two_state_boost_machine();
        let mut st = AiPolicyRuntimeState::reset(&p, 10.0);
        assert_eq!(st.memory_at(10.0).state_time_secs(), 0.0);
        assert_eq!(st.memory_at(11.0).state_time_secs(), 1.0);
        assert_eq!(st.memory_at(12.5).state_time_secs(), 2.5);
        // Never negative — an absent/behind reading clamps rather than panics.
        assert_eq!(st.memory_at(9.0).state_time_secs(), 0.0);

        st.enter("surge", 20.0);
        assert_eq!(
            st.memory_at(20.0).state_time_secs(),
            0.0,
            "entering a state restarts its clock"
        );
        // The dwell guard out of `surge` is `state_time >= param(3)`: it holds
        // below the dwell and fires at/after it.
        let facts = calm_facts();
        assert_eq!(
            p.resolve_transition("surge", &facts, &st.memory_at(22.9), &[]),
            None
        );
        assert_eq!(
            p.resolve_transition("surge", &facts, &st.memory_at(23.0), &[])
                .map(|t| t.to.as_str()),
            Some("cruise")
        );
    }

    /// AC3 (memory scoping): `memory(...)` reads ONLY the bag handed to this
    /// evaluation. A different fine system's bag — or no bag at all — simply
    /// finds no reading and the guard evaluates `false`, never a panic and
    /// never another system's value. There is no API through which a policy
    /// could reach a bag it was not given, which is the structural half of
    /// "cannot become a ship-wide state machine".
    #[test]
    fn memory_is_scoped_to_the_bag_the_owning_system_seeds() {
        let mut params = AiParams::new();
        params.set("armed_threshold", 1.0);
        let p = AiPolicy {
            params,
            rules: Vec::new(),
            idle: false,
            machine: Some(AiPolicyMachine {
                initial: "idle".into(),
                initial_memory: AiPolicyMemory::new(),
                states: vec![AiPolicyState {
                    id: "idle".into(),
                    rules: vec![AiPolicyRule {
                        priority: 0,
                        channel: "boost".into(),
                        when: parse_predicate("memory(armed) >= param(armed_threshold)").unwrap(),
                        verb: AiPolicyVerb::EngageBoost,
                    }],
                    transitions: Vec::new(),
                }],
            }),
        };
        // This system's own bag, armed → the rule fires.
        let mut own = AiPolicyMemory::new();
        own.set("armed", 1.0);
        assert_eq!(
            p.resolve_channel_in_state("idle", "boost", &AiFacts::new(), &own, &[]),
            Some(&AiPolicyVerb::EngageBoost)
        );
        // A SIBLING system's bag — same slot name, different owner — is simply
        // never handed in; all this system can be given is its own. Standing in
        // for that, an empty bag (what a system with no memory of its own has)
        // resolves the guard false.
        assert_eq!(
            p.resolve_channel_in_state(
                "idle",
                "boost",
                &AiFacts::new(),
                &AiPolicyMemory::new(),
                &[]
            ),
            None
        );
        // A memory slot is not a fact and a fact is not a memory slot: seeding
        // `armed` as a world FACT does not satisfy a `memory(armed)` guard.
        let mut facts = AiFacts::new();
        facts.set("armed", 1.0);
        assert_eq!(
            p.resolve_channel_in_state("idle", "boost", &facts, &AiPolicyMemory::new(), &[]),
            None
        );
    }

    /// An idle declaration wins over an authored machine on every entry point —
    /// "the AI does not operate this system" is absolute, exactly as it is on
    /// the stateless path.
    #[test]
    fn idle_holds_every_stateful_entry_point() {
        let p = AiPolicy {
            idle: true,
            ..two_state_boost_machine()
        };
        let m = AiPolicyMemory::new();
        assert_eq!(
            p.resolve_channel_in_state("surge", "boost", &urgent_facts(), &m, &[]),
            None
        );
        assert_eq!(
            p.resolve_transition("cruise", &urgent_facts(), &m, &[]),
            None
        );
    }

    // ── Fly-through attack pass doctrine (issue #883) ────────────────────────

    /// The Steering half of the destroyer's fly-through doctrine, in the shape
    /// the hull TOML authors it: three states, and — the load-bearing part —
    /// TWO DIFFERENT verbs on the ONE `yaw` channel. `inbound` re-solves the
    /// facing against the moving target every tick; `escape` closes the
    /// solution and flies the frozen heading.
    fn fly_through_steering_machine() -> AiPolicy {
        let mut params = AiParams::new();
        params.set("commit_range", 120.0);
        params.set("closing_rate_epsilon", -0.05);
        params.set("closest_approach_hysteresis", 4.0);
        params.set("escape_duration_secs", 6.0);
        let mut initial_memory = AiPolicyMemory::new();
        initial_memory.set("min_range_seen", 100_000.0);
        AiPolicy {
            params,
            rules: Vec::new(),
            idle: false,
            machine: Some(AiPolicyMachine {
                initial: "acquire".into(),
                initial_memory,
                states: vec![
                    AiPolicyState {
                        id: "acquire".into(),
                        rules: vec![AiPolicyRule {
                            priority: 0,
                            channel: "yaw".into(),
                            when: parse_predicate("true").unwrap(),
                            verb: AiPolicyVerb::ActuateDesiredFacing,
                        }],
                        transitions: vec![AiPolicyTransition {
                            priority: 10,
                            to: "inbound".into(),
                            when: parse_predicate(
                                "fact(target_valid) > 0 and \
                                 fact(range_to_target) <= param(commit_range)",
                            )
                            .unwrap(),
                        }],
                    },
                    AiPolicyState {
                        id: "inbound".into(),
                        rules: vec![AiPolicyRule {
                            priority: 0,
                            channel: "yaw".into(),
                            when: parse_predicate("true").unwrap(),
                            verb: AiPolicyVerb::ActuateDesiredFacing,
                        }],
                        transitions: vec![
                            AiPolicyTransition {
                                priority: 20,
                                to: "escape".into(),
                                when: parse_predicate(
                                    "fact(target_valid) > 0 and \
                                     fact(closing_rate) < param(closing_rate_epsilon) and \
                                     fact(range_above_min_seen) > \
                                     param(closest_approach_hysteresis)",
                                )
                                .unwrap(),
                            },
                            AiPolicyTransition {
                                priority: 10,
                                to: "acquire".into(),
                                when: parse_predicate("fact(target_valid) < 1").unwrap(),
                            },
                        ],
                    },
                    AiPolicyState {
                        id: "escape".into(),
                        rules: vec![AiPolicyRule {
                            priority: 0,
                            channel: "yaw".into(),
                            when: parse_predicate("true").unwrap(),
                            verb: AiPolicyVerb::HoldCommittedHeading,
                        }],
                        transitions: vec![AiPolicyTransition {
                            priority: 0,
                            to: "acquire".into(),
                            when: parse_predicate("state_time >= param(escape_duration_secs)")
                                .unwrap(),
                        }],
                    },
                ],
            }),
        }
    }

    /// Facts as the host seeds them mid-approach: valid target, closing, and the
    /// range still at (or below) the running minimum.
    fn inbound_facts(range: f64, closing_rate: f64, above_min: f64) -> AiFacts {
        let mut f = AiFacts::new();
        f.set("target_valid", 1.0);
        f.set("range_to_target", range);
        f.set("closing_rate", closing_rate);
        f.set("range_above_min_seen", above_min);
        f
    }

    /// AC1/AC2 as an FSM property: the pass tracks while closing, and flips to
    /// the frozen-heading verb only once the range has opened past the authored
    /// hysteresis AND the closing rate has gone negative. Neither condition
    /// alone fires it — a momentary negative blip inside the hysteresis band is
    /// exactly the false positive the running minimum exists to reject.
    #[test]
    fn closest_approach_flips_the_yaw_channel_from_tracking_to_frozen() {
        let p = fly_through_steering_machine();
        let mut st = AiPolicyRuntimeState::reset(&p, 0.0);
        assert_eq!(st.current, "acquire");

        // Target comes inside the commit range -> inbound.
        let t = p
            .resolve_transition("acquire", &inbound_facts(100.0, 12.0, 0.0), &st.memory, &[])
            .expect("commit-range guard fires");
        assert_eq!(t.to, "inbound");
        st.enter(&t.to, 0.0);

        // Closing hard: tracking verb, no transition.
        let closing = inbound_facts(40.0, 12.0, 0.0);
        assert_eq!(
            p.resolve_channel_in_state("inbound", "yaw", &closing, &st.memory, &[]),
            Some(&AiPolicyVerb::ActuateDesiredFacing)
        );
        assert_eq!(
            p.resolve_transition("inbound", &closing, &st.memory, &[]),
            None
        );

        // Range opening but still inside the hysteresis band: NOT yet closest
        // approach. This is the guard doing real work.
        let blip = inbound_facts(12.0, -0.5, 1.0);
        assert_eq!(
            p.resolve_transition("inbound", &blip, &st.memory, &[]),
            None
        );

        // Opened past the authored hysteresis with a negative closing rate.
        let past = inbound_facts(16.0, -9.0, 5.0);
        let t = p
            .resolve_transition("inbound", &past, &st.memory, &[])
            .expect("closest approach fires");
        assert_eq!(t.to, "escape");
        st.enter(&t.to, 10.0);

        // Same tick: the yaw channel now answers with the FROZEN-heading verb.
        assert_eq!(
            p.resolve_channel_in_state("escape", "yaw", &past, &st.memory_at(10.0), &[]),
            Some(&AiPolicyVerb::HoldCommittedHeading),
            "past closest approach the facing solution is closed, not merely held"
        );
    }

    /// The escape leg does not re-acquire because the target moved: only the
    /// authored dwell ends it. A target swinging back into a hard closing rate
    /// mid-escape leaves the state — and therefore the frozen heading — alone.
    #[test]
    fn escape_leg_is_ended_by_its_authored_dwell_not_by_the_target() {
        let p = fly_through_steering_machine();
        let mut st = AiPolicyRuntimeState::reset(&p, 0.0);
        st.enter("escape", 0.0);

        let target_closing_again = inbound_facts(30.0, 25.0, 0.0);
        assert_eq!(
            p.resolve_transition("escape", &target_closing_again, &st.memory_at(1.0), &[]),
            None,
            "nothing about the target may cut the escape short"
        );
        assert_eq!(
            p.resolve_channel_in_state(
                "escape",
                "yaw",
                &target_closing_again,
                &st.memory_at(1.0),
                &[]
            ),
            Some(&AiPolicyVerb::HoldCommittedHeading)
        );
        // Only the authored dwell releases it.
        assert_eq!(
            p.resolve_transition("escape", &target_closing_again, &st.memory_at(6.0), &[])
                .map(|t| t.to.as_str()),
            Some("acquire")
        );
    }

    /// AC5 reset semantics at the pure layer: a reset returns the runtime to
    /// the authored initial state, restarts the clock, and restores authored
    /// memory — whatever state or memory it had drifted into.
    #[test]
    fn reset_returns_to_initial_state_and_authored_memory() {
        let mut p = two_state_boost_machine();
        if let Some(m) = p.machine.as_mut() {
            m.initial_memory.set("engagements", 0.0);
        }
        let mut st = AiPolicyRuntimeState::reset(&p, 0.0);
        st.enter("surge", 5.0);
        st.memory.set("engagements", 7.0);
        assert_eq!(st.current, "surge");
        assert_eq!(st.memory.get("engagements"), Some(7.0));

        // Re-bind OVER the drifted value: the assertions below have to be about
        // the value that actually drifted, not about a pristine second runtime
        // built alongside it — otherwise "restores the AUTHORED memory, not the
        // drifted runtime value" is not exercised at all.
        st = AiPolicyRuntimeState::reset(&p, 42.0);
        assert_eq!(st.current, "cruise");
        assert_eq!(st.entered_at_secs, 42.0);
        assert_eq!(
            st.memory.get("engagements"),
            Some(0.0),
            "reset restores the AUTHORED memory, not the drifted runtime value"
        );
    }
}
