//! Increment 12.1: ordinary delivery AI, exact finite stock and full continuation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use phoenix_model::interior::RouteLocation;
use project_phoenix::{
    console::weapons::torpedo::TorpedoSystemResource,
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    resources::logistics_server::{CompiledShipLogistics, ShipLogistics},
    server_app::LocalShip,
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
use std::collections::{BTreeMap, BTreeSet};

// Reuse the isolated, actual-hull fixtures from the physical route proof. They
// contain no hostile actors and retain the ordinary shipped AI and field data.
fn unstarted(world_path: &str) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: world_path.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156_212),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app
}

fn boot(world_path: &str) -> App {
    let mut app = unstarted(world_path);
    app.finish();
    app.cleanup();
    app
}

fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}

/// Incoming claims promise space and never add rounds. Loading reservations
/// already belong to magazine level; tubes and pending bursts remain aboard.
fn quantities(app: &App, entity: Entity) -> (i64, i64, i64) {
    let hold = app
        .world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock("hold-ammunition")
        .unwrap()
        .level;
    let weapons = &app.world().get::<TorpedoSystemResource>(entity).unwrap().0;
    assert!(
        weapons.in_flight.is_empty(),
        "quiet fixture cannot expend ammunition"
    );
    let local = i64::from(weapons.rounds_aboard());
    let state = &app.world().get::<ShipLogistics>(entity).unwrap().0;
    let transit = state.batches.values().map(|batch| batch.amount).sum();
    assert_eq!(
        hold + local + transit,
        6,
        "every round has one actual owner"
    );
    (hold, local, transit)
}

fn crossing(app: &App, entity: Entity) -> bool {
    app.world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .values()
        .any(|batch| {
            matches!(&batch.navigation.location,
            RouteLocation::Crossing { elapsed, .. } if *elapsed > 0.0)
        })
}

fn frontier(app: &App, entity: Entity) -> serde_json::Value {
    let uuid = &app.world().get::<EntityUuid>(entity).unwrap().0;
    let saved = capture(app.world());
    let row = saved.entities.iter().find(|row| &row.uuid == uuid).unwrap();
    serde_json::json!({"logistics":row.logistics,"inventory":row.infrastructure,
        "weapons":row.weapons,"power":row.power,"ai":row.ai_trees})
}

fn started() -> (App, Entity) {
    let mut app = boot("tests/fixtures/physical-route-world.toml");
    for _ in 0..10 {
        app.update();
        if app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .iter(app.world())
            .count()
            == 1
        {
            let entity = local(&mut app);
            return (app, entity);
        }
    }
    panic!("ordinary start must spawn its actual player hull");
}

fn completion_bound(app: &App, entity: Entity) -> usize {
    let profile = &app.world().get::<CompiledShipLogistics>(entity).unwrap().0;
    let handling = profile.handling();
    // Retain all 720 comparisons, then permit the authored serial loading,
    // physical travel and unloading work for all four initially held rounds,
    // using the actual configured batch size.
    let batches = 4u64.div_ceil(handling.max_batch as u64) as f32;
    let route_bound: f32 = profile
        .graph()
        .config
        .connections
        .iter()
        .map(|edge| edge.travel_secs)
        .sum();
    let work_secs =
        (2.0 * handling.max_batch as f32 / handling.units_per_sec + route_bound) * batches;
    let tick_secs =
        sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz).as_secs_f32();
    720.max((work_secs / tick_secs).ceil() as usize + 120)
}

fn mid_route() -> (App, Entity) {
    let (mut app, entity) = started();
    assert_eq!(quantities(&app, entity).0, 4);
    for _ in 0..600 {
        app.update();
        quantities(&app, entity);
        if crossing(&app, entity) {
            let weapons = &app.world().get::<TorpedoSystemResource>(entity).unwrap().0;
            let transit = quantities(&app, entity).2;
            assert!(transit > 0);
            assert_eq!(weapons.magazine.incoming_total(), transit);
            return (app, entity);
        }
    }
    panic!("ordinary authored Logistics AI must physically carry a positive batch");
}

