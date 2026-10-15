use super::*;
#[test]
fn low_hint_needs_acquired_history_and_never_samples_live_shields() {
    let mut app = test_app();
    start_game_with_sensors_and_tactical(&mut app);
    set_local_weapons_target(&mut app, Some("unprobed-target".into()));
    let target = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("unprobed-target".into()),
            crate::ship::shields::ShipShields(
                crate::weapons::shield::ShieldSystem::default(),
                0.92,
            ),
        ))
        .id();
    tick(&mut app);
    assert!(!app
        .world()
        .resource::<EnqueueLog>()
        .0
        .iter()
        .any(|row| matches!(row.payload, CoordinationPayload::FrequencyHint { .. })));
    acquire_local_hint(&mut app, "unprobed-target", 0.71);
    tick(&mut app);
    assert!(app.world().resource::<EnqueueLog>().0.iter().any(|row| matches!(row.payload, CoordinationPayload::FrequencyHint { frequency } if frequency == 0.71)));
    app.world_mut().resource_mut::<EnqueueLog>().0.clear();
    app.world_mut()
        .get_mut::<crate::ship::shields::ShipShields>(target)
        .unwrap()
        .1 = 0.19;
    tick(&mut app);
    assert!(
        app.world().resource::<EnqueueLog>().0.is_empty(),
        "target rotation cannot synthesize another finding"
    );
}
