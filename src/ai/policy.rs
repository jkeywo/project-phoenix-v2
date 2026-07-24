//! Pure, Bevy-free stateless AI fine-system policy runtime (issue #775).
//!
//! An AI-capable fine system declares an *inline stateless policy*: a set of
//! prioritised reactive rules, each bound to a named *output channel*, guarded
//! by a `when` predicate over typed facts, read-only world flags/counters, and
//! authored named parameters. For each output channel the runtime resolves the
//! single highest-priority rule whose guard fires and emits that channel's
//! typed verb. No private memory, no lifecycle state — the decision is a pure
//! function of the immutable per-tick snapshot handed in.
//!
//! This module owns the *typed* policy (already parsed + validated); the TOML
//! schema and content validation live in `entities::config`, and the predicate
//! grammar lives in `world::flags`.

use crate::world::flags::{AiFacts, AiParams, FlagStore, Predicate};

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

/// An inline stateless fine-system policy: authored parameters plus the rule
/// set, or an explicit idle declaration.
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
}

impl AiPolicy {
    /// Resolve the winning verb for one output channel against an immutable
    /// snapshot of typed facts and read-only world flags.
    ///
    /// Evaluates every rule bound to `channel`, keeps those whose guard fires,
    /// and returns the highest-priority survivor's verb (earliest-authored on
    /// a tie). `None` when no rule fires (or the policy is idle) — the caller
    /// then applies no output to that channel.
    pub fn resolve_channel(
        &self,
        channel: &str,
        facts: &AiFacts,
        flags: &[&FlagStore],
    ) -> Option<&AiPolicyVerb> {
        if self.idle {
            return None;
        }
        let mut best: Option<&AiPolicyRule> = None;
        for rule in &self.rules {
            if rule.channel != channel {
                continue;
            }
            if !rule.when.evaluate_with(facts, &self.params, flags) {
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
}
