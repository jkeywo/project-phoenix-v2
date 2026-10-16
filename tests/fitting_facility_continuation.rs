//! Whole-host facility work and intact salvage through admitted crew commands.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{
        ClientMessage, StationId, SystemBlackboard, SystemControlPayload as Control, SystemId,
    },
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    resources::{equipment_recovery_server::ShipEquipmentRecovery, fitting_server::ShipFitting},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
use std::collections::BTreeSet;

fn sid(value: &str) -> SystemId {
    SystemId(value.into())
}
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn build(salvage: bool) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: if salvage {
            "tests/fixtures/fitting-salvage-world.toml"
        } else {
            "tests/fixtures/fitting-facility-world.toml"
        }
        .into(),
        ship_path: if salvage {
            "tests/fixtures/fitting-salvage-cruiser.toml"
        } else {
            "assets/entities/alliance_cruiser.toml"
        }
        .into(),
        deterministic: true,
        seed: Some(156_215 + u64::from(salvage)),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    app
}
fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn claim(app: &mut App, station: &str) {
    let entity = local(app);
    let rating = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|row| row.id.0 == station)
        .unwrap()
        .ratings[0]
        .name
        .clone();
    for msg in [
        ClientMessage::Identify {
            token: station.into(),
            name: station.into(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ] {
        send(app, station, msg);
        steps(app, 4);
    }
}
fn control(app: &mut App, station: &str, target: &str, payload: Control) {
    let entity = local(app);
    let sessions = app.world().resource::<Sessions>();
    assert_eq!(
        sessions.0.holder_for_station(&StationId(station.into())),
        Some(station)
    );
    assert!(
        project_phoenix::command_admission::policy::is_command_authorized(
            station,
            &sid(target),
            &payload,
            app.world().get::<ShipSystemControlSources>(entity).unwrap(),
            sessions,
            &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
            app.world().get::<HumanSeekingHosts>(entity)
        ),
        "{station} owns {target}"
    );
    send(
        app,
        station,
        ClientMessage::ControlSystem {
            target: sid(target),
            payload,
        },
    );
    steps(app, 4);
}
fn fit(app: &App, entity: Entity) -> &ShipFitting {
    app.world().get::<ShipFitting>(entity).unwrap()
}
fn parts(app: &App, entity: Entity) -> i64 {
    let inventory = &app
        .world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0;
    ["machinery_parts", "hold-machinery-parts"]
        .iter()
        .filter_map(|stock| inventory.capacity_stock(stock))
        .map(|stock| stock.level)
        .sum()
}
fn identities(app: &mut App) -> BTreeSet<String> {
    let items: Vec<_> = app
        .world_mut()
        .query::<&ShipFitting>()
        .iter(app.world())
        .flat_map(|state| state.equipment.assemblies.keys().cloned())
        .collect();
    let unique: BTreeSet<_> = items.iter().cloned().collect();
    assert_eq!(
        items.len(),
        unique.len(),
        "each physical assembly has exactly one owner"
    );
    unique
}
fn assert_same(live: &App, resumed: &App, step: usize) {
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "digest at {step}"
    );
    let left = serde_json::to_value(capture(live.world())).unwrap();
    let right = serde_json::to_value(capture(resumed.world())).unwrap();
    assert!(
        left == right,
        "complete snapshot at {step}: {}",
        difference(&left, &right, "$").unwrap_or_default()
    );
}
fn difference(left: &serde_json::Value, right: &serde_json::Value, path: &str) -> Option<String> {
    use serde_json::Value;
    if left == right {
        return None;
    }
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                if let Some(diff) = difference(
                    a.get(key).unwrap_or(&Value::Null),
                    b.get(key).unwrap_or(&Value::Null),
                    &format!("{path}.{key}"),
                ) {
                    return Some(diff);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                if let Some(diff) = difference(a, b, &format!("{path}[{index}]")) {
                    return Some(diff);
                }
            }
        }
        _ => {}
    }
    Some(format!("{path}: live={left}, restored={right}"))
}
fn facility_ready() -> (App, Entity, String) {
    let mut app = build(false);
    claim(&mut app, "engineering");
    claim(&mut app, "helm");
    let entity = local(&mut app);
    // A contextual Dock command needs the same earned current picture as its
    // GUI. Sending it before the Radar has acquired a target correctly refuses.
    let available = |app: &App| {
        let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
        let boards = app.world().get::<ShipSystemBlackboards>(entity).unwrap();
        let picture = config.observations_for_system(&sid("dock"), &boards.0);
        matches!(picture.get(&sid("dock")), Some(SystemBlackboard::Dock(board))
            if board.available && board.available_target.is_some())
    };
    for _ in 0..1800 {
        if available(&app) {
            break;
        }
        app.update();
    }
    assert!(
        available(&app),
        "actual own paid Dock picture offers the authored berth"
    );
    control(&mut app, "helm", "dock", Control::Dock);
    let mut partner = None;
    for _ in 0..1800 {
        partner = app
            .world()
            .get::<project_phoenix::dock::DockControl>(entity)
            .unwrap()
            .docked_partner()
            .map(str::to_owned);
        if partner.is_some() {
            break;
        }
        app.update();
    }
    let partner = partner.expect("ordinary Dock must actually couple the authored Axiom facility");
    control(
        &mut app,
        "engineering",
        "power-reactor",
        Control::RequestEquipment {
            slot: "reactor".into(),
            equipment: "cruiser-reactor-responsive".into(),
        },
    );
    control(
        &mut app,
        "engineering",
        "repair",
        Control::DispatchRefit {
            team_idx: 0,
            slot: "reactor".into(),
        },
    );
    assert!(fit(&app, entity).work.contains_key("reactor"));
    (app, entity, partner)
}

