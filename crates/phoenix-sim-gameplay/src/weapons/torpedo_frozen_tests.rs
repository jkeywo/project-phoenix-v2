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
    let original = system.in_flight[0].profile.clone();
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
    let original = system.in_flight[0].profile.clone();
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
    expected.collision_interval_secs = None;
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

#[test]
fn final_clipped_guided_interval_is_interceptable_before_terminal_payload_settlement() {
    let mut system = loaded(1);
    system.in_flight[0].lifespan_remaining = 0.125;
    let result = system.advance_with_work(
        0.5,
        &HashMap::new(),
        &mut |_| panic!("already emitted round needs no machinery work"),
        &mut || unreachable!(),
    );
    assert!(
        result.expired.is_empty(),
        "Physics cannot settle terminal work before PDC"
    );
    let round = &system.in_flight[0];
    assert_eq!(round.lifespan_remaining, 0.0);
    assert_eq!(
        round.z, -1.0,
        "8 units/sec for only the last 0.125 live seconds"
    );
    assert_eq!(round.collision_interval_secs, Some(0.125));
    assert_eq!(system.apply_interception("first", 6.0).unwrap(), Some(0.0));
    assert!(system
        .find_detonation_hits(&[("target".into(), 0.0, 0.0, -1.0, 1.0)])
        .is_empty());
    assert!(
        system.settle_expired().is_empty(),
        "interception removes the actual payload once"
    );
}

#[test]
fn surviving_expiry_settles_once_after_only_its_actual_live_travel() {
    let mut system = loaded(1);
    system.in_flight[0].lifespan_remaining = 0.125;
    system.advance_with_work(
        1.0,
        &HashMap::new(),
        &mut |_| unreachable!(),
        &mut || unreachable!(),
    );
    assert_eq!(system.in_flight[0].z, -1.0);
    assert_eq!(system.in_flight[0].collision_interval_secs, Some(0.125));
    system.in_flight[0].tick(0.0, None);
    assert_eq!(system.in_flight[0].collision_interval_secs, Some(0.125));
    system.finish_collision_interval();
    assert_eq!(system.in_flight[0].collision_interval_secs, None);
    assert_eq!(system.settle_expired(), ["first"]);
    assert!(system.settle_expired().is_empty());
    assert!(system.handle_collision_full("first").is_none());
}

#[test]
fn accepted_point_cannot_be_replaced_by_another_intent_older_sample_or_unfitted_receiver() {
    let mut system = loaded(1);
    let original = GuidanceObservation {
        target_uuid: "target".into(),
        position: [4.0, 2.0, -20.0],
        sampled_tick: 42,
        source: GuidanceSource::Seeker,
    };
    assert!(system.in_flight[0].accept_guidance(&original));
    for case in 0..3 {
        let mut bad = original.clone();
        match case {
            0 => bad.target_uuid = "different-intent".into(),
            1 => bad.sampled_tick = 41,
            2 => {
                bad.source = GuidanceSource::Console {
                    ship_uuid: "source".into(),
                    console: phoenix_model::identity::ConsoleId("science".into()),
                    system: phoenix_model::identity::SystemId("illumination".into()),
                    samples: Vec::new(),
                    observation: None,
                }
            }
            _ => unreachable!(),
        }
        assert!(!system.in_flight[0].accept_guidance(&bad));
        assert_eq!(system.in_flight[0].last_guidance.as_ref(), Some(&original));
    }
}

#[test]
fn actual_receiver_retains_an_original_sample_with_later_forwarded_receipts() {
    let mut system = loaded(1);
    system.in_flight[0].profile.receiver =
        Some(phoenix_model::guided_guidance::GuidanceReceiverProfile {
            range: 100.0,
            minimum_transmission: 0.1,
            maximum_sample_age_secs: 2.0,
            accept_external_illumination: false,
        });
    let received = GuidanceObservation {
        target_uuid: "target".into(),
        position: [4.0, 2.0, -20.0],
        sampled_tick: 17,
        source: GuidanceSource::Console {
            ship_uuid: "source".into(),
            console: phoenix_model::identity::ConsoleId("science".into()),
            system: phoenix_model::identity::SystemId("illumination".into()),
            samples: vec![phoenix_model::sensing_sharing::RadarSampleProvenance {
                origin: phoenix_model::sensing_sharing::RadarSampleOrigin {
                    source: "radar-origin-0123456789abcdef".into(),
                    system: phoenix_model::identity::SystemId("broad".into()),
                    contact: "5:broad:contact-1".into(),
                },
                sample_id: "29:radar-origin-0123456789abcdef5:broad17:5:broad:contact-1:17".into(),
                observed_tick: 17,
                observer_position: Some([1.0, -2.0]),
                velocity_error: 0.4,
                drift_error_per_sec: 0.1,
                received_tick: Some(22),
            }],
            observation: Some(phoenix_model::observation::ObservationMetadata {
                observed_tick: Some(17),
                received_tick: Some(23),
                ..Default::default()
            }),
        },
    };
    received.validate(24).unwrap();
    assert!(received.validate(21).is_err());
    assert!(system.in_flight[0].accept_guidance(&received));
    assert_eq!(system.in_flight[0].last_guidance.as_ref(), Some(&received));
    system.in_flight[0].tick(0.1, None);
    assert_eq!(system.in_flight[0].last_guidance.as_ref(), Some(&received));
}

