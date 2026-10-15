//! Admitted local postings; existing Repair jobs remain the movement/work owner.
use crate::{
    core::messages::{AdmittedCommands, SystemControlPayload, SystemControlPayloadDiscriminants},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_sim_gameplay::personnel::{CompiledPersonnel, PersonnelRoster, PersonnelSource};

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
    app.declare_state::<CompiledPersonnel>(StateClass::Derived, "accepted-personnel-roster")
        .declare_state::<PersonnelRoster>(StateClass::Folded, "retained-officer-roster")
        .init_resource::<PersonnelRoster>()
        .declare_state::<super::personnel_transfer::PersonnelTransferContext>(
            StateClass::Derived,
            "actual-personnel-transfer-context",
        )
        .init_resource::<super::personnel_transfer::PersonnelTransferContext>();
    register_hooks(app.world_mut());
    for payloads in [
        &[SystemControlPayloadDiscriminants::SetDutyOfficerPosting]
            as &'static [SystemControlPayloadDiscriminants],
        &[SystemControlPayloadDiscriminants::TreatDutyOfficer]
            as &'static [SystemControlPayloadDiscriminants],
        &[SystemControlPayloadDiscriminants::SetDutyOfficerDestination]
            as &'static [SystemControlPayloadDiscriminants],
        &[SystemControlPayloadDiscriminants::DispatchOfficerTransfer]
            as &'static [SystemControlPayloadDiscriminants],
    ] {
        let payload = payloads[0];
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(crate::ship::system_registry::REPAIR_KIND, payload)
                .with_feedback(
                    crate::command_admission::FeedbackAddress::DeclaredKindOnly,
                    payloads,
                ),
        );
    }
    app.add_systems(
        FixedUpdate,
        handle
            .in_set(crate::sim_sets::SimSet::Physics)
            .in_set(crate::sim_sets::FixedStep::HandleDutyOfficerPosting)
            .after(super::server::operate_repair_ai)
            .after(super::dispatch::handle_set_repair_target_priority)
            .before(super::server::tick_repair_teams),
    );
}

/// One existing fixed-step owner. Refresh derived readiness before the same
/// admitted command consumer; neither operation is scheduled a second time.
fn handle(world: &mut World) {
    super::personnel_transfer::prepare(world);
    world
        .run_system_cached(apply)
        .expect("accepted personnel consumer must have its registered world inputs");
}

pub(crate) fn register_hooks(world: &mut World) {
    world
        .register_component_hooks::<CompiledPersonnel>()
        .on_despawn(origin_removed);
    world
        .register_component_hooks::<crate::entities::craft::ProbeCustody>()
        .on_despawn(origin_removed);
    world
        .register_component_hooks::<crate::entities::spawner::EntitySpawnOrigin>()
        .on_insert(origin_source_inserted);
}

#[cfg(test)]
#[path = "personnel_tests.rs"]
mod tests;

