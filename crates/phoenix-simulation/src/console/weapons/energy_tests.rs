use super::*;
use crate::ship::power::ShipPowerSystem;

fn local_energy(app: &App, ship: Entity, id: &SystemId) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(ship)
        .unwrap()
        .0
        .network()
        .consumers[id]
        .energy
}
fn set_local_energy(app: &mut App, ship: Entity, id: &SystemId, energy: f32) {
    app.world_mut()
        .get_mut::<ShipPowerSystem>(ship)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(id)
        .unwrap()
        .energy = energy;
}
fn fire_torpedo(app: &mut App) -> Vec<OutboundMessage> {
    push(
        app,
        "weapons",
        ClientMessage::ControlSystemCorrelated {
            correlation: ActionCorrelationId::new("energy-shot").unwrap(),
            target: crate::ship::system_registry::torpedo_tube_fore_port_system_id(),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    tick(app)
}

#[test]
fn charged_tube_fires_with_zero_incoming_flow_and_pays_once() {
    let mut app = test_app();
    setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");
    let ship = local_ship_entity(&mut app);
    let id = crate::ship::system_registry::torpedo_tube_fore_port_system_id();
    app.world_mut()
        .get_mut::<ShipPowerSystem>(ship)
        .unwrap()
        .0
        .set_flow(&PowerGroupId("weapons".into()), 0.0)
        .unwrap();
    set_local_energy(&mut app, ship, &id, 2.0);
    let before = app
        .world()
        .get::<TorpedoSystemResource>(ship)
        .unwrap()
        .0
        .rounds_aboard();
    let out = fire_torpedo(&mut app);
    assert_eq!(
        feedback_for(&out, "energy-shot"),
        Some(ActionFeedbackOutcome::Applied)
    );
    assert_eq!(local_energy(&app, ship, &id), 1.0);
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(ship)
            .unwrap()
            .0
            .rounds_aboard(),
        before - 1
    );
}

#[test]
fn discrete_shortfall_keeps_loaded_ammunition_and_waits_for_full_cost() {
    let mut app = test_app();
    setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");
    let ship = local_ship_entity(&mut app);
    let id = crate::ship::system_registry::torpedo_tube_fore_port_system_id();
    set_local_energy(&mut app, ship, &id, 0.5);
    let before = app
        .world()
        .get::<TorpedoSystemResource>(ship)
        .unwrap()
        .0
        .rounds_aboard();
    let out = fire_torpedo(&mut app);
    assert_eq!(
        feedback_for(&out, "energy-shot"),
        Some(ActionFeedbackOutcome::Refused)
    );
    assert!(!out
        .iter()
        .any(|message| matches!(message.msg, ServerMessage::TorpedoLaunched { .. })));
    assert_eq!(local_energy(&app, ship, &id), 0.5);
    let system = &app.world().get::<TorpedoSystemResource>(ship).unwrap().0;
    assert_eq!(system.rounds_aboard(), before);
    assert_eq!(system.tube("fore_port").unwrap().loaded_count, 1);
    set_local_energy(&mut app, ship, &id, 1.0);
    let retried = fire_torpedo(&mut app);
    let power = app.world().get::<ShipPowerSystem>(ship).unwrap();
    assert_eq!(
        feedback_for(&retried, "energy-shot"),
        Some(ActionFeedbackOutcome::Applied),
        "actual retry: tube={:?}, energy={:?}, restraint={}, control={:?}, target={:?}",
        app.world()
            .get::<TorpedoSystemResource>(ship)
            .unwrap()
            .0
            .tube("fore_port"),
        power.0.network().consumers.get(&id),
        power.0.weapons_restrained(),
        app.world()
            .get::<crate::ship_plugin::ShipSystemControlSources>(ship)
            .unwrap()
            .0
            .source_for(&id),
        app.world()
            .get::<TacticalRadarSelection>(ship)
            .map(|selection| &selection.0),
    );
    assert_eq!(local_energy(&app, ship, &id), 0.0);
}

#[test]
fn missing_physical_profile_does_not_grant_free_weapon_work() {
    let mut app = test_app();
    setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");
    let ship = local_ship_entity(&mut app);
    app.insert_resource(NoFixtureEnergy);
    app.world_mut().entity_mut(ship).remove::<ShipPowerSystem>();
    let before = app
        .world()
        .get::<TorpedoSystemResource>(ship)
        .unwrap()
        .0
        .rounds_aboard();
    assert_eq!(
        feedback_for(&fire_torpedo(&mut app), "energy-shot"),
        Some(ActionFeedbackOutcome::Refused)
    );
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(ship)
            .unwrap()
            .0
            .rounds_aboard(),
        before
    );
}

#[test]
fn beam_work_scales_burn_progress_and_preserves_starved_intent() {
    let mut app = test_app();
    setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    let ship = local_ship_entity(&mut app);
    let id = SystemId("phaser-port".into());
    app.world_mut()
        .get_mut::<ActiveBeam>(ship)
        .unwrap()
        .start("port", "target-uuid", 10.0, 3.0);
    set_local_energy(&mut app, ship, &id, 0.0);
    tick(&mut app);
    assert_eq!(
        app.world_mut()
            .get_mut::<ActiveBeam>(ship)
            .unwrap()
            .bank_slot_mut("port")
            .unwrap()
            .remaining_secs,
        10.0
    );
    set_local_energy(&mut app, ship, &id, 0.1);
    tick(&mut app);
    let remaining = app
        .world_mut()
        .get_mut::<ActiveBeam>(ship)
        .unwrap()
        .bank_slot_mut("port")
        .unwrap()
        .remaining_secs;
    assert!((remaining - 9.9).abs() < 0.00001);
    assert_eq!(local_energy(&app, ship, &id), 0.0);
}

#[test]
fn restraint_command_uses_tactical_authority_and_is_published() {
    let mut app = test_app();
    setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    let ship = local_ship_entity(&mut app);
    let target = crate::ship::system_registry::tactical_radar_system_id();
    for (token, held, expected) in [
        ("captain", true, false),
        ("weapons", true, true),
        ("weapons", false, false),
    ] {
        push(
            &mut app,
            token,
            ClientMessage::ControlSystem {
                target: target.clone(),
                payload: SystemControlPayload::SetWeaponsRestraint { held },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world()
                .get::<ShipPowerSystem>(ship)
                .unwrap()
                .0
                .weapons_restrained(),
            expected
        );
        let boards = &app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(ship)
            .unwrap()
            .0;
        let SystemBlackboard::Weapons(board) =
            &boards[&crate::ship::system_registry::tactical_station_key()]
        else {
            panic!("Weapons publication")
        };
        assert_eq!(board.weapons_restrained, expected);
        assert!(!board.weapons_restraint_auto);
    }
}

#[test]
fn unbound_console_cannot_read_canonical_restraint() {
    let config = test_ship_config();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::tactical_station_key(),
        SystemBlackboard::Weapons(WeaponsBlackboard::default()),
    );
    let id = SystemId("phaser-port".into());
    assert!(!observed_weapons_restraint(
        Some(&config),
        &id,
        Some(&boards)
    ));
    let mut isolated = config;
    isolated.0.consoles[0]
        .feeds
        .retain(|feed| feed.kind != phoenix_model::console::ConsoleObservationKind::Weapons);
    assert!(observed_weapons_restraint(
        Some(&isolated),
        &id,
        Some(&boards)
    ));
}