fn assert_delivered(app: &App, entity: Entity) {
    assert_eq!(quantities(app, entity), (0, 6, 0));
    let state = &app.world().get::<ShipLogistics>(entity).unwrap().0;
    let outcome = state.last_outcome.as_ref().unwrap();
    assert!(!outcome.returned && outcome.amount > 0);
    let handling = app
        .world()
        .get::<CompiledShipLogistics>(entity)
        .unwrap()
        .0
        .handling();
    let required = 4u64.div_ceil(u64::try_from(handling.max_batch).unwrap());
    assert!(
        state.next_id > required,
        "the configured bounded batches must have executed"
    );
    assert!(
        outcome.amount <= handling.max_batch,
        "actual delivery respects authored batch capacity"
    );
    assert!(app
        .world()
        .get::<TorpedoSystemResource>(entity)
        .unwrap()
        .0
        .magazine
        .incoming
        .is_empty());
}

#[test]
fn actual_delivery_json_restore_preserves_claims_work_and_every_round() {
    let (mut live, entity) = mid_route();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored_entity) = started();
    assert_ne!(frontier(&live, entity), frontier(&resumed, restored_entity));
    assert!(ready_to_restore(resumed.world(), &saved));
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    // Retain a substantial next-tick comparison after positive mid-crossing
    // progress, then require actual completion rather than equal idle states.
    for step in 0..completion_bound(&live, entity) {
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "step {step}"
        );
        assert_eq!(
            frontier(&live, entity),
            frontier(&resumed, restored_entity),
            "step {step}"
        );
        assert_eq!(
            quantities(&live, entity),
            quantities(&resumed, restored_entity)
        );
        let tick = live.world().resource::<SimTick>().0;
        live.update();
        resumed.update();
        assert_eq!(live.world().resource::<SimTick>().0, tick + 1);
        assert_eq!(resumed.world().resource::<SimTick>().0, tick + 1);
        if step >= 719 && quantities(&live, entity) == (0, 6, 0) {
            break;
        }
    }
    assert_delivered(&live, entity);
    assert_delivered(&resumed, restored_entity);
}

#[test]
fn corrupt_incoming_or_missing_owner_refuses_before_any_live_write() {
    let (mut app, entity) = mid_route();
    let uuid = app.world().get::<EntityUuid>(entity).unwrap().0.clone();
    let mut original = capture(app.world());
    assert!(
        original
            .entities
            .iter()
            .find(|row| row.uuid == uuid)
            .unwrap()
            .weapons
            .as_ref()
            .unwrap()
            .magazine
            .as_ref()
            .unwrap()
            .incoming_total()
            > 0
    );
    original.tick += 250;
    let row = original
        .entities
        .iter_mut()
        .find(|row| row.uuid == uuid)
        .unwrap();
    row.red_alert = Some(!row.red_alert.unwrap_or(false));
    let (mut positive, _) = started();
    let report = restore(positive.world_mut(), &original);
    assert!(
        report.is_complete(),
        "the unchanged claim frontier and other changed fields must be admissible: {:?}",
        report.gaps
    );
    for mode in 0..4 {
        let mut candidate = original.clone();
        let row = candidate
            .entities
            .iter_mut()
            .find(|row| row.uuid == uuid)
            .unwrap();
        let magazine = row.weapons.as_mut().unwrap().magazine.as_mut().unwrap();
        match mode {
            0 => magazine.incoming.clear(),
            1 => {
                let (_, amount) = magazine.incoming.pop_first().unwrap();
                magazine.incoming.insert("foreign-owner/999".into(), amount);
            }
            2 => {
                let amount = magazine.incoming.values_mut().next().unwrap();
                assert!(*amount >= 2);
                *amount -= 1;
            }
            _ => row.logistics = None,
        }
        row.weapons
            .as_ref()
            .unwrap()
            .magazine
            .as_ref()
            .unwrap()
            .validate()
            .unwrap();
        // The stock shape itself remains valid: only complete owner/claim
        // agreement can reject this. JSON parsing is a real persistence step.
        let candidate: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&candidate).unwrap()).unwrap();
        let before = serde_json::to_value(capture(app.world())).unwrap();
        let digest = world_digest(app.world());
        let entities = app.world().entities().len();
        let report = restore(app.world_mut(), &candidate);
        assert!(!report.is_complete(), "corrupt mode {mode} must refuse");
        let gaps = format!("{:?}", report.gaps);
        assert!(
            gaps.contains("incoming") || gaps.contains("logistics"),
            "refusal must identify actual owner/claim disagreement: {gaps}"
        );
        assert_eq!(report.entities_restored, 0);
        assert_eq!(report.entities_spawned, 0);
        assert_eq!(report.despawned, 0);
        assert_eq!(app.world().entities().len(), entities);
        assert_eq!(world_digest(app.world()), digest);
        assert_eq!(serde_json::to_value(capture(app.world())).unwrap(), before);
        quantities(&app, entity);
    }
}

