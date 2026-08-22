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

/// Issue #788: the `yaw` channel now carries FOUR mode verbs, and a state
/// resolves exactly one of them. They are distinct values — not a flag on a
/// shared verb — because the host reads the leg off the verb and pairs each
/// one with a different upstream facing solution and throttle.
#[test]
fn the_yaw_channel_resolves_all_four_distinct_mode_verbs() {
    let state = |id: &str, verb: AiPolicyVerb| AiPolicyState {
        id: id.into(),
        yields_to_arc_requests: true,
        rules: vec![AiPolicyRule {
            priority: 0,
            channel: "yaw".into(),
            when: parse_predicate("true").unwrap(),
            verb,
        }],
        transitions: Vec::new(),
    };
    let p = AiPolicy {
        params: AiParams::new(),
        rules: Vec::new(),
        idle: false,
        machine: Some(AiPolicyMachine {
            initial: "track".into(),
            initial_memory: AiPolicyMemory::new(),
            states: vec![
                state("track", AiPolicyVerb::ActuateDesiredFacing),
                state("escape", AiPolicyVerb::HoldCommittedHeading),
                state("recover", AiPolicyVerb::HoldRecoveryOrbit),
                state("reenter", AiPolicyVerb::PivotToReengage),
            ],
        }),
    };
    let memory = AiPolicyMemory::new();
    for (id, expected) in [
        ("track", AiPolicyVerb::ActuateDesiredFacing),
        ("escape", AiPolicyVerb::HoldCommittedHeading),
        ("recover", AiPolicyVerb::HoldRecoveryOrbit),
        ("reenter", AiPolicyVerb::PivotToReengage),
    ] {
        assert_eq!(
            p.resolve_channel_in_state(id, "yaw", &AiFacts::new(), &memory, &[]),
            Some(&expected),
            "state '{id}' must answer the yaw channel with its own verb"
        );
        // ...and drives no other axis.
        assert_eq!(
            p.resolve_channel_in_state(id, "longitudinal", &AiFacts::new(), &memory, &[]),
            None
        );
    }
    // The four are genuinely different values: a host matching on them can
    // tell the legs apart.
    assert_ne!(
        AiPolicyVerb::HoldRecoveryOrbit,
        AiPolicyVerb::HoldCommittedHeading
    );
    assert_ne!(
        AiPolicyVerb::PivotToReengage,
        AiPolicyVerb::ActuateDesiredFacing
    );
}

/// Issue #791: the `yaw` channel's SIXTH mode verb is a distinct value, and
/// distinct from `pivot_to_reengage` in particular — the one it would
/// otherwise be tempting to reuse, since the two fly identical geometry.
///
/// The host tells the legs apart by matching on the verb, so if these ever
/// became the same value a cruiser with no shield-recovery doctrine would
/// silently take a leg gated on six scalars it does not author, and simply
/// never fly the phase.
#[test]
fn the_torpedo_bearing_verb_is_its_own_yaw_mode_verb() {
    let p = AiPolicy {
        params: AiParams::new(),
        rules: Vec::new(),
        idle: false,
        machine: Some(AiPolicyMachine {
            initial: "orbit".into(),
            initial_memory: AiPolicyMemory::new(),
            states: vec![
                AiPolicyState {
                    id: "orbit".into(),
                    yields_to_arc_requests: true,
                    rules: vec![AiPolicyRule {
                        priority: 0,
                        channel: "yaw".into(),
                        when: parse_predicate("true").unwrap(),
                        verb: AiPolicyVerb::HoldCombatOrbit,
                    }],
                    transitions: Vec::new(),
                },
                AiPolicyState {
                    id: "torpedo_run".into(),
                    yields_to_arc_requests: true,
                    rules: vec![AiPolicyRule {
                        priority: 0,
                        channel: "yaw".into(),
                        when: parse_predicate("true").unwrap(),
                        verb: AiPolicyVerb::HoldTorpedoBearing,
                    }],
                    transitions: Vec::new(),
                },
            ],
        }),
    };
    let memory = AiPolicyMemory::new();
    assert_eq!(
        p.resolve_channel_in_state("torpedo_run", "yaw", &AiFacts::new(), &memory, &[]),
        Some(&AiPolicyVerb::HoldTorpedoBearing)
    );
    assert_eq!(
        p.resolve_channel_in_state("orbit", "yaw", &AiFacts::new(), &memory, &[]),
        Some(&AiPolicyVerb::HoldCombatOrbit),
        "the two legs must stay tellable apart"
    );
    assert_ne!(
        AiPolicyVerb::HoldTorpedoBearing,
        AiPolicyVerb::PivotToReengage
    );
    assert_ne!(
        AiPolicyVerb::HoldTorpedoBearing,
        AiPolicyVerb::ActuateDesiredFacing
    );
}