#[test]
fn actual_powered_allied_facility_refits_reactor_and_continues_paid_work() {
    let (mut live, entity, partner) = facility_ready();
    let original = identities(&mut live);
    let facility = live
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(live.world())
        .find_map(|(entity, id)| (id.0 == partner).then_some(entity))
        .unwrap();
    let initial_parts = parts(&live, entity) + parts(&live, facility);
    let mut paid = false;
    for _ in 0..2400 {
        paid |= fit(&live, entity).work["reactor"].progress > 0.0
            && live
                .world()
                .get::<ShipPowerSystem>(facility)
                .unwrap()
                .0
                .last_work_fraction(&sid("dock"))
                .is_some_and(|fraction| fraction > 0.0);
        if fit(&live, entity).work["reactor"].progress > 0.0 {
            break;
        }
        live.update();
    }
    assert!(
        paid,
        "the real facility's fitted Dock must pay its own work cost"
    );
    assert!(fit(&live, entity).work["reactor"].progress > 0.0);
    let checkpoint: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let mut resumed = build(false);
    claim(&mut resumed, "engineering");
    claim(&mut resumed, "helm");
    let restored = restore(resumed.world_mut(), &checkpoint);
    assert!(restored.is_complete(), "{:?}", restored.gaps);
    let mut done = 0;
    for step in 0..12000 {
        assert_same(&live, &resumed, step);
        live.update();
        resumed.update();
        if fit(&live, entity).work.is_empty()
            && fit(&live, entity)
                .equipment
                .installed("reactor")
                .is_some_and(|item| item.definition == "cruiser-reactor-responsive")
        {
            done += 1;
            if done >= 20 && step >= 359 {
                break;
            }
        }
    }
    assert!(
        done >= 20,
        "facility-supported removal and installation must complete"
    );
    assert_eq!(
        parts(&live, entity) + parts(&live, facility),
        initial_parts - 1
    );
    assert_eq!(identities(&mut live), original);
    assert_eq!(
        live.world()
            .get::<project_phoenix::dock::DockControl>(entity)
            .unwrap()
            .docked_partner(),
        Some(partner.as_str())
    );
}

