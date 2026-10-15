//! The last game-owned projection before browser/native physical delivery.
//!
//! Authoritative ECS, host render channels, GM views and mesh frames keep truth.
//! A phone on a hull with fitted broad instruments receives contacts through its
//! existing typed Console publications, never the renderer's entity registry.
//! This is a transient read projection, not another knowledge or network owner.
use bevy::{ecs::system::SystemParam, prelude::*};
use std::collections::BTreeSet;

use crate::core::messages::{EntitySnapshot, ModifierSource, ServerMessage, WorldData};
use crate::entities::spawner::EntityUuid;
use crate::lobby::{OutboundMessage, Sessions, Target, WorldResource};
use crate::server_app::LocalShip;

#[path = "console_delivery_references.rs"]
mod references;
use references::ReferenceProjection;

/// Authored permission to publish a fixed chart landmark, independent of scans.
pub const CHARTED_TAG: &str = "charted";

#[derive(SystemParam)]
pub struct ConsoleDeliveryProjection<'w, 's> {
    sessions: Option<Res<'w, Sessions>>,
    local: Query<
        'w,
        's,
        (
            Option<&'static EntityUuid>,
            Option<&'static crate::science::broad::BroadProfile>,
            Option<&'static crate::ship::components::ShipConfigComponent>,
            Option<&'static crate::server_app::ShipSystemBlackboards>,
            Option<&'static crate::ship::components::ConsoleHosts>,
            Option<&'static crate::science::broad::BroadTracks>,
        ),
        With<LocalShip>,
    >,
    world: Option<Res<'w, WorldResource>>,
    runtime: Option<Res<'w, crate::world::server::WorldContentRuntime>>,
}

impl ConsoleDeliveryProjection<'_, '_> {
    /// Capture once per flush. No resource, cache, observation or event is written.
    pub fn capture(&self) -> CapturedConsoleDelivery {
        let local = self.local.iter().find(|(_, broad, ..)| broad.is_some());
        let own = local.and_then(|(uuid, ..)| uuid.map(|uuid| uuid.0.clone()));
        let concealed = own
            .as_ref()
            .and_then(|own| self.runtime.as_ref()?.contact_overrides.get(own))
            .into_iter()
            .flat_map(|rows| rows.iter())
            .filter(|(_, mode)| **mode == crate::gm::operations::contact::ContactMode::Conceal)
            .map(|(uuid, _)| uuid.clone())
            .collect::<BTreeSet<_>>();
        let references = local.map_or_else(
            ReferenceProjection::default,
            |(_, broad, config, boards, hosts, tracks)| {
                ReferenceProjection::capture(
                    config,
                    boards,
                    hosts,
                    broad,
                    tracks,
                    self.sessions.as_deref(),
                    &concealed,
                )
            },
        );
        CapturedConsoleDelivery {
            restricted: local.is_some(),
            own,
            recipients: self
                .sessions
                .as_ref()
                .into_iter()
                .flat_map(|sessions| sessions.0.players())
                .filter(|player| player.connected)
                .map(|player| player.token.clone())
                .collect(),
            concealed,
            references,
            world: self
                .world
                .as_ref()
                .map(|world| world.0.clone())
                .unwrap_or_default(),
        }
    }
}

/// One flush's immutable projection inputs. It is deliberately not an ECS owner.
pub struct CapturedConsoleDelivery {
    restricted: bool,
    own: Option<String>,
    recipients: BTreeSet<String>,
    concealed: BTreeSet<String>,
    world: WorldData,
    references: ReferenceProjection,
}

