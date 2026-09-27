use super::*;
use crate::modifiers::power_system::{PowerConfig, PowerSystem, StrikeReserveConfig};
use crate::modifiers::strike_reserve::StrikeWeaponConfig;
use crate::ship::power::{PowerConfigResource, ShipPowerSystem};

fn reserve(app: &mut App, charge: f32, ids: &[&str]) -> Entity {
    // This focused weapons fixture omits unrelated production owners. Preserve
    // the production spending order for the owners it actually installs.
    use crate::sim_sets::FixedStep;
    app.configure_sets(
        FixedUpdate,
        (
            FixedStep::HandleFireBlaster,
            FixedStep::TickBlasterSystem,
            FixedStep::TickTorpedoLifecycle,
            FixedStep::HandleFirePhaser,
            FixedStep::HandleFireTorpedo,
        )
            .chain(),
    );
    let ship = local_ship_entity(app);
    let config = PowerConfig {
        strike_reserve: Some(StrikeReserveConfig {
            group: "reserve".into(),
            units_per_level: 1.0,
            ai_enable_at: None,
            weapons: ids
                .iter()
                .map(|id| {
                    (
                        (*id).into(),
                        StrikeWeaponConfig {
                            cost: 3.0,
                            damage_multiplier: 2.0,
                        },
                    )
                })
                .collect(),
        }),
        ..Default::default()
    };
    let mut power = PowerSystem::new(&config);
    power.battery_charge = charge;
    power.set_strike_boost(true);
    app.world_mut()
        .entity_mut(ship)
        .insert((ShipPowerSystem(power), PowerConfigResource(config)));
    ship
}

fn charge(app: &App, ship: Entity) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(ship)
        .unwrap()
        .0
        .battery_charge
}

#[test]
fn strike_toggle_uses_ordinary_human_authority_and_backfill_admission() {
    use crate::ship::control_source::ControlSource;
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    let ship = reserve(&mut app, 9.0, &["phaser-port"]);
    let target = SystemId(crate::ship::system_registry::PHASER_CONTROL_SYSTEM_ID.into());
    set_system_control_source(&mut app, target.clone(), ControlSource::Human);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: target.clone(),
            payload: SystemControlPayload::SetStrikeBoost { enabled: false },
        },
    );
    tick(&mut app);
    assert!(
        !app.world()
            .get::<ShipPowerSystem>(ship)
            .unwrap()
            .0
            .strike_boost()
            .unwrap()
            .enabled
    );
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: target.clone(),
            payload: SystemControlPayload::SetStrikeBoost { enabled: true },
        },
    );
    tick(&mut app);
    assert!(
        !app.world()
            .get::<ShipPowerSystem>(ship)
            .unwrap()
            .0
            .strike_boost()
            .unwrap()
            .enabled
    );
    app.world_mut()
        .get_mut::<PowerConfigResource>(ship)
        .unwrap()
        .0
        .strike_reserve
        .as_mut()
        .unwrap()
        .ai_enable_at = Some(6.0);
    set_system_control_source(&mut app, target, ControlSource::Ai);
    tick(&mut app);
    assert!(
        app.world()
            .get::<ShipPowerSystem>(ship)
            .unwrap()
            .0
            .strike_boost()
            .unwrap()
            .enabled
    );
    assert_eq!(charge(&app, ship), 9.0, "enabling is free");
}

#[test]
fn strike_beam_pays_once_at_start_and_rejected_repeat_does_not_spend() {
    let mut app = test_app();
    let ship = reserve(&mut app, 9.0, &["phaser-port", "phaser-starboard"]);
    lock_and_fire(&mut app, 0.0, -20.0);
    assert_eq!(charge(&app, ship), 6.0);
    let beam = app.world().get::<ActiveBeam>(ship).unwrap();
    assert_eq!(beam.live_banks().next().unwrap().1.strike_damage_bonus, 1.0);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("phaser-port".into()),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);
    assert_eq!(
        charge(&app, ship),
        6.0,
        "active-beam refusal costs no charge"
    );
}

