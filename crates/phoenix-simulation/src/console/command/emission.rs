//! Ordinary admitted Command intent and own measured advisory projection.
use crate::{
    core::messages::{AdmittedCommands, SystemBlackboard, SystemControlPayload, SystemId},
    ship::power::ShipPowerSystem,
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_model::emission_profile::{EmissionAdvisoryView, EmissionIntent};
use phoenix_sim_gameplay::ship::emission_profile::{profile, EmissionIntentState};

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ShipEmissionIntent(pub EmissionIntentState);

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    app.declare_state::<ShipEmissionIntent>(StateClass::Folded, "ship-emission-intent")
        .register_admitted_consumer(
            ConsumerMatcher::kind_payload(
                crate::ship::system_registry::COMMAND_KIND,
                Payload::SetEmissionIntent,
            )
            .with_feedback(
                FeedbackAddress::DeclaredKindOnly,
                &[Payload::SetEmissionIntent],
            ),
        )
        .add_systems(
            FixedUpdate,
            handle_commands
                .in_set(crate::sim_sets::SimSet::Input)
                .in_set(crate::sim_sets::FixedStep::HandleEmissionIntent),
        );
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some((owner, profile)) = config
        .ship_config
        .as_ref()
        .and_then(|ship| profile(ship).ok().flatten())
    {
        commands.insert(ShipEmissionIntent(EmissionIntentState::new(
            owner, &profile,
        )));
    }
}

