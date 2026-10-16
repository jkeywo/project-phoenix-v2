use super::*;

fn measured() -> (HelmAiShipFrame, uuid::Uuid, uuid::Uuid) {
    let actual = uuid::Uuid::from_u128(123);
    let public = uuid::Uuid::from_u128(456);
    let view = crate::ai::WorldView {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: public,
            position: [10.0, 0.0, 0.0],
            ..Default::default()
        }],
        ..Default::default()
    };
    (
        HelmAiShipFrame {
            visible_view: view.clone(),
            merged_view: view,
            weapons_target: Some(public),
            current_contact_routes: Some(std::collections::BTreeMap::from([(actual, public)])),
            ..Default::default()
        },
        actual,
        public,
    )
}

#[test]
fn own_weapon_arc_assistance_uses_current_measured_pose_and_keeps_actual_request_identity() {
    let (frame, actual, public) = measured();
    let mut pending = PendingArcBearingRequest {
        target: Some(actual),
        arcs: vec![crate::core::messages::WeaponEmitterArc {
            facing_deg: 0.0,
            arc_deg: 30.0,
            range: 100.0,
        }],
    };
    let mut steering = 0.0;
    apply_frame_arc_bearing_request(
        &mut steering,
        Some(&mut pending),
        &frame,
        &ShipPhysics::default(),
        true,
    );
    assert!(
        steering > 0.0,
        "known return to starboard should request a turn"
    );
    assert_eq!(pending.target, Some(actual));
    assert_ne!(pending.target, Some(public));
    let mut lost = frame.clone();
    lost.current_contact_routes.as_mut().unwrap().clear();
    steering = 0.0;
    apply_frame_arc_bearing_request(
        &mut steering,
        Some(&mut pending),
        &lost,
        &ShipPhysics::default(),
        true,
    );
    assert_eq!(steering, 0.0);
    assert_eq!(
        pending.target, None,
        "lost route cannot refresh from a real actor"
    );
}

#[test]
fn unknown_foreign_motion_reach_and_arcs_do_not_become_known_zero_facts() {
    let (frame, _, public) = measured();
    let mut facts = crate::world::flags::AiFacts::new();
    assert_eq!(
        seed_helm_travel_facts(&mut facts, Some(&frame), &ShipPhysics::default(), 10.0),
        Some(public)
    );
    assert_eq!(facts.get(RANGE_TO_TARGET_FACT), Some(10.0));
    assert!(facts.get(BEARING_TO_TARGET_FACT).is_some());
    for unknown in [
        CLOSING_RATE_FACT,
        TARGET_DIRECT_FIRE_RANGE_FACT,
        HOSTILE_ARC_EXPOSURE_FACT,
        HOSTILE_ARC_ESCAPE_DEG_FACT,
        HOSTILE_ARC_INESCAPABLE_FACT,
    ] {
        assert_eq!(
            facts.get(unknown),
            None,
            "{unknown} is not an observed zero"
        );
    }
    let mut legacy = frame.clone();
    legacy.current_contact_routes = None;
    let mut known = crate::world::flags::AiFacts::new();
    seed_helm_travel_facts(&mut known, Some(&legacy), &ShipPhysics::default(), 10.0);
    assert_eq!(
        known.get(TARGET_DIRECT_FIRE_RANGE_FACT),
        Some(0.0),
        "legacy equipment keeps its existing input semantics"
    );
}

#[test]
fn actual_observed_velocity_seeds_closing_rate_without_inventing_hull_heading() {
    let (mut frame, _, public) = measured();
    frame.merged_view.entities[0].position = [0.0, 0.0, -10.0];
    frame.observed_velocities.insert(public, [0.0, -3.0]);
    let physics = ShipPhysics {
        forward_speed: 5.0,
        ..Default::default()
    };
    let mut facts = crate::world::flags::AiFacts::new();
    seed_helm_travel_facts(&mut facts, Some(&frame), &physics, 10.0);
    assert_eq!(facts.get(CLOSING_RATE_FACT), Some(2.0));
    assert_eq!(frame.merged_view.entities[0].yaw, None);
}

#[test]
fn unknown_enemy_reach_cannot_define_a_safe_ring_or_outside_threat_verdict() {
    let (frame, _, public) = measured();
    let mut facts = crate::world::flags::AiFacts::new();
    seed_helm_travel_facts(&mut facts, Some(&frame), &ShipPhysics::default(), 10.0);
    let mut params = crate::world::flags::AiParams::default();
    params.set(SAFE_RANGE_MARGIN_PARAM, 20.0);
    let mut history = HelmRecoveryHistory::default();
    assert_eq!(
        seed_recovery_facts(&mut facts, &params, Some(0.5), &mut history, Some(public)),
        None
    );
    seed_pressed_facts(&mut facts, &params, &mut history);
    assert_eq!(
        facts.get(SHIELD_FRACTION_FACT),
        Some(0.5),
        "own shield measurement remains available"
    );
    for unknown in [
        SAFE_RANGE_FACT,
        SAFE_DISTANCE_HELD_FACT,
        INSIDE_THREAT_RANGE_FACT,
    ] {
        assert_eq!(
            facts.get(unknown),
            None,
            "{unknown} cannot use an absent reach as zero"
        );
    }
}