fn disable_source(app: &mut App, source: &str) {
    use project_phoenix::gm_action::{
        submit_native, GmAction, GmActionId, GmActionRequest, NativeGmAuthority,
        NATIVE_GM_OPERATOR_ID,
    };
    use project_phoenix::gm_roster::{GmOperator, GmRoster};
    // These are the native bridge's boot capabilities, not crew/controller or
    // damage state. The physical incident still passes actual GM admission.
    app.insert_resource(NativeGmAuthority {
        connected: true,
        screen_pause: false,
    });
    app.insert_resource(
        GmRoster::try_new(vec![GmOperator::new(
            NATIVE_GM_OPERATOR_ID.into(),
            "Fixture GM".into(),
            true,
        )])
        .unwrap(),
    );
    submit_native(
        app.world_mut(),
        GmActionRequest {
            operator_id: NATIVE_GM_OPERATOR_ID.into(),
            correlation: GmActionId::new("source-engineering-incident").unwrap(),
            action: GmAction::ApplyDirectEffect {
                target: source.into(),
                scope: project_phoenix::gm_effect::GmDirectEffectScope::System(sid(
                    "power-reactor",
                )),
                effect: project_phoenix::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 1_000_000,
            },
        },
    )
    .expect("actual privileged GM admission");
    steps(app, 8);
}

