//! Join paid Security work to Session vessel ownership and ordinary NPC doctrine.
//! This exclusive adapter is chained directly after the existing paid team tick.
use super::server::{SecurityTargetActions, ShipSecurityTeams};
use crate::entities::spawner::{EntityUuid, FactionComponent};
use crate::security::{SecurityAction, SecurityRefusal, SecurityTeamState};
use crate::ship::compartment_damage::ShipDamage;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::prelude::*;
use phoenix_model::vessel_ownership::{VesselAcquisitionCause, VesselOperationalOwnerData};
pub use phoenix_sim_session::vessel_ownership::VesselOperationalOwner;

/// Accepted recipe data; not another mutable faction or personnel owner.
#[derive(Component, Clone, Debug)]
pub struct AcquisitionTargetDefinition {
    pub original_faction: uuid::Uuid,
}

/// A capture candidate is knowledge-limited: an independently selectable
/// contact and its observed disabled assessment must both be held by Security.
pub(super) fn acquisition_alias(
    config: &crate::ship::config::ShipConfig,
    boards: &crate::server_app::ShipSystemBlackboards,
    tracks: &crate::science::broad::BroadTracks,
    source: &str,
) -> Option<String> {
    let consumer = crate::ship::system_registry::security_system_id();
    let alias = crate::science::contact_routing::alias_for_source(
        config,
        boards,
        Some(tracks),
        &consumer,
        source,
    )?;
    let permitted = config.observations_for_system(&consumer, &boards.0);
    boards
        .0
        .keys()
        .filter_map(|owner| permitted.get(owner))
        .filter_map(crate::science::contact_routing::observations)
        .flatten()
        .any(|track| {
            track.contact_id == alias
                && !track.predicted
                && track.selectable
                && track
                    .assessment
                    .as_ref()
                    .is_some_and(|assessment| assessment.disabled == Some(true))
        })
        .then_some(alias)
}

/// Both original population and actual foreign custody disqualify this initial
/// uncrewed-only slice. Lost people elsewhere do not become a prize crew.
fn has_personnel(roster: &phoenix_sim_gameplay::personnel::PersonnelRoster, target: &str) -> bool {
    use phoenix_model::personnel::OfficerLocation;
    roster.origins.get(target).is_some_and(|origin| {
        !origin.definition.config.officers.is_empty() || !origin.state.officers.is_empty()
    }) || roster
        .origins
        .values()
        .flat_map(|origin| origin.state.officers.values())
        .any(|person| match &person.location {
            OfficerLocation::AtCompartment { actor, .. }
            | OfficerLocation::WithRepairTeam { actor, .. }
            | OfficerLocation::WithTransfer { actor, .. } => actor == target,
            OfficerLocation::AboardCraft { craft, .. } => craft == target,
            OfficerLocation::Lost { .. } => false,
        })
}

