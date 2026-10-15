//! Actual existing admission/executor/finite-energy harness, not another beam model.
use super::*;
use crate::console::weapons::beam::ActiveBeamSlot;
use phoenix_model::weapon_operation::BeamOperation;

fn fixture() -> (App, Entity) {
    let mut app = test_app();
    let ship = local_ship_entity(&mut app);
    for bank in &mut app
        .world_mut()
        .get_mut::<PhaserCombatConfigResource>(ship)
        .unwrap()
        .0
        .banks
    {
        bank.operation = BeamOperation::Sustained;
        bank.beam_duration_secs = 0.05;
        bank.beam_damage_per_sec = 1.0;
        bank.cycle_jitter = 0.0;
        bank.cooldown_secs = 3.0;
    }
    let config = app
        .world()
        .get::<PhaserCombatConfigResource>(ship)
        .unwrap()
        .clone();
    app.insert_resource(config);
    let out = lock_and_fire(&mut app, 0.0, -20.0);
    assert!(out
        .iter()
        .any(|m| matches!(&m.msg, ServerMessage::BeamStarted {bank, ..} if bank == "port")));
    assert!(app
        .world()
        .get::<ActiveBeam>(ship)
        .unwrap()
        .is_bank_firing("port"));
    (app, ship)
}
fn charge(app: &App, ship: Entity) -> f32 {
    app.world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .network()
        .consumers[&SystemId("phaser-port".into())]
        .energy
}
fn current(app: &App, ship: Entity) -> ActiveBeamSlot {
    app.world()
        .get::<ActiveBeam>(ship)
        .unwrap()
        .live_banks()
        .find(|(bank, _)| bank.as_str() == "port")
        .unwrap()
        .1
        .clone()
}
#[test]
fn sustained_actual_light_keeps_paying_past_duration_until_normal_stop_with_cooldown() {
    let (mut app, ship) = fixture();
    let frozen = current(&app, ship);
    assert_eq!(frozen.operation, BeamOperation::Sustained);
    let before = charge(&app, ship);
    // Later equipment edits cannot turn the already-lit slot into a fixed burn.
    app.world_mut()
        .get_mut::<PhaserCombatConfigResource>(ship)
        .unwrap()
        .0
        .banks[0]
        .operation = BeamOperation::Fixed;
    app.world_mut()
        .resource_mut::<PhaserCombatConfigResource>()
        .0
        .banks[0]
        .operation = BeamOperation::Fixed;
    for _ in 0..10 {
        tick(&mut app);
    }
    assert_eq!(current(&app, ship).remaining_secs, frozen.remaining_secs);
    assert!(charge(&app, ship) < before);
    let before_stop = charge(&app, ship);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystemCorrelated {
            correlation: ActionCorrelationId::new("sustained-stop").unwrap(),
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::StopPhaser,
        },
    );
    let out = tick(&mut app);
    assert_eq!(
        feedback_for(&out, "sustained-stop"),
        Some(ActionFeedbackOutcome::Applied)
    );
    assert!(!app
        .world()
        .get::<ActiveBeam>(ship)
        .unwrap()
        .is_bank_firing("port"));
    assert_eq!(
        charge(&app, ship),
        before_stop,
        "Stop buys no new firing work"
    );
    assert!(app
        .world()
        .get::<PhaserCooldown>(ship)
        .unwrap()
        .is_bank_active("port"));
    assert!(out
        .iter()
        .any(|m| matches!(&m.msg,ServerMessage::BeamEnded {bank,..} if bank=="port")));
}
#[test]
fn sustained_ends_once_on_actual_empty_reserve_offline_or_restraint_without_free_work() {
    for variant in 0..3 {
        let (mut app, ship) = fixture();
        match variant {
            0 => {
                app.world_mut()
                    .get_mut::<crate::ship::power::ShipPowerSystem>(ship)
                    .unwrap()
                    .0
                    .network_mut()
                    .consumers
                    .get_mut(&SystemId("phaser-port".into()))
                    .unwrap()
                    .energy = 0.0;
            }
            1 => {
                app.world_mut()
                    .get_mut::<ShipSystemControlSources>(ship)
                    .unwrap()
                    .0
                    .set(
                        SystemId("phaser-port".into()),
                        crate::ship::control_source::ControlSource::Offline,
                    );
            }
            _ => {
                app.world_mut()
                    .get_mut::<crate::ship::power::ShipPowerSystem>(ship)
                    .unwrap()
                    .0
                    .set_weapons_restrained(true);
            }
        }
        let before = charge(&app, ship);
        let out = tick(&mut app);
        assert!(
            !app.world()
                .get::<ActiveBeam>(ship)
                .unwrap()
                .is_bank_firing("port"),
            "variant {variant}"
        );
        assert_eq!(charge(&app, ship), before);
        assert_eq!(
            out.iter()
                .filter(|m| matches!(&m.msg,ServerMessage::BeamEnded {bank,..} if bank=="port"))
                .count(),
            1
        );
        let out = tick(&mut app);
        assert!(!out
            .iter()
            .any(|m| matches!(&m.msg,ServerMessage::BeamEnded {bank,..} if bank=="port")));
    }
}
