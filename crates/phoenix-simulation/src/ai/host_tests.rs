use super::*;
use crate::ai::policy::AiPolicyRule;
use crate::core::messages::SystemId;
use crate::ship::control_source::ControlSource;
use crate::world::flags::parse_predicate;

fn sid() -> SystemId {
    SystemId("red-alert".into())
}

/// A one-rule red-alert policy that fires only while `fact(threat) > 0`, so
/// the same policy resolves `Act` or `Held` depending purely on the facts.
fn threat_policy() -> AiPolicy {
    AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![AiPolicyRule {
            priority: 0,
            channel: "red_alert".into(),
            when: parse_predicate("fact(threat) > 0").unwrap(),
            verb: AiPolicyVerb::SetRedAlert(true),
        }],
        idle: false,
        machine: None,
    }
}

fn ai_sources() -> ControlSourceResolver {
    let mut s = ControlSourceResolver::new();
    s.set(sid(), ControlSource::Ai);
    s
}

fn tick_with<'a>(facts: &'a AiFacts, flags: &'a [&'a FlagStore]) -> HostTick<'a> {
    HostTick {
        system: sid(),
        channel: "red_alert",
        facts,
        flags,
        state: None,
    }
}

#[test]
fn direct_gate_and_decide_respect_control_sources_and_overrides() {
    let policy = threat_policy();
    let mut facts = AiFacts::new();
    facts.set("threat", 1.0);
    let tick = tick_with(&facts, &[]);
    for (source, operates) in [
        (None, false),
        (Some(ControlSource::Human), false),
        (Some(ControlSource::Offline), false),
        (Some(ControlSource::Ai), true),
        (Some(ControlSource::Simplified), true),
    ] {
        for (damage_offline, gm_disabled, destroyed) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut sources = ControlSourceResolver::new();
            if let Some(source) = source {
                sources.set(sid(), source);
            }
            sources.set_offline(sid(), damage_offline);
            sources.set_gm_disabled(sid(), gm_disabled);
            sources.set_destroyed(destroyed);
            let expected = operates && !damage_offline && !gm_disabled && !destroyed;
            let context = format!(
                "source={source:?}, damage={damage_offline}, GM={gm_disabled}, destroyed={destroyed}"
            );
            assert_eq!(ai_operates(&sources, &tick.system), expected, "{context}");
            assert_eq!(
                decide(&sources, Some(&policy), &tick),
                if expected {
                    HostOutcome::Act(&AiPolicyVerb::SetRedAlert(true))
                } else {
                    HostOutcome::NotAiOperated
                },
                "{context}"
            );
        }
    }
}

#[test]
fn not_ai_operated_when_a_human_holds_the_system() {
    // Default source is Human → operate_ai is false → the AI stands down
    // before the policy is ever consulted (an armed policy proves the gate,
    // not the resolution, produced this).
    let sources = ControlSourceResolver::new();
    let policy = threat_policy();
    let mut facts = AiFacts::new();
    facts.set("threat", 1.0);
    assert_eq!(
        decide(&sources, Some(&policy), &tick_with(&facts, &[])),
        HostOutcome::NotAiOperated
    );
}

#[test]
fn not_ai_operated_when_the_system_is_offline() {
    // Damage/rating offline overrides even an explicit Ai source.
    let mut sources = ai_sources();
    sources.set_offline(sid(), true);
    let policy = threat_policy();
    let mut facts = AiFacts::new();
    facts.set("threat", 1.0);
    assert_eq!(
        decide(&sources, Some(&policy), &tick_with(&facts, &[])),
        HostOutcome::NotAiOperated
    );
}

#[test]
fn undeclared_when_ai_operated_but_no_policy_is_authored() {
    // AI holds the system, but there is no policy: strict declaration mode
    // means it does nothing, and the outcome names WHY.
    let sources = ai_sources();
    assert_eq!(
        decide(&sources, None, &tick_with(&AiFacts::new(), &[])),
        HostOutcome::Undeclared
    );
}

#[test]
fn held_when_declared_but_no_rule_fires_this_tick() {
    // AI + declared, but the guard's fact is unseeded so it reads absent and
    // the only rule holds. This is the ordinary steady-state, distinct from
    // both undeclared and not-AI.
    let sources = ai_sources();
    let policy = threat_policy();
    assert_eq!(
        decide(&sources, Some(&policy), &tick_with(&AiFacts::new(), &[])),
        HostOutcome::Held
    );
}

#[test]
fn act_returns_the_winning_verb_when_a_rule_fires() {
    // AI + declared + the guard's fact crosses its threshold → the winning
    // verb is handed back by borrow.
    let sources = ai_sources();
    let policy = threat_policy();
    let mut facts = AiFacts::new();
    facts.set("threat", 1.0);
    assert_eq!(
        decide(&sources, Some(&policy), &tick_with(&facts, &[])),
        HostOutcome::Act(&AiPolicyVerb::SetRedAlert(true))
    );
}