/// A recovery gate needs BOTH conjuncts (issue #788, AC6): the transition
/// fires only when the shield fraction AND the held-distance reading pass.
/// Pinned at the evaluator so the doctrine's `and` cannot silently degrade
/// to an `or` under a grammar change.
#[test]
fn a_two_conjunct_recovery_gate_takes_neither_half_alone() {
    let mut params = AiParams::new();
    params.set("reentry_shield_fraction", 0.75);
    let p = AiPolicy {
        params,
        rules: Vec::new(),
        idle: false,
        machine: Some(AiPolicyMachine {
            initial: "recover".into(),
            initial_memory: AiPolicyMemory::new(),
            states: vec![
                AiPolicyState {
                    id: "recover".into(),
                    yields_to_arc_requests: true,
                    rules: Vec::new(),
                    transitions: vec![AiPolicyTransition {
                        priority: 0,
                        to: "reenter".into(),
                        when: parse_predicate(
                            "fact(shield_fraction) >= param(reentry_shield_fraction) \
                             and fact(safe_distance_held) > 0",
                        )
                        .unwrap(),
                    }],
                },
                AiPolicyState {
                    id: "reenter".into(),
                    yields_to_arc_requests: true,
                    rules: Vec::new(),
                    transitions: Vec::new(),
                },
            ],
        }),
    };
    let memory = AiPolicyMemory::new();
    let facts = |shields: f64, held: f64| {
        let mut f = AiFacts::new();
        f.set("shield_fraction", shields);
        f.set("safe_distance_held", held);
        f
    };
    assert!(p
        .resolve_transition("recover", &facts(1.0, 0.0), &memory, &[])
        .is_none());
    assert!(p
        .resolve_transition("recover", &facts(0.5, 1.0), &memory, &[])
        .is_none());
    assert_eq!(
        p.resolve_transition("recover", &facts(0.75, 1.0), &memory, &[])
            .map(|t| t.to.as_str()),
        Some("reenter"),
        "the authored fraction is inclusive, and both halves together open the gate"
    );
    // Unseeded facts read absent, so an unwired host holds the ship on its
    // ring rather than letting it re-enter for free.
    assert!(p
        .resolve_transition("recover", &AiFacts::new(), &memory, &[])
        .is_none());
}