impl CapturedConsoleDelivery {
    /// Both host sinks call this before encoding/dispatch. Preserve input order
    /// and delivery class; a private broadcast becomes explicit recipient sends.
    pub fn project(&self, outgoing: &OutboundMessage) -> Vec<OutboundMessage> {
        if !self.restricted || !requires_projection(&outgoing.msg) {
            return vec![outgoing.clone()];
        }
        let mut projected = Vec::new();
        let include = |token: &str| match &outgoing.target {
            Target::All => true,
            Target::AllExcept(excluded) => token != excluded,
            Target::Token(target) => token == target,
        };
        // GM operators are authenticated host-class peers, not phone Sessions.
        // Their existing host channels/mesh never enter this boundary. Preserve
        // the actual reserved local host route; do not invent a gm-token bypass.
        let local = crate::console_bridge::LOCAL_CONSOLE_TOKEN;
        if include(local) {
            projected.push(OutboundMessage {
                target: Target::Token(local.into()),
                ..outgoing.clone()
            });
        }
        for token in self.recipients.iter().filter(|token| include(token)) {
            if let Some(msg) = self.phone_message(&outgoing.msg, token) {
                projected.push(OutboundMessage {
                    target: Target::Token(token.clone()),
                    msg,
                    delivery: outgoing.delivery,
                });
            }
        }
        projected
    }

    fn is_own(&self, uuid: &str) -> bool {
        self.own.as_deref() == Some(uuid)
    }

    fn charted(&self, uuid: &str) -> bool {
        !self.concealed.contains(uuid)
            && self
                .world
                .entities
                .iter()
                .any(|entity| entity.uuid == uuid && has_chart_permission(entity))
    }

    fn entity(&self, entity: &EntitySnapshot) -> Option<EntitySnapshot> {
        if self.is_own(&entity.uuid) {
            Some(entity.clone())
        } else if !self.concealed.contains(&entity.uuid) && has_chart_permission(entity) {
            Some(chart_snapshot(entity))
        } else {
            None
        }
    }

    fn world(&self, world: &WorldData) -> WorldData {
        WorldData {
            entities: world
                .entities
                .iter()
                .filter_map(|entity| self.entity(entity))
                .collect(),
            scenario_title: world.scenario_title.clone(),
            scenario_description: world.scenario_description.clone(),
        }
    }

    fn public_target(&self, uuid: &str) -> bool {
        self.is_own(uuid) || self.charted(uuid)
    }

