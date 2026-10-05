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
