//! Ordinary per-axis policy decisions retain combat and docking precedence.
use super::*;
use crate::{
    ai::{host::HostOutcome, policy::AiPolicyVerb},
    core::messages::{SystemControlPayload, SystemId},
    ship::{helm_control::CompiledHelmControl, helm_planner::ShipMotionPlan},
};

#[test]
fn only_ordinary_travel_verbs_admit_follow_and_existing_bearing_docking_decisions_survive() {
    let movement = CompiledHelmControl(phoenix_model::helm_control::HelmControlConfig {
        thrust: SystemId("drive".into()),
        steering: SystemId("rudder".into()),
        lateral: Some(SystemId("sway".into())),
    });
    let plan = ShipMotionPlan {
        navigation_generation: Some(7),
        ..Default::default()
    };
    let frame = HelmAiShipFrame {
        has_objective: true,
        ..Default::default()
    };
    let physics = ShipPhysics::default();
    let anchors = Default::default();
    let mut cx = HelmAxisCtx {
        movement: Some(&movement),
        physics: Some(&physics),
        max_speed: 20.0,
        plan: Some(&plan),
        frame: Some(&frame),
        anchors: &anchors,
        impulse: None,
        impulse_cfg: None,
        boost_cfg: None,
        boost: None,
        behaviour: None,
        capability: None,
        cursors: None,
    };
    let mut io = HelmAxisIo {
        policy: None,
        state: None,
        pending: None,
    };
    let travel = AiPolicyVerb::ActuateDesiredTravel;
    let facing = AiPolicyVerb::ActuateDesiredFacing;
    let lateral = AiPolicyVerb::ActuateLateralThrust;
    for payload in [
        EnginesAxis::act(HostOutcome::Act(&travel), &cx, &mut io),
        SteeringAxis::act(HostOutcome::Act(&facing), &cx, &mut io),
        LateralAxis::act(HostOutcome::Act(&lateral), &cx, &mut io),
    ] {
        assert!(matches!(
            payload,
            Some(SystemControlPayload::SetFollowNavigation {
                generation: Some(7)
            })
        ));
    }
    for verb in [
        AiPolicyVerb::HoldCommittedHeading,
        AiPolicyVerb::HoldRecoveryOrbit,
        AiPolicyVerb::PivotToReengage,
        AiPolicyVerb::HoldCombatOrbit,
        AiPolicyVerb::HoldTorpedoBearing,
        AiPolicyVerb::HoldArtilleryPosition,
    ] {
        assert!(
            matches!(
                SteeringAxis::act(HostOutcome::Act(&verb), &cx, &mut io),
                Some(SystemControlPayload::SetSteering { .. })
            ),
            "authored doctrine must retain its steering verb"
        );
    }
    let mut bearing = PendingArcBearingRequest {
        target: Some(uuid::Uuid::from_u128(17)),
        ..Default::default()
    };
    io.pending = Some(&mut bearing);
    assert!(matches!(
        SteeringAxis::act(HostOutcome::Act(&facing), &cx, &mut io),
        Some(SystemControlPayload::SetSteering { .. })
    ));
    io.pending = None;
    let dock = ShipMotionPlan {
        docking_active: true,
        ..plan
    };
    cx.plan = Some(&dock);
    assert!(matches!(
        LateralAxis::pre_override(&cx),
        Some(SystemControlPayload::LateralThrustInput { .. })
    ));
    cx.plan = Some(&plan);
    cx.movement = None;
    assert!(matches!(
        EnginesAxis::act(HostOutcome::Act(&travel), &cx, &mut io),
        Some(SystemControlPayload::SetThrust { .. })
    ));
}