#[test]
fn authored_unknown_reach_ring_does_not_invent_threat_information() {
    let (frame, _, public) = measured();
    let mut params = crate::world::flags::AiParams::default();
    params.set(SAFE_RANGE_MARGIN_PARAM, 20.0);
    params.set(UNKNOWN_TARGET_SAFE_RANGE_PARAM, 120.0);
    params.set(SAFE_RING_TOLERANCE_PARAM, 1.0);
    params.set(SAFE_DISTANCE_WINDOW_TICKS_PARAM, 1.0);
    for (observed_reach, valid, expected) in [
        (None, true, Some(120.0)),
        (Some(40.0), true, Some(60.0)),
        (Some(0.0), true, Some(20.0)),
        (None, false, None),
    ] {
        let mut facts = crate::world::flags::AiFacts::new();
        seed_helm_travel_facts(&mut facts, Some(&frame), &ShipPhysics::default(), 10.0);
        facts.set(TARGET_VALID_FACT, if valid { 1.0 } else { 0.0 });
        facts.set(RANGE_TO_TARGET_FACT, 130.0);
        if let Some(reach) = observed_reach {
            facts.set(TARGET_DIRECT_FIRE_RANGE_FACT, reach);
        }
        let mut history = HelmRecoveryHistory::default();
        assert_eq!(
            seed_recovery_facts(&mut facts, &params, Some(0.5), &mut history, Some(public)),
            expected
        );
        assert_eq!(facts.get(TARGET_DIRECT_FIRE_RANGE_FACT), observed_reach);
        seed_pressed_facts(&mut facts, &params, &mut history);
        if valid && observed_reach.is_none() {
            assert_eq!(facts.get(INSIDE_THREAT_RANGE_FACT), None);
            assert_eq!(facts.get(SAFE_DISTANCE_HELD_FACT), Some(1.0));
        }
    }
}

#[test]
fn optional_torpedo_requirement_handles_unknown_shields_on_both_axes() {
    let authored: toml::Value = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/entities/fragments/ai/movement_broadside_orbit.toml"
    )))
    .unwrap();
    for axis in ["engines_ai", "steering_ai"] {
        let config: crate::entities::config::FineSystemAiConfigToml =
            authored["helm_console"][axis].clone().try_into().unwrap();
        for (threshold, observed, should_enter) in [
            (0.0, None, true),
            (0.0, Some(0.0), true),
            (0.0, Some(1.0), true),
            (1.0, None, false),
            (1.0, Some(0.0), false),
            (1.0, Some(1.0), true),
        ] {
            let mut policy = config.to_policy().unwrap();
            policy.params.set("torpedo_run_shield_gap", threshold);
            let mut facts = crate::world::flags::AiFacts::new();
            facts.set("posture", policy.params.get("press_posture").unwrap());
            facts.set(TARGET_VALID_FACT, 1.0);
            facts.set(
                RANGE_TO_TARGET_FACT,
                policy.params.get("torpedo_run_range").unwrap(),
            );
            facts.set("tubes_full", 1.0);
            facts.set("tubes_fillable", 1.0);
            facts.set("torpedoes_in_flight", 0.0);
            if let Some(shield) = observed {
                facts.set(TARGET_FACING_SHIELD_DOWN_FACT, shield);
            }
            let mut state = crate::ai::policy::AiPolicyRuntimeState::reset(&policy, 0.0);
            state.enter("orbit", 0.0);
            let target = Some(uuid::Uuid::from_u128(456));
            tick_policy_machine(
                &policy,
                &mut state,
                true,
                &facts,
                target,
                1.0,
                0.0,
                &[],
                |_| {},
            );
            assert_eq!(
                state.current == "torpedo_run",
                should_enter,
                "{axis}: {threshold}, {observed:?}"
            );
            assert_eq!(facts.get(TARGET_FACING_SHIELD_DOWN_FACT), observed);
            if threshold == 0.0 {
                // A loaded run with nothing in flight must not abort simply
                // because an optional shield finding is absent or still up.
                tick_policy_machine(
                    &policy,
                    &mut state,
                    true,
                    &facts,
                    target,
                    2.0,
                    0.0,
                    &[],
                    |_| {},
                );
                assert_eq!(state.current, "torpedo_run");
            }
        }
    }
}