fn eligible(world: &World, actor: Entity, target: Entity) -> Result<uuid::Uuid, String> {
    let actor_id = &world
        .get::<EntityUuid>(actor)
        .ok_or("missing acquiring actor")?
        .0;
    let target_id = &world.get::<EntityUuid>(target).ok_or("missing target")?.0;
    let faction = world
        .get::<FactionComponent>(actor)
        .ok_or("acquirer has no faction")?
        .0;
    let registered = world
        .get_resource::<crate::entities::config_cache::FactionRegistryResource>()
        .is_some_and(|registry| registry.0.get(&faction).is_some());
    if actor == target
        || !registered
        || world
            .get::<FactionComponent>(target)
            .is_none_or(|f| f.0 == faction)
        || world.get::<AcquisitionTargetDefinition>(target).is_none()
        || world.get::<VesselOperationalOwner>(target).is_some()
        || world.get::<crate::server_app::Ship>(target).is_none()
        || world.get::<crate::server_app::LocalShip>(target).is_some()
        || world.get::<crate::lockstep::FleetSlotOf>(target).is_some()
        || world
            .get::<crate::ship_slots::AuthoredShipSlotId>(target)
            .is_some()
        || world
            .get::<crate::lobby::crew_replication::ShipCrewPresence>(target)
            .is_none_or(|crew| !crew.0.is_empty())
        || world
            .get::<ShipSystemControlSources>(target)
            .is_none_or(|sources| {
                sources
                    .0
                    .entries()
                    .any(|(_, source)| *source == crate::ship::control_source::ControlSource::Human)
            })
        || world
            .get_resource::<crate::gm::operations::puppet::StationPuppets>()
            .is_some_and(|puppets| puppets.operates_ship(target_id))
        || world
            .get_resource::<phoenix_sim_gameplay::personnel::PersonnelRoster>()
            .is_some_and(|roster| has_personnel(roster, target_id))
    {
        return Err("target is not an eligible uncrewed autonomous non-player vessel".into());
    }
    let config = world
        .get::<ShipConfigComponent>(target)
        .ok_or("missing actual target equipment")?;
    let damage = world
        .get::<ShipDamage>(target)
        .ok_or("missing physical target damage")?;
    if damage.catastrophic()
        || !damage.disabled(
            &config.0,
            world.get::<crate::ship::power::ShipPowerSystem>(target),
        )
    {
        return Err("target is not physically disabled and recoverable".into());
    }
    let own_config = world
        .get::<ShipConfigComponent>(actor)
        .ok_or("missing acquiring equipment")?;
    let boards = world
        .get::<crate::server_app::ShipSystemBlackboards>(actor)
        .ok_or("missing own observations")?;
    let tracks = world
        .get::<crate::science::broad::BroadTracks>(actor)
        .ok_or("missing measured contact")?;
    if acquisition_alias(&own_config.0, boards, tracks, target_id).is_none() {
        return Err("Security Console no longer holds a selectable target contact".into());
    }
    let _ = actor_id;
    Ok(faction)
}

