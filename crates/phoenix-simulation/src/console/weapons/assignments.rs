//! Retained own weapon intent and its ephemeral, Console-permitted routing.
//! No public handle is resolved by consulting the scene or an entity name.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    science::broad::{BroadTracks, SensingControls},
    server_app::ShipSystemBlackboards,
    ship::config::ShipConfig,
};
use bevy::prelude::*;

#[derive(Component, Clone, Debug, Default)]
pub struct ShipWeaponTargets(
    pub phoenix_sim_gameplay::weapons::assignments::WeaponTargetAssignments,
);

pub struct WeaponAim {
    pub route: String,
    pub position: [f32; 2],
    pub velocity: Option<[f32; 2]>,
    pub measured: bool,
    pub tags: Vec<phoenix_model::tags::EntityTag>,
    pub ordinary_domain: bool,
}

impl WeaponAim {
    pub fn allows(&self, filter: Option<&[phoenix_model::tags::EntityTag]>) -> bool {
        crate::weapons::operation::target_allowed(filter, &self.tags, self.ordinary_domain)
    }
}

/// Borrow actual owner intent, publications and physical mount state together.
/// The legacy scene closure is never called for fitted observation equipment.
#[derive(bevy::ecs::system::SystemParam)]
pub struct WeaponAimInputs<'w, 's> {
    measurement_markers: Query<'w, 's, (Has<BroadTracks>, Has<SensingControls>)>,
    ordinary: Query<
        'w,
        's,
        (
            Option<&'static crate::entities::spawner::EntityUuid>,
            Option<&'static crate::server_app::AsteroidUuid>,
        ),
        Or<(
            With<crate::entities::spawner::EntityUuid>,
            With<crate::server_app::AsteroidUuid>,
        )>,
    >,
    profiles: Query<'w, 's, &'static crate::science::broad::BroadProfile>,
    tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    fixed: Option<Res<'w, Time<Fixed>>>,
    scopes: Query<
        'w,
        's,
        (
            &'static ShipSystemBlackboards,
            Option<&'static crate::ship_plugin::ShipConfigComponent>,
            Option<&'static BroadTracks>,
            Option<&'static SensingControls>,
            Option<&'static ShipWeaponTargets>,
            Option<&'static crate::weapons::mounts::MountProfiles>,
            Option<&'static crate::weapons::mounts::ShipMounts>,
            Option<&'static crate::entities::spawner::EntityUuid>,
        ),
    >,
    runtime: Option<Res<'w, crate::world::server::WorldContentRuntime>>,
}

impl WeaponAimInputs<'_, '_> {
    pub fn observed_shield_cover(
        &self,
        entity: Entity,
        owner: &SystemId,
        aim: &WeaponAim,
        own: [f32; 2],
    ) -> Option<i32> {
        let (boards, config, broad, controls, assignments, _, _, _) =
            self.scopes.get(entity).ok()?;
        let config = &config?.0;
        let contact =
            effective_weapon_contact(config, owner, boards, broad, controls, assignments)?;
        let view = config.observations_for_system(owner, &boards.0);
        let profiles = self.profiles.get(entity).ok()?;
        let now = self.tick.as_deref()?.0;
        let step = self.fixed.as_deref()?.timestep().as_secs_f64();
        if !step.is_finite() || step <= 0.0 {
            return None;
        }
        let sector =
            phoenix_sim_gameplay::science::tracks::shield_observation_sector(own, aim.position);
        let mut readings = Vec::new();
        for (source, profile) in &profiles.0 {
            let Some(records) = view
                .get(source)
                .and_then(crate::science::contact_routing::observations)
            else {
                continue;
            };
            for record in records.iter().filter(|record| {
                record.contact_id == contact && record.selectable && !record.predicted
            }) {
                let Some(taken) = record.assessment_tick.filter(|tick| *tick <= now) else {
                    continue;
                };
                if (now - taken) as f64 * step
                    > 2.0 * f64::from(profile.profile.sample_interval_secs) + step
                {
                    continue;
                }
                let Some(protection) = record
                    .assessment
                    .as_ref()
                    .and_then(|assessment| assessment.protection.as_ref())
                    .filter(|protection| protection.validate().is_ok())
                else {
                    continue;
                };
                let delta = sector.abs_diff(protection.incoming_sector);
                if delta.min(360 - delta) <= 1 {
                    readings.push((taken, protection.blocked));
                }
            }
        }
        let newest = readings.iter().map(|(tick, _)| *tick).max()?;
        let mut latest = readings
            .into_iter()
            .filter(|(tick, _)| *tick == newest)
            .map(|(_, blocked)| blocked);
        let blocked = latest.next()?;
        latest
            .all(|other| other == blocked)
            .then_some(i32::from(blocked))
    }
    pub fn active(
        &self,
        entity: Entity,
        owner: &SystemId,
        frozen: &str,
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
    ) -> Option<WeaponAim> {
        let row = self.scopes.get(entity).ok();
        if row.is_some_and(|row| row.1.is_some()) {
            self.resolve(entity, owner, legacy)
                .filter(|aim| aim.route == frozen)
        } else {
            if self
                .measurement_markers
                .get(entity)
                .is_ok_and(|(broad, focused)| broad || focused)
            {
                return None;
            }
            Some(WeaponAim {
                route: frozen.to_owned(),
                position: legacy(frozen)?,
                velocity: None,
                measured: false,
                tags: Vec::new(),
                ordinary_domain: true,
            })
        }
    }

