#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

#[path = "common/default_pool.rs"]
// This target only needs the helper's pinned/default-pool switch; its observer
// and child-process guards belong to the dedicated determinism test binaries.
#[allow(dead_code)]
mod default_pool;

use bevy::prelude::*;
use project_phoenix::command_admission::HostSlot;
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};
use project_phoenix::lockstep::{FleetRoster, FleetShip, FleetSlotOf};
use project_phoenix::objective_instances::player_ship_memberships;
use project_phoenix::server_app::{LocalShip, Ship};
use project_phoenix::ship_slots::{
    AuthoredShipSlotId, FrozenShipSlots, LaunchSource, LaunchedSlot,
};
use project_phoenix::world::server::{ObjectiveInstanceManagerRes, ObjectiveManagerRes};
use project_phoenix::{
    core::messages::{StationId, SystemId},
    ship::control_source::ControlSource,
};

const WORLD: &str = "tests/fixtures/worlds/multi_ship_slots_objectives.toml";
const HULL: &str = "assets/entities/alliance_cruiser.toml";
const HELM_TOKEN: &str = "lead-helm";

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        seed: Some(1_523),
        max_ticks: 120,
        deterministic: default_pool::deterministic(),
        ..Default::default()
    }
}

fn frozen() -> FrozenShipSlots {
    FrozenShipSlots(vec![
        LaunchedSlot {
            slot_id: "lead".into(),
            hull: HULL.into(),
            claimant: Some(HostSlot::SOLO.slot_id()),
            source: LaunchSource::Claimed,
        },
        LaunchedSlot {
            slot_id: "wing".into(),
            hull: HULL.into(),
            claimant: None,
            source: LaunchSource::Backfill,
        },
    ])
}

fn boot() -> App {
    let mut app = build_headless_app(&args()).expect("seeded multi-ship fixture builds");
    {
        let mut sessions = app
            .world_mut()
            .resource_mut::<project_phoenix::lobby::server::Sessions>();
        sessions
            .0
            .register(HELM_TOKEN.into(), "Lead Helm".into())
            .unwrap();
        sessions
            .0
            .set_station(HELM_TOKEN, Some(StationId("helm".into())));
    }
    app.insert_resource(FleetRoster::new(
        vec![FleetShip {
            host: HostSlot::SOLO,
            ship_path: Some(HULL.into()),
            authored_slot_id: Some("lead".into()),
            crew: vec![(StationId("helm".into()), "Std".into())],
        }],
        HostSlot::SOLO,
    ));
    app.insert_resource(frozen());
    run(&mut app, 30);
    app
}

fn census(app: &mut App) -> Vec<(String, HostSlot, bool)> {
    let mut query = app
        .world_mut()
        .query::<(&AuthoredShipSlotId, &FleetSlotOf, Has<LocalShip>, &Ship)>();
    let mut rows: Vec<_> = query
        .iter(app.world())
        .map(|(slot, host, local, _)| (slot.0.clone(), host.0, local))
        .collect();
    rows.sort();
    rows
}

#[test]
fn one_host_launches_claimed_and_backfill_slots_but_not_absent_then_survives_host_loss() {
    let mut app = boot();
    assert_eq!(
        census(&mut app),
        vec![
            ("lead".into(), HostSlot::SOLO, true),
            ("wing".into(), HostSlot(2), false),
        ],
        "the reserve GameStart row belongs to the omitted Absent slot and must not spawn"
    );

    let helm_source = |app: &mut App, slot: &str| {
        let mut query = app.world_mut().query::<(
            &AuthoredShipSlotId,
            &project_phoenix::ship::components::ShipSystemControlSources,
        )>();
        query
            .iter(app.world())
            .find(|(id, _)| id.0 == slot)
            .unwrap()
            .1
             .0
            .source_for(&SystemId("helm-thrust".into()))
    };
    assert_eq!(helm_source(&mut app, "lead"), ControlSource::Human);
    assert_eq!(helm_source(&mut app, "wing"), ControlSource::Ai);

    app.world_mut()
        .resource_mut::<project_phoenix::lobby::server::Sessions>()
        .0
        .disconnect(HELM_TOKEN);
    let loss_tick = app
        .world()
        .resource::<project_phoenix::sim_tick::SimTick>()
        .0
        + 1;
    app.world_mut()
        .resource_mut::<project_phoenix::lockstep::PendingHostLoss>()
        .observe(HostSlot::SOLO, loss_tick);
    run(&mut app, 30);
    assert_eq!(
        census(&mut app).len(),
        2,
        "post-start host loss cannot reshape the fleet"
    );
    assert_eq!(helm_source(&mut app, "lead"), ControlSource::Ai);
    assert!(app
        .world()
        .resource::<project_phoenix::lockstep::PendingHostLoss>()
        .is_applied(HostSlot::SOLO));
    assert_eq!(app.world().resource::<FrozenShipSlots>(), &frozen());

    let snapshot = project_phoenix::snapshot::capture(app.world());
    let encoded = serde_json::to_string(&snapshot).unwrap();
    let restored: project_phoenix::snapshot::PhoenixSnapshot =
        serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored.frozen_ship_slots, Some(frozen()));
    app.world_mut().remove_resource::<FrozenShipSlots>();
    let report = project_phoenix::snapshot::restore(app.world_mut(), &restored);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(app.world().resource::<FrozenShipSlots>(), &frozen());
}

#[test]
fn seeded_two_ship_world_projects_distinct_directives_and_reports_fixed_completion_members() {
    let mut app = boot();
    let memberships = player_ship_memberships(app.world_mut());
    assert_eq!(memberships.len(), 2);

    let definitions = app.world().resource::<ObjectiveManagerRes>();
    let instances = app.world().resource::<ObjectiveInstanceManagerRes>();
    let mut directives = std::collections::BTreeMap::new();
    for ship in &memberships {
        let projected = instances.0.project_scored_for_ship(
            &ship.ship_id,
            definitions.0.scored_pool_for(
                &project_phoenix::objectives::WorldConditions::default(),
                &ship.ship_id,
            ),
        );
        let directive = projected
            .first()
            .map(|row| row.directive.clone())
            .expect("each ship receives its own actionable directive");
        directives.insert(ship.slot_id.clone(), directive);
    }
    assert_ne!(directives["lead"], directives["wing"]);

    let lead = project_phoenix::objective_instances::ObjectiveInstanceKey {
        objective_id: "take-position".into(),
        instance_id: "lead-order".into(),
    };
    app.world_mut()
        .resource_mut::<ObjectiveInstanceManagerRes>()
        .0
        .complete(&lead, &memberships)
        .unwrap();
    let report = build_report(&mut app, &args(), 0.0);
    let records: serde_json::Value = serde_json::from_str(&report.objective_instances).unwrap();
    let lead_record = records["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["spec"]["key"]["instance_id"] == "lead-order")
        .unwrap();
    let lead_ship = memberships
        .iter()
        .find(|ship| ship.slot_id == "lead")
        .unwrap();
    assert_eq!(lead_record["completion_members"][0], lead_ship.ship_id);
}
