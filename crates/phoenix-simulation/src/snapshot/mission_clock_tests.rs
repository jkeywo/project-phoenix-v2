use super::*;

#[test]
fn saved_mission_age_requires_clock_owners_before_any_live_write() {
    for missing_fixed in [true, false] {
        let mut world = World::new();
        world.insert_resource(SimTick(7));
        if missing_fixed {
            world.insert_resource(WorldContentRuntime::default());
        } else {
            world.insert_resource(Time::<Fixed>::default());
        }
        let entity = world
            .spawn((EntityUuid("clock-test".into()), ShipRedAlert(false)))
            .id();
        let before = capture(&world);
        let mut saved = before.clone();
        saved.tick = 99;
        saved.scenario = Some(ScenarioState {
            mission_elapsed: Some(std::time::Duration::from_secs(1)),
            ..Default::default()
        });
        // Also attempt an unrelated entity write, to prove global refusal.
        saved.entities = vec![EntityState {
            uuid: "clock-test".into(),
            red_alert: Some(true),
            ..Default::default()
        }];
        let report = restore(&mut world, &saved);
        assert_eq!(report.gaps, vec![RestoreGap::MissionClockUnavailable]);
        assert_eq!(report.entities_restored, 0);
        assert_eq!(report.entities_spawned, 0);
        assert_eq!(report.despawned, 0);
        assert_eq!(capture(&world), before);
        assert!(!world.get::<ShipRedAlert>(entity).unwrap().0);
        assert_eq!(world.resource::<SimTick>().0, 7);
        // A truly unanchored sparse world still needs no clock owners.
        saved.scenario.as_mut().unwrap().mission_elapsed = None;
        assert!(super::preflight::mission_clock_gap(&world, &saved).is_none());
    }
}