#[test]
fn intact_recovery_preserves_actual_condition_then_vacant_repair_fetches_and_installs_it() {
    let mut live = build(true);
    claim(&mut live, "engineering");
    claim(&mut live, "science");
    claim(&mut live, "tactical");
    let entity = local(&mut live);
    let source = live
        .world_mut()
        .query::<(Entity, &EntityUuid, &ShipFitting)>()
        .iter(live.world())
        .find_map(|(actor, id, _)| (actor != entity).then_some((actor, id.0.clone())))
        .unwrap();
    let original = identities(&mut live);
    let recovered = fit(&live, source.0)
        .equipment
        .installed("fore-beam")
        .unwrap()
        .id
        .clone();
    disable_source(&mut live, &source.1);
    let damage = live.world().get::<ShipDamage>(source.0).unwrap();
    assert!(damage.neutralized(
        &live.world().get::<ShipConfigComponent>(source.0).unwrap().0,
        live.world().get::<ShipPowerSystem>(source.0)
    ));
    let condition = damage.state.components[&sid("phaser-fore")]["operation"].clone();
    assert!(
        condition.dose > 0.0 && condition.last_source.is_some(),
        "the source must carry a real physical incident frontier"
    );
    // Choose the sole outside body from the addressed Science Console's
    // actual paid public records. Private UUIDs are observer assertions only.
    let science_contact = |app: &App| {
        let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
        let boards = app.world().get::<ShipSystemBlackboards>(entity).unwrap();
        let picture = config.observations_for_system(&sid("sensors"), &boards.0);
        project_phoenix::science::contact_routing::observations(picture.get(&sid("sensor-radar"))?)?
            .iter()
            .find(|row| row.selectable && !row.predicted && row.identity.is_some())
            .map(|row| row.contact_id.clone())
    };
    for _ in 0..1800 {
        if science_contact(&live).is_some() {
            break;
        }
        live.update();
    }
    let contact = science_contact(&live).expect("actual paid own Science contact");
    control(
        &mut live,
        "science",
        "sensors",
        Control::SetScienceTarget {
            uuid: contact.clone(),
        },
    );
    let recovery_feed = |app: &App| {
        let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
        let boards = app.world().get::<ShipSystemBlackboards>(entity).unwrap();
        config
            .observations_for_system(&sid("resource-recovery"), &boards.0)
            .get(&sid("sensor-radar"))
            .cloned()
    };
    assert!(
        matches!(recovery_feed(&live), Some(SystemBlackboard::SensorRadar(board))
            if board.selected_target.as_deref() == Some(contact.as_str())),
        "the actual Recovery Console must receive the earned Science selection: {:?}",
        recovery_feed(&live)
    );
    let offered = |app: &App| {
        project_phoenix::resources::equipment_recovery_server::project(app.world(), entity)?
            .options
            .into_iter()
            .find(|option| {
                option.slot == "fore-beam"
                    && option.equipment == "recovered-beam"
                    && option.destination_locker == "support-equipment"
                    && option.blocked_reason.is_none()
            })
    };
    for _ in 0..1800 {
        if offered(&live).is_some() {
            break;
        }
        live.update();
    }
    let option = offered(&live).unwrap_or_else(|| {
        let world = live.world();
        let damage = world.get::<ShipDamage>(source.0).unwrap();
        let source_config = &world.get::<ShipConfigComponent>(source.0).unwrap().0;
        eprintln!(
            "intact Recovery failure: own_position={:?} source_position={:?} neutralized={} source_condition={:?} receiving_tool={:?} source_fitting={:?} recovery={:?} permitted_feed={:?} projection={:?}",
            world.get::<Transform>(entity).map(|pose| pose.translation),
            world.get::<Transform>(source.0).map(|pose| pose.translation),
            damage.neutralized(source_config, world.get::<ShipPowerSystem>(source.0)),
            damage.state.components[&sid("phaser-fore")]["operation"],
            world.get::<project_phoenix::resources::recovery_server::CompiledShipRecovery>(entity).map(|tool| &tool.0),
            fit(&live, source.0),
            world.get::<ShipEquipmentRecovery>(entity),
            recovery_feed(&live),
            project_phoenix::resources::equipment_recovery_server::project(world, entity),
        );
        panic!("normal current Recovery projection offers real intact equipment");
    });
    control(
        &mut live,
        "engineering",
        "resource-recovery",
        Control::RecoverEquipment {
            target_uuid: option.target_uuid,
            slot: option.slot,
            destination_locker: option.destination_locker,
        },
    );
    assert!(live
        .world()
        .get::<ShipEquipmentRecovery>(entity)
        .unwrap()
        .job
        .is_some());
    let recovery_checkpoint: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let mut recovery_resumed = build(true);
    for station in ["engineering", "science", "tactical"] {
        claim(&mut recovery_resumed, station);
    }
    let report = restore(recovery_resumed.world_mut(), &recovery_checkpoint);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..1800 {
        assert_same(&live, &recovery_resumed, step);
        live.update();
        recovery_resumed.update();
        if step >= 359
            && fit(&live, entity)
                .equipment
                .assemblies
                .contains_key(&recovered)
        {
            break;
        }
    }
    assert!(!fit(&live, source.0)
        .equipment
        .assemblies
        .contains_key(&recovered));
    let item = &fit(&live, entity).equipment.assemblies[&recovered];
    assert_eq!(item.condition.as_ref(), Some(&condition));
    assert!(
        matches!(&item.location,phoenix_model::fitting::EquipmentLocation::Stored{locker}if locker=="support-equipment")
    );
    assert_eq!(identities(&mut live), original);
    control(
        &mut live,
        "tactical",
        "phaser-fore",
        Control::RequestEquipment {
            slot: "fore-beam".into(),
            equipment: "recovered-beam".into(),
        },
    );
    send(&mut live, "engineering", ClientMessage::ReleaseStation);
    steps(&mut live, 4);
    let mut carried = false;
    for _ in 0..2400 {
        carried |= matches!(
            &fit(&live, entity).equipment.assemblies[&recovered].location,
            phoenix_model::fitting::EquipmentLocation::InWork { .. }
        );
        if carried
            && fit(&live, entity)
                .work
                .get("fore-beam")
                .is_some_and(|work| work.progress > 0.0)
        {
            break;
        }
        live.update();
    }
    assert!(
        carried,
        "ordinary vacant Repair must fetch the same actual Support item"
    );
    let checkpoint: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let initial_parts = parts(&live, entity);
    let mut resumed = build(true);
    for station in ["engineering", "science", "tactical"] {
        claim(&mut resumed, station);
    }
    send(&mut resumed, "engineering", ClientMessage::ReleaseStation);
    steps(&mut resumed, 4);
    let restored = restore(resumed.world_mut(), &checkpoint);
    assert!(restored.is_complete(), "{:?}", restored.gaps);
    let mut done = 0;
    for step in 0..2400 {
        assert_same(&live, &resumed, step);
        live.update();
        resumed.update();
        if fit(&live, entity).work.is_empty()
            && fit(&live, entity)
                .equipment
                .installed("fore-beam")
                .is_some_and(|item| item.id == recovered)
        {
            done += 1;
            if done >= 20 && step >= 359 {
                break;
            }
        }
    }
    assert!(
        done >= 20,
        "same recovered assembly must finish field installation"
    );
    assert_eq!(
        live.world()
            .get::<ShipDamage>(entity)
            .unwrap()
            .state
            .components[&sid("phaser-fore")]["operation"],
        condition
    );
    assert_eq!(parts(&live, entity), initial_parts - 1);
    assert_eq!(identities(&mut live), original);
}