    fn phone_message(&self, raw: &ServerMessage, token: &str) -> Option<ServerMessage> {
        use ServerMessage::*;
        let mut message = raw.clone();
        match &mut message {
            Welcome { state, .. } => {
                state.world = state.world.as_ref().map(|world| self.world(world));
            }
            WorldSetup { world } => *world = self.world(world),
            SimState { snapshot } => {
                snapshot.entity_states.retain_mut(|state| {
                    if self.is_own(&state.uuid) {
                        return true;
                    }
                    if !self.charted(&state.uuid) {
                        return false;
                    }
                    // A chart grants geometry, not a live health/shield sensor.
                    state.hull_fraction = None;
                    state.shield_fraction = None;
                    state.shields = None;
                    state.shield_freq = None;
                    state.warp_out_remaining_secs = None;
                    state.flags.clear();
                    true
                });
            }
            EntitySpawned { snapshot } => *snapshot = self.entity(snapshot)?,
            // The actual lifecycle publisher emits a public roster only when
            // a chart landmark disappears. An unseen death creates no phone
            // traffic, including no replacement packet that reveals its timing.
            EntityDespawned { uuid } | StationDestroyed { uuid } | AsteroidDestroyed { uuid } => {
                if !self.is_own(uuid) {
                    return None;
                }
            }
            StationSpawned { uuid, .. } | AsteroidSpawned { uuid, .. } => {
                let entity = self
                    .world
                    .entities
                    .iter()
                    .find(|entity| entity.uuid == *uuid)?;
                return Some(EntitySpawned {
                    snapshot: self.entity(entity)?,
                });
            }
            WeaponsUpdate {
                target_uuid,
                target_name,
                ..
            } => {
                let target = self.references.tactical(token, target_uuid.as_deref());
                *target_uuid = target.as_ref().map(|target| target.id.clone());
                *target_name = target.and_then(|target| target.name);
            }
            TargetLock { uuid, .. }
            | PhaserFired {
                target_uuid: uuid, ..
            } => {
                if let Some(target) = self.references.tactical(token, Some(uuid)) {
                    *uuid = target.id;
                } else if !self.public_target(uuid) {
                    return None;
                }
            }
            BeamStarted {
                source_uuid,
                target_uuid,
                ..
            }
            | BeamEnded {
                source_uuid,
                target_uuid,
                ..
            } => {
                if !self.is_own(source_uuid) {
                    return None;
                }
                if let Some(target) = self.references.tactical(token, Some(target_uuid)) {
                    *target_uuid = target.id;
                } else if !self.public_target(target_uuid) {
                    return None;
                }
            }
            BlasterFired { source_uuid, .. } => {
                if !self.is_own(source_uuid) {
                    return None;
                }
            }
            // These visual-only envelopes contain no authenticated source
            // owner. Do not infer one from proximity or retain projectile IDs
            // in another state store. The host renderer still gets originals.
            TorpedoLaunched { .. } | TorpedoDestroyed { .. } | BlasterHit { .. } => return None,
            ModifierAdded {
                source: ModifierSource::RegionEffect { uuid },
                ..
            }
            | ModifierRemoved {
                source: ModifierSource::RegionEffect { uuid },
                ..
            } => {
                if !self.public_target(&uuid.to_string()) {
                    return None;
                }
            }
            BlackboardUpdate {
                console_scope,
                updates,
                ..
            } => {
                updates.retain(|(source, _)| {
                    self.references.permits(token, source)
                        && console_scope
                            .as_ref()
                            .is_none_or(|scope| scope.contains(source))
                });
                for (source, board) in updates.iter_mut() {
                    self.references.board(source, board);
                }
                if let Some(scope) = console_scope {
                    scope.retain(|source| self.references.permits(token, source));
                } else {
                    *console_scope =
                        Some(updates.iter().map(|(source, _)| source.clone()).collect());
                }
            }
            _ => {}
        }
        Some(message)
    }
}

fn has_chart_permission(entity: &EntitySnapshot) -> bool {
    entity.tags.iter().any(|tag| tag == CHARTED_TAG)
}

fn chart_snapshot(entity: &EntitySnapshot) -> EntitySnapshot {
    EntitySnapshot {
        uuid: entity.uuid.clone(),
        name: entity.name.clone(),
        position: entity.position,
        tags: entity.tags.clone(),
        shape: entity.shape.clone(),
        radius: entity.radius,
        colour: entity.colour,
        yaw: entity.yaw,
        inner_radius: entity.inner_radius,
        radar_size: entity.radar_size,
        half_extents: entity.half_extents,
        radar_icon: entity.radar_icon.clone(),
        region_colour: entity.region_colour,
        ..Default::default()
    }
}

fn requires_projection(message: &ServerMessage) -> bool {
    use ServerMessage::*;
    matches!(
        message,
        Welcome { .. }
            | WorldSetup { .. }
            | SimState { .. }
            | EntitySpawned { .. }
            | EntityDespawned { .. }
            | AsteroidSpawned { .. }
            | AsteroidDestroyed { .. }
            | StationSpawned { .. }
            | StationDestroyed { .. }
            | TargetLock { .. }
            | WeaponsUpdate { .. }
            | BlackboardUpdate { .. }
            | BeamStarted { .. }
            | BeamEnded { .. }
            | PhaserFired { .. }
            | BlasterFired { .. }
            | BlasterHit { .. }
            | TorpedoLaunched { .. }
            | TorpedoDestroyed { .. }
            | ModifierAdded {
                source: ModifierSource::RegionEffect { .. },
                ..
            }
            | ModifierRemoved {
                source: ModifierSource::RegionEffect { .. },
                ..
            }
    )
}

#[cfg(test)]
#[path = "console_delivery_tests.rs"]
mod tests;