    pub fn bearing(&self, entity: Entity, owner: &SystemId, fixed: f32) -> Option<f32> {
        let Ok(row) = self.scopes.get(entity) else {
            return Some(fixed);
        };
        let Some(profile) = row.5.and_then(|profiles| profiles.0.get(owner)) else {
            return Some(fixed);
        };
        Some(row.6?.0.get(owner)?.bearing_deg(profile))
    }
    pub fn resolve(
        &self,
        entity: Entity,
        owner: &SystemId,
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
    ) -> Option<WeaponAim> {
        let (boards, config, broad, controls, assignments, _, _, _) =
            self.scopes.get(entity).ok()?;
        let Some(config) = config else {
            let Some(SystemBlackboard::Viewscreen(board)) = boards
                .0
                .get(&crate::ship::system_registry::viewscreen_system_id())
            else {
                return None;
            };
            return self.resolve_reference(entity, owner, board.combat_lock.as_deref()?, legacy);
        };
        let contact =
            effective_weapon_contact(&config.0, owner, boards, broad, controls, assignments)?;
        self.resolve_reference(entity, owner, &contact, legacy)
    }

    pub fn has_target_intent(&self, entity: Entity, owner: &SystemId) -> bool {
        let Ok((boards, config, _, _, assignments, _, _, _)) = self.scopes.get(entity) else {
            return false;
        };
        if assignments
            .and_then(|state| state.0 .0.get(owner))
            .is_some_and(|state| !state.follows_primary())
        {
            return true;
        }
        let primary = crate::ship::system_registry::viewscreen_system_id();
        if let Some(config) = config {
            let view = config.0.observations_for_system(owner, &boards.0);
            matches!(view.get(&primary), Some(SystemBlackboard::Viewscreen(board)) if board.combat_lock.is_some())
        } else {
            matches!(boards.0.get(&primary), Some(SystemBlackboard::Viewscreen(board)) if board.combat_lock.is_some())
        }
    }