#[test]
fn strike_torpedo_spends_each_actual_burst_launch_and_shuts_off_globally() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    let ship = reserve(&mut app, 5.0, &["torpedo-tube-fore-port"]);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    tick(&mut app);
    assert_eq!(charge(&app, ship), 5.0, "an unloaded tube pays nothing");
    {
        let mut system = app
            .world_mut()
            .get_mut::<TorpedoSystemResource>(ship)
            .unwrap();
        let tube = system.0.tube_mut("fore_port").unwrap();
        tube.volley_max = 3;
        tube.loaded_count = 3;
        tube.target_count = 3;
    }
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    tick(&mut app);
    assert_eq!(
        charge(&app, ship),
        2.0,
        "only the immediate round has fired"
    );
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(ship)
            .unwrap()
            .0
            .in_flight[0]
            .strike_damage_bonus,
        1.0
    );
    for _ in 0..5 {
        tick(&mut app);
    }
    assert_eq!(charge(&app, ship), 2.0);
    let power = &app.world().get::<ShipPowerSystem>(ship).unwrap().0;
    assert!(power.strike_boost().unwrap().depleted);
    assert!(!power.strike_boost().unwrap().enabled);
    let system = &app.world().get::<TorpedoSystemResource>(ship).unwrap().0;
    assert_eq!(system.in_flight.len(), 3);
    assert_eq!(system.in_flight[1].strike_damage_bonus, 0.0);
    assert_eq!(system.in_flight[2].strike_damage_bonus, 0.0);
}

#[test]
fn strike_simultaneous_families_use_fixed_schedule_order_on_two_peers() {
    fn run() -> (f32, f32, f32, bool) {
        let mut app = test_app();
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);
        let ship = reserve(&mut app, 3.0, &["phaser-port", "torpedo-tube-fore-port"]);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::ship::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app);
        load_tube_now(&mut app, "fore_port");
        // Input order cannot invert the established family schedule.
        for (target, payload) in [
            (
                "torpedo-tube-fore-port",
                SystemControlPayload::FireTorpedo { target_uuid: None },
            ),
            ("phaser-port", SystemControlPayload::FirePhaser),
        ] {
            push(
                &mut app,
                "weapons",
                ClientMessage::ControlSystem {
                    target: SystemId(target.into()),
                    payload,
                },
            );
        }
        tick(&mut app);
        let beam = app
            .world()
            .get::<ActiveBeam>(ship)
            .unwrap()
            .live_banks()
            .next()
            .unwrap()
            .1
            .strike_damage_bonus;
        let torpedo = app
            .world()
            .get::<TorpedoSystemResource>(ship)
            .unwrap()
            .0
            .in_flight[0]
            .strike_damage_bonus;
        let depleted = app
            .world()
            .get::<ShipPowerSystem>(ship)
            .unwrap()
            .0
            .strike_boost()
            .unwrap()
            .depleted;
        (charge(&app, ship), beam, torpedo, depleted)
    }
    let first = run();
    assert_eq!(first, (0.0, 1.0, 0.0, true));
    assert_eq!(run(), first);
}

#[test]
fn strike_blaster_charge_and_cancel_are_free_and_emission_pays_exactly_once() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    let ship = reserve(&mut app, 3.0, &["blaster-fore"]);
    let mut bank =
        crate::weapons::blaster::BlasterSystem::new(crate::weapons::blaster::BlasterBankConfig {
            id: "fore".into(),
            charge_time_secs: 1.0,
            volley_count: 1,
            range: 1000.0,
            ..Default::default()
        });
    assert!(bank.request_charge_start());
    app.world_mut()
        .entity_mut(ship)
        .insert(BlasterSystemResource(vec![bank]));
    tick(&mut app);
    assert_eq!(charge(&app, ship), 3.0);
    app.world_mut()
        .get_mut::<BlasterSystemResource>(ship)
        .unwrap()
        .0[0]
        .request_charge_cancel();
    tick(&mut app);
    assert_eq!(charge(&app, ship), 3.0);
    assert!(app
        .world_mut()
        .get_mut::<BlasterSystemResource>(ship)
        .unwrap()
        .0[0]
        .request_charge_start());
    for _ in 0..6 {
        tick(&mut app);
    }
    assert_eq!(charge(&app, ship), 0.0);
    let bank = &app.world().get::<BlasterSystemResource>(ship).unwrap().0[0];
    assert_eq!(bank.in_flight.len(), 1);
    assert_eq!(bank.in_flight[0].damage, bank.config.damage * 2);
}