fn mesh_step(hosts: &mut [App; 2]) {
    use project_phoenix::lockstep::{MeshInbox, MeshOutbox};
    let frames = hosts
        .each_mut()
        .map(|app| app.world_mut().resource_mut::<MeshOutbox>().drain());
    for index in 0..2 {
        for frame in &frames[1 - index] {
            hosts[index]
                .world_mut()
                .resource_mut::<MeshInbox>()
                .push(frame.clone());
        }
    }
    for host in hosts.iter_mut() {
        host.update();
    }
    assert_eq!(
        hosts[0].world().resource::<SimTick>().0,
        hosts[1].world().resource::<SimTick>().0
    );
    assert_eq!(
        world_digest(hosts[0].world()),
        world_digest(hosts[1].world())
    );
}

#[test]
fn actual_deliveries_conserve_six_rounds_on_both_local_ship_peers() {
    use project_phoenix::{
        command_admission::HostSlot,
        lockstep::{authored_delay, join_fleet, FleetRoster, FleetShip, FleetSlotOf},
    };
    let mut hosts = [HostSlot(1), HostSlot(2)].map(|local_slot| {
        let mut app = unstarted("tests/fixtures/physical-fleet-world.toml");
        let roster = FleetRoster::new(
            [HostSlot(1), HostSlot(2)]
                .into_iter()
                .map(|host| FleetShip {
                    host,
                    ship_path: Some("assets/entities/alliance_cruiser.toml".into()),
                    authored_slot_id: None,
                    crew: vec![],
                })
                .collect(),
            local_slot,
        );
        let delay = authored_delay(app.world());
        assert!(join_fleet(app.world_mut(), roster, delay));
        app.finish();
        app.cleanup();
        app
    });
    let mut crossed = BTreeSet::new();
    let mut delivered = BTreeSet::new();
    let mut locals = BTreeMap::new();
    let mut bound = 720;
    let mut step = 0;
    while step < bound {
        mesh_step(&mut hosts);
        for (index, app) in hosts.iter_mut().enumerate() {
            let actors: BTreeMap<_, _> = app
                .world_mut()
                .query::<(Entity, &EntityUuid, &FleetSlotOf)>()
                .iter(app.world())
                .map(|(entity, uuid, _)| (uuid.0.clone(), entity))
                .collect();
            if actors.is_empty() {
                assert!(step < 10, "ordinary fleet start must produce actual hulls");
                continue;
            }
            assert_eq!(actors.len(), 2, "both actual fleet hulls must exist");
            let local = local(app);
            locals
                .entry(index)
                .or_insert(app.world().get::<FleetSlotOf>(local).unwrap().0);
            if step < 10 {
                bound = bound.max(completion_bound(app, local) + 10);
            }
            for (uuid, entity) in actors {
                quantities(app, entity);
                if crossing(app, entity) {
                    crossed.insert((index, uuid.clone()));
                }
                let state = &app.world().get::<ShipLogistics>(entity).unwrap().0;
                if state
                    .last_outcome
                    .as_ref()
                    .is_some_and(|outcome| !outcome.returned && outcome.amount > 0)
                {
                    delivered.insert((index, uuid));
                }
            }
        }
        step += 1;
        if step >= 720
            && hosts.iter_mut().all(|app| {
                let actors: Vec<_> = app
                    .world_mut()
                    .query_filtered::<Entity, With<ShipLogistics>>()
                    .iter(app.world())
                    .collect();
                actors.len() == 2
                    && actors
                        .into_iter()
                        .all(|entity| quantities(app, entity) == (0, 6, 0))
            })
        {
            break;
        }
    }
    assert_eq!(locals.len(), 2);
    assert_ne!(
        locals[&0], locals[&1],
        "peers must own different LocalShip projections"
    );
    assert_eq!(
        crossed.len(),
        4,
        "each hull must carry real cargo on each peer"
    );
    assert_eq!(
        delivered.len(),
        4,
        "each hull must complete real delivery on each peer"
    );
    for app in &mut hosts {
        let actors: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<ShipLogistics>>()
            .iter(app.world())
            .collect();
        assert_eq!(actors.len(), 2);
        for entity in actors {
            assert_delivered(app, entity);
        }
    }
}
