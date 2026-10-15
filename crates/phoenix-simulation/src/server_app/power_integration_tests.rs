//! Whole-host checks for numeric energy control and scoped observations. The
//! detailed owner/admission/continuation matrix lives in ship::power::tests.
use super::*;
use crate::ship::power::ShipPowerSystem;
use crate::sim_rng::InstallSimRng;

fn power_host() -> App {
    let mut app = test_app();
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_battleship.toml",
    )
    .unwrap();
    let entity = app.world().resource::<ShipEntity>().0;
    let power = crate::modifiers::power_system::PowerSystem::from_energy(
        config.energy.clone().unwrap(),
        config.ship_config.clone().unwrap(),
    )
    .unwrap();
    app.world_mut().entity_mut(entity).insert((
        ShipPowerSystem(power),
        crate::entities::spawner::EntityUuid("host-energy-ship".into()),
        crate::resources::server::ResourceServices(config.resource_services),
        crate::infrastructure::InfrastructureCondition(
            crate::infrastructure::InfrastructureState::from_config(
                config.infrastructure.as_ref().unwrap(),
            ),
        ),
    ));
    app.init_resource::<crate::world::server::WorldContentRuntime>()
        .insert_sim_rng(crate::sim_rng::SimRng::new(
            715,
            crate::sim_rng::SeedSource::World,
        ));
    app
}
fn start_power_crew(app: &mut App) {
    for (token, station) in [("captain", "Captain"), ("power", "Power")] {
        push(
            app,
            token,
            ClientMessage::Identify {
                token: token.into(),
                name: token.into(),
            },
        );
        tick(app);
        push(
            app,
            token,
            ClientMessage::SelectStation {
                station: station.into(),
            },
        );
        tick(app);
    }
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "power", ClientMessage::SetReady { ready: true });
    tick(app);
    fast_forward_countdown(app);
    tick(app);
    tick(app);
}
fn owner(app: &App) -> &crate::modifiers::power_system::PowerSystem {
    &app.world()
        .get::<ShipPowerSystem>(app.world().resource::<ShipEntity>().0)
        .unwrap()
        .0
}
fn send_energy(
    app: &mut App,
    token: &str,
    id: &str,
    payload: SystemControlPayload,
) -> Vec<OutboundMessage> {
    push(
        app,
        token,
        ClientMessage::ControlSystemCorrelated {
            target: crate::ship::system_registry::power_reactor_system_id(),
            correlation: ActionCorrelationId::new(id).unwrap(),
            payload,
        },
    );
    tick(app)
}
fn group_request(flow: f32, cooling: f32) -> SystemControlPayload {
    SystemControlPayload::SetEnergyGroup {
        group: PowerGroupId("helm".into()),
        flow_per_sec: flow,
        cooling_per_sec: cooling,
    }
}
fn expect_feedback(
    out: &[OutboundMessage],
    token: &str,
    id: &str,
    expected: ActionFeedbackOutcome,
) {
    let matching: Vec<_> = out
        .iter()
        .filter_map(|message| match (&message.target, &message.msg) {
            (
                Target::Token(actual),
                ServerMessage::ActionFeedback {
                    correlation,
                    outcome,
                },
            ) if actual == token && correlation.as_str() == id => Some(*outcome),
            _ => None,
        })
        .collect();
    assert_eq!(matching, vec![expected]);
}

#[test]
fn power_host_numeric_command_reaches_the_owner_and_next_scoped_snapshot() {
    let mut app = power_host();
    start_power_crew(&mut app);
    let maximum = owner(&app)
        .config()
        .groups
        .iter()
        .find(|group| group.id.0 == "helm")
        .unwrap()
        .max_flow_per_sec;
    let out = send_energy(
        &mut app,
        "power",
        "host-flow",
        group_request(maximum * 0.4, 0.0),
    );
    expect_feedback(&out, "power", "host-flow", ActionFeedbackOutcome::Applied);
    assert_eq!(
        owner(&app).flow_for(&PowerGroupId("helm".into())),
        Some(maximum * 0.4)
    );
    let out = tick(&mut app);
    let projection = out
        .iter()
        .rev()
        .find_map(|message| match (&message.target, &message.msg) {
            (
                Target::Token(token),
                ServerMessage::BlackboardUpdate {
                    updates,
                    console_scope: Some(scope),
                    ..
                },
            ) if token == "power" => updates.iter().find_map(|(id, board)| match board {
                SystemBlackboard::Power(bb) if scope.contains(id) => Some((bb, message.delivery)),
                _ => None,
            }),
            _ => None,
        })
        .expect("the actual host must deliver a scoped Power observation");
    assert_eq!(projection.1, DeliveryClass::Snapshot);
    let energy = projection.0.energy.as_ref().unwrap();
    assert_eq!(
        energy
            .groups
            .iter()
            .find(|group| group.id == "helm")
            .unwrap()
            .desired_flow_per_sec,
        maximum * 0.4
    );
    assert_eq!(energy.reactor.energy, owner(&app).network().reactor.energy);
}

