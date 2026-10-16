//! Joined-crew, real finite shield recovery and fresh JSON continuation.
//! A normal authored field pulse establishes damage. Tests never write shield
//! charge, Power state, controllers, observations or machinery grants.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{ClientMessage, GamePhase, SystemControlPayload as Control, SystemId},
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    server_app::LocalShip,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem, shields::ShipShields},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};

const FORE: &str = "shield-arc-fore";
const AFT: &str = "shield-arc-aft";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}
fn boot() -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/shield-energy-world.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156209),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    let entity = local(&mut app);
    let stations = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .clone();
    for station in stations {
        let token = format!("shield-{}", station.id.0);
        for msg in [
            ClientMessage::Identify {
                token: token.clone(),
                name: token.clone(),
            },
            ClientMessage::SelectStation {
                station: station.id.0.clone(),
            },
            ClientMessage::SetStationRating {
                rating_name: station.ratings[0].name.clone(),
            },
            ClientMessage::SetReady { ready: true },
        ] {
            send(&mut app, &token, msg);
            steps(&mut app, 4);
        }
    }
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );
    (app, entity)
}
fn control(app: &mut App, entity: Entity, owner: &str, payload: Control) {
    let token = {
        let sessions = app.world().resource::<Sessions>();
        let sources = app.world().get::<ShipSystemControlSources>(entity).unwrap();
        let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
        sessions
            .0
            .players()
            .iter()
            .find(|player| {
                player.ready
                    && player.connected
                    && project_phoenix::command_admission::policy::is_command_authorized(
                        &player.token,
                        &id(owner),
                        &payload,
                        sources,
                        sessions,
                        config,
                        app.world().get::<HumanSeekingHosts>(entity),
                    )
            })
            .unwrap_or_else(|| panic!("actual Human authority required for {owner}"))
            .token
            .clone()
    };
    send(
        app,
        &token,
        ClientMessage::ControlSystem {
            target: id(owner),
            payload,
        },
    );
    steps(app, 4);
}

#[derive(Debug)]
struct Reading {
    donor: f64,
    recipient: f64,
    reserve: f32,
    paid: f64,
    intake: f32,
    recovery: Option<SystemId>,
}
fn reading(app: &App, entity: Entity) -> Reading {
    let shields = &app.world().get::<ShipShields>(entity).unwrap().0;
    let fore = shields.facings.iter().find(|arc| arc.id == "fore").unwrap();
    let aft = shields.facings.iter().find(|arc| arc.id == "aft").unwrap();
    let power = &app.world().get::<ShipPowerSystem>(entity).unwrap().0;
    let state = power.capture_continuation();
    Reading {
        donor: aft.stored_strength(),
        recipient: fore.stored_strength(),
        reserve: state.network.consumers[&id(FORE)].energy,
        paid: power.emitted_work(&id(FORE)),
        intake: state
            .last_intake_per_sec
            .get(&id(FORE))
            .copied()
            .unwrap_or(0.0),
        recovery: aft.recovery_target.clone(),
    }
}
fn same(left: &App, right: &App, step: usize) {
    assert_eq!(
        capture(left.world()),
        capture(right.world()),
        "complete continuation step {step}"
    );
    assert_eq!(
        world_digest(left.world()),
        world_digest(right.world()),
        "digest step {step}"
    );
}

