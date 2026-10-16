//! The admitted request owner for explicitly fitted movement controls.
//! Observations enter only at this Console's measured Match Speed boundary;
//! the existing paid integrator remains the sole physical actuator.
use crate::core::messages::{AdmittedCommands, SystemControlPayload, SystemId};
use crate::entities::spawner::EntityUuid;
use crate::science::broad::BroadTracks;
use crate::ship::components::{
    ShipConfigComponent, ShipPhysicsConfigResource, ShipSystemControlSources,
};
use crate::ship::helm::{LateralThrustInput, SteeringInput, ThrustInput};
use crate::ship::state::ShipPhysics;
use bevy::prelude::*;
use phoenix_sim_gameplay::ship::helm_control::{
    AxisIntent, CompiledHelmControl, HelmControlState, LateralIntent, SteeringIntent, ThrustIntent,
};

/// A current permitted held velocity, never a physical actor's live velocity.
pub(super) fn observed_speed(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    tracks: &BroadTracks,
    owner: &SystemId,
    contact: &str,
    observer: &str,
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    now: u64,
) -> Option<(f32, u64)> {
    let view = config.observations_for_system(owner, &boards.0);
    let mut candidate = None;
    for (instrument, state) in &tracks.0 {
        let Some(record) = view
            .get(instrument)
            .and_then(crate::science::contact_routing::observations)
            .and_then(|records| records.iter().find(|record| record.contact_id == contact))
        else {
            continue;
        };
        if record.predicted || !record.selectable || record.observed_tick > now {
            continue;
        }
        let Some(source) = state.resolve_current(contact) else {
            continue;
        };
        if runtime.is_some_and(|runtime| {
            crate::gm::operations::contact::mode(&runtime.contact_overrides, observer, source)
                == crate::gm::operations::contact::ContactMode::Conceal
        }) {
            continue;
        }
        let velocity = record.velocity?;
        let speed = (velocity[0] as f64).hypot(velocity[1] as f64);
        if speed.is_finite() && speed <= f32::MAX as f64 {
            let measured = (speed as f32, record.observed_tick);
            if candidate.is_some_and(|held| held != measured) {
                return None;
            }
            candidate = Some(measured);
        }
    }
    candidate
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    knowledge: crate::console::navigation::knowledge::Environment,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    regions: Query<&crate::entities::spawner::RegionEffectsSection>,
    mut ships: Query<(
        &CompiledHelmControl,
        (
            &mut HelmControlState,
            &mut phoenix_sim_gameplay::ship::helm_control::HelmNavigationInputs,
        ),
        &AdmittedCommands,
        &ShipPhysics,
        &ShipPhysicsConfigResource,
        &ShipSystemControlSources,
        &ShipConfigComponent,
        (
            &EntityUuid,
            Entity,
            Has<crate::server_app::LocalShip>,
            Option<&mut crate::ship_plugin::LastHelmInput>,
        ),
        Option<&crate::server_app::ShipSystemBlackboards>,
        Option<&BroadTracks>,
        &mut ThrustInput,
        &mut SteeringInput,
        Option<&mut LateralThrustInput>,
        (
            Option<&crate::ship::helm::ImpulseCommand>,
            Option<&crate::console::navigation::NavigationWaypoint>,
        ),
    )>,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    for (
        profile,
        (mut state, mut navigation_inputs),
        admitted,
        physics,
        tuning,
        sources,
        config,
        (uuid, entity, is_local, mut last),
        boards,
        tracks,
        mut thrust,
        mut steering,
        mut lateral,
        (impulse, waypoint),
    ) in &mut ships
    {
        let profile = &profile.0;
        navigation_inputs.tick = tick.as_ref().map(|tick| tick.0);
        navigation_inputs.axes.clear();
        if let Some(boards) = boards {
            for axis in [&profile.thrust, &profile.steering]
                .into_iter()
                .chain(profile.lateral.iter())
            {
                if let Some(input) =
                    super::navigation::motion(&knowledge, entity, config, boards, axis, waypoint)
                {
                    navigation_inputs.axes.insert(axis.clone(), input);
                }
            }
        }
        // Offline work loses its old stick latch, while mode and actual momentum
        // survive. Repaired Direct axes cannot revive a stale force request.
        for owner in [&profile.thrust, &profile.steering]
            .into_iter()
            .chain(profile.lateral.iter())
        {
            if sources.0.is_offline(owner)
                || sources.0.source_for(owner)
                    == crate::ship::control_source::ControlSource::Offline
                || !sources.0.entries().any(|(id, _)| id == owner)
            {
                match state.axes.get_mut(owner) {
                    Some(AxisIntent::Thrust(intent)) => match intent {
                        ThrustIntent::Direct { force } => *force = 0.0,
                        _ => *intent = ThrustIntent::default(),
                    },
                    Some(AxisIntent::Steering(intent)) => match intent {
                        SteeringIntent::Direct { rate } => *rate = 0.0,
                        _ => *intent = SteeringIntent::default(),
                    },
                    Some(AxisIntent::Lateral(intent)) => match intent {
                        LateralIntent::Direct { force } => *force = 0.0,
                        _ => *intent = LateralIntent::Assisted { value: 0.0 },
                    },
                    None => {}
                }
            }
        }
        let actual = crate::ship::physics::ShipPhysicsState {
            x: physics.x,
            y: physics.y,
            z: physics.z,
            yaw: physics.yaw,
            forward_speed: physics.forward_speed,
            lateral_speed: physics.lateral_speed,
            vertical_speed: physics.vertical_speed,
        };
        let speed_for = |contact: &str| {
            observed_speed(
                &config.0,
                boards?,
                tracks?,
                &profile.thrust,
                contact,
                &uuid.0,
                runtime.as_deref(),
                tick.as_ref()?.0,
            )
        };
        // Existing Match holds its last dated demand when observation is lost.
        // Receipt/publication cadence cannot extend or improve that sample.
        let matched = match state.axes.get(&profile.thrust) {
            Some(AxisIntent::Thrust(ThrustIntent::Assisted {
                demand: phoenix_model::helm_control::SpeedDemand::Match { contact, .. },
            })) => Some(contact.clone()),
            _ => None,
        };
        if let Some(contact) = matched {
            if let Some((speed, observed)) = speed_for(&contact) {
                let _ = state.refresh_match(
                    &profile.thrust,
                    &contact,
                    speed,
                    observed,
                    tick.as_ref().unwrap().0,
                );
            }
        }
        for command in &admitted.0 {
            let owner = &command.target;
            if state.axes.contains_key(owner)
                && (sources.0.is_offline(owner)
                    || sources.0.source_for(owner)
                        == crate::ship::control_source::ControlSource::Offline
                    || !sources.0.entries().any(|(id, _)| id == owner))
            {
                crate::command_admission::finish_action_feedback(
                    command,
                    &mut outbound,
                    crate::core::messages::ActionFeedbackOutcome::Refused,
                );
                continue;
            }
            let outcome = match &command.payload {
                SystemControlPayload::SetFollowNavigation { generation } => Some(
                    if generation.is_some_and(|generation| {
                        !navigation_inputs
                            .axes
                            .get(owner)
                            .is_some_and(|input| input.generation == generation)
                    }) {
                        Err("Following requires this axis's current permitted Navigation course and leader measurement".into())
                    } else {
                        state.follow_navigation(owner, *generation, actual)
                    },
                ),
                SystemControlPayload::SetHelmMode { mode } => {
                    Some(state.set_mode(owner, *mode, actual, &tuning.0))
                }
                SystemControlPayload::SetDesiredSpeed { speed } => {
                    Some(state.set_speed(owner, *speed))
                }
                SystemControlPayload::SetDesiredHeading { heading_rad } => {
                    Some(state.set_heading(owner, *heading_rad))
                }
                SystemControlPayload::SetMatchSpeed { contact } => Some(match contact {
                    None => state.cancel_match(owner).map(|_| ()),
                    Some(contact) if owner == &profile.thrust => match speed_for(contact) {
                        Some((speed, observed)) => state.match_speed(
                            owner,
                            contact,
                            speed,
                            observed,
                            tick.as_ref().unwrap().0,
                        ),
                        None => Err(
                            "Match Speed requires a current permitted velocity observation".into(),
                        ),
                    },
                    Some(_) => Err("Match Speed belongs to the actual thrust owner".into()),
                }),
                SystemControlPayload::SetThrust { value } if owner == &profile.thrust => {
                    Some(state.set_thrust(owner, *value))
                }
                SystemControlPayload::SetSteering { value } if owner == &profile.steering => {
                    Some(state.set_steering(owner, *value))
                }
                SystemControlPayload::LateralThrustInput { lateral }
                    if profile.lateral.as_ref() == Some(owner) =>
                {
                    Some(state.set_lateral(owner, *lateral))
                }
                SystemControlPayload::StartImpulseCharge
                    if impulse.is_some()
                        && config.0.system(owner).is_some_and(|system| {
                            system.kind == crate::ship::system_registry::HELM_IMPULSE_KIND
                        })
                        && !crate::ship::helm_admission::entity_inside_blocks_impulse(
                            entity,
                            &membership,
                            &regions,
                        ) =>
                {
                    let _ = state.set_thrust(&profile.thrust, 0.0);
                    let _ = state.set_steering(&profile.steering, 0.0);
                    None // Existing drive applier owns its one terminal feedback.
                }
                _ => None,
            };
            if let Some(outcome) = outcome {
                crate::command_admission::finish_action_feedback(
                    command,
                    &mut outbound,
                    if outcome.is_ok() {
                        crate::core::messages::ActionFeedbackOutcome::Applied
                    } else {
                        crate::core::messages::ActionFeedbackOutcome::Refused
                    },
                );
            }
        }
        let display = state.stick_display(profile);
        thrust.0 = display[0];
        steering.0 = display[1];
        if is_local {
            if let Some(last) = last.as_deref_mut() {
                last.thrust = display[0];
                last.steering = display[1];
                if profile.lateral.is_some() {
                    last.lateral = display[2];
                }
            }
        }
        if profile.lateral.is_some() {
            if let Some(lateral) = lateral.as_deref_mut() {
                lateral.0 = display[2];
            }
        }
    }
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