#[test]
fn a_flag_guard_reads_the_supplied_flag_chain() {
    // The flags handed in the tick actually reach the evaluator: a rule
    // gated on a world flag fires only when that flag is set in the chain.
    let sources = ai_sources();
    let policy = AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![AiPolicyRule {
            priority: 0,
            channel: "red_alert".into(),
            when: parse_predicate("flag(general_quarters)").unwrap(),
            verb: AiPolicyVerb::SetRedAlert(true),
        }],
        idle: false,
        machine: None,
    };
    let facts = AiFacts::new();

    // Absent flag → Held.
    assert_eq!(
        decide(&sources, Some(&policy), &tick_with(&facts, &[])),
        HostOutcome::Held
    );

    // Set flag in the chain → Act.
    let mut store = FlagStore::new();
    store.set_flag("general_quarters");
    let chain = [&store];
    assert_eq!(
        decide(&sources, Some(&policy), &tick_with(&facts, &chain)),
        HostOutcome::Act(&AiPolicyVerb::SetRedAlert(true))
    );
}

#[test]
fn a_stateful_tick_resolves_the_named_state_only() {
    // With a HostState the per-state path runs: the `armed` state fires, and
    // a tick naming a state that authors no rule on the channel holds.
    let sources = ai_sources();
    let state = |id: &str, verb: AiPolicyVerb| crate::ai::policy::AiPolicyState {
        id: id.into(),
        yields_to_arc_requests: true,
        rules: vec![AiPolicyRule {
            priority: 0,
            channel: "red_alert".into(),
            when: parse_predicate("true").unwrap(),
            verb,
        }],
        transitions: Vec::new(),
    };
    let policy = AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: Vec::new(),
        idle: false,
        machine: Some(crate::ai::policy::AiPolicyMachine {
            initial: "calm".into(),
            initial_memory: AiPolicyMemory::new(),
            states: vec![
                crate::ai::policy::AiPolicyState {
                    id: "calm".into(),
                    yields_to_arc_requests: true,
                    rules: Vec::new(),
                    transitions: Vec::new(),
                },
                state("armed", AiPolicyVerb::SetRedAlert(true)),
            ],
        }),
    };
    let facts = AiFacts::new();
    let memory = AiPolicyMemory::new();

    let armed = HostTick {
        system: sid(),
        channel: "red_alert",
        facts: &facts,
        flags: &[],
        state: Some(HostState {
            current: "armed",
            memory: &memory,
        }),
    };
    assert_eq!(
        decide(&sources, Some(&policy), &armed),
        HostOutcome::Act(&AiPolicyVerb::SetRedAlert(true))
    );

    let calm = HostTick {
        state: Some(HostState {
            current: "calm",
            memory: &memory,
        }),
        ..armed.clone()
    };
    assert_eq!(decide(&sources, Some(&policy), &calm), HostOutcome::Held);
}

/// The point of the whole slice (issue #1207): a fixture that runs a host
/// through [`AiHostEnv`] but forgot [`register_ai_host_env`] fails LOUDLY,
/// so a bare `App` cannot silently take a different code path than the
/// shipped app. The env's fields are bare [`Res`], and Bevy validates a
/// missing required resource by panicking (`Res` is a required param, unlike
/// `Option<Res<..>>`), so the same system that runs in production panics at
/// the first schedule run in an unregistered fixture — and runs cleanly once
/// the one registration call every real app makes has been made.
#[test]
fn a_host_reading_the_env_panics_in_a_bare_app_that_skipped_registration() {
    use bevy::prelude::*;

    // Stand-in for every converted host: it consumes `AiHostEnv` and nothing
    // else, so the only resources its validation can trip over are the ones
    // `register_ai_host_env` installs.
    fn reads_the_env(env: AiHostEnv) {
        // Touch a behaviour so the borrow of the bare `Res` fields is real.
        let _ = env.content_runtime();
    }

    let run = |register: bool| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut app = App::new();
            if register {
                register_ai_host_env(&mut app);
            }
            app.add_systems(Update, reads_the_env);
            app.update();
        }))
    };

    // Swallow the deliberate panic's backtrace so the passing test is quiet.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let unregistered = run(false);
    let registered = run(true);
    std::panic::set_hook(prev);

    assert!(
        unregistered.is_err(),
        "a host consuming AiHostEnv in a bare App that never called \
             register_ai_host_env must panic at schedule run — the guarantee \
             that a fixture cannot diverge from production"
    );
    assert!(
        registered.is_ok(),
        "the identical host runs cleanly once register_ai_host_env has \
             installed the env's resources"
    );
}