#[test]
fn power_host_rejects_wrong_operator_and_invalid_pair_without_changing_flow() {
    let mut app = power_host();
    start_power_crew(&mut app);
    let before = owner(&app).flow_for(&PowerGroupId("helm".into()));
    let out = send_energy(
        &mut app,
        "captain",
        "host-wrong-holder",
        group_request(0.0, 0.0),
    );
    expect_feedback(
        &out,
        "captain",
        "host-wrong-holder",
        ActionFeedbackOutcome::Refused,
    );
    assert_eq!(owner(&app).flow_for(&PowerGroupId("helm".into())), before);
    let maximum = owner(&app)
        .config()
        .groups
        .iter()
        .find(|group| group.id.0 == "helm")
        .unwrap()
        .max_cooling_per_sec;
    let out = send_energy(
        &mut app,
        "power",
        "host-invalid-pair",
        group_request(0.0, maximum + 1.0),
    );
    expect_feedback(
        &out,
        "power",
        "host-invalid-pair",
        ActionFeedbackOutcome::Refused,
    );
    assert_eq!(owner(&app).flow_for(&PowerGroupId("helm".into())), before);
}

#[test]
fn power_host_incoming_flow_does_not_multiply_equipment_outputs() {
    let mut app = power_host();
    start_power_crew(&mut app);
    let before = get_ship_modifiers(&mut app);
    let maximum = owner(&app)
        .config()
        .groups
        .iter()
        .find(|group| group.id.0 == "helm")
        .unwrap()
        .max_flow_per_sec;
    for flow in [0.0, maximum] {
        let out = send_energy(&mut app, "power", "host-output", group_request(flow, 0.0));
        expect_feedback(&out, "power", "host-output", ActionFeedbackOutcome::Applied);
        let after = get_ship_modifiers(&mut app);
        for slot in [
            ModifierSlot::MaxSpeed,
            ModifierSlot::PhaserDamage,
            ModifierSlot::ShieldRegen,
        ] {
            assert_eq!(
                after.get(&slot),
                before.get(&slot),
                "incoming flow changes a reserve's fill rate, not {slot:?}"
            );
        }
    }
}

#[test]
fn power_host_supply_loss_cuts_intake_but_keeps_requests_reserves_and_controls() {
    let mut app = power_host();
    start_power_crew(&mut app);
    let requested: Vec<_> = owner(&app)
        .network()
        .groups
        .iter()
        .map(|(id, group)| (id.clone(), group.desired_flow_per_sec))
        .collect();
    let entity = app.world().resource::<ShipEntity>().0;
    {
        let mut power = app.world_mut().get_mut::<ShipPowerSystem>(entity).unwrap();
        power.0.network_mut().reactor.energy = 0.0;
        power.0.network_mut().output_actual_per_sec = 0.0;
    }
    let out = send_energy(
        &mut app,
        "power",
        "host-no-supply",
        SystemControlPayload::SetEnergyReactor {
            output_per_sec: 0.0,
            cooling_per_sec: 0.0,
        },
    );
    expect_feedback(
        &out,
        "power",
        "host-no-supply",
        ActionFeedbackOutcome::Applied,
    );
    assert!(owner(&app)
        .network()
        .groups
        .values()
        .any(|group| group.cut && group.effective_flow_per_sec == 0.0));
    assert!(owner(&app)
        .network()
        .consumers
        .values()
        .any(|reserve| reserve.energy > 0.0));
    for (id, flow) in requested {
        assert_eq!(owner(&app).flow_for(&id), Some(flow));
    }
    let output = owner(&app).config().reactor.sustainable_output_per_sec;
    let out = send_energy(
        &mut app,
        "power",
        "host-recover",
        SystemControlPayload::SetEnergyReactor {
            output_per_sec: output,
            cooling_per_sec: 0.0,
        },
    );
    expect_feedback(
        &out,
        "power",
        "host-recover",
        ActionFeedbackOutcome::Applied,
    );
    assert_eq!(owner(&app).network().output_target_per_sec, output);
}
