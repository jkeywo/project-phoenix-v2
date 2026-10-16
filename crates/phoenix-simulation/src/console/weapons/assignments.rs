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
    pub elevation: Option<phoenix_model::sensing_tracks::ObservedElevation>,
    pub measured: bool,
    pub tags: Vec<phoenix_model::tags::EntityTag>,
    pub ordinary_domain: bool,
}

impl WeaponAim {
    pub fn allows(&self, filter: Option<&[phoenix_model::tags::EntityTag]>) -> bool {
        (!self.tags.contains(&phoenix_model::tags::EntityTag::Missile) || self.elevation.is_some())
            && crate::weapons::operation::target_allowed(filter, &self.tags, self.ordinary_domain)
    }

    /// Legacy ordinary planar targets retain planar aim. An observed missile
    /// requires its measured height; no hidden physical pose can fill it in.
    pub fn point_3d(&self, own_height: f32) -> Option<[f32; 3]> {
        let height = match self.elevation {
            Some(value) => value.height,
            None if self.ordinary_domain => own_height,
            None => return None,
        };
        Some([self.position[0], height, self.position[1]])
    }

    pub fn velocity_3d(&self) -> [f32; 3] {
        let planar = self.velocity.unwrap_or([0.0, 0.0]);
        [
            planar[0],
            self.elevation
                .and_then(|value| value.velocity)
                .unwrap_or(0.0),
            planar[1],
        ]
    }
}

/// The same immutable owner view is used by executors and publishers. The
/// caller may prove ordinary physical identity, but cannot supply target facts.
pub(crate) struct WeaponAimContext<'a> {
    pub config: Option<&'a crate::ship_plugin::ShipConfigComponent>,
    pub boards: &'a ShipSystemBlackboards,
    pub broad: Option<&'a BroadTracks>,
    pub controls: Option<&'a SensingControls>,
    pub assignments: Option<&'a ShipWeaponTargets>,
    pub profiles: Option<&'a crate::weapons::mounts::MountProfiles>,
    pub mounts: Option<&'a crate::weapons::mounts::ShipMounts>,
    pub observer: Option<&'a crate::entities::spawner::EntityUuid>,
    pub runtime: Option<&'a crate::world::server::WorldContentRuntime>,
}

