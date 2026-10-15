//! Actual finite sensing on ordinary entity equipment; deployment remains later19.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    entities::spawner::EntityId,
    headless::{build_headless_app, HeadlessArgs},
    science::broad::BroadTracks,
    ship::power::ShipPowerSystem,
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
use std::collections::BTreeSet;

fn boot() -> App {
    project_phoenix::entities::include_resolve::load_entity_config(
        "assets/entities/battery_probe.toml",
    )
    .expect("ordinary probe source must pass the real include/entity schema");
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/battery-probe-world.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156219),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    app
}
fn probe(app: &mut App) -> Entity {
    app.world_mut()
        .query::<(Entity, &EntityId)>()
        .iter(app.world())
        .find_map(|(entity, id)| (id.0 == "battery-probe").then_some(entity))
        .unwrap()
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
            keys.into_iter().find_map(|key| {
                first_difference(
                    a.get(key).unwrap_or(&serde_json::Value::Null),
                    b.get(key).unwrap_or(&serde_json::Value::Null),
                    &format!("{path}.{key}"),
                )
            })
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (a, b))| first_difference(a, b, &format!("{path}[{i}]"))),
        _ => (a != b).then(|| format!("{path}: {a} != {b}")),
    }
}

#[test]
fn finite_ordinary_probe_retains_paid_history_through_exhaustion_and_json_continuation() {
    let mut live = boot();
    let entity = probe(&mut live);
    let owner = project_phoenix::core::messages::SystemId("sensor-radar".into());
    let initial = live
        .world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&owner]
        .energy;
    for _ in 0..590 {
        live.update();
    }
    let power = &live.world().get::<ShipPowerSystem>(entity).unwrap().0;
    assert_eq!(power.config().reactor.sustainable_output_per_sec, 0.0);
    assert_eq!(power.config().reactor.max_output_per_sec, 0.0);
    assert!(power.network().consumers[&owner].energy < initial);
    assert!(
        power.network().consumers[&owner].energy > 0.0,
        "capture actual ongoing finite work"
    );
    let acquired = &live.world().get::<BroadTracks>(entity).unwrap().0[&owner];
    let profile = &live
        .world()
        .get::<project_phoenix::science::broad::BroadProfile>(entity)
        .unwrap()
        .0[&owner]
        .profile;
    let published = acquired
        .projection(profile, live.world().resource::<SimTick>().0, 1.0 / 60.0)
        .unwrap();
    assert!(
        published.iter().any(|record| record
            .identity
            .as_ref()
            .is_some_and(|identity| identity.name == "subject")),
        "ordinary instrument must actually identify the static subject"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_str(&serde_json::to_string(&capture(live.world())).unwrap()).unwrap();
    let mut resumed = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..360 {
        let original = serde_json::to_value(capture(live.world())).unwrap();
        let restored = serde_json::to_value(capture(resumed.world())).unwrap();
        assert!(
            original == restored,
            "whole host continuation {step}: {:?}",
            first_difference(&original, &restored, "$")
        );
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "digest {step}"
        );
        live.update();
        resumed.update();
    }
    let power = &live.world().get::<ShipPowerSystem>(entity).unwrap().0;
    assert_eq!(power.network().consumers[&owner].energy, 0.0);
    assert_eq!(power.emitted_work(&owner), 0.0);
    let tick = live.world().resource::<SimTick>().0;
    let profile = &live
        .world()
        .get::<project_phoenix::science::broad::BroadProfile>(entity)
        .unwrap()
        .0[&owner]
        .profile;
    let held = live.world().get::<BroadTracks>(entity).unwrap().0[&owner]
        .projection(profile, tick, 1.0 / 60.0)
        .unwrap();
    assert!(
        !held.is_empty(),
        "actual acquired evidence remains available"
    );
    assert!(held
        .iter()
        .all(|track| track.predicted && track.observed_tick < tick - 60));
}
