use super::*;

fn seed(
    positions: &mut LastBroadcastEntityPositions,
    health: &mut LastBroadcastEntityHealth,
    uuid: &str,
) {
    positions.0.insert(uuid.into(), (Vec3::ZERO, 0.0));
    health
        .0
        .insert(uuid.into(), (Some(1.0), Some(1.0), None, None));
}

#[test]
fn uuid_pruning_removes_only_named_entities() {
    let mut positions = LastBroadcastEntityPositions::default();
    let mut health = LastBroadcastEntityHealth::default();
    for uuid in ["keep", "gone-a", "gone-b"] {
        seed(&mut positions, &mut health, uuid);
    }

    prune_entity_replication_caches(
        &mut positions,
        &mut health,
        &["gone-a".into(), "gone-b".into(), "unknown".into()],
    );

    assert_eq!(positions.0.keys().collect::<Vec<_>>(), vec!["keep"]);
    assert_eq!(health.0.keys().collect::<Vec<_>>(), vec!["keep"]);
}

#[test]
fn repeated_uuid_pruning_keeps_long_session_caches_bounded() {
    let mut positions = LastBroadcastEntityPositions::default();
    let mut health = LastBroadcastEntityHealth::default();
    const LIVE_AT_ONCE: usize = 5;

    for cycle in 0..500 {
        seed(&mut positions, &mut health, &format!("asteroid-{cycle}"));
        if cycle >= LIVE_AT_ONCE {
            prune_entity_replication_caches(
                &mut positions,
                &mut health,
                &[format!("asteroid-{}", cycle - LIVE_AT_ONCE)],
            );
        }
    }

    assert_eq!(positions.0.len(), LIVE_AT_ONCE);
    assert_eq!(health.0.len(), LIVE_AT_ONCE);
}

#[test]
fn console_health_keeps_detached_and_same_seat_scopes_distinct() {
    use crate::core::messages::{ConsoleId, SystemId};
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = "Pilot"
rank = "Ens."
[[console]]
id = "flight"
home_station = "helm"
screen = "fixture.html"
[[console]]
id = "drive"
home_station = "helm"
screen = "fixture.html"
[[console]]
id = "detached"
[[console]]
id = "unmodeled"
[[system]]
id = "a"
kind = "fixture"
console = "flight"
[[system]]
id = "b"
kind = "fixture"
console = "drive"
[[system]]
id = "c"
kind = "fixture"
console = "detached"
ai_only = true
[[system]]
id = "d"
kind = "fixture"
console = "detached"
ai_only = true
"#,
        &["fixture"],
    )
    .unwrap();
    let mut hull = crate::ship::damage::SystemHull::from_config(&[
        (SystemId("a".into()), 100.0),
        (SystemId("b".into()), 100.0),
        (SystemId("c".into()), 100.0),
        (SystemId("d".into()), 300.0),
        (SystemId("core".into()), 900.0),
    ]);
    hull.restore(&SystemId("a".into()), -75.0);
    hull.restore(&SystemId("c".into()), -100.0);
    let rows = console_health_from_parts(&config, Some(&hull));
    assert_eq!(
        rows.iter().map(|r| r.console.clone()).collect::<Vec<_>>(),
        ["flight", "drive", "detached", "unmodeled"].map(|id| ConsoleId(id.into()))
    );
    assert_eq!(
        rows.iter().map(|r| r.health).collect::<Vec<_>>(),
        vec![Some(0.25), Some(1.0), Some(0.75), None]
    );
    assert!(console_health_from_parts(&config, None)
        .iter()
        .all(|r| r.health.is_none()));
}