#[test]
fn delayed_burst_retains_the_first_actual_emitted_identity_after_round_retargeting() {
    let mut system = loaded(3);
    assert_eq!(system.burst_states[0].launch_uuid, "first");
    system.in_flight[0].target_uuid = Some("different-intent".into());
    let result =
        system.advance_with_work(0.2, &HashMap::new(), &mut |_| 1.0, &mut || "later".into());
    assert_eq!(result.burst_launched.len(), 1);
    assert_eq!(system.burst_states[0].launch_uuid, "first");
    assert_eq!(
        system
            .in_flight
            .iter()
            .find(|round| round.uuid == "later")
            .unwrap()
            .target_uuid
            .as_deref(),
        Some("target")
    );
}

#[test]
fn received_retarget_and_stop_change_only_existing_guidance_intent() {
    let mut system = loaded(1);
    let round = &mut system.in_flight[0];
    let mut reading = GuidanceObservation {
        target_uuid: "new-intent".into(),
        position: [5.0, 3.0, -30.0],
        sampled_tick: 17,
        source: GuidanceSource::Console {
            ship_uuid: "launcher".into(),
            console: phoenix_model::identity::ConsoleId("tactical".into()),
            system: phoenix_model::identity::SystemId("torpedo-tube-fore-port".into()),
            samples: Vec::new(),
            observation: Some(phoenix_model::observation::ObservationMetadata {
                observed_tick: Some(17),
                received_tick: Some(18),
                ..Default::default()
            }),
        },
    };
    reading.validate(20).unwrap();
    let unfitted = round.clone();
    assert!(!round.receive_guidance(Some(&reading)));
    assert!(!round.receive_guidance(None));
    assert_eq!(*round, unfitted);
    round.profile.receiver = Some(phoenix_model::guided_guidance::GuidanceReceiverProfile {
        range: 100.0,
        minimum_transmission: 0.1,
        maximum_sample_age_secs: 2.0,
        accept_external_illumination: false,
    });
    let before = round.clone();
    assert!(round.receive_guidance(Some(&reading)));
    assert_eq!(round.target_uuid.as_deref(), Some("new-intent"));
    assert_eq!(round.last_guidance.as_ref(), Some(&reading));
    assert!(round.receive_guidance(None));
    let mut expected = before.clone();
    expected.target_uuid = None;
    expected.last_guidance = None;
    assert_eq!(
        *round, expected,
        "stop preserves identity, physical pose, endurance, HP and payload"
    );
    reading.source = GuidanceSource::Seeker;
    assert!(
        !round.receive_guidance(Some(&reading)),
        "a seeker cannot retarget itself through the remote seam"
    );
    assert_eq!(*round, expected);
}

