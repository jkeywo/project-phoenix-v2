//! Stable instance identities shared at domain boundaries.
use bevy::prelude::*;
/// Every entity spawned by the generic spawner carries a UUID.
#[derive(Component, Clone, Debug)]
pub struct EntityUuid(pub String);

/// Optional human-readable identifier for the entity instance.
#[derive(Component, Clone, Debug)]
pub struct EntityId(pub String);

/// Display name from the top-level `name = "..."` scalar in the entity TOML.
/// Used by the renderer for HUD labels and by triggers/comms for named instances.
#[derive(Component, Clone, Debug)]
pub struct EntityName(pub String);

/// Canonical entity-template path that produced this live entity.
///
/// Unlike [`EntitySpawnOrigin`], this identity is present for authored and
/// runtime spawns alike. Live inspection must never recover provenance by
/// comparing resolved configs: two templates may intentionally resolve to the
/// same ship topology while remaining different authored sources.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct EntityTemplatePath(pub String);

impl EntityTemplatePath {
    pub fn new(path: &str) -> Self {
        Self(phoenix_content::include_resolve::canonical_template_path(
            path,
        ))
    }
}

/// Marker component recording which loaded world layer spawned this entity
/// (perf fix, issue #891 review finding 1). Stamped exactly once, at the two
/// sites that add an entity to a `WorldRuntime::spawned_entities` list — the
/// `SpawnEntity` trigger action and the bulk layer-load spawn in
/// `apply_world_layer_changes` — so [`entity_flag_chain`] can read a ship's
/// origin layer in O(1) (a `Query::get`) instead of the O(layers) scan
/// `entity_origin_layer` used to run on every call, including per-claim
/// inside `handle_torpedo_magazine_inter_system`.
///
/// Absent on a base-world (or otherwise unrecorded) entity — exactly the
/// entities the old scan resolved to `None` — so a missing component keeps
/// meaning "anchored at the base world", not "not spawned yet".
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct EntityOriginLayer(pub String);
