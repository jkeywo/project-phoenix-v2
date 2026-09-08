//! Shared collision attribution driven by the real collision-producing world.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::{
    core::{
        balance::{BalanceEvent, VictimKind, WEAPON_KIND_COLLISION},
        collision_history::CollisionHistory,
        telemetry::RunTelemetry,
    },
    headless::{build_headless_app, HeadlessArgs},
    sim_digest::{digest_stages, fold_f32, fold_str, fold_u64, world_digest},
    snapshot,
};

#[test]
fn real_collision_history_roundtrips_without_report_telemetry_in_the_digest() {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "assets/worlds/rng_coverage.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        seed: Some(20260720),
        dt: 1.0 / 30.0,
        deterministic: true,
        max_ticks: 2700,
        ..Default::default()
    })
    .unwrap();
    app.finish();
    app.cleanup();
    for _ in 0..2701 {
        app.update();
        if !app
            .world()
            .resource::<CollisionHistory>()
            .records()
            .is_empty()
        {
            break;
        }
    }
    let history = app.world().resource::<CollisionHistory>().records();
    assert!(
        !history.is_empty(),
        "the real collision responder must apply damage"
    );
    let capture = snapshot::capture(app.world());
    assert_eq!(capture.collisions, history);
    let expected = world_digest(app.world());

    // Compare the preserved byte-level fold to the original report-attribution
    // projection on this real run. The new owner changes neither field order nor
    // victim/damage/shield/hull meaning.
    let stages = digest_stages(app.world());
    let mut legacy = stages
        .iter()
        .find(|(name, _)| *name == "asteroid")
        .unwrap()
        .1;
    let reported: Vec<_> = app
        .world()
        .resource::<RunTelemetry>()
        .balance_events
        .iter()
        .filter_map(|row| match &row.event {
            BalanceEvent::DamageApplied {
                weapon,
                victim,
                amount,
                shield_absorbed,
                hull_damage,
                ..
            } if weapon == WEAPON_KIND_COLLISION => {
                Some((victim, *amount, *shield_absorbed, *hull_damage))
            }
            _ => None,
        })
        .collect();
    assert_eq!(reported.len(), history.len());
    legacy = fold_u64(legacy, reported.len() as u64);
    for (victim, amount, shield, hull) in reported {
        legacy = fold_str(legacy, victim);
        legacy = fold_f32(legacy, amount);
        legacy = fold_f32(legacy, shield);
        legacy = fold_f32(legacy, hull);
    }
    assert_eq!(
        legacy,
        stages
            .iter()
            .find(|(name, _)| *name == "collisions")
            .unwrap()
            .1
    );
    let report = app.world_mut().remove_resource::<RunTelemetry>().unwrap();
    assert_eq!(
        world_digest(app.world()),
        expected,
        "browser-oriented composition has no report observer"
    );
    assert_eq!(
        snapshot::capture(app.world()).collisions,
        capture.collisions
    );
    app.insert_resource(report);

    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(BalanceEvent::DamageApplied {
            attacker: None,
            victim: "unread-bootstrap-collision".into(),
            victim_kind: VictimKind::Ship,
            weapon: WEAPON_KIND_COLLISION.into(),
            amount: 100.0,
            shield_absorbed: 0.0,
            hull_damage: 100.0,
            system_hit: None,
        });
    let restored = snapshot::restore(app.world_mut(), &capture);
    assert!(restored.is_complete(), "{:?}", restored.gaps);
    assert_eq!(world_digest(app.world()), expected);
    assert_eq!(
        snapshot::capture(app.world()).collisions,
        capture.collisions
    );
    app.update();
    let continued = app.world().resource::<CollisionHistory>().records();
    assert!(continued.starts_with(&capture.collisions));
    assert!(continued
        .iter()
        .all(|row| row.victim != "unread-bootstrap-collision"));
    assert!(app.world().resource::<RunTelemetry>().balance_events.iter().all(|row| !matches!(
        &row.event, BalanceEvent::DamageApplied { victim, .. } if victim == "unread-bootstrap-collision"
    )));
}
