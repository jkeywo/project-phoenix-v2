//! Runtime adapters for the canonical Debug Surface catalogue (issue #1267).
//!
//! `core::debug_surface` owns identity, order, and wire names.  This module
//! connects each row to the Bevy Resource owned by that diagnostic module.  A
//! new surface therefore adds one canonical row and one adapter beside its
//! implementation; bridge drains and state reporting contain no surface match
//! and no positional list of booleans.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::core::debug_surface::{DebugSurface, DEBUG_SURFACE_CATALOGUE};

/// Minimal contract implemented by each module's enabled Resource.
pub trait DebugSurfaceState: Resource {
    fn is_enabled(&self) -> bool;
    fn set_enabled(&mut self, enabled: bool);
}

/// One module-owned connection between a catalogue identity and live state.
#[derive(Clone, Copy)]
pub struct DebugSurfaceAdapter {
    pub surface: DebugSurface,
    read: fn(&World) -> bool,
    write: fn(&mut World, bool),
}

impl DebugSurfaceAdapter {
    /// Build an adapter for a module-owned enabled Resource.
    pub const fn for_resource<T: DebugSurfaceState>(surface: DebugSurface) -> Self {
        Self {
            surface,
            read: read_resource::<T>,
            write: write_resource::<T>,
        }
    }

    pub fn is_enabled(self, world: &World) -> bool {
        (self.read)(world)
    }

    pub fn set_enabled(self, world: &mut World, enabled: bool) {
        (self.write)(world, enabled);
    }

    pub fn toggle(self, world: &mut World) {
        let enabled = self.is_enabled(world);
        self.set_enabled(world, !enabled);
    }
}

fn read_resource<T: DebugSurfaceState>(world: &World) -> bool {
    world
        .get_resource::<T>()
        .map(DebugSurfaceState::is_enabled)
        .unwrap_or(false)
}

fn write_resource<T: DebugSurfaceState>(world: &mut World, enabled: bool) {
    let current = world.get_resource::<T>().map(DebugSurfaceState::is_enabled);
    match current {
        Some(current) if current == enabled => {}
        Some(_) => world.resource_mut::<T>().set_enabled(enabled),
        None => panic!(
            "Debug Surface adapter {} has no enabled Resource",
            std::any::type_name::<T>()
        ),
    }
}

/// Exactly one adapter per canonical surface, in the catalogue's stable order.
///
/// The constants are declared beside their owning modules.  This one assembly
/// list is deliberately explicit: it is the registration seam and the
/// completeness test below proves that no catalogue row is missing or doubled.
pub const DEBUG_SURFACE_ADAPTERS: [DebugSurfaceAdapter; DEBUG_SURFACE_CATALOGUE.len()] = [
    crate::debug_overlay::DEBUG_REGIONS_ADAPTER,
    crate::debug::modifiers::DEBUG_MODIFIERS_ADAPTER,
    crate::debug::damage::DEBUG_DAMAGE_ADAPTER,
    crate::debug::entities::DEBUG_ENTITIES_ADAPTER,
    crate::debug::inspector::DEBUG_INSPECTOR_ADAPTER,
    crate::debug::station_activity::DEBUG_STATION_ACTIVITY_ADAPTER,
    crate::debug::ai_state::DEBUG_AI_DOCTRINE_ADAPTER,
    crate::debug::scenario::DEBUG_SCENARIO_STATE_ADAPTER,
    crate::debug::console_latency::DEBUG_CONSOLE_LATENCY_ADAPTER,
];

fn adapter(surface: DebugSurface) -> &'static DebugSurfaceAdapter {
    DEBUG_SURFACE_ADAPTERS
        .iter()
        .find(|adapter| adapter.surface == surface)
        .expect("every Debug Surface has one registered adapter")
}

/// Read every surface through its owning adapter in canonical order.
pub fn readback(world: &World) -> Vec<(DebugSurface, bool)> {
    DEBUG_SURFACE_CATALOGUE
        .into_iter()
        .map(|descriptor| {
            let adapter = adapter(descriptor.surface);
            (descriptor.surface, adapter.is_enabled(world))
        })
        .collect()
}

/// Apply an absolute state through the same adapter host and phone routes use.
pub fn set_surface(world: &mut World, surface: DebugSurface, enabled: bool) {
    adapter(surface).set_enabled(world, enabled);
}

/// Apply an absolute pending-state batch in catalogue order.
///
/// Multiple host requests for one surface collapse to the latest requested
/// value. Applying the resulting map in catalogue order keeps Bevy change
/// edges deterministic even though the bridge's transient inbox is a map.
pub fn apply_pending_states(
    world: &mut World,
    pending: impl IntoIterator<Item = (DebugSurface, bool)>,
) {
    let pending: HashMap<DebugSurface, bool> = pending.into_iter().collect();
    for descriptor in DEBUG_SURFACE_CATALOGUE {
        if let Some(enabled) = pending.get(&descriptor.surface) {
            adapter(descriptor.surface).set_enabled(world, *enabled);
        }
    }
}

/// Apply a pending relative-toggle batch.
///
/// Repeated occurrences collapse to one flip, matching the pre-catalogue
/// `HashSet` behavior.  Iteration follows the catalogue rather than hash order,
/// keeping resource change order deterministic.
pub fn apply_pending_toggles(world: &mut World, pending: impl IntoIterator<Item = DebugSurface>) {
    let pending: HashSet<DebugSurface> = pending.into_iter().collect();
    for descriptor in DEBUG_SURFACE_CATALOGUE {
        if pending.contains(&descriptor.surface) {
            adapter(descriptor.surface).toggle(world);
        }
    }
}

/// Canonical all-build state consumed by `ServerMessage::DebugState` and the
/// host bridge getter.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct DebugSurfaceReadback(pub Vec<(DebugSurface, bool)>);

impl Default for DebugSurfaceReadback {
    fn default() -> Self {
        Self(
            DebugSurface::ALL
                .into_iter()
                .map(|surface| (surface, false))
                .collect(),
        )
    }
}

/// Refresh [`DebugSurfaceReadback`] from module-owned resources.
///
/// Exclusive by design: the whole point of the adapter registry is that this
/// reader does not grow one `Res<_>` parameter per surface.
pub fn refresh_readback(world: &mut World) {
    let current = readback(world);
    if world.resource::<DebugSurfaceReadback>().0 != current {
        world.resource_mut::<DebugSurfaceReadback>().0 = current;
    }
}

/// Runtime half of the public-demo gate, paired with the cfg on both mutation
/// routes.  Readback remains available regardless of this answer.
pub const fn mutation_route_available() -> bool {
    !cfg!(phoenix_demo_build)
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