/// The read-only diagnostic scan (issue #1152): `blocking_transition` is the
/// exact inverse of `resolve_transition`. When a guard does NOT fire it names
/// the highest-priority such transition (with the same priority tie-break),
/// and when the guard DOES fire that transition is no longer "blocked".
#[test]
fn blocking_transition_names_the_highest_priority_unsatisfied_edge() {
    let p = AiPolicy {
        params: AiParams::new(),
        rules: Vec::new(),
        idle: false,
        machine: Some(AiPolicyMachine {
            initial: "hold".into(),
            initial_memory: AiPolicyMemory::new(),
            states: vec![
                AiPolicyState {
                    id: "hold".into(),
                    yields_to_arc_requests: true,
                    rules: Vec::new(),
                    transitions: vec![
                        AiPolicyTransition {
                            priority: 10,
                            to: "engage".into(),
                            when: parse_predicate("fact(threat) > 0").unwrap(),
                        },
                        AiPolicyTransition {
                            priority: 0,
                            to: "patrol".into(),
                            when: parse_predicate("fact(bored) > 0").unwrap(),
                        },
                    ],
                },
                AiPolicyState {
                    id: "engage".into(),
                    yields_to_arc_requests: true,
                    rules: Vec::new(),
                    transitions: Vec::new(),
                },
                AiPolicyState {
                    id: "patrol".into(),
                    yields_to_arc_requests: true,
                    rules: Vec::new(),
                    transitions: Vec::new(),
                },
            ],
        }),
    };
    let memory = AiPolicyMemory::new();

    // No fact fires: BOTH guards are unsatisfied, so the blocked edge is the
    // highest-priority one (`engage`), and nothing resolves.
    assert!(p
        .resolve_transition("hold", &AiFacts::new(), &memory, &[])
        .is_none());
    let blocked = p
        .blocking_transition("hold", &AiFacts::new(), &memory, &[])
        .expect("a guard is blocking");
    assert_eq!(blocked.to, "engage");
    assert_eq!(blocked.when.render(), "fact(threat) > 0");

    // Threat present: `engage` now FIRES, so it is no longer blocked — the
    // blocked edge falls through to the still-unsatisfied `patrol`.
    let mut facts = AiFacts::new();
    facts.set("threat", 1.0);
    assert_eq!(
        p.resolve_transition("hold", &facts, &memory, &[])
            .map(|t| t.to.as_str()),
        Some("engage")
    );
    assert_eq!(
        p.blocking_transition("hold", &facts, &memory, &[])
            .map(|t| t.to.as_str()),
        Some("patrol")
    );

    // Every guard satisfied: nothing is blocked.
    facts.set("bored", 1.0);
    assert!(p
        .blocking_transition("hold", &facts, &memory, &[])
        .is_none());
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
                    yields_to_arc_requests: true,
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
                    yields_to_arc_requests: true,
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
                    yields_to_arc_requests: true,
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
                    yields_to_arc_requests: true,
                    rules: Vec::new(),
                    transitions: Vec::new(),
                },
                AiPolicyState {
                    id: "second".into(),
                    yields_to_arc_requests: true,
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
                yields_to_arc_requests: true,
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
                    yields_to_arc_requests: true,
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
                    yields_to_arc_requests: true,
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
                    yields_to_arc_requests: true,
                    rules: vec![AiPolicyRule {
                        priority: 0,
                        channel: "yaw".into(),
                        when: parse_predicate("true").unwrap(),
                        verb: AiPolicyVerb::HoldCommittedHeading,
                    }],
                    transitions: vec![AiPolicyTransition {
                        priority: 0,
                        to: "acquire".into(),
                        when: parse_predicate("state_time >= param(escape_duration_secs)").unwrap(),
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

// ── Authored history operators (issue #890) ─────────────────────────────

/// A two-state machine whose TRANSITION asks a windowed question and whose
/// per-state RULE asks the other one, both over the same authored fact.
///
/// Both positions matter: a transition guard is resolved by the state-tick
/// host, a per-state rule guard by a per-axis actuator system later in the
/// same tick. They read one bag, so they must agree.
fn windowed_policy() -> AiPolicy {
    let mut params = AiParams::new();
    params.set("standoff_ticks", 3.0);
    params.set("safe_range", 40.0);
    params.set("escape_ticks", 3.0);
    AiPolicy {
        params,
        rules: Vec::new(),
        idle: false,
        machine: Some(AiPolicyMachine {
            initial: "closing".into(),
            initial_memory: AiPolicyMemory::new(),
            states: vec![
                AiPolicyState {
                    id: "closing".into(),
                    yields_to_arc_requests: true,
                    rules: Vec::new(),
                    transitions: vec![AiPolicyTransition {
                        priority: 0,
                        to: "standing_off".into(),
                        when: parse_predicate(
                            "history(min, range_to_target, param(standoff_ticks)) \
                             >= param(safe_range)",
                        )
                        .unwrap(),
                    }],
                },
                AiPolicyState {
                    id: "standing_off".into(),
                    yields_to_arc_requests: true,
                    rules: vec![AiPolicyRule {
                        priority: 0,
                        channel: "longitudinal".into(),
                        when: parse_predicate(
                            "history(net_change, range_to_target, param(escape_ticks)) > 0",
                        )
                        .unwrap(),
                        verb: AiPolicyVerb::ActuateDesiredTravel,
                    }],
                    transitions: Vec::new(),
                },
            ],
        }),
    }
}

fn range_facts(range: f64) -> AiFacts {
    let mut f = AiFacts::new();
    f.set("range_to_target", range);
    f
}

/// The host's contract: the windows a policy asks for, resolved against its
/// own authored params, collected from EVERY authorable guard position.
#[test]
fn history_windows_collects_every_authorable_position() {
    let specs = windowed_policy().history_windows();
    assert_eq!(
        specs,
        vec![crate::world::flags::HistorySpec {
            fact: "range_to_target".into(),
            ticks: 3,
        }],
        "one shared window: both guards ask the same fact over the same span"
    );

    // Two atoms over the same fact with DIFFERENT authored lengths are two
    // windows, not one — the #789 shape.
    let mut params = AiParams::new();
    params.set("short", 2.0);
    params.set("long", 9.0);
    let two = AiPolicy {
        params,
        rules: vec![AiPolicyRule {
            priority: 0,
            channel: "c".into(),
            when: parse_predicate(
                "history(min, r, param(short)) > 0 and history(net_change, r, param(long)) > 0",
            )
            .unwrap(),
            verb: AiPolicyVerb::SetRedAlert(true),
        }],
        idle: false,
        machine: None,
    };
    let specs = two.history_windows();
    assert_eq!(specs.len(), 2);
    assert_eq!(
        specs.iter().map(|s| s.ticks).collect::<Vec<_>>(),
        vec![2, 9]
    );
}

/// AC: a windowed question is genuinely evaluated in a TRANSITION guard,
/// and only once the authored span has really been held.
#[test]
fn a_windowed_transition_guard_fires_only_after_the_authored_span() {
    let policy = windowed_policy();
    let specs = policy.history_windows();
    let mut state = AiPolicyRuntimeState::reset(&policy, 0.0);

    for tick in 1..=2 {
        let facts = range_facts(50.0);
        state.memory.fold_history(&specs, &facts);
        assert!(
            policy
                .resolve_transition(&state.current, &facts, &state.memory_at(tick as f64), &[])
                .is_none(),
            "tick {tick}: a window shorter than the authored three has not been held"
        );
    }

    let facts = range_facts(50.0);
    state.memory.fold_history(&specs, &facts);
    let to = policy
        .resolve_transition(&state.current, &facts, &state.memory_at(3.0), &[])
        .expect("the third sample completes the authored window")
        .to
        .clone();
    assert_eq!(to, "standing_off");
}

/// AC: the SAME window is readable from a per-state RULE guard — the
/// position the #788/#789 bespoke facts could not be authored in.
#[test]
fn a_windowed_rule_guard_reads_the_same_folded_window() {
    let policy = windowed_policy();
    let specs = policy.history_windows();
    let mut state = AiPolicyRuntimeState::reset(&policy, 0.0);
    state.enter("standing_off", 0.0);

    // Two samples of the authored three: the trend is not yet measurable.
    for range in [10.0, 20.0] {
        state.memory.fold_history(&specs, &range_facts(range));
    }
    assert_eq!(
        policy.resolve_channel_in_state(
            &state.current,
            "longitudinal",
            &range_facts(20.0),
            &state.memory_at(2.0),
            &[],
        ),
        None,
        "a rule guard over an unfilled window must hold, not fire"
    );

    state.memory.fold_history(&specs, &range_facts(40.0));
    assert_eq!(
        policy.resolve_channel_in_state(
            &state.current,
            "longitudinal",
            &range_facts(40.0),
            &state.memory_at(3.0),
            &[],
        ),
        Some(&AiPolicyVerb::ActuateDesiredTravel),
        "the rule guard must read the window the host folded, exactly as the \
         transition guard does"
    );
}

/// AC5's reset covers the window too: a system that loses and regains AI
/// control never answers a windowed question from evidence gathered before
/// it was in charge.
#[test]
fn resetting_the_runtime_state_discards_the_history_windows() {
    let policy = windowed_policy();
    let specs = policy.history_windows();
    let mut state = AiPolicyRuntimeState::reset(&policy, 0.0);
    for _ in 0..3 {
        state.memory.fold_history(&specs, &range_facts(50.0));
    }
    assert!(policy
        .resolve_transition(
            &state.current,
            &range_facts(50.0),
            &state.memory_at(3.0),
            &[]
        )
        .is_some());

    state = AiPolicyRuntimeState::reset(&policy, 4.0);
    assert!(state.memory.history().is_empty());
    assert!(
        policy
            .resolve_transition(
                &state.current,
                &range_facts(50.0),
                &state.memory_at(4.0),
                &[]
            )
            .is_none(),
        "a re-entered policy must re-earn its window rather than inherit one"
    );
}