    pub fn resolve_reference(
        &self,
        entity: Entity,
        owner: &SystemId,
        contact: &str,
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
    ) -> Option<WeaponAim> {
        let (boards, config, broad, controls, _, _, _, observer) = self.scopes.get(entity).ok()?;
        let Some(config) = config else {
            if crate::science::contact_routing::measured_machinery(broad, controls) {
                return None;
            }
            return Some(WeaponAim {
                route: contact.to_owned(),
                position: legacy(contact)?,
                velocity: None,
                measured: false,
                tags: Vec::new(),
                ordinary_domain: true,
            });
        };
        let route = crate::science::contact_routing::resolve_machinery(
            Some(&config.0),
            Some(boards),
            broad,
            controls,
            owner,
            contact,
        )?;
        if self.runtime.as_deref().is_some_and(|runtime| {
            observer.is_none_or(|observer| {
                crate::gm::operations::contact::mode(
                    &runtime.contact_overrides,
                    &observer.0,
                    &route,
                ) == crate::gm::operations::contact::ContactMode::Conceal
            })
        }) {
            return None;
        }
        let measured = crate::science::contact_routing::measured_machinery(broad, controls);
        let position = if measured {
            crate::weapons::mounts::measured_point(&config.0, boards, broad?, owner, contact)?
        } else {
            legacy(&route)?
        };
        let velocity = if measured {
            let view = config.0.observations_for_system(owner, &boards.0);
            boards
                .0
                .keys()
                .filter_map(|key| view.get(key))
                .filter_map(crate::science::contact_routing::observations)
                .flatten()
                .find(|track| track.contact_id == contact && track.selectable && !track.predicted)
                .and_then(|track| track.velocity)
                .filter(|velocity| velocity.iter().all(|value| value.is_finite()))
        } else {
            None
        };
        let tags = if measured {
            let view = config.0.observations_for_system(owner, &boards.0);
            let mut classifications = boards
                .0
                .keys()
                .filter_map(|key| view.get(key))
                .filter_map(crate::science::contact_routing::observations)
                .flatten()
                .filter(|track| track.contact_id == contact && track.selectable && !track.predicted)
                .filter_map(|track| track.classification.as_deref());
            match classifications.next() {
                Some(first) if classifications.all(|other| other == first) => {
                    phoenix_model::tags::EntityTag::from_str(first)
                        .into_iter()
                        .collect()
                }
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        // Routing has already proven this exact permitted actuator. Inspect
        // domain membership only; no hidden pose, health or target enters aim.
        let ordinary_domain = !measured
            || self.ordinary.iter().any(|(entity, rock)| {
                entity.is_some_and(|id| id.0 == route) || rock.is_some_and(|id| id.0 == route)
            });
        position
            .iter()
            .all(|value| value.is_finite())
            .then_some(WeaponAim {
                route,
                position,
                velocity,
                measured,
                tags,
                ordinary_domain,
            })
    }

    pub fn geometry(
        &self,
        entity: Entity,
        owner: &SystemId,
        mount: &crate::weapons::mount::MountConfig,
        aim: &WeaponAim,
        physics: &crate::ship::state::ShipPhysics,
    ) -> crate::core::messages::WeaponTargetGeometry {
        let state = self.scopes.get(entity).ok();
        crate::weapons::mounts::geometry(
            state.and_then(|row| row.5),
            state.and_then(|row| row.6),
            owner,
            mount,
            aim.position,
            [physics.x, physics.z],
            physics.yaw,
        )
    }
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if config.ship_config.as_ref().is_some_and(|ship| {
        ship.systems
            .iter()
            .any(|system| phoenix_sim_gameplay::weapons::assignments::is_weapon(&system.kind))
    }) {
        commands.insert(ShipWeaponTargets::default());
    }
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<ShipWeaponTargets>(StateClass::Folded, "weapon-target-assignments")
        .add_systems(
            FixedUpdate,
            handle_assignments
                .in_set(crate::sim_sets::SimSet::Input)
                .in_set(crate::sim_sets::FixedStep::HandleWeaponAssignments),
        );
}

fn handle_assignments(
    mut ships: Query<(
        &crate::core::messages::AdmittedCommands,
        &crate::ship_plugin::ShipConfigComponent,
        Option<&crate::entities::spawner::WeaponsConsoleSection>,
        &ShipSystemBlackboards,
        Option<&BroadTracks>,
        Option<&SensingControls>,
        &mut ShipWeaponTargets,
        Option<&crate::entities::spawner::EntityUuid>,
    )>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    use crate::core::messages::SystemControlPayload as Payload;
    for (admitted, config, weapons, boards, broad, controls, mut assignments, observer) in
        &mut ships
    {
        for command in &admitted.0 {
            if !matches!(
                command.payload,
                Payload::SetWeaponTarget { .. }
                    | Payload::SetWeaponGroupTarget { .. }
                    | Payload::FollowWeaponGroup
                    | Payload::FollowPrimaryTarget
            ) {
                continue;
            }
            let accepted = (|| {
                let system = config.0.system(&command.target)?;
                if !phoenix_sim_gameplay::weapons::assignments::is_weapon(&system.kind) {
                    return None;
                }
                let target = match &command.payload {
                    Payload::SetWeaponTarget { uuid }
                    | Payload::SetWeaponGroupTarget { uuid, .. } => uuid,
                    Payload::FollowWeaponGroup => {
                        assignments.0.follow_group(&command.target);
                        return Some(());
                    }
                    Payload::FollowPrimaryTarget => {
                        assignments.0.follow_primary(&command.target);
                        return Some(());
                    }
                    _ => return None,
                };
                if let Some(target) = target {
                    if target.is_empty()
                        || target.len() > 512
                        || target.chars().any(char::is_control)
                    {
                        return None;
                    }
                    let route = crate::science::contact_routing::resolve_machinery(
                        Some(&config.0),
                        Some(boards),
                        broad,
                        controls,
                        &command.target,
                        target,
                    )?;
                    if runtime.as_deref().is_some_and(|runtime| {
                        observer.is_none_or(|observer| {
                            crate::gm::operations::contact::mode(
                                &runtime.contact_overrides,
                                &observer.0,
                                &route,
                            ) == crate::gm::operations::contact::ContactMode::Conceal
                        })
                    }) {
                        return None;
                    }
                }
                match &command.payload {
                    Payload::SetWeaponTarget { uuid } => assignments
                        .0
                        .set_individual(command.target.clone(), uuid.clone()),
                    Payload::SetWeaponGroupTarget { group, uuid } => {
                        let prepared = weapons?.0.groups.iter().find(|row| &row.id == group)?;
                        if !prepared.members.contains(&command.target) {
                            return None;
                        }
                        assignments.0.set_group(
                            command.target.clone(),
                            group.clone(),
                            uuid.clone(),
                        );
                    }
                    _ => return None,
                }
                Some(())
            })()
            .is_some();
            super::finish_action_feedback(
                command,
                &mut outbound,
                if accepted {
                    super::WeaponActionResult::Applied
                } else {
                    super::WeaponActionResult::Refused(super::WeaponActionRefusal::MissingTarget)
                },
            );
        }
    }
}

pub fn effective_weapon_contact(
    config: &ShipConfig,
    owner: &SystemId,
    boards: &ShipSystemBlackboards,
    broad: Option<&BroadTracks>,
    controls: Option<&SensingControls>,
    assignments: Option<&ShipWeaponTargets>,
) -> Option<String> {
    let view = config.observations_for_system(owner, &boards.0);
    let primary = match view.get(&crate::ship::system_registry::viewscreen_system_id()) {
        Some(SystemBlackboard::Viewscreen(board)) => board.combat_lock.as_deref(),
        _ => None,
    };
    let explicit = assignments
        .and_then(|state| state.0 .0.get(owner))
        .filter(|assignment| !assignment.follows_primary());
    let selected = match explicit {
        Some(assignment) => assignment.target(None),
        None => primary,
    }?;
    if !crate::science::contact_routing::measured_machinery(broad, controls) {
        return Some(selected.to_owned());
    }
    let broad = broad?;
    if crate::science::contact_routing::resolve(config, boards, Some(broad), owner, selected)
        .is_some()
    {
        return Some(selected.to_owned());
    }
    // Only the pre-existing primary machinery selection may carry a private
    // route. Explicit assignments always retain their original public handle.
    if explicit.is_some() {
        return None;
    }
    crate::science::contact_routing::alias_for_source(config, boards, Some(broad), owner, selected)
}

pub fn effective_weapon_target(
    config: &ShipConfig,
    owner: &SystemId,
    boards: &ShipSystemBlackboards,
    broad: Option<&BroadTracks>,
    controls: Option<&SensingControls>,
    assignments: Option<&ShipWeaponTargets>,
) -> Option<String> {
    let contact = effective_weapon_contact(config, owner, boards, broad, controls, assignments)?;
    crate::science::contact_routing::resolve_machinery(
        Some(config),
        Some(boards),
        broad,
        controls,
        owner,
        &contact,
    )
}

#[cfg(test)]
#[path = "assignments_tests.rs"]
mod tests;
