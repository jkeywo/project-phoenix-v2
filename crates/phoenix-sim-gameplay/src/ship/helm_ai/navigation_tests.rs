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

#[test]
fn ordinary_vertical_axis_uses_only_cleared_full3d_chart_plan_and_keeps_bounded_hazard_behavior() {
    use crate::entities::{
        config::{HelmCapabilityConfig, VerticalMovementMode},
        spawner::HelmCapabilitySection,
    };
    let physics = ShipPhysics {
        y: 4.0,
        ..Default::default()
    };
    let frame = HelmAiShipFrame {
        nav_waypoint_xyz: Some([10.25, 18.5, -23.75]),
        ..Default::default()
    };
    let mut plan = ShipMotionPlan {
        navigation_generation: Some(7),
        ..Default::default()
    };
    plan.motion.desired_velocity_local.y = 0.75;
    let mut capability = HelmCapabilitySection(HelmCapabilityConfig {
        vertical_movement_mode: VerticalMovementMode::Full3D,
        ..Default::default()
    });
    let anchors = Default::default();
    let mut cx = HelmAxisCtx {
        movement: None,
        physics: Some(&physics),
        max_speed: 0.0,
        plan: Some(&plan),
        frame: Some(&frame),
        anchors: &anchors,
        impulse: None,
        impulse_cfg: None,
        boost_cfg: None,
        boost: None,
        behaviour: None,
        capability: Some(&capability),
        cursors: None,
    };
    let mut io = HelmAxisIo {
        policy: None,
        state: None,
        pending: None,
    };
    let verb = AiPolicyVerb::ActuateVerticalThrust;
    assert!(
        matches!(VerticalAxis::act(HostOutcome::Act(&verb),&cx,&mut io),Some(SystemControlPayload::VerticalThrustInput{vertical}) if vertical==0.75)
    );
    cx.plan = None;
    assert!(
        matches!(VerticalAxis::act(HostOutcome::Act(&verb),&cx,&mut io),Some(SystemControlPayload::VerticalThrustInput{vertical}) if vertical==0.0)
    );
    cx.plan = Some(&plan);
    cx.capability = None;
    assert!(
        matches!(VerticalAxis::act(HostOutcome::Act(&verb),&cx,&mut io),Some(SystemControlPayload::VerticalThrustInput{vertical}) if vertical==0.0)
    );
    capability.0.vertical_movement_mode = VerticalMovementMode::Bounded;
    cx.capability = Some(&capability);
    assert!(
        matches!(VerticalAxis::act(HostOutcome::Act(&verb),&cx,&mut io),Some(SystemControlPayload::VerticalThrustInput{vertical}) if vertical<0.0)
    );
}