#[test]
fn ordinary_human_recovery_pays_recipient_recharge_and_restores_exactly_for_360_steps() {
    let (mut live, entity) = boot();
    // Handoff keeps ordinary AI intent. Explicit Human controls stop any
    // retained donor before arranging this new, normally admitted transfer.
    for arc in ["fore", "port", "aft", "starboard"] {
        control(
            &mut live,
            entity,
            &format!("shield-arc-{arc}"),
            Control::SetShieldRecovery { target: None },
        );
    }
    control(
        &mut live,
        entity,
        "power-reactor",
        Control::SetEnergyMode {
            mode: phoenix_model::energy::EnergyAllocationMode::Manual,
            cooling_usage: phoenix_model::energy::CoolingUsage::Low,
        },
    );
    control(
        &mut live,
        entity,
        "power-reactor",
        Control::SetEnergySystemFlow {
            system: id(FORE),
            flow_per_sec: Some(0.0),
        },
    );
    control(
        &mut live,
        entity,
        FORE,
        Control::SetShieldRaised { raised: true },
    );
    // The actual field has left. Ordinary funded recharge spends the finite
    // initial Fore reserve; no damage, charge or offline timer is injected.
    for _ in 0..1200 {
        let value = reading(&live, entity);
        if value.reserve <= 0.0001 && value.recipient < 95.0 && value.recipient > 0.0 {
            break;
        }
        live.update();
    }
    let exhausted = reading(&live, entity);
    assert!(
        exhausted.reserve <= 0.0001 && exhausted.recipient > 0.0 && exhausted.recipient < 95.0,
        "normal pulse/recharge must establish actual shortage: {exhausted:?}"
    );
    assert_eq!(
        exhausted.intake, 0.0,
        "manual zero flow gives no reactor intake"
    );
    let power = &live.world().get::<ShipPowerSystem>(entity).unwrap().0;
    assert_eq!(power.network().fine_flows.get(&id(FORE)), Some(&0.0));
    let (hull, maximum) = live
        .world()
        .get::<ShipDamage>(entity)
        .unwrap()
        .structural_totals();
    assert_eq!(
        hull, maximum,
        "finite shield pulse must not silently damage the hull"
    );
    let fore_before = exhausted.recipient;
    control(
        &mut live,
        entity,
        AFT,
        Control::SetShieldRaised { raised: false },
    );
    control(
        &mut live,
        entity,
        AFT,
        Control::SetShieldRecovery {
            target: Some(id(FORE)),
        },
    );
    for _ in 0..120 {
        let value = reading(&live, entity);
        if value.intake > 0.0 && value.paid > 0.0 && value.recipient > fore_before {
            break;
        }
        live.update();
    }
    let funded = reading(&live, entity);
    assert_eq!(funded.recovery, Some(id(FORE)));
    assert!(
        funded.intake > 0.0
            && funded.paid > 0.0
            && funded.recipient > fore_before
            && funded.donor < exhausted.donor,
        "actual battery-mediated recovery is required: {funded:?}"
    );
    let uuid = live.world().get::<EntityUuid>(entity).unwrap().0.clone();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..7 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == uuid)
            .unwrap();
        match fault {
            0 => row.weapons.as_mut().unwrap().shield_operations.clear(),
            1 => {
                row.weapons
                    .as_mut()
                    .unwrap()
                    .shield_operations
                    .iter_mut()
                    .find(|arc| arc.id == "aft")
                    .unwrap()
                    .recovery_target = Some(id("foreign-battery"))
            }
            2 => row.weapons.as_mut().unwrap().shield_operations[0].protection_fraction = 1.5,
            3 => row.weapons.as_mut().unwrap().shield_operations[0].interception_remainder = 1.0,
            4 => {
                row.power
                    .as_mut()
                    .unwrap()
                    .network
                    .consumers
                    .get_mut(&id(FORE))
                    .unwrap()
                    .energy = 1_000_000.0
            }
            5 => {
                row.weapons
                    .as_mut()
                    .unwrap()
                    .shield_charge
                    .iter_mut()
                    .find(|arc| arc.0 == "fore")
                    .unwrap()
                    .1 = i32::MAX
            }
            _ => row.power = None,
        }
        assert!(
            !restore(resumed.world_mut(), &bad).is_complete(),
            "malformed shield/energy candidate {fault}"
        );
        assert_eq!(
            capture(resumed.world()),
            before,
            "no partial writes for {fault}"
        );
        assert_eq!(
            world_digest(resumed.world()),
            digest,
            "no digest writes for {fault}"
        );
    }
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    same(&live, &resumed, 0);
    let period = live
        .world()
        .resource::<Time<Fixed>>()
        .timestep()
        .as_secs_f64();
    let conversion = f64::from(
        live.world()
            .get::<ShipShields>(entity)
            .unwrap()
            .0
            .facings
            .iter()
            .find(|arc| arc.id == "aft")
            .unwrap()
            .energy_config
            .recovery_energy_per_hp,
    );
    let mut accepted = 0.0;
    for step in 0..360 {
        let previous = reading(&live, entity);
        live.update();
        resumed.update();
        same(&live, &resumed, step + 1);
        let current = reading(&live, entity);
        let restored_current = reading(&resumed, restored);
        assert_eq!(current.recovery, Some(id(FORE)));
        assert_eq!(restored_current.recovery, Some(id(FORE)));
        assert!(
            current.paid > 0.0 && restored_current.paid > 0.0 && current.intake > 0.0,
            "continued equality must do real paid work at {step}: {current:?}"
        );
        let credit = f64::from(current.intake) * period;
        assert!(
            (credit - (previous.donor - current.donor) * conversion).abs() < 0.00001,
            "accepted energy must debit actual donor strength at {step}"
        );
        accepted += credit;
        assert_eq!(
            live.world()
                .get::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .network()
                .fine_flows
                .get(&id(FORE)),
            Some(&0.0)
        );
    }
    let final_reading = reading(&live, entity);
    assert!(
        accepted > 0.0
            && final_reading.recipient > funded.recipient
            && final_reading.donor < funded.donor
    );
}
