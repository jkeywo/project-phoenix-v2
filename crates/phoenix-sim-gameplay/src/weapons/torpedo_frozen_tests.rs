use super::*;
use std::collections::HashMap;

fn loaded(count: u32) -> TorpedoSystem {
    let config = TorpedoConfig {
        count: 6,
        seeker: Some(phoenix_model::guided_guidance::SeekerProfile {
            range: 2000.0,
            fire_arc_deg: 360.0,
            minimum_transmission: 0.01,
        }),
        speed: 8.0,
        turn_rate: 0.4,
        lifespan: 90.0,
        detonation_radius: 0.5,
        damage_hull: 40,
        damage_shields: 12,
        shield_pierce: 0.2,
        durability: 6.0,
        load_time: 0.1,
        burst_interval_secs: 0.2,
        ..Default::default()
    };
    let mut system = TorpedoSystem::new(config);
    system.tubes[0].volley_max = count;
    let id = system.tubes[0].id.clone();
    system.set_volley_target(&id, count);
    assert!(system.start_load(&id));
    for _ in 0..count {
        system.tick(0.1, &HashMap::new(), &mut || "no-flight".into());
    }
    assert_eq!(system.tube(&id).unwrap().loaded_count, count);
    assert!(matches!(
        system.launch(
            &id,
            "first".into(),
            0.0,
            0.0,
            0.0,
            0.0,
            Some("target".into()),
            Some("launcher".into())
        ),
        LaunchResult::Launched { .. }
    ));
    system.set_volley_target(&id, 0);
    system
}

#[test]
fn committed_burst_keeps_flight_payload_and_cadence_without_inheriting_paid_boost() {
    let mut system = loaded(3);
    let original = system.in_flight[0].profile;
    system.in_flight[0].strike_damage_bonus = 0.5; // Work granted only to the emitted first round.
    system.config.speed = 80.0;
    system.config.turn_rate = 4.0;
    system.config.lifespan = 900.0;
    system.config.detonation_radius = 50.0;
    system.config.damage_hull = 400;
    system.config.damage_shields = 120;
    system.config.shield_pierce = 1.0;
    system.config.durability = 60.0;
    system.config.burst_interval_secs = 9.0;
    system.tubes[0].impact.penetration = 100.0;
    let targets = ["first", "burst-1", "burst-2"]
        .into_iter()
        .map(|id| {
            (
                id.into(),
                GuidanceObservation {
                    target_uuid: "target".into(),
                    position: [100.0, 30.0, -50.0],
                    sampled_tick: 0,
                    source: GuidanceSource::Seeker,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let mut next = 0;
    let mut uuid = || {
        next += 1;
        format!("burst-{next}")
    };
    let denied = system.tick_with_work(
        0.25,
        &targets,
        &mut |work| {
            if matches!(work, TorpedoWork::Launch { .. }) {
                0.0
            } else {
                1.0
            }
        },
        &mut uuid,
    );
    assert!(denied.burst_launched.is_empty());
    assert_eq!(system.burst_states[0].pending, 2);
    let funded = system.tick_with_work(0.0, &targets, &mut |_| 1.0, &mut uuid);
    assert_eq!(funded.burst_launched.len(), 1);
    assert_eq!(system.in_flight[1].profile, original);
    assert_eq!(
        system.in_flight[1].strike_damage_bonus, 0.0,
        "later work does not inherit the first shot's paid boost"
    );
    assert_eq!(system.burst_states[0].timer, 0.2);
    system.tick(0.21, &targets, &mut uuid);
    assert_eq!(system.in_flight.len(), 3);
    assert!(system.burst_states.is_empty());
    assert_eq!(system.rounds_aboard(), 3);
    for round in &system.in_flight {
        assert_eq!(round.profile, original);
        assert_eq!(round.durability_remaining, original.durability);
        assert!(round.y > 0.0 && round.pitch > 0.0);
        assert!(round.pitch <= original.turn_rate * 0.46 + f32::EPSILON);
    }
    assert_eq!(system.in_flight[0].strike_damage_bonus, 0.5);
    assert_eq!(system.in_flight[2].strike_damage_bonus, 0.0);
}

#[test]
fn contact_and_detonation_read_launched_equipment_not_current_launcher() {
    let mut system = loaded(1);
    let original = system.in_flight[0].profile;
    system.in_flight[0].strike_damage_bonus = 0.5;
    system.config.detonation_radius = 0.0;
    system.config.damage_hull = 400;
    system.config.damage_shields = 120;
    system.config.shield_pierce = 1.0;
    let hits = system.find_detonation_hits(&[("target".into(), 0.4, 0.0, 0.0, 0.0)]);
    assert_eq!(hits, [("first".into(), "target".into())]);
    let detonation = system.handle_collision_full("first").unwrap();
    assert_eq!(detonation.damage_hull, 60);
    assert_eq!(detonation.damage_shields, 18);
    assert_eq!(detonation.shield_pierce, original.shield_pierce);
    assert_eq!(detonation.impact, original.impact);
    assert!(system.handle_collision_full("first").is_none());
}

#[test]
fn physical_interception_uses_frozen_finite_durability_and_removes_payload_once() {
    let mut system = loaded(1);
    system.config.durability = 60.0;
    assert!(system.apply_interception("first", f32::NAN).is_err());
    assert_eq!(system.in_flight[0].durability_remaining, 6.0);
    assert_eq!(system.apply_interception("first", 2.0).unwrap(), Some(4.0));
    assert_eq!(system.apply_interception("first", 4.0).unwrap(), Some(0.0));
    assert!(system.in_flight.is_empty());
    assert_eq!(system.apply_interception("first", 2.0).unwrap(), None);
    assert!(
        system.handle_collision_full("first").is_none(),
        "intercepted payload cannot also detonate"
    );
    assert_eq!(system.rounds_aboard(), 5);
}

#[test]
fn actual_guided_motion_exposes_one_relative_collision_interval_without_saved_shadow() {
    let mut system = loaded(1);
    assert_eq!(system.in_flight[0].previous_position, None);
    let first = [
        system.in_flight[0].x,
        system.in_flight[0].y,
        system.in_flight[0].z,
    ];
    let targets = HashMap::from([(
        "first".into(),
        GuidanceObservation {
            target_uuid: "target".into(),
            position: [10.0, 6.0, -20.0],
            sampled_tick: 0,
            source: GuidanceSource::Seeker,
        },
    )]);
    system.tick_with_work(
        0.2,
        &targets,
        &mut |_| panic!("already launched flight buys no machinery work"),
        &mut || unreachable!(),
    );
    assert_eq!(system.in_flight[0].previous_position, Some(first));
    system.in_flight[0].tick(0.0, None);
    assert_eq!(
        system.in_flight[0].previous_position,
        Some(first),
        "a zero-time machinery opportunity cannot erase the physical interval"
    );
    let current = system.in_flight[0].clone();
    assert_ne!([current.x, current.y, current.z], first);
    system.finish_collision_interval();
    let mut expected = current;
    expected.previous_position = None;
    assert_eq!(system.in_flight[0], expected);
    system.tick_with_work(
        0.1,
        &targets,
        &mut |_| panic!("already launched flight buys no machinery work"),
        &mut || unreachable!(),
    );
    assert_eq!(
        system.in_flight[0].previous_position,
        Some([expected.x, expected.y, expected.z])
    );
    system.finish_collision_interval();
    assert_eq!(system.in_flight[0].previous_position, None);
}
