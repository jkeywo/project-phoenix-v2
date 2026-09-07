//! One capability boundary for NPC offers and canonical Station commands.
//!
//! This is intentionally an audited interface set, not a second dispatcher.
//! Registered command addresses are necessary but cannot prove that an NPC
//! has the components and per-ship producers its authored interface needs.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::command_admission::router::AdmittedConsumerRegistry;
use crate::core::messages::{ShipClientConfig, StationId};
use crate::entities::spawner::EntityUuid;
use crate::gm_action::GmActionRefusalReason;
use crate::ship::components::ShipConfigComponent;
use crate::ship::system_registry::SystemKindRegistry;

/// Content-derived instance replica, created from the fully resolved spawn
/// config, including world/palette overrides. Restore rebuilds it by the same
/// spawn path. The capability check reads the authored interface; no runtime
/// authority is inferred from the local player's selected hull.
#[derive(Component, Clone, Debug)]
pub struct NpcStationConfig(pub ShipClientConfig);

pub struct CoreSystemDescriptors(SystemKindRegistry);

impl Default for CoreSystemDescriptors {
    fn default() -> Self {
        Self(SystemKindRegistry::with_core_systems().expect("built-in System descriptors"))
    }
}

#[derive(SystemParam)]
pub struct StationCapabilities<'w, 's> {
    ships: Query<'w, 's, EntityRef<'static>, With<crate::server_app::Ship>>,
    descriptors: Local<'s, CoreSystemDescriptors>,
    consumers: Option<Res<'w, AdmittedConsumerRegistry>>,
}

impl StationCapabilities<'_, '_> {
    pub fn check(&self, ship: &str, station: &StationId) -> Result<(), GmActionRefusalReason> {
        let mut matches = self.ships.iter().filter(|entity| {
            entity
                .get::<EntityUuid>()
                .is_some_and(|uuid| uuid.0 == ship)
        });
        let entity = matches
            .next()
            .ok_or(GmActionRefusalReason::UnknownStation)?;
        if matches.next().is_some() {
            return Err(GmActionRefusalReason::UnknownStation);
        }
        station_capability(
            &entity,
            station,
            Some(&self.descriptors.0),
            self.consumers.as_deref(),
        )
    }
}

pub fn station_capability(
    entity: &EntityRef<'_>,
    station: &StationId,
    descriptors: Option<&SystemKindRegistry>,
    consumers: Option<&AdmittedConsumerRegistry>,
) -> Result<(), GmActionRefusalReason> {
    let config = entity
        .get::<ShipConfigComponent>()
        .ok_or(GmActionRefusalReason::UnknownStation)?;
    let authored = config
        .0
        .station(station)
        .ok_or(GmActionRefusalReason::UnknownStation)?;
    // Existing player Station authority is unchanged. Its ordinary host boot
    // provides the complete interface/producer graph; NPCs have a separate
    // spawn graph and therefore need the explicit audit below.
    if entity.contains::<crate::lockstep::FleetSlotOf>() {
        return Ok(());
    }
    let unavailable = GmActionRefusalReason::SystemUnavailable;
    // The complete normal Welcome projection comes from this NPC instance's
    // resolved spawn config, never the local player's selected hull.
    entity.get::<NpcStationConfig>().ok_or(unavailable)?;
    let console = authored
        .console
        .as_deref()
        .filter(|url| !url.is_empty())
        .ok_or(unavailable)?;
    if !entity.contains::<crate::core::messages::AdmittedCommands>()
        || !entity.contains::<crate::ship::components::ActiveStationRatings>()
        || !entity.contains::<crate::ship::components::ShipSystemControlSources>()
        || !entity.contains::<crate::server_app::ShipSystemBlackboards>()
        || !entity.contains::<crate::ship::state::ShipPhysics>()
        || !entity.contains::<crate::ship_plugin::ShipPhysicsConfigResource>()
        || !entity.contains::<crate::modifiers::ShipModifiers>()
    {
        return Err(unavailable);
    }
    // Mixed interfaces stay closed until EVERY family they expose has the
    // same per-ship producer and consumer fidelity. In particular Captain's
    // camera/priority and Power's blackboards are still player-only.
    let kinds: &[&str] = match console {
        "gui/battleship/helm.html"
            if entity.contains::<crate::entities::spawner::HelmConsoleSection>()
                && entity.contains::<crate::server_app::ShipImpulse>()
                && entity.contains::<crate::server_app::ShipBoost>()
                && entity.contains::<crate::ship::components::LastHelmInput>() =>
        {
            &[
                "helm_thrust",
                "helm_steering",
                "helm_impulse",
                "helm_boost",
                "lateral_thrust",
                "vertical_thrust",
            ]
        }
        _ => return Err(unavailable),
    };
    let descriptors = descriptors.ok_or(unavailable)?;
    let consumers = consumers.ok_or(unavailable)?;
    let mut commandable = false;
    for system in config.0.systems_for_station(station) {
        if !kinds.contains(&system.kind.as_str()) || system.ai_only {
            return Err(unavailable);
        }
        let descriptor = descriptors.descriptor(&system.kind).ok_or(unavailable)?;
        if descriptor.accepts_admitted_commands() {
            if !consumers.is_system_routed(system) {
                return Err(unavailable);
            }
            commandable = true;
        }
    }
    commandable.then_some(()).ok_or(unavailable)
}