#[test]
fn external_illumination_opt_in_improves_only_existing_intent_without_own_seeker() {
    let mut system = loaded(1);
    let round = &mut system.in_flight[0];
    round.profile.seeker = None;
    round.profile.receiver = Some(phoenix_model::guided_guidance::GuidanceReceiverProfile {
        range: 200.0,
        minimum_transmission: 0.2,
        maximum_sample_age_secs: 2.0,
        accept_external_illumination: true,
    });
    let reading = GuidanceObservation {
        target_uuid: "target".into(),
        position: [15.0, 5.0, -70.0],
        sampled_tick: 40,
        source: GuidanceSource::Console {
            ship_uuid: "source".into(),
            console: phoenix_model::identity::ConsoleId("science".into()),
            system: phoenix_model::identity::SystemId("science-light".into()),
            samples: Vec::new(),
            observation: Some(phoenix_model::observation::ObservationMetadata {
                observed_tick: Some(40),
                ..Default::default()
            }),
        },
    };
    reading.validate(40).unwrap();
    let before = round.clone();
    assert!(round.assist_guidance(&reading));
    let mut expected = before;
    expected.last_guidance = Some(reading.clone());
    assert_eq!(round, &expected);
    let mut different = reading.clone();
    different.target_uuid = "other-intent".into();
    assert!(!round.assist_guidance(&different));
    assert_eq!(round, &expected);
    round
        .profile
        .receiver
        .as_mut()
        .unwrap()
        .accept_external_illumination = false;
    let refused = round.clone();
    assert!(!round.assist_guidance(&reading));
    assert_eq!(round, &refused);
    round
        .profile
        .receiver
        .as_mut()
        .unwrap()
        .accept_external_illumination = true;
    round.tick(0.1, None);
    assert!(
        round.heading > 0.0 && round.pitch > 0.0,
        "held external point provides real steering beyond absent own seeker"
    );
    assert!(round.receive_guidance(None));
    let stopped = round.clone();
    assert!(!round.assist_guidance(&reading));
    assert_eq!(round, &stopped);
}

#[test]
fn marker_launch_freezes_real_faction_recipe_and_remains_nonproximity_finite_ammunition() {
    let mut system = TorpedoSystem::new(TorpedoConfig {
        count: 3,
        load_time: 0.1,
        burst_interval_secs: 0.1,
        ..Default::default()
    });
    let tube = system.tubes[0].id.clone();
    system.tubes[0].marking = Some(phoenix_model::marking::MarkingPayloadConfig {
        template: "assets/entities/marking_beacon.toml".into(),
    });
    system.tubes[0].volley_max = 2;
    system.set_volley_target(&tube, 2);
    system.tick(0.1, &HashMap::new(), &mut || {
        panic!("loading cannot mint a round")
    });
    system.tick(0.1, &HashMap::new(), &mut || {
        panic!("loading cannot mint a round")
    });
    system.tick(0.1, &HashMap::new(), &mut || {
        panic!("loading cannot mint a round")
    });
    assert_eq!(system.tubes[0].loaded_count, 2);
    let aboard = system.rounds_aboard();
    assert_eq!(
        system.launch_with_barrels(
            &tube,
            "refused".into(),
            &[],
            0.0,
            0.0,
            0.0,
            0.0,
            None,
            Some("source".into())
        ),
        LaunchResult::MissingMarkingIdentity
    );
    assert_eq!(system.rounds_aboard(), aboard);
    assert_eq!(system.tubes[0].loaded_count, 2);
    assert!(system.in_flight.is_empty() && system.burst_states.is_empty());
    let faction = uuid::Uuid::from_u128(17);
    assert!(matches!(
        system.launch_with_barrels_faction(
            &tube,
            "marker-0".into(),
            &[],
            0.0,
            0.0,
            0.0,
            0.0,
            None,
            Some("source".into()),
            Some(faction)
        ),
        LaunchResult::Launched { .. }
    ));
    let frozen = system.in_flight[0].profile.clone();
    frozen.validate().unwrap();
    assert_eq!(frozen.marking.as_ref().unwrap().faction, faction);
    assert_eq!(system.burst_states[0].profile, frozen);
    assert_eq!(system.rounds_aboard() + system.in_flight.len() as u32, 3);
    // Existing coarse target radius must never consume mark-only payloads.
    assert!(system
        .find_detonation_hits(&[("hull".into(), 0.0, 0.0, 0.0, 100.0)])
        .is_empty());
    system.tubes[0].marking = None;
    system.config.damage_hull = 999;
    system.config.detonation_radius = 100.0;
    system.set_volley_target(&tube, 0);
    system.tick(0.1, &HashMap::new(), &mut || "marker-1".into());
    assert_eq!(system.in_flight.len(), 2);
    assert!(system.burst_states.is_empty());
    assert!(system.in_flight.iter().all(|round| round.profile == frozen));
    assert_eq!(system.rounds_aboard() + system.in_flight.len() as u32, 3);
    let delivery = system.handle_collision_full("marker-0").unwrap();
    assert_eq!(delivery.marking, frozen.marking);
    assert_eq!((delivery.damage_hull, delivery.damage_shields), (0, 0));
    assert_eq!(
        system
            .apply_interception("marker-1", frozen.durability)
            .unwrap(),
        Some(0.0)
    );
    assert!(system.in_flight.is_empty());
    assert!(system.handle_collision_full("marker-1").is_none());
    assert_eq!(system.rounds_aboard(), 1);
}