pub(crate) fn handle_commands(
    mut owners: Query<(
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &AdmittedCommands,
        &mut ShipEmissionIntent,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::server::OutboundMessage>>>,
) {
    for (config, sources, commands, mut state) in &mut owners {
        for command in &commands.0 {
            let SystemControlPayload::SetEmissionIntent { intent } = &command.payload else {
                continue;
            };
            if command.target != state.0.owner {
                continue;
            }
            let control = sources.0.policy_for(&command.target);
            let applied = state.0.validate(&config.0).is_ok()
                && sources
                    .0
                    .entries()
                    .any(|(owner, _)| owner == &command.target)
                && (control.accept_human_input || control.operate_ai);
            if applied {
                state.0.intent = *intent;
            }
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbound,
                command,
                if applied {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}

/// Only the requesting System's explicit permitted Command feed conveys intent.
/// Co-hosted tabs and the physical canonical intent are deliberately not inputs.
pub(crate) fn permitted_intent(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    owner: &SystemId,
) -> Option<EmissionIntent> {
    let (command_owner, _) = profile(config).ok()??;
    let permitted = config.observations_for_system(owner, &boards.0);
    let SystemBlackboard::Command(board) = permitted.get(&command_owner)? else {
        return None;
    };
    let view = board.emission.as_ref()?;
    (view.owner == command_owner && board.command_system_id == command_owner).then_some(view.intent)
}

/// Unknown context remains distinct from a known normal alert/profile. No live
/// ship alert or other Console's publication enters the controller implicitly.
pub(crate) fn policy_facts(
    config: &crate::ship::config::ShipConfig,
    boards: Option<&crate::server_app::ShipSystemBlackboards>,
    owner: &SystemId,
) -> std::collections::BTreeMap<String, bool> {
    let intent = boards.and_then(|boards| permitted_intent(config, boards, owner));
    let mut alerts = std::collections::BTreeSet::new();
    if let Some(boards) = boards {
        let permitted = config.observations_for_system(owner, &boards.0);
        for source in boards.0.keys() {
            if let Some(SystemBlackboard::Captain(board)) = permitted.get(source) {
                alerts.insert(board.red_alert);
            }
        }
    }
    let alert = (alerts.len() == 1).then(|| *alerts.first().unwrap());
    [
        ("emission_profile_known".into(), intent.is_some()),
        (
            "low_emission".into(),
            intent == Some(EmissionIntent::LowEmission),
        ),
        ("alert_known".into(), alert.is_some()),
        ("red_alert".into(), alert == Some(true)),
    ]
    .into()
}

/// Called with existing per-System details at Publish, after actual machinery
/// work. It does not publish a whole machinery census or change any controls.
pub(crate) fn advisory(
    world: &World,
    entity: Entity,
    owner: &SystemId,
) -> Option<EmissionAdvisoryView> {
    let config = world.get::<ShipConfigComponent>(entity)?;
    let boards = world.get::<crate::server_app::ShipSystemBlackboards>(entity)?;
    if permitted_intent(&config.0, boards, owner)? != EmissionIntent::LowEmission {
        return None;
    }
    let (_, profile) = profile(&config.0).ok()??;
    let system = config.0.system(owner)?;
    let tick = world.get_resource::<crate::sim_tick::SimTick>()?.0;
    let dt = world
        .get_resource::<Time<Fixed>>()?
        .timestep()
        .as_secs_f64();
    if !dt.is_finite() || dt <= 0.0 {
        return None;
    }
    let power = world.get::<ShipPowerSystem>(entity)?;
    let (rate, cause) = if system.kind == crate::ship::system_registry::POWER_REACTOR_KIND {
        (
            if power.0.network().generation_enabled {
                f64::from(power.0.network().output_actual_per_sec)
            } else {
                0.0
            },
            phoenix_model::sensing::PassiveSignature::Reactor,
        )
    } else {
        (
            power.0.emitted_work(owner) / dt,
            phoenix_sim_gameplay::science::signatures::work_cause(&system.kind),
        )
    };
    if !rate.is_finite() || rate < 0.0 {
        return None;
    }
    Some(EmissionAdvisoryView {
        observed_tick: tick,
        rate: rate.min(f64::from(f32::MAX)).to_string(),
        cause,
        advisory_rate: profile.advisory_rate.to_string(),
        high: rate > f64::from(profile.advisory_rate),
    })
}

#[cfg(test)]
#[path = "emission_integration_tests.rs"]
mod integration_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use phoenix_model::emission_profile::EmissionIntentView;
    #[test]
    fn optional_fit_uses_the_actual_accepted_hull_without_replacing_hardware_or_seats() {
        let base = crate::entities::include_resolve::load_entity_config(
            "assets/entities/identity_cruiser.toml",
        )
        .unwrap();
        let fit = crate::entities::include_resolve::load_entity_config(
            "assets/entities/emission_profile_cruiser.toml",
        )
        .unwrap();
        assert_eq!(base.energy, fit.energy);
        assert_eq!(base.interior, fit.interior);
        let base = base.ship_config.unwrap();
        let fit = fit.ship_config.unwrap();
        assert_eq!(base.stations, fit.stations);
        assert_eq!(base.systems.len(), fit.systems.len());
        for system in &base.systems {
            let mut expected = system.clone();
            let mut actual = fit.system(&system.id).unwrap().clone();
            if matches!(
                system.kind.as_str(),
                crate::ship::system_registry::COMMAND_KIND
                    | crate::ship::system_registry::IDENTITY_EMITTER_KIND
            ) {
                expected.config = None;
                actual.config = None;
            }
            assert_eq!(
                actual, expected,
                "actual inherited machinery {}",
                system.id.0
            );
        }
        let (owner, profile) = profile(&fit).unwrap().unwrap();
        assert_eq!(owner, SystemId("command".into()));
        assert_eq!(profile.initial, EmissionIntent::Normal);
        // Exercise the actual inherited tree/tasks, not a recreated quiet policy.
        use phoenix_sim_gameplay::ai::state_tree::{compile, step, AiTreeConfig, TreeRuntime};
        let ai: AiTreeConfig = fit
            .system(&SystemId("identity-emitter".into()))
            .unwrap()
            .config
            .as_ref()
            .unwrap()["ai"]
            .clone()
            .try_into()
            .unwrap();
        let program = compile(
            &ai,
            &phoenix_sim_gameplay::electronic::identity::bindings("enabled"),
        )
        .unwrap();
        let mut runtime = TreeRuntime::default();
        for (index, (known, low, alert_known, red_alert, expected)) in [
            (true, false, true, false, true),
            (true, true, true, false, false),
            (true, true, true, true, true),
            (false, false, false, false, true),
        ]
        .into_iter()
        .enumerate()
        {
            let facts = [
                ("enabled".into(), true),
                ("emission_profile_known".into(), known),
                ("low_emission".into(), low),
                ("alert_known".into(), alert_known),
                ("red_alert".into(), red_alert),
            ]
            .into();
            let pending = [("enabled".into(), true)].into();
            assert_eq!(
                step(&program, &facts, &pending, &mut runtime, index as u64 + 1)
                    .unwrap()
                    .intents["enabled"],
                expected
            );
        }
    }
    #[test]
    fn cohosting_does_not_convey_profile_or_alert_and_conflicting_alerts_are_unknown() {
        let mut config: crate::ship::config::ShipConfig = toml::from_str(
            r#"
[[console]]
id = "orders"
[[console]]
id = "signals"
[[system]]
id = "quiet-command"
kind = "command"
console = "orders"
[system.config.emission_profile]
advisory_rate = 0.5
[[system]]
id = "transponder"
kind = "identity-emitter"
console = "signals"
"#,
        )
        .unwrap();
        let owner = SystemId("transponder".into());
        let command = SystemId("quiet-command".into());
        let mut boards = crate::server_app::ShipSystemBlackboards::default();
        boards.0.insert(
            command.clone(),
            SystemBlackboard::Command(crate::core::messages::CommandBlackboard {
                command_system_id: command.clone(),
                emission: Some(EmissionIntentView {
                    owner: command.clone(),
                    intent: EmissionIntent::LowEmission,
                    controllable: false,
                }),
                ..Default::default()
            }),
        );
        boards.0.insert(
            SystemId("alert-report".into()),
            SystemBlackboard::Captain(Default::default()),
        );
        let facts = policy_facts(&config, Some(&boards), &owner);
        assert!(!facts["emission_profile_known"] && !facts["alert_known"]);
        config.consoles[1].feeds = vec![
            phoenix_model::console::ConsoleFeed {
                source: command,
                kind: phoenix_model::console::ConsoleObservationKind::Command,
            },
            phoenix_model::console::ConsoleFeed {
                source: SystemId("alert-report".into()),
                kind: phoenix_model::console::ConsoleObservationKind::Captain,
            },
        ];
        let facts = policy_facts(&config, Some(&boards), &owner);
        assert!(facts["emission_profile_known"] && facts["low_emission"] && facts["alert_known"]);
        assert!(!facts["red_alert"]);
        boards.0.insert(
            SystemId("other-alert".into()),
            SystemBlackboard::Captain(crate::core::messages::CaptainBlackboard {
                red_alert: true,
                ..Default::default()
            }),
        );
        config.consoles[1]
            .feeds
            .push(phoenix_model::console::ConsoleFeed {
                source: SystemId("other-alert".into()),
                kind: phoenix_model::console::ConsoleObservationKind::Captain,
            });
        assert!(!policy_facts(&config, Some(&boards), &owner)["alert_known"]);
        assert!(!policy_facts(&config, None, &owner)["low_emission"]);
    }
}