impl WeaponAimContext<'_> {
    pub fn resolve(
        &self,
        owner: &SystemId,
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
        ordinary: &impl Fn(&str) -> bool,
    ) -> Option<WeaponAim> {
        let Self {
            boards,
            config,
            broad,
            controls,
            assignments,
            ..
        } = *self;
        let Some(config) = config else {
            let Some(SystemBlackboard::Viewscreen(board)) = boards
                .0
                .get(&crate::ship::system_registry::viewscreen_system_id())
            else {
                return None;
            };
            return self.resolve_reference(owner, board.combat_lock.as_deref()?, legacy, ordinary);
        };
        let contact =
            effective_weapon_contact(&config.0, owner, boards, broad, controls, assignments)?;
        self.resolve_reference(owner, &contact, legacy, ordinary)
    }

    pub fn resolve_allowed(
        &self,
        owner: &SystemId,
        filter: Option<&[phoenix_model::tags::EntityTag]>,
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
        ordinary: &impl Fn(&str) -> bool,
    ) -> Option<WeaponAim> {
        let resolved = self
            .resolve(owner, &legacy, ordinary)
            .filter(|aim| aim.allows(filter));
        let assignment = self.assignments.and_then(|state| state.0 .0.get(owner));
        if resolved.is_some() || assignment.is_some_and(|state| state.has_explicit_order()) {
            return resolved;
        }
        self.resolve_reference(owner, assignment?.automatic.as_deref()?, legacy, ordinary)
            .filter(|aim| aim.allows(filter))
    }

    pub fn resolve_reference(
        &self,
        owner: &SystemId,
        contact: &str,
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
        ordinary: &impl Fn(&str) -> bool,
    ) -> Option<WeaponAim> {
        let Self {
            boards,
            config,
            broad,
            controls,
            observer,
            ..
        } = *self;
        let Some(config) = config else {
            if crate::science::contact_routing::measured_machinery(broad, controls) {
                return None;
            }
            return Some(WeaponAim {
                route: contact.to_owned(),
                position: legacy(contact)?,
                velocity: None,
                elevation: None,
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
        if self.runtime.is_some_and(|runtime| {
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
        let elevation = if measured {
            crate::weapons::mounts::measured_elevation(&config.0, boards, broad?, owner, contact)
        } else {
            None
        };
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
        let ordinary_domain = !measured || ordinary(&route);
        position
            .iter()
            .all(|value| value.is_finite())
            .then_some(WeaponAim {
                route,
                position,
                velocity,
                elevation,
                measured,
                tags,
                ordinary_domain,
            })
    }

    pub fn geometry(
        &self,
        owner: &SystemId,
        mount: &crate::weapons::mount::MountConfig,
        aim: &WeaponAim,
        physics: &crate::ship::state::ShipPhysics,
    ) -> crate::core::messages::WeaponTargetGeometry {
        let mut geometry = crate::weapons::mounts::geometry(
            self.profiles,
            self.mounts,
            owner,
            mount,
            aim.position,
            [physics.x, physics.z],
            physics.yaw,
        );
        if let Some(elevation) = aim.elevation {
            let distance = ((f64::from(aim.position[0]) - f64::from(physics.x)).powi(2)
                + (f64::from(aim.position[1]) - f64::from(physics.z)).powi(2)
                + (f64::from(elevation.height) - f64::from(physics.y)).powi(2))
            .sqrt();
            geometry.in_range = distance.is_finite() && distance <= f64::from(mount.range);
            geometry.range = distance.min(f64::from(f32::MAX)) as f32;
        }
        geometry
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
        if !aim.measured {
            return None;
        }
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
                if phoenix_sim_gameplay::science::tracks::shield_ray_compatible(
                    own,
                    record.position,
                    record.position_error,
                    protection.incoming_sector,
                ) {
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
            let context = self.context(entity)?;
            // A running cycle owns its captured route, not the next-shot
            // selection. Reacquire only an alias still permitted to this
            // Console; a frozen private route grants no target knowledge.
            let contact = crate::science::contact_routing::machinery_alias(
                context.config.map(|config| &config.0),
                Some(context.boards),
                context.broad,
                context.controls,
                owner,
                frozen,
            )?;
            context
                .resolve_reference(owner, &contact, legacy, &|route| self.ordinary_route(route))
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
                elevation: None,
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
        self.context(entity)?
            .resolve(owner, legacy, &|route| self.ordinary_route(route))
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

    /// Explicit orders, including Hold or an unavailable target, never fall
    /// through. A defensive fallback is usable only when primary is ineligible.
    pub fn resolve_allowed(
        &self,
        entity: Entity,
        owner: &SystemId,
        filter: Option<&[phoenix_model::tags::EntityTag]>,
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
    ) -> Option<WeaponAim> {
        self.context(entity)?
            .resolve_allowed(owner, filter, legacy, &|route| self.ordinary_route(route))
    }

    pub fn has_explicit_order(&self, entity: Entity, owner: &SystemId) -> bool {
        self.scopes
            .get(entity)
            .ok()
            .and_then(|row| row.4)
            .and_then(|state| state.0 .0.get(owner))
            .is_some_and(|state| state.has_explicit_order())
    }

    pub fn automatic_contact(&self, entity: Entity, owner: &SystemId) -> Option<String> {
        self.scopes
            .get(entity)
            .ok()?
            .4?
            .0
             .0
            .get(owner)?
            .automatic
            .clone()
    }

    /// Ordinary defensive policy sees current permitted missile observations.
    /// Approaching motion and reach are measured facts, not launcher allegiance
    /// or the missile's hidden attack order. Retain a viable previous choice.
    pub fn defensive_contact(
        &self,
        entity: Entity,
        owner: &SystemId,
        mount: &crate::weapons::mount::MountConfig,
        physics: &crate::ship::state::ShipPhysics,
    ) -> Option<String> {
        use phoenix_model::tags::EntityTag;
        let (boards, config, _, _, _, _, _, _) = self.scopes.get(entity).ok()?;
        let view = config?.0.observations_for_system(owner, &boards.0);
        let contacts = boards
            .0
            .keys()
            .filter_map(|key| view.get(key))
            .filter_map(crate::science::contact_routing::observations)
            .flatten()
            .filter(|record| {
                record.selectable
                    && !record.predicted
                    && record
                        .classification
                        .as_deref()
                        .and_then(EntityTag::from_str)
                        == Some(EntityTag::Missile)
            })
            .map(|record| record.contact_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let current = self.automatic_contact(entity, owner);
        let mut candidates = Vec::new();
        for contact in contacts {
            let Some(aim) = self
                .resolve_reference(entity, owner, &contact, |_| None)
                .filter(|aim| aim.allows(Some(&[EntityTag::Missile])))
            else {
                continue;
            };
            let Some(velocity) = aim.velocity else {
                continue;
            };
            let Some(point) = aim.point_3d(physics.y) else {
                continue;
            };
            let relative = [
                f64::from(point[0]) - f64::from(physics.x),
                f64::from(point[1]) - f64::from(physics.y),
                f64::from(point[2]) - f64::from(physics.z),
            ];
            let own_x = physics.forward_speed * crate::simmath::sin(physics.yaw)
                + physics.lateral_speed * crate::simmath::cos(physics.yaw);
            let own_z = -physics.forward_speed * crate::simmath::cos(physics.yaw)
                + physics.lateral_speed * crate::simmath::sin(physics.yaw);
            let motion = [
                f64::from(velocity[0]) - f64::from(own_x),
                f64::from(
                    aim.elevation
                        .and_then(|height| height.velocity)
                        .unwrap_or(0.0),
                ) - f64::from(physics.vertical_speed),
                f64::from(velocity[1]) - f64::from(own_z),
            ];
            let approach = relative
                .into_iter()
                .zip(motion)
                .map(|(p, v)| p * v)
                .sum::<f64>();
            let speed_sq = motion.into_iter().map(|v| v * v).sum::<f64>();
            let geometry = self.geometry(entity, owner, mount, &aim, physics);
            if approach >= 0.0 || speed_sq <= 0.0 || !geometry.in_range {
                continue;
            }
            // Finite turrets may still be traversing towards this designation.
            // The launch path separately enforces the actual pointing arc.
            let seconds = -approach / speed_sq;
            if !seconds.is_finite() {
                continue;
            }
            candidates.push((contact, seconds));
        }
        if let Some(current) =
            current.filter(|current| candidates.iter().any(|(id, _)| id == current))
        {
            return Some(current);
        }
        candidates
            .into_iter()
            .min_by(|(left, lt), (right, rt)| lt.total_cmp(rt).then_with(|| left.cmp(right)))
            .map(|(contact, _)| contact)
    }

    fn context(&self, entity: Entity) -> Option<WeaponAimContext<'_>> {
        let (boards, config, broad, controls, assignments, profiles, mounts, observer) =
            self.scopes.get(entity).ok()?;
        Some(WeaponAimContext {
            config,
            boards,
            broad,
            controls,
            assignments,
            profiles,
            mounts,
            observer,
            runtime: self.runtime.as_deref(),
        })
    }

    fn ordinary_route(&self, route: &str) -> bool {
        self.ordinary.iter().any(|(entity, rock)| {
            entity.is_some_and(|id| id.0 == route) || rock.is_some_and(|id| id.0 == route)
        })
    }

    pub fn resolve_reference(
        &self,
        entity: Entity,
        owner: &SystemId,
        contact: &str,
        legacy: impl Fn(&str) -> Option<[f32; 2]>,
    ) -> Option<WeaponAim> {
        self.context(entity)?
            .resolve_reference(owner, contact, legacy, &|route| self.ordinary_route(route))
    }

    pub fn geometry(
        &self,
        entity: Entity,
        owner: &SystemId,
        mount: &crate::weapons::mount::MountConfig,
        aim: &WeaponAim,
        physics: &crate::ship::state::ShipPhysics,
    ) -> crate::core::messages::WeaponTargetGeometry {
        // Bare fixtures without boards still use the fixed physical mount.
        if let Some(context) = self.context(entity) {
            context.geometry(owner, mount, aim, physics)
        } else {
            crate::weapons::mounts::geometry(
                None,
                None,
                owner,
                mount,
                aim.position,
                [physics.x, physics.z],
                physics.yaw,
            )
        }
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
                    | Payload::SetAutomaticWeaponTarget { .. }
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
                if matches!(command.payload, Payload::SetAutomaticWeaponTarget { .. })
                    && system.kind != crate::ship::system_registry::BLASTER_BANK_KIND
                {
                    return None;
                }
                let target = match &command.payload {
                    Payload::SetWeaponTarget { uuid }
                    | Payload::SetAutomaticWeaponTarget { uuid }
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
                    Payload::SetAutomaticWeaponTarget { uuid } => assignments
                        .0
                        .set_automatic(command.target.clone(), uuid.clone()),
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
    effective_weapon_contact_filtered(config, owner, boards, broad, controls, assignments, None)
}

pub fn effective_weapon_contact_filtered(
    config: &ShipConfig,
    owner: &SystemId,
    boards: &ShipSystemBlackboards,
    broad: Option<&BroadTracks>,
    controls: Option<&SensingControls>,
    assignments: Option<&ShipWeaponTargets>,
    filter: Option<&[phoenix_model::tags::EntityTag]>,
) -> Option<String> {
    let view = config.observations_for_system(owner, &boards.0);
    let primary = match view.get(&crate::ship::system_registry::viewscreen_system_id()) {
        Some(SystemBlackboard::Viewscreen(board)) => board.combat_lock.as_deref(),
        _ => None,
    };
    let assignment = assignments.and_then(|state| state.0 .0.get(owner));
    let normalise = |selected: &str, allow_route: bool| {
        if !crate::science::contact_routing::measured_machinery(broad, controls) {
            return Some(selected.to_owned());
        }
        let tracks = broad?;
        if crate::science::contact_routing::resolve(config, boards, Some(tracks), owner, selected)
            .is_some()
        {
            return Some(selected.to_owned());
        }
        allow_route
            .then(|| {
                crate::science::contact_routing::alias_for_source(
                    config,
                    boards,
                    Some(tracks),
                    owner,
                    selected,
                )
            })
            .flatten()
    };
    if let Some(explicit) = assignment.filter(|assignment| assignment.has_explicit_order()) {
        return normalise(explicit.target(None)?, false);
    }
    let eligible = |contact: &str| {
        let Some(filter) = filter else {
            return true;
        };
        let mut tags = boards
            .0
            .keys()
            .filter_map(|key| view.get(key))
            .filter_map(crate::science::contact_routing::observations)
            .flatten()
            .filter(|record| record.contact_id == contact && record.selectable && !record.predicted)
            .filter_map(|record| record.classification.as_deref())
            .map(phoenix_model::tags::EntityTag::from_str);
        let Some(Some(first)) = tags.next() else {
            return false;
        };
        tags.all(|other| other == Some(first)) && filter.contains(&first)
    };
    primary
        .and_then(|primary| normalise(primary, true))
        .filter(|contact| eligible(contact))
        .or_else(|| {
            normalise(assignment?.automatic.as_deref()?, false).filter(|contact| eligible(contact))
        })
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
