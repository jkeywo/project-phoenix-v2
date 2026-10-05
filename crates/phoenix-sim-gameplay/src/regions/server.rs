use bevy::prelude::*;
use std::collections::HashMap;
/// Resource tracking which entities are inside which regions.
#[derive(Resource, Default)]
pub struct RegionMembership {
    /// Maps ship entity → set of region entities the ship is currently inside.
    /// A `BTreeSet` of regions, not a `HashSet` (issue #965). The set
    /// differences in `update_region_membership` are what emit
    /// `RegionEntered`/`RegionExited`, and a ship that crosses two boundaries
    /// on one tick emits one event per region — so the set's iteration order
    /// IS the event order. Those events queue `WorldEvent::EnteredRegion` for
    /// the world-trigger pipeline and queue `ModifierEvent`s for broadcast,
    /// neither of which may depend on a hash seed. Ordering by `Entity` costs
    /// nothing at these sizes (a ship is inside a handful of regions at most)
    /// and is stable across processes because ECS entity allocation in a
    /// seeded run is.
    pub inside: HashMap<Entity, std::collections::BTreeSet<Entity>>,
    /// Cached UUIDs for region entities (persists after entity despawn).
    pub region_uuids: HashMap<Entity, String>,
}

/// Fired when a subject entity enters a region.
#[derive(Event, Clone, Debug)]
pub struct RegionEntered {
    pub subject: Entity,
    pub region_entity: Entity,
}

/// Fired when a subject entity exits a region (or the region is despawned).
#[derive(Event, Clone, Debug)]
pub struct RegionExited {
    pub subject: Entity,
    pub region_entity: Entity,
}