/// No duplicate progress/state: readiness comes from the same paid Job clock.
/// No ready work is observable outside this immediately chained transaction.
pub fn commit_ready_acquisitions(world: &mut World) {
    let mut faction_changed = false;
    let mut ready = {
        let mut q = world.query::<(Entity, &EntityUuid, &ShipSecurityTeams)>();
        q.iter(world)
            .flat_map(|(entity, uuid, security)| {
                security
                    .teams
                    .jobs
                    .teams
                    .iter()
                    .enumerate()
                    .filter_map(move |(index, job)| {
                        let job = job.as_ref()?;
                        let team = &job.activity.team;
                        (job.activity.acquisition_order.is_some()
                            && team.state == SecurityTeamState::Working
                            && team.elapsed >= team.phase_duration
                            && team.phase_duration > 0.0)
                            .then(|| (uuid.0.clone(), index, entity, job.clone()))
                    })
            })
            .collect::<Vec<_>>()
    };
    ready.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let now = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    for (actor, index, entity, job) in ready {
        let order = job
            .activity
            .acquisition_order
            .as_ref()
            .expect("ready acquisition order");
        let target = {
            let mut q = world.query::<(Entity, &EntityUuid)>();
            q.iter(world)
                .find_map(|(entity, uuid)| (uuid.0 == order.target).then_some(entity))
        };
        let prepared = target
            .ok_or_else(|| "acquisition target removed".to_owned())
            .and_then(|target| {
                let faction = eligible(world, entity, target)?;
                let target_actions = world
                    .get::<SecurityTargetActions>(target)
                    .ok_or("lost accepted target action")?;
                let action = target_actions
                    .0
                    .action(SecurityAction::Board)
                    .ok_or("lost Board action")?;
                if action
                    .acquisition
                    .as_ref()
                    .is_none_or(|config| config.doctrine != order.doctrine)
                    || job.activity.team.risk != action.risk
                    || job.activity.team.phase_duration != action.duration_secs
                    || order.accepted_tick > now
                {
                    return Err("acquisition order no longer matches accepted Board work".into());
                }
                let owner = VesselOperationalOwner::acquire(
                    None,
                    VesselOperationalOwnerData {
                        controller: actor.clone(),
                        faction,
                        acquired_tick: now,
                        doctrine: order.doctrine.clone(),
                        cause: VesselAcquisitionCause::SecurityBoard {
                            scope: phoenix_model::activity::ActivityJob {
                                actor: actor.clone(),
                                owner: job.owner.clone(),
                                job: job.id,
                            },
                            team: index as u32,
                            target: order.target.clone(),
                            accepted_tick: order.accepted_tick,
                            work_completed_tick: now,
                        },
                    },
                    &order.target,
                    now,
                )?;
                Ok((target, owner))
            });
        let mut acquired = None;
        if let Ok((target, owner)) = prepared {
            if world.contains_resource::<crate::world::server::WorldContentRuntime>() {
                world.resource_scope(
                    |world, runtime: Mut<crate::world::server::WorldContentRuntime>| {
                        let mut state = bevy::ecs::system::SystemState::<
                            crate::gm::operations::npc::NpcDoctrineControl,
                        >::new(world);
                        let safe = runtime
                            .gm_npc_doctrine_palette
                            .iter()
                            .find(|profile| profile.id == order.doctrine)
                            .is_some_and(|profile| {
                                profile.origin_layer.is_none()
                                    && !profile.doctrine.is_empty()
                                    && profile.doctrine.iter().all(|objective| {
                                        matches!(
                                            crate::ai::core::parse_doctrine_directive(objective),
                                            Ok(crate::core::messages::AiDirective::None)
                                        )
                                    })
                            });
                        if !safe {
                            return;
                        }
                        let applied = {
                            let mut control = state.get_mut(world);
                            match control.prepare(&runtime, &order.target, &order.doctrine) {
                                Ok(doctrine) => {
                                    control.commit_prepared(doctrine);
                                    true
                                }
                                Err(_) => false,
                            }
                        };
                        if applied {
                            state.apply(world);
                            world
                                .entity_mut(target)
                                .insert((FactionComponent(owner.0.faction), owner.clone()));
                            acquired = Some(owner.0);
                            faction_changed = true;
                        }
                    },
                );
            }
        }
        // Same physical job always closes this frontier, including failed preflight.
        let succeeded = acquired.is_some();
        let mut security = world
            .get_mut::<ShipSecurityTeams>(entity)
            .expect("same acquiring owner");
        let withdraw = security.config.withdraw_duration_secs;
        let actual = security.teams.jobs.teams[index]
            .as_mut()
            .expect("same paid Board job");
        assert_eq!(actual.id, job.id);
        actual.activity.acquisition = acquired;
        security.teams.withdraw(
            index,
            withdraw,
            if succeeded {
                phoenix_model::job::JobOutcome::Completed
            } else {
                phoenix_model::job::JobOutcome::Failed
            },
        );
        security.last_refusal = (!succeeded).then_some(SecurityRefusal::ActionUnavailable);
        if let Some(mut lifecycle) = world.get_resource_mut::<crate::effect_queue::EffectQueue<crate::core::task_lifecycle::TaskLifecycleRequest>>() {
            lifecycle.0.push(crate::core::task_lifecycle::TaskLifecycleRequest::End {
                slot: super::server::team_slot(&actor, index as u8),
                reason: if succeeded { crate::core::task_lifecycle::TaskTerminalReason::Completed }
                    else { crate::core::task_lifecycle::TaskTerminalReason::Disabled },
            });
        }
    }
    if faction_changed {
        use bevy::ecs::system::RunSystemOnce;
        world
            .run_system_once(revalidate_faction_locks)
            .expect("ordinary faction lock revalidation");
    }
}

/// Reuse the ordinary faction-change target owner; emitted projectiles and
/// active physical effects are deliberately untouched.
fn revalidate_faction_locks(
    registry: Res<crate::entities::config_cache::FactionRegistryResource>,
    non_ai: Query<
        (&EntityUuid, &FactionComponent),
        Without<crate::entities::spawner::BehaviourSection>,
    >,
    mut ai: Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&FactionComponent>,
        ),
        With<crate::entities::spawner::BehaviourSection>,
    >,
) {
    let factions = ai
        .iter()
        .filter_map(|(uuid, _, faction)| Some((uuid::Uuid::parse_str(&uuid.0).ok()?, faction?.0)))
        .collect::<Vec<_>>();
    let map = crate::world::server::build_uuid_to_faction(&non_ai, &factions);
    crate::world::server::revalidate_ai_targets_after_faction_change(&mut ai, &registry.0, &map);
}

#[cfg(test)]
#[path = "acquisition_tests.rs"]
mod tests;