pub(crate) fn apply(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut roster: Option<ResMut<PersonnelRoster>>,
    context: Option<Res<super::personnel_transfer::PersonnelTransferContext>>,
    mut ships: Query<
        (
            &AdmittedCommands,
            &ShipConfigComponent,
            &ShipSystemControlSources,
            Option<&mut super::server::ShipRepairTeams>,
            Option<&CompiledPersonnel>,
            &crate::entities::spawner::EntityUuid,
            Option<&mut super::server::RepairRequestQueue>,
            Option<&crate::server_app::ShipSystemBlackboards>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    for (admitted, config, sources, mut teams, profile, actor, mut queue, boards) in &mut ships {
        for command in &admitted.0 {
            if !matches!(
                &command.payload,
                SystemControlPayload::SetDutyOfficerPosting { .. }
                    | SystemControlPayload::TreatDutyOfficer { .. }
                    | SystemControlPayload::SetDutyOfficerDestination { .. }
                    | SystemControlPayload::DispatchOfficerTransfer { .. }
            ) {
                continue;
            }
            if config
                .0
                .system(&command.target)
                .is_none_or(|system| system.kind != crate::ship::system_registry::REPAIR_KIND)
            {
                continue;
            }
            let result = (|| {
                use phoenix_model::personnel::{
                    OfficerLocation, PersonnelDestination, PersonnelDestinationRequest,
                };
                let teams = teams.as_deref_mut().ok_or("missing actual Repair teams")?;
                if &command.target != teams.0.owner()
                    || !sources.0.entries().any(|(id, _)| id == &command.target)
                    || sources.0.is_offline(&command.target)
                    || sources.0.source_for(&command.target)
                        == crate::ship::control_source::ControlSource::Offline
                {
                    return Err("actual Repair owner is unavailable");
                }
                let profile = profile.ok_or("no accepted officer roster")?;
                let roster = roster
                    .as_deref_mut()
                    .ok_or("missing actual officer roster")?;
                let mut state = roster
                    .state(&actor.0)
                    .cloned()
                    .ok_or("missing actual officer custody")?;
                let layout = teams.0.layout().ok_or("missing actual team navigation")?;
                let tick = tick
                    .as_deref()
                    .ok_or("missing actual logical posting clock")?
                    .0;
                match &command.payload {
                    SystemControlPayload::SetDutyOfficerPosting { officer, team_idx } => {
                        state
                            .post(
                                profile,
                                teams.0.job_state(),
                                layout,
                                officer,
                                *team_idx,
                                tick,
                            )
                            .map_err(|_| "posting is not physically available")?;
                        *roster
                            .state_mut(&actor.0)
                            .ok_or("missing actual officer custody")? = state;
                    }
                    SystemControlPayload::TreatDutyOfficer { officer, team_idx } => {
                        state
                            .validate(profile, teams.0.job_state(), tick)
                            .map_err(|_| "invalid actual personnel state")?;
                        teams
                            .0
                            .claim_treatment(profile, &state, officer, *team_idx as usize)
                            .ok_or("treatment is not physically available")?;
                    }
                    SystemControlPayload::SetDutyOfficerDestination {
                        officer,
                        destination,
                    } => {
                        let person = state
                            .officers
                            .get(officer)
                            .ok_or("unknown actual officer")?;
                        let queue = queue
                            .as_deref_mut()
                            .ok_or("missing actual Repair request owner")?;
                        if let Some(destination) = destination {
                            let context = context
                                .as_deref()
                                .filter(|context| context.tick == Some(tick))
                                .ok_or("missing current actual carried-craft context")?;
                            if matches!(person.location, OfficerLocation::Lost { .. }) {
                                return Err("officer is already committed or unavailable");
                            }
                            let permitted = boards
                                .map(|boards| {
                                    super::personnel_transfer::permitted_carried(&config.0, boards)
                                })
                                .unwrap_or_default();
                            let craft = match destination {
                                PersonnelDestination::Craft { craft } => Some(craft),
                                PersonnelDestination::Home => match &person.location {
                                    OfficerLocation::AboardCraft { craft, .. } => Some(craft),
                                    _ => None,
                                },
                            };
                            if let Some(craft) = craft {
                                if !permitted.contains(craft)
                                    || context.pod(&actor.0, craft, tick).is_none()
                                {
                                    return Err(
                                        "destination has no permitted actual carried craft",
                                    );
                                }
                            }
                            if matches!(destination, PersonnelDestination::Home)
                                && matches!(&person.location, OfficerLocation::AtCompartment { .. })
                            {
                                queue.personnel_destinations.remove(officer);
                            } else if queue
                                .personnel_destinations
                                .get(officer)
                                .is_none_or(|request| request.destination != *destination)
                            {
                                queue.personnel_destinations.insert(
                                    officer.clone(),
                                    PersonnelDestinationRequest {
                                        destination: destination.clone(),
                                        requested_tick: tick,
                                    },
                                );
                            }
                        } else {
                            queue.personnel_destinations.remove(officer);
                        }
                        let recall:Vec<_>=teams.0.job_state().teams.iter().enumerate().filter_map(|(team,entry)| {
                            let job=entry.as_ref().filter(|job|job.outcome.is_none())?;
                            let phoenix_sim_gameplay::jobs::repair::RepairActivity::PersonnelTransfer {work,..}=&job.activity else{return None;};
                            (work.officer==*officer && !work.completed && queue.personnel_destinations.get(officer)!=Some(&work.request)).then_some(team)
                        }).collect();
                        for team in recall {
                            teams.0.recall(team);
                        }
                    }
                    SystemControlPayload::DispatchOfficerTransfer {
                        officer,
                        team_idx,
                        request: expected,
                    } => {
                        state
                            .validate(profile, teams.0.job_state(), tick)
                            .map_err(|_| "invalid actual officer custody")?;
                        let request = queue
                            .as_deref()
                            .and_then(|queue| queue.personnel_destinations.get(officer))
                            .cloned()
                            .ok_or("no actual destination request")?;
                        if &request != expected {
                            return Err("destination request changed since dispatch was selected");
                        }
                        let context = context
                            .as_deref()
                            .ok_or("missing actual carried-craft context")?;
                        let permitted = boards
                            .map(|boards| {
                                super::personnel_transfer::permitted_carried(&config.0, boards)
                            })
                            .unwrap_or_default();
                        let plan = super::personnel_transfer::plan(
                            context, &actor.0, profile, roster, teams, officer, &request, tick,
                            &permitted,
                        )
                        .ok_or("physical transfer is not currently available")?;
                        teams
                            .0
                            .claim_personnel_transfer(
                                profile,
                                &state,
                                officer,
                                *team_idx as usize,
                                request,
                                plan,
                            )
                            .ok_or("actual escort team is not available")?;
                    }
                    _ => unreachable!(),
                }
                Ok(())
            })();
            crate::command_admission::finish_action_feedback(
                command,
                &mut outbound,
                if result.is_ok() {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}

pub(crate) fn source(
    origin: &crate::world::spawn_origin::SpawnOrigin,
    actor: &str,
) -> Result<PersonnelSource, String> {
    let id = crate::world_id::WorldId::parse(actor)
        .filter(|id| id.namespace == crate::world_id::IdNamespace::Entity && id.render() == actor)
        .ok_or("runtime personnel requires its actual canonical Entity WorldId receipt")?;
    Ok(PersonnelSource {
        birth_tick: id.tick,
        birth_sequence: id.seq,
        template_path: origin.template_path.clone(),
        overrides: origin.overrides.clone(),
    })
}

fn origin_source_inserted(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    let Some(actor) = world
        .get::<crate::entities::spawner::EntityUuid>(context.entity)
        .map(|id| id.0.clone())
    else {
        return;
    };
    let Some(Ok(source)) = world
        .get::<crate::entities::spawner::EntitySpawnOrigin>(context.entity)
        .map(|origin| source(&origin.0, &actor))
    else {
        return;
    };
    if let Some(mut roster) = world.get_resource_mut::<PersonnelRoster>() {
        if let Some(origin) = roster.origins.get_mut(&actor) {
            if origin.source.is_none() {
                origin.accepted_tick = source.birth_tick;
                origin.source = Some(source);
            }
        }
    }
}

fn origin_removed(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    if world
        .get_resource::<crate::entities::craft::RestoreGuard>()
        .is_some_and(|guard| guard.0)
    {
        return;
    }
    let Some(actor) = world
        .get::<crate::entities::spawner::EntityUuid>(context.entity)
        .map(|id| id.0.clone())
    else {
        return;
    };
    let tick = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    if let Some(mut roster) = world.get_resource_mut::<PersonnelRoster>() {
        roster.remove_actor(&actor, tick);
    }
}
