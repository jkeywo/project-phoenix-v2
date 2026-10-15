use super::*;
use crate::ship::combat_activity::RecentCombatActivity;

fn fixture(tick: u64) -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(SimTick(tick));
    world.insert_resource(Time::<Fixed>::from_duration(
        crate::sim_tick::sim_tick_period(60.0),
    ));
    let entity = world
        .spawn((
            EntityUuid("combat-clock".into()),
            ShipRedAlert(false),
            RecentCombatActivity::default(),
        ))
        .id();
    (world, entity)
}

#[test]
fn canonical_combat_snapshot_keeps_old_event_bits_at_arbitrary_accepted_ticks() {
    for tick in [1_000_000_000, u64::MAX - 360] {
        let (mut live, entity) = fixture(tick);
        let period = crate::sim_tick::sim_tick_period(60.0).as_secs_f64();
        let original = [
            (87.0 * period) as f32,
            (107.0 * period) as f32,
            (tick as f64 * period) as f32,
        ];
        live.get_mut::<RecentCombatActivity>(entity)
            .unwrap()
            .replace_continuation(
                Some(original[0]),
                Some(original[1]),
                Some(original[2]),
                73.0,
            );
        let saved: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&capture(&live)).unwrap()).unwrap();
        let (mut resumed, restored) = fixture(7);
        let report = restore(&mut resumed, &saved);
        assert!(report.is_complete(), "{report:?}");
        let actual = resumed
            .get::<RecentCombatActivity>(restored)
            .unwrap()
            .continuation();
        assert_eq!(
            [
                actual.0.unwrap().to_bits(),
                actual.1.unwrap().to_bits(),
                actual.2.unwrap().to_bits()
            ],
            original.map(f32::to_bits)
        );
        for step in 0..360 {
            live.resource_mut::<SimTick>().0 = tick + step;
            resumed.resource_mut::<SimTick>().0 = tick + step;
            assert_eq!(capture(&live), capture(&resumed));
        }
    }
}

#[test]
fn malformed_combat_history_refuses_the_whole_candidate_before_any_write() {
    for value in [f32::NAN, f32::INFINITY, -1.0, 10.0] {
        for missing_entity in [false, true] {
            let (mut world, entity) = fixture(71);
            let before = capture(&world);
            let mut saved = before.clone();
            saved.tick = 99;
            saved.entities[0].red_alert = Some(true);
            let row = if missing_entity {
                saved.entities.push(EntityState {
                    uuid: "not-materialized".into(),
                    ..Default::default()
                });
                saved.entities.last_mut().unwrap()
            } else {
                &mut saved.entities[0]
            };
            row.recent_combat = Some(RecentCombatActivityState {
                last_damage_taken_secs: Some(value),
                ..Default::default()
            });
            let report = restore(&mut world, &saved);
            assert!(
                matches!(report.gaps.as_slice(), [RestoreGap::ResourceStateInvalid {reason, ..}] if reason.contains("combat timestamp")),
                "{report:?}"
            );
            assert_eq!(report.entities_restored, 0);
            assert_eq!(report.entities_spawned, 0);
            assert_eq!(report.despawned, 0);
            assert!(!world.get::<ShipRedAlert>(entity).unwrap().0);
            assert_eq!(capture(&world), before);
        }
    }
}

#[test]
fn old_epoch_age_keys_cannot_decode_as_canonical_combat_history() {
    assert!(serde_json::from_str::<RecentCombatActivityState>(
        r#"{"last_damage_taken_age_secs":0.25,"prev_hull":73.0}"#
    )
    .is_err());
}

#[test]
fn malformed_hull_or_missing_logical_clock_refuses_before_mutating_other_owners() {
    for previous in [f32::NAN, f32::INFINITY, -1.0] {
        let (mut world, _) = fixture(71);
        let before = capture(&world);
        let mut saved = before.clone();
        saved.tick = 99;
        saved.entities[0].red_alert = Some(true);
        saved.entities[0].recent_combat.as_mut().unwrap().prev_hull = previous;
        let report = restore(&mut world, &saved);
        assert!(
            matches!(report.gaps.as_slice(), [RestoreGap::ResourceStateInvalid { reason, .. }] if reason.contains("previous hull")),
            "{report:?}"
        );
        assert_eq!(capture(&world), before);
    }
    let (mut world, _) = fixture(71);
    world.remove_resource::<Time<Fixed>>();
    let before = capture(&world);
    let mut saved = before.clone();
    saved.tick = 99;
    saved.entities[0].red_alert = Some(true);
    saved.entities[0]
        .recent_combat
        .as_mut()
        .unwrap()
        .last_weapon_fired_secs = Some(0.0);
    let report = restore(&mut world, &saved);
    assert!(
        matches!(report.gaps.as_slice(), [RestoreGap::ResourceStateInvalid { reason, .. }] if reason.contains("fixed clock")),
        "{report:?}"
    );
    assert_eq!(capture(&world), before);
}
