use super::super::{capture, restore, BurstState, EntityState, RestoreGap, TorpedoInFlight};
use super::*;
use bevy::prelude::*;
use phoenix_model::guided_round::GuidedRoundProfile;

fn profile() -> GuidedRoundProfile {
    crate::weapons::torpedo::TorpedoConfig {
        durability: 6.0,
        ..Default::default()
    }
    .round_profile(Default::default())
}

fn candidate() -> PhoenixSnapshot {
    PhoenixSnapshot {
        tick: 999,
        entities: vec![EntityState {
            uuid: "launcher-removed".into(),
            weapons: Some(WeaponState {
                torpedoes_in_flight: vec![TorpedoInFlight {
                    profile: profile(),
                    durability_remaining: 4.0,
                    uuid: "actual-round".into(),
                    lifespan_remaining: 10.0,
                    tube_id: "fore".into(),
                    source_uuid: Some("launcher-removed".into()),
                    target_uuid: Some("last-target-removed".into()),
                    ..Default::default()
                }],
                bursts: vec![BurstState {
                    profile: profile(),
                    burst_interval_secs: 0.3,
                    tube_id: "fore".into(),
                    pending: 1,
                    timer: -0.1, // A starved emission can be due and waiting.
                    next_shot_index: 1,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn required_frozen_rounds_and_due_bursts_round_trip_without_live_source_or_target_truth() {
    let snapshot = candidate();
    validate_roster(&snapshot).unwrap();
    let value = serde_json::to_value(&snapshot).unwrap();
    let restored: PhoenixSnapshot = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(restored, snapshot);
    let mut missing = value;
    missing["entities"][0]["weapons"]["torpedoes_in_flight"][0]
        .as_object_mut()
        .unwrap()
        .remove("profile");
    assert!(serde_json::from_value::<PhoenixSnapshot>(missing).is_err());
}

#[test]
fn malformed_later_candidate_rounds_and_bursts_refuse_before_any_live_writes() {
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(42));
    world.insert_resource(Time::<Fixed>::default());
    let sentinel = world
        .spawn((
            crate::entities::spawner::EntityUuid("sentinel".into()),
            Transform::from_xyz(3.0, 4.0, 5.0),
        ))
        .id();
    let before = serde_json::to_value(capture(&world)).unwrap();
    let entity_count = world.entities().len();
    for case in 0..13 {
        let mut snapshot = candidate();
        let mut second = snapshot.entities[0].clone();
        second.uuid = "second-launcher".into();
        second.weapons.as_mut().unwrap().torpedoes_in_flight[0].uuid = "second-round".into();
        snapshot.entities.push(second);
        let weapons = snapshot.entities[1].weapons.as_mut().unwrap();
        match case {
            0 => weapons.torpedoes_in_flight[0].durability_remaining = 0.0,
            1 => weapons.torpedoes_in_flight[0].durability_remaining = 7.0,
            2 => weapons.torpedoes_in_flight[0].lifespan_remaining = 21.0,
            3 => weapons.torpedoes_in_flight[0].position[1] = f32::INFINITY,
            4 => weapons.torpedoes_in_flight[0].pitch = std::f32::consts::PI,
            5 => weapons.torpedoes_in_flight[0].profile.speed = f32::NAN,
            6 => weapons.torpedoes_in_flight[0].uuid = "actual-round".into(),
            7 => weapons.bursts[0].burst_interval_secs = 0.0,
            8 => weapons.bursts[0].pending = 0,
            9 => weapons.bursts[0].next_shot_index = u32::MAX,
            10 => weapons.bursts[0].timer = 1.0,
            11 => {
                weapons.bursts[0].barrel_origins.push([0.0; 3]);
                weapons.bursts[0].barrel_sequence.push((1, 0));
            }
            12 => weapons.torpedoes_in_flight[0].uuid = "launcher-removed".into(),
            _ => unreachable!(),
        }
        let report = restore(&mut world, &snapshot);
        assert!(
            matches!(report.gaps.as_slice(), [RestoreGap::ResourceStateInvalid { reason, .. }] if reason.contains("guided")),
            "case{case}: {report:?}"
        );
        assert_eq!(world.resource::<crate::sim_tick::SimTick>().0, 42);
        assert_eq!(world.entities().len(), entity_count);
        assert_eq!(
            world.get::<Transform>(sentinel).unwrap().translation,
            Vec3::new(3.0, 4.0, 5.0)
        );
        assert_eq!(serde_json::to_value(capture(&world)).unwrap(), before);
    }
}

#[test]
fn actual_weapon_capture_application_and_full_flight_digest_continue_ordered_committed_bursts() {
    use crate::console::weapons::TorpedoSystemResource;
    use crate::weapons::torpedo::{TorpedoConfig, TorpedoSystem};
    use std::collections::HashMap;
    fn host(config: TorpedoConfig) -> (World, Entity) {
        let mut world = World::new();
        world.register_component::<crate::ship::state::ShipPhysics>();
        world.register_component::<crate::entities::spawner::EntitySystemHull>();
        world.register_component::<crate::ship::state::ShipRedAlert>();
        let entity = world
            .spawn((
                crate::entities::spawner::EntityUuid("same-launcher".into()),
                TorpedoSystemResource(TorpedoSystem::new(config)),
            ))
            .id();
        (world, entity)
    }
    fn state(world: &World, entity: Entity) -> WeaponState {
        super::super::capture::weapon_state(
            None,
            None,
            world.get::<TorpedoSystemResource>(entity),
            None,
            None,
            None,
        )
    }
    let config = TorpedoConfig {
        count: 6,
        lifespan: 90.0,
        speed: 8.0,
        durability: 6.0,
        load_time: 0.1,
        ..Default::default()
    };
    let (mut live, source) = host(config.clone());
    {
        let mut provider = live.get_mut::<TorpedoSystemResource>(source).unwrap();
        let system = &mut provider.0;
        let tube = system.tubes[0].id.clone();
        system.tubes[0].volley_max = 3;
        system.set_volley_target(&tube, 3);
        assert!(system.start_load(&tube));
        for _ in 0..3 {
            system.tick(0.1, &HashMap::new(), &mut || unreachable!());
        }
        assert!(matches!(
            system.launch(
                &tube,
                "initial".into(),
                0.0,
                0.0,
                0.0,
                0.0,
                Some("target".into()),
                Some("same-launcher".into())
            ),
            crate::weapons::torpedo::LaunchResult::Launched { .. }
        ));
        system.set_volley_target(&tube, 0);
        let second = system.tubes[1].id.clone();
        system.tubes[1].volley_max = 2;
        system.set_volley_target(&second, 2);
        assert!(system.start_load(&second));
        for _ in 0..2 {
            system.tick(0.1, &HashMap::new(), &mut || unreachable!());
        }
        assert!(matches!(
            system.launch(
                &second,
                "other-initial".into(),
                2.0,
                0.0,
                0.0,
                0.0,
                Some("target".into()),
                Some("same-launcher".into())
            ),
            crate::weapons::torpedo::LaunchResult::Launched { .. }
        ));
        system.set_volley_target(&second, 0);
        system.validate_magazine().unwrap();
    }
    let saved: WeaponState =
        serde_json::from_slice(&serde_json::to_vec(&state(&live, source)).unwrap()).unwrap();
    validate_weapon_state(&saved).unwrap();
    assert_eq!(saved.torpedoes_in_flight.len(), 2);
    assert_eq!(saved.bursts.len(), 2);
    assert_eq!(saved.bursts[0].pending, 2);
    let (mut resumed, receiver) = host(TorpedoConfig {
        speed: 80.0,
        lifespan: 900.0,
        damage_hull: 500,
        burst_interval_secs: 9.0,
        ..config
    });
    resumed
        .get_mut::<TorpedoSystemResource>(receiver)
        .unwrap()
        .0
        .tubes[0]
        .volley_max = 3;
    resumed
        .get_mut::<TorpedoSystemResource>(receiver)
        .unwrap()
        .0
        .tubes[1]
        .volley_max = 2;
    super::super::application::apply_weapons(&mut resumed.entity_mut(receiver), &saved);
    let expected = crate::sim_digest::world_digest(&resumed);
    resumed
        .get_mut::<TorpedoSystemResource>(receiver)
        .unwrap()
        .0
        .in_flight
        .swap(0, 1);
    assert_ne!(crate::sim_digest::world_digest(&resumed), expected);
    resumed
        .get_mut::<TorpedoSystemResource>(receiver)
        .unwrap()
        .0
        .in_flight
        .swap(0, 1);
    resumed
        .get_mut::<TorpedoSystemResource>(receiver)
        .unwrap()
        .0
        .burst_states
        .swap(0, 1);
    assert_ne!(crate::sim_digest::world_digest(&resumed), expected);
    resumed
        .get_mut::<TorpedoSystemResource>(receiver)
        .unwrap()
        .0
        .burst_states
        .swap(0, 1);
    resumed
        .get::<TorpedoSystemResource>(receiver)
        .unwrap()
        .0
        .validate_magazine()
        .unwrap();
    let positions = HashMap::from([("target".into(), (50.0, 30.0, -100.0))]);
    for step in 0..360 {
        assert_eq!(
            state(&live, source),
            state(&resumed, receiver),
            "step{step}"
        );
        assert_eq!(
            crate::sim_digest::world_digest(&live),
            crate::sim_digest::world_digest(&resumed),
            "step{step}"
        );
        for (world, entity) in [(&mut live, source), (&mut resumed, receiver)] {
            let mut provider = world.get_mut::<TorpedoSystemResource>(entity).unwrap();
            let mut id = 0;
            provider.0.tick(1.0 / 60.0, &positions, &mut || {
                id += 1;
                format!("round-{step}-{id}")
            });
        }
    }
    let final_state = state(&live, source);
    assert!(final_state.bursts.is_empty());
    assert_eq!(final_state.torpedoes_in_flight.len(), 5);
    assert!(final_state
        .torpedoes_in_flight
        .iter()
        .all(|round| round.position[1] > 0.0));
}

#[test]
fn saved_burst_ammunition_and_actual_tube_ownership_refuse_before_any_live_write() {
    use crate::console::weapons::TorpedoSystemResource;
    use crate::weapons::torpedo::{TorpedoConfig, TorpedoSystem};
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(42));
    world.insert_resource(Time::<Fixed>::default());
    world.register_component::<crate::ship::state::ShipPhysics>();
    world.register_component::<crate::entities::spawner::EntitySystemHull>();
    world.register_component::<crate::ship::state::ShipRedAlert>();
    let mut system = TorpedoSystem::new(TorpedoConfig {
        count: 6,
        ..Default::default()
    });
    system.tubes[0].volley_max = 3;
    let entity = world
        .spawn((
            crate::entities::spawner::EntityUuid("same-launcher".into()),
            TorpedoSystemResource(system),
        ))
        .id();
    let weapons = super::super::capture::weapon_state(
        None,
        None,
        world.get::<TorpedoSystemResource>(entity),
        None,
        None,
        None,
    );
    let before = serde_json::to_value(capture(&world)).unwrap();
    let before_digest = crate::sim_digest::world_digest(&world);
    for case in 0..3 {
        let mut saved = weapons.clone();
        saved.bursts = vec![BurstState {
            tube_id: "fore_port".into(),
            pending: 2,
            timer: 0.3,
            profile: profile(),
            burst_interval_secs: 0.3,
            next_shot_index: 1,
            ..Default::default()
        }];
        // These are valid shape records. Their actual provider must refuse
        // capacity creation, an absent tube, and an oversized tube commitment.
        match case {
            0 => {} // The real full magazine plus committed rounds exceeds six.
            1 => {
                saved.magazine.as_mut().unwrap().level = 0;
                saved.bursts[0].tube_id = "unknown-tube".into();
            }
            2 => {
                saved.magazine.as_mut().unwrap().level = 0;
                saved.bursts[0].pending = 3;
            }
            _ => unreachable!(),
        }
        validate_weapon_state(&saved).unwrap();
        let snapshot = PhoenixSnapshot {
            tick: 999,
            entities: vec![EntityState {
                uuid: "same-launcher".into(),
                weapons: Some(saved),
                ..Default::default()
            }],
            ..Default::default()
        };
        let report = restore(&mut world, &snapshot);
        let expected = [
            "ammunition capacity",
            "unknown tube",
            "committed burst ammunition",
        ][case];
        assert!(report.gaps.iter().any(|gap| matches!(gap, RestoreGap::ResourceStateInvalid {reason,..} if reason.contains(expected))), "case{case}: {report:?}");
        assert_eq!(world.resource::<crate::sim_tick::SimTick>().0, 42);
        assert_eq!(serde_json::to_value(capture(&world)).unwrap(), before);
        assert_eq!(crate::sim_digest::world_digest(&world), before_digest);
    }
}
