// Asteroid lifecycle managed by a ring-buffer window.
//
// This module provides:
// - AsteroidWindow component: 2D ring-buffer tracking which grid cells are loaded
//   (one per spawned `AsteroidFieldSection` entity — multi-field support)
// - PlayerGridPosition component: last known player grid cell per field
// - AsteroidEntityMap resource: UUID → Entity lookup for despawning (global,
//   keyed by globally-unique asteroid UUID)
// - FieldOwner component: links each spawned asteroid back to its field entity
//   so check_destroyed_asteroids can route slot clearing correctly
// - FieldIndex component: stable per-field seed source (captured at spawn time
//   from entity spawn order), used by eval_cell so the deterministic density
//   check is reproducible across runs
// - check_destroyed_asteroids: despawns asteroids with HP ≤ 0, clears slot
// - update_asteroid_window: drives spawn/despawn for every field based on
//   player movement
// - attach_field_components: idempotently attaches AsteroidWindow,
//   PlayerGridPosition, FieldIndex to every AsteroidFieldSection entity

use bevy::prelude::*;
use std::collections::HashMap;

use crate::asteroid_spawner::{cell_in_field, eval_cell};
use crate::asteroid_window::{
    compute_player_grid_cell, compute_slot_for_world_cell, eval_on_player_move,
};
use crate::entity_spawner::{AsteroidFieldSection, MeshSection};
use crate::lobby::Target;
use crate::lobby::WorldResource;
use crate::messages::{EntitySnapshot, ServerMessage};
use crate::ship_state::ShipPhysics;
use crate::simulation::SimOutbox;

pub use crate::entity_spawner::EntitySystemHull;
pub use crate::simulation::{Asteroid, AsteroidShieldPierce, AsteroidUuid};

// ── Components ───────────────────────────────────────────────────────────

/// The 2D ring-buffer window. One per spawned `AsteroidFieldSection` entity.
///
/// Indexed as [slot_z][slot_x] where (despawn_cells, despawn_cells) is the
/// player center.
///
/// Pre-#475 this was a `Resource` — a single global window for the whole
/// session. Promoted to a `Component` so multiple asteroid fields can coexist;
/// each field entity carries its own window with its own resolution, radii,
/// anchor, and ring-buffer slot grid.
#[derive(Component)]
pub struct AsteroidWindow {
    pub slots: Vec<Vec<Option<AsteroidData>>>,
    /// Cosmetic asteroids above the gameplay plane. Indexed [slot_z][slot_x].
    /// Stores raw Entity handles only — cosmetics have no UUID / hull tracking.
    pub cosmetic_upper_slots: Vec<Vec<Option<Entity>>>,
    /// Cosmetic asteroids below the gameplay plane. Indexed [slot_z][slot_x].
    pub cosmetic_lower_slots: Vec<Vec<Option<Entity>>>,
    pub arena_gx: i32,
    pub arena_gz: i32,
    pub despawn_cells: u32,
    pub spawn_cells: u32,
    /// Grid resolution (world units per cell).
    pub resolution: f32,
    /// Index of the asteroid field this window manages.
    pub field_idx: usize,
    /// Inner radius of the torus (cells closer than this have no asteroids).
    pub inner_radius: f32,
    /// Outer radius of the torus (cells farther than this have no asteroids).
    pub outer_radius: f32,
    /// Optional shape variant. When `None`, cell-centre eligibility is used
    /// (legacy default). When `Some(Torus)`, bbox-overlap eligibility is used.
    pub shape: Option<crate::entity_config::AsteroidFieldShape>,
    /// World-space offset applied as a pure post-seed translation to every
    /// cell coordinate. Defaults to `[0, 0, 0]` (world origin). When the
    /// owning `AsteroidFieldConfig` references a named world anchor, this
    /// is the resolved anchor position. Per AGENTS.md the per-cell density
    /// seed `(field_idx, gx, gz)` must NOT include the anchor — the offset
    /// is applied only when converting between world-space and the
    /// anchor-relative grid space used by `cell_in_field` and `eval_cell`.
    pub anchor_offset: [f32; 3],
    /// `true` until the first `update_asteroid_window` tick for this field
    /// has run a `full_rebuild`. Replaces the old global
    /// `PlayerGridPosition.is_none()` check; needed because the window
    /// component is inserted before the player position has been observed.
    pub needs_init: bool,
}

impl Default for AsteroidWindow {
    fn default() -> Self {
        let dc = 12u32;
        let size = (2 * dc + 1) as usize;
        Self {
            slots: vec![vec![None; size]; size],
            cosmetic_upper_slots: vec![vec![None; size]; size],
            cosmetic_lower_slots: vec![vec![None; size]; size],
            arena_gx: 0,
            arena_gz: 0,
            despawn_cells: dc,
            spawn_cells: 10,
            resolution: 10.0,
            field_idx: 0,
            inner_radius: 0.0,
            outer_radius: 0.0,
            shape: None,
            anchor_offset: [0.0, 0.0, 0.0],
            needs_init: true,
        }
    }
}

/// Data stored in each window slot for a spawned asteroid.
#[derive(Clone)]
pub struct AsteroidData {
    pub uuid: String,
    pub config_path: String,
    pub hp: i32,
    pub max_hp: i32,
    pub y: f32,
}

/// Player grid position from the previous frame, in this field's grid
/// coordinates. One per field entity (since different fields can have
/// different `resolution` and `anchor_offset`).
#[derive(Component, Default)]
pub struct PlayerGridPosition(pub Option<(i32, i32)>);

/// Stable per-field seed source. Set at spawn time from spawn order. The
/// first field declared in a world TOML gets `FieldIndex(0)`, the second
/// `1`, etc. Used as the `field_idx` seed for `eval_cell` so the
/// deterministic density check is reproducible across runs and stable
/// when more fields are added. Single-field worlds get `FieldIndex(0)` —
/// bit-for-bit identical seed to the pre-refactor behaviour.
#[derive(Component, Clone, Copy, Debug)]
pub struct FieldIndex(pub usize);

/// Marker component attached to each spawned gameplay asteroid pointing back
/// to the field entity that produced it. Used by `check_destroyed_asteroids`
/// to find the correct field's `AsteroidWindow` slot when an asteroid's
/// hull reaches zero.
#[derive(Component, Clone, Copy, Debug)]
pub struct FieldOwner(pub Entity);

// ── Resources ────────────────────────────────────────────────────────────

/// Maps asteroid UUID to spawned Entity for despawn and slot lookup.
#[derive(Resource, Default)]
pub struct AsteroidEntityMap(pub HashMap<String, Entity>);

// ── Systems ─────────────────────────────────────────────────────────────

/// Ensure every `AsteroidFieldSection` entity carries an `AsteroidWindow`,
/// `PlayerGridPosition`, and `FieldIndex` component. Runs each `Update`
/// (cheap — only inserts on entities that lack the components, which
/// happens on the frame they first appear).
///
/// `FieldIndex` is assigned by enumerating the live `AsteroidFieldSection`
/// entities in `Entity` order. Bevy `Entity` IDs are allocated in spawn
/// order, so the first field declared in a world TOML gets `FieldIndex(0)`,
/// the second `1`, etc. Single-field worlds get `FieldIndex(0)` —
/// bit-for-bit identical seed to the pre-refactor behaviour. (#475)
pub fn attach_field_components(
    mut commands: Commands,
    fields: Query<
        (Entity, Option<&AsteroidWindow>, Option<&FieldIndex>),
        With<AsteroidFieldSection>,
    >,
) {
    let mut indexed: Vec<(Entity, bool, bool)> = fields
        .iter()
        .map(|(e, win, idx)| (e, win.is_some(), idx.is_some()))
        .collect();
    indexed.sort_by_key(|(e, _, _)| *e);

    for (next_idx, (entity, has_window, has_index)) in indexed.into_iter().enumerate() {
        if !has_window {
            commands
                .entity(entity)
                .insert(AsteroidWindow::default())
                .insert(PlayerGridPosition::default());
        }
        if !has_index {
            commands.entity(entity).insert(FieldIndex(next_idx));
        }
    }
}

/// Check for destroyed asteroids, clear their window slot, broadcast, and
/// despawn the entity.
///
/// Routes each destroyed asteroid to its owning field's `AsteroidWindow`
/// via the `FieldOwner` component attached at spawn time. Asteroids without
/// `FieldOwner` (legacy or hand-spawned) skip the slot-clear step but still
/// despawn correctly via `entity_map` + the ECS despawn. (#475)
pub fn check_destroyed_asteroids(
    mut commands: Commands,
    mut field_windows: Query<&mut AsteroidWindow>,
    mut entity_map: ResMut<AsteroidEntityMap>,
    mut world: ResMut<WorldResource>,
    asteroid_query: Query<(
        Entity,
        &Transform,
        &AsteroidUuid,
        &EntitySystemHull,
        Option<&FieldOwner>,
    )>,
    mut outbox: ResMut<SimOutbox>,
    mut positions_cache: ResMut<crate::core::broadcast::LastBroadcastEntityPositions>,
    mut health_cache: ResMut<crate::core::broadcast::LastBroadcastEntityHealth>,
) {
    for (entity, transform, uuid, hull_comp, owner) in asteroid_query.iter() {
        if !hull_comp.0.is_destroyed() {
            continue;
        }
        if let Some(owner) = owner {
            if let Ok(mut window) = field_windows.get_mut(owner.0) {
                let (cell_gx, cell_gz) = compute_player_grid_cell(
                    transform.translation.x - window.anchor_offset[0],
                    transform.translation.z - window.anchor_offset[2],
                    window.resolution,
                );
                if let Some((sx, sz)) = compute_slot_for_world_cell(
                    window.arena_gx,
                    window.arena_gz,
                    cell_gx,
                    cell_gz,
                    window.despawn_cells,
                ) {
                    if let Some(row) = window.slots.get_mut(sz) {
                        if let Some(slot) = row.get_mut(sx) {
                            if slot.as_ref().is_some_and(|d| d.uuid == uuid.0) {
                                *slot = None;
                            }
                        }
                    }
                }
            }
        }
        entity_map.0.remove(&uuid.0);
        world.0.entities.retain(|e| e.uuid != uuid.0);

        // Prune the despawned UUID from the delta caches (issue #613) —
        // respawning asteroids get a fresh UUID every cycle, so without this
        // the position/health caches would grow by one stale entry per
        // historical asteroid forever.
        crate::core::broadcast::cache_registry::prune(
            &mut positions_cache,
            &mut health_cache,
            std::slice::from_ref(&uuid.0),
        );

        outbox.0.push((
            Target::All,
            ServerMessage::AsteroidDestroyed {
                uuid: uuid.0.clone(),
            },
        ));
        commands.entity(entity).try_despawn();
    }
}

/// Update every asteroid field's ring-buffer window when the player moves.
/// Runs every frame; per-field no-op if the player has not crossed that
/// field's cell boundary since the previous tick.
///
/// Each `AsteroidFieldSection` entity carries its own `AsteroidWindow` +
/// `PlayerGridPosition` + `FieldIndex` (attached by `attach_field_components`).
/// This system iterates every such field independently — multi-field worlds
/// drive multiple concurrent ring buffers off the same player position.
pub fn update_asteroid_window(
    mut commands: Commands,
    physics_q: Query<&ShipPhysics, With<crate::simulation::LocalShip>>,
    mut fields: Query<(
        Entity,
        &AsteroidFieldSection,
        &mut AsteroidWindow,
        &mut PlayerGridPosition,
        &FieldIndex,
    )>,
    mut world: ResMut<WorldResource>,
    mut entity_map: ResMut<AsteroidEntityMap>,
    mut outbox: ResMut<SimOutbox>,
    mut positions_cache: ResMut<crate::core::broadcast::LastBroadcastEntityPositions>,
    mut health_cache: ResMut<crate::core::broadcast::LastBroadcastEntityHealth>,
) {
    let physics = physics_q.single().ok().copied().unwrap_or_default();
    for (field_entity, section, mut window, mut player_grid, field_index) in fields.iter_mut() {
        let field = &section.0;
        let grid = match &field.grid {
            Some(g) => g,
            None => continue,
        };
        let field_idx = field_index.0;

        let (gx, gz) = compute_player_grid_cell(
            physics.x - field.anchor_offset[0],
            physics.z - field.anchor_offset[2],
            grid.resolution,
        );

        let needs_init = window.needs_init || player_grid.0.is_none();
        let (old_gx, old_gz) = player_grid.0.unwrap_or((gx, gz));

        if !needs_init && old_gx == gx && old_gz == gz {
            continue;
        }

        let delta =
            eval_on_player_move(old_gx, old_gz, gx, gz, grid.spawn_cells, grid.despawn_cells);

        if needs_init || delta.full_rebuild {
            full_rebuild(
                &mut commands,
                &mut window,
                &mut entity_map,
                &mut world,
                &mut outbox,
                &mut positions_cache,
                &mut health_cache,
                gx,
                gz,
                field_idx,
                field_entity,
                grid,
                field.inner_radius,
                field.outer_radius,
                &field.asteroid_type_paths,
                &field.cosmetic_type_paths,
                field.shield_pierce,
                field.shape,
                field.anchor_offset,
                field.random_rotation,
            );
            window.needs_init = false;
        } else {
            for (cell_gx, cell_gz) in &delta.cells_to_despawn {
                if let Some((sx, sz)) = compute_slot_for_world_cell(
                    window.arena_gx,
                    window.arena_gz,
                    *cell_gx,
                    *cell_gz,
                    window.despawn_cells,
                ) {
                    clear_slot(
                        &mut window,
                        &mut commands,
                        &mut entity_map,
                        &mut world,
                        &mut positions_cache,
                        &mut health_cache,
                        sx,
                        sz,
                    );
                }
            }

            window.arena_gx = gx;
            window.arena_gz = gz;

            for (cell_gx, cell_gz) in &delta.cells_to_spawn {
                if let Some((sx, sz)) = compute_slot_for_world_cell(
                    window.arena_gx,
                    window.arena_gz,
                    *cell_gx,
                    *cell_gz,
                    window.despawn_cells,
                ) {
                    try_spawn_cell(
                        &mut commands,
                        &mut window,
                        &mut entity_map,
                        &mut world,
                        &mut outbox,
                        *cell_gx,
                        *cell_gz,
                        sx,
                        sz,
                        field_idx,
                        field_entity,
                        grid,
                        field.inner_radius,
                        field.outer_radius,
                        &field.asteroid_type_paths,
                        field.shield_pierce,
                        field.shape,
                        field.anchor_offset,
                        field.random_rotation,
                    );
                    try_spawn_cosmetic_cell(
                        &mut commands,
                        &mut window,
                        *cell_gx,
                        *cell_gz,
                        sx,
                        sz,
                        field_idx,
                        grid,
                        field.inner_radius,
                        field.outer_radius,
                        &field.cosmetic_type_paths,
                        field.shape,
                        field.anchor_offset,
                    );
                }
            }
        }

        player_grid.0 = Some((gx, gz));
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Full rebuild: despawn all tracked entities, clear the window, re-evaluate
/// Full rebuild: despawn all of THIS field's tracked entities, clear the
/// window, re-evaluate every cell within the spawn window.
///
/// Multi-field safe (#475): only entities owned by this field (the ones
/// currently in `window.slots`) are despawned; other fields' asteroids in
/// the global `entity_map` are left alone. Cosmetics are despawned by
/// walking THIS field's `cosmetic_*_slots` (cosmetics aren't tracked in
/// the global map).
#[allow(clippy::too_many_arguments)]
fn full_rebuild(
    commands: &mut Commands,
    window: &mut AsteroidWindow,
    entity_map: &mut AsteroidEntityMap,
    world: &mut ResMut<WorldResource>,
    outbox: &mut ResMut<SimOutbox>,
    positions_cache: &mut crate::core::broadcast::LastBroadcastEntityPositions,
    health_cache: &mut crate::core::broadcast::LastBroadcastEntityHealth,
    gx: i32,
    gz: i32,
    field_idx: usize,
    field_entity: Entity,
    grid: &crate::entity_config::GridConfig,
    inner_radius: f32,
    outer_radius: f32,
    gameplay_type_paths: &[String],
    cosmetic_type_paths: &[String],
    shield_pierce: f32,
    shape: Option<crate::entity_config::AsteroidFieldShape>,
    anchor_offset: [f32; 3],
    random_rotation: Option<[f32; 3]>,
) {
    // Despawn ONLY this field's gameplay asteroids. Collect UUIDs from the
    // current window slots, despawn the entities, and prune entries from
    // the global map + world snapshot. Other fields' asteroids untouched.
    let owned_uuids: Vec<String> = window
        .slots
        .iter()
        .flat_map(|row| row.iter())
        .filter_map(|slot| slot.as_ref().map(|d| d.uuid.clone()))
        .collect();
    for uuid in &owned_uuids {
        if let Some(&entity) = entity_map.0.get(uuid) {
            commands.entity(entity).try_despawn();
        }
        entity_map.0.remove(uuid);
        world.0.entities.retain(|e| &e.uuid != uuid);
    }
    // Prune despawned UUIDs from the delta caches (issue #613) — same
    // rationale as `clear_slot`'s window-eviction prune below.
    crate::core::broadcast::cache_registry::prune(positions_cache, health_cache, &owned_uuids);

    // Despawn all existing cosmetic entities in this field's slots before resizing.
    for row in &window.cosmetic_upper_slots {
        for slot in row {
            if let Some(&entity) = slot.as_ref() {
                commands.entity(entity).try_despawn();
            }
        }
    }
    for row in &window.cosmetic_lower_slots {
        for slot in row {
            if let Some(&entity) = slot.as_ref() {
                commands.entity(entity).try_despawn();
            }
        }
    }

    // (#475) NOTE: pre-refactor this used to globally retain-out every
    // "asteroid" tagged snapshot from WorldResource. With multi-field
    // support that would clobber OTHER fields' asteroids. We now scoped
    // the removal above (only owned UUIDs).

    // Sync window extents from grid config so TOML-specified values take effect.
    window.spawn_cells = grid.spawn_cells;
    window.despawn_cells = grid.despawn_cells;

    let size = (2 * window.despawn_cells + 1) as usize;
    window.slots = vec![vec![None; size]; size];
    window.cosmetic_upper_slots = vec![vec![None; size]; size];
    window.cosmetic_lower_slots = vec![vec![None; size]; size];
    window.arena_gx = gx;
    window.arena_gz = gz;
    window.resolution = grid.resolution;
    window.field_idx = field_idx;
    window.inner_radius = inner_radius;
    window.outer_radius = outer_radius;
    window.shape = shape;
    window.anchor_offset = anchor_offset;

    let s_cells = window.spawn_cells as i32;
    for cx in (gx - s_cells)..=(gx + s_cells) {
        for cz in (gz - s_cells)..=(gz + s_cells) {
            if let Some((sx, sz)) =
                compute_slot_for_world_cell(gx, gz, cx, cz, window.despawn_cells)
            {
                try_spawn_cell(
                    commands,
                    window,
                    entity_map,
                    world,
                    outbox,
                    cx,
                    cz,
                    sx,
                    sz,
                    field_idx,
                    field_entity,
                    grid,
                    inner_radius,
                    outer_radius,
                    gameplay_type_paths,
                    shield_pierce,
                    shape,
                    anchor_offset,
                    random_rotation,
                );
                try_spawn_cosmetic_cell(
                    commands,
                    window,
                    cx,
                    cz,
                    sx,
                    sz,
                    field_idx,
                    grid,
                    inner_radius,
                    outer_radius,
                    cosmetic_type_paths,
                    shape,
                    anchor_offset,
                );
            }
        }
    }
}

/// A stable v4-formatted UUID for the rock in one field cell.
///
/// Two runs of the same scenario must name the same rock the same thing, or
/// the headless report's per-uuid damage ledgers cannot be compared. A rock
/// destroyed and respawned on re-entering its cell is the same rock as far as
/// the world is concerned, so reusing its identity is correct rather than
/// merely convenient.
///
/// The whole identifying tuple is *hashed* into all 16 bytes rather than
/// packed into byte positions, because packing aliases two ways and both were
/// live bugs: `uuid::Builder::from_random_bytes` rewrites byte 8's top two bits
/// (and byte 6's top four) to stamp the v4 variant/version, silently discarding
/// whatever field landed there, and any field narrower than the bytes it shares
/// collides with its neighbour. Two rocks sharing a uuid merge into one
/// `damage_by_ship` row, so uniqueness here is a reporting correctness
/// requirement, not a nicety.
fn deterministic_cell_uuid(
    field_idx: usize,
    cell_gx: i32,
    cell_gz: i32,
    slot_x: usize,
    slot_z: usize,
) -> String {
    // Each component is folded in through the whole 64-bit state before the
    // next arrives, so no component owns a byte range another can overwrite.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    mix(field_idx as u64);
    mix(cell_gx as u32 as u64);
    mix(cell_gz as u32 as u64);
    mix(slot_x as u64);
    mix(slot_z as u64);

    // Two splitmix64 draws fill all 16 bytes; the builder is then free to
    // overwrite its version/variant bits without costing us any input entropy.
    let mut bytes = [0u8; 16];
    let lo = splitmix64(hash);
    let hi = splitmix64(lo);
    bytes[0..8].copy_from_slice(&lo.to_le_bytes());
    bytes[8..16].copy_from_slice(&hi.to_le_bytes());
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

/// SplitMix64's finaliser. Local twin of the one in [`crate::sim_rng`]: this
/// path deliberately does not depend on the master seed (a rock's identity is
/// a pure function of its cell), so it does not reach for that module's state.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Evaluate a single cell for asteroid spawning. If the cell passes the
/// density check and is within the torus, spawn a gameplay asteroid entity
/// and populate the window slot.
///
/// `field_entity` is the owning `AsteroidFieldSection` entity — attached
/// to the spawned asteroid as a `FieldOwner` component so
/// `check_destroyed_asteroids` can route the slot-clear to the correct
/// field's window. (#475)
#[allow(clippy::too_many_arguments)]
fn try_spawn_cell(
    commands: &mut Commands,
    window: &mut AsteroidWindow,
    entity_map: &mut AsteroidEntityMap,
    world: &mut ResMut<WorldResource>,
    outbox: &mut ResMut<SimOutbox>,
    cell_gx: i32,
    cell_gz: i32,
    slot_x: usize,
    slot_z: usize,
    field_idx: usize,
    field_entity: Entity,
    grid: &crate::entity_config::GridConfig,
    inner_radius: f32,
    outer_radius: f32,
    gameplay_type_paths: &[String],
    shield_pierce: f32,
    shape: Option<crate::entity_config::AsteroidFieldShape>,
    anchor_offset: [f32; 3],
    random_rotation: Option<[f32; 3]>,
) {
    if window.slots[slot_z][slot_x].is_some() {
        return;
    }

    if !cell_in_field(
        cell_gx,
        cell_gz,
        grid.resolution,
        inner_radius,
        outer_radius,
        shape,
    ) {
        return;
    }

    if gameplay_type_paths.is_empty() {
        return;
    }

    let Some(spawn) = eval_cell(
        field_idx as u64,
        cell_gx,
        cell_gz,
        grid,
        inner_radius,
        outer_radius,
        gameplay_type_paths,
        &[],
    ) else {
        return;
    };

    // Look up the entity config from the cache so the collider radius,
    // visual mesh, HP, and tags come from the TOML rather than hard-coded values.
    let config_cache = crate::config_cache::get_config_cache();
    let entity_config = config_cache.get(&spawn.config_path);
    let collider_radius = entity_config
        .and_then(|c| c.collider.as_ref())
        .map(|c| c.radius)
        .unwrap_or(2.0);
    let max_hp = entity_config
        .and_then(|c| c.hull.as_ref())
        .map(|h| {
            if h.hull_integrity > 0.0 {
                h.hull_integrity
            } else {
                30.0
            }
        })
        .unwrap_or(30.0);
    let snapshot_tags = entity_config
        .map(|c| c.tags.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| vec!["asteroid".into()]);
    // Radar appearance comes straight from the rock's own TOML, exactly like
    // collider/hull/tags above. Cosmetic variants have no [radar_appearance]
    // section at all, so these stay None and the rock never appears on radar.
    let radar_appearance = entity_config.and_then(|c| c.radar_appearance.as_ref());
    let radar_icon = radar_appearance.and_then(|r| r.icon.clone());
    let radar_colour = radar_appearance.and_then(|r| {
        r.colour
            .as_ref()
            .filter(|c| c.len() >= 3)
            .map(|c| [c[0], c[1], c[2]])
    });
    let radar_size = radar_appearance.and_then(|r| r.size);

    // Derived from the cell, not drawn at random. Everything else about a
    // streamed rock — whether it exists, where it sits, how it is rotated — is
    // already a pure function of `(field_idx, cell)` (Key Constraint 8), and a
    // random uuid was the one thing making two identical runs report different
    // `damage_by_ship` keys once a torpedo hit one. Deliberately independent of
    // the `SimRng` master seed: the rock's own identity does not vary with it,
    // and threading the resource through here would mean plumbing it into the
    // whole streaming spawner.
    let uuid = deterministic_cell_uuid(field_idx, cell_gx, cell_gz, slot_x, slot_z);

    // Apply anchor offset as a pure post-seed translation. Seeds and
    // (cell_gx, cell_gz) remain anchor-relative; only the final world
    // position is translated. y is left unchanged (anchor is XZ-only).
    let world_x = spawn.x + anchor_offset[0];
    let world_z = spawn.z + anchor_offset[2];

    // Deterministic random rotation seeded from field+cell coordinates.
    let rotation = if let Some(max_deg) = random_rotation {
        use rand::SeedableRng;
        let rot_seed = {
            let mut s = field_idx as u64;
            s = s.wrapping_mul(2654435761);
            s = s.wrapping_add(cell_gx as u64);
            s = s.wrapping_mul(2654435761);
            s = s.wrapping_add(cell_gz as u64);
            s = s.wrapping_add(0xCAFE_BABE_1337_0000);
            s
        };
        let mut rng = rand::rngs::StdRng::seed_from_u64(rot_seed);
        use rand::Rng;
        let to_rad = std::f32::consts::PI / 180.0;
        let pitch = (rng.random::<f32>() * 2.0 - 1.0) * max_deg[0] * to_rad;
        let roll = (rng.random::<f32>() * 2.0 - 1.0) * max_deg[1] * to_rad;
        let yaw = (rng.random::<f32>() * 2.0 - 1.0) * max_deg[2] * to_rad;
        bevy::math::Quat::from_euler(bevy::math::EulerRot::XYZ, pitch, yaw, roll)
    } else {
        bevy::math::Quat::IDENTITY
    };

    let asteroid_hull = EntitySystemHull(crate::damage::SystemHull::from_config(&[(
        crate::messages::SystemId("captain".into()),
        max_hp,
    )]));

    // `ColliderSection` alongside the Rapier collider, because two consumers
    // read the radius off the *component* rather than the physics body and got
    // 0.0 from a rock that only had the latter: `handle_collisions`, whose
    // de-overlap then left the ship sitting inside the asteroid, and the AI
    // `WorldSnapshot`, which is how collision *avoidance* learns an obstacle's
    // size. Field asteroids bypass `spawn_entity` (which inserts this for every
    // other entity), so it has to be added by hand here.
    let collider_section = crate::entity_spawner::ColliderSection(
        entity_config.and_then(|c| c.collider.clone()).unwrap_or(
            crate::entity_config::ColliderConfig {
                shape: crate::entity_config::ColliderShape::Ball,
                radius: collider_radius,
                length: 0.0,
            },
        ),
    );

    let mut entity_cmd = commands.spawn((
        Asteroid,
        AsteroidUuid(uuid.clone()),
        AsteroidShieldPierce(shield_pierce),
        FieldOwner(field_entity),
        asteroid_hull,
        collider_section,
        Transform::from_xyz(world_x, spawn.y, world_z).with_rotation(rotation),
        Visibility::default(),
        bevy_rapier3d::prelude::Collider::ball(collider_radius),
        bevy_rapier3d::prelude::RigidBody::Fixed,
    ));

    // Attach MeshSection so render_spawned_entities can add a 3-D visual mesh.
    if let Some(entity_config) = entity_config {
        if let Some(mesh) = &entity_config.mesh {
            entity_cmd.insert(MeshSection(mesh.clone()));
        }
    }

    let entity = entity_cmd.id();

    window.slots[slot_z][slot_x] = Some(AsteroidData {
        uuid: uuid.clone(),
        config_path: spawn.config_path.clone(),
        hp: max_hp as i32,
        max_hp: max_hp as i32,
        y: spawn.y,
    });
    entity_map.0.insert(uuid.clone(), entity);
    world.0.entities.push(EntitySnapshot {
        uuid: uuid.clone(),
        position: Some([world_x, spawn.y, world_z]),
        tags: snapshot_tags,
        radius: Some(collider_radius),
        radar_icon: radar_icon.clone(),
        colour: radar_colour,
        radar_size,
        ..EntitySnapshot::default()
    });

    outbox.0.push((
        Target::All,
        ServerMessage::AsteroidSpawned {
            uuid,
            x: world_x,
            y: spawn.y,
            z: world_z,
            config_path: spawn.config_path,
            max_hp: max_hp as i32,
            current_hp: max_hp as i32,
            radius: collider_radius,
            radar_icon,
            radar_colour,
            radar_size,
        },
    ));
}

/// Clear a single window slot: remove data and despawn the associated entity.
///
/// Window-eviction despawn (issue #613): the asteroid scrolled out of the
/// active window and the client was never told about it (no broadcast), but
/// its UUID may still be sitting in the position/health delta caches from a
/// previous tick, so prune it here too.
fn clear_slot(
    window: &mut AsteroidWindow,
    commands: &mut Commands,
    entity_map: &mut AsteroidEntityMap,
    world: &mut ResMut<WorldResource>,
    positions_cache: &mut crate::core::broadcast::LastBroadcastEntityPositions,
    health_cache: &mut crate::core::broadcast::LastBroadcastEntityHealth,
    slot_x: usize,
    slot_z: usize,
) {
    if let Some(slot) = window
        .slots
        .get_mut(slot_z)
        .and_then(|row| row.get_mut(slot_x))
    {
        if let Some(data) = slot.take() {
            if let Some(&entity) = entity_map.0.get(&data.uuid) {
                commands.entity(entity).try_despawn();
            }
            entity_map.0.remove(&data.uuid);
            world.0.entities.retain(|e| e.uuid != data.uuid);
            crate::core::broadcast::cache_registry::prune(
                positions_cache,
                health_cache,
                std::slice::from_ref(&data.uuid),
            );
        }
    }
    if let Some(entity) = window
        .cosmetic_upper_slots
        .get_mut(slot_z)
        .and_then(|row| row.get_mut(slot_x))
        .and_then(|s| s.take())
    {
        commands.entity(entity).try_despawn();
    }
    if let Some(entity) = window
        .cosmetic_lower_slots
        .get_mut(slot_z)
        .and_then(|row| row.get_mut(slot_x))
        .and_then(|s| s.take())
    {
        commands.entity(entity).try_despawn();
    }
}

/// Spawn a single cosmetic asteroid entity (no hull, no UUID tracking).
/// Returns the spawned `Entity` so the caller can store it in a cosmetic slot.
///
/// Deliberately **not** physical. These rocks are set dressing: they carry no
/// UUID, so they can never reach the AI `WorldSnapshot` and collision avoidance
/// is structurally blind to them. Giving them a Rapier body meant ships took
/// real collision damage from an obstacle no pilot — human or AI — was given
/// any way to see coming. Field asteroids (`spawn_asteroid_entity`) remain
/// solid; those are the ones you are meant to hit.
fn spawn_cosmetic_entity(
    commands: &mut Commands,
    spawn: &crate::asteroid_spawner::AsteroidSpawn,
    y: f32,
    anchor_offset: [f32; 3],
) -> Entity {
    let config_cache = crate::config_cache::get_config_cache();
    let entity_config = config_cache.get(&spawn.config_path);

    let mut entity_cmd = commands.spawn((
        Transform::from_xyz(spawn.x + anchor_offset[0], y, spawn.z + anchor_offset[2]),
        Visibility::default(),
    ));

    if let Some(cfg) = entity_config {
        if let Some(mesh) = &cfg.mesh {
            entity_cmd.insert(MeshSection(mesh.clone()));
        }
    }

    entity_cmd.id()
}

/// Evaluate and spawn cosmetic asteroids (upper and lower) for a single grid cell.
/// Uses seed offsets that are independent from the gameplay seed to avoid overlap.
fn try_spawn_cosmetic_cell(
    commands: &mut Commands,
    window: &mut AsteroidWindow,
    cell_gx: i32,
    cell_gz: i32,
    slot_x: usize,
    slot_z: usize,
    field_idx: usize,
    grid: &crate::entity_config::GridConfig,
    inner_radius: f32,
    outer_radius: f32,
    cosmetic_type_paths: &[String],
    shape: Option<crate::entity_config::AsteroidFieldShape>,
    anchor_offset: [f32; 3],
) {
    if cosmetic_type_paths.is_empty() {
        return;
    }

    if !cell_in_field(
        cell_gx,
        cell_gz,
        grid.resolution,
        inner_radius,
        outer_radius,
        shape,
    ) {
        return;
    }

    // Upper layer — large seed offset keeps this independent from gameplay seeds.
    if window.cosmetic_upper_slots[slot_z][slot_x].is_none() {
        if let Some(spawn) = eval_cell(
            field_idx as u64 + 0x0001_0000_0000,
            cell_gx,
            cell_gz,
            grid,
            inner_radius,
            outer_radius,
            &[],
            cosmetic_type_paths,
        ) {
            let entity = spawn_cosmetic_entity(commands, &spawn, spawn.y, anchor_offset);
            window.cosmetic_upper_slots[slot_z][slot_x] = Some(entity);
        }
    }

    // Lower layer — separate seed offset so upper and lower differ.
    if window.cosmetic_lower_slots[slot_z][slot_x].is_none() {
        if let Some(spawn) = eval_cell(
            field_idx as u64 + 0x0002_0000_0000,
            cell_gx,
            cell_gz,
            grid,
            inner_radius,
            outer_radius,
            &[],
            cosmetic_type_paths,
        ) {
            let entity = spawn_cosmetic_entity(commands, &spawn, -spawn.y, anchor_offset);
            window.cosmetic_lower_slots[slot_z][slot_x] = Some(entity);
        }
    }
}

// ── Plugin ──────────────────────────────────────────────────────────────

/// Plugin for the ring-buffer asteroid window lifecycle.
pub struct AsteroidLifecyclePlugin;

impl Plugin for AsteroidLifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AsteroidEntityMap>()
            .init_resource::<crate::core::broadcast::LastBroadcastEntityPositions>()
            .init_resource::<crate::core::broadcast::LastBroadcastEntityHealth>()
            .add_systems(
                Update,
                (
                    attach_field_components,
                    check_destroyed_asteroids,
                    update_asteroid_window,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::{AsteroidFieldConfig, GridConfig};
    use crate::entity_spawner::AsteroidFieldSection;
    use crate::lobby::OutboundMessage;
    use crate::simulation::SimOutbox;

    /// A rock's uuid keys its row in the headless report's `damage_by_ship`
    /// ledger, so two distinct rocks sharing one is silent data corruption
    /// rather than a cosmetic clash. The first packed implementation aliased
    /// two ways and both are pinned here: `cell_gx` values differing only in
    /// the top two bits of their low byte (the bits the v4 variant stamp
    /// overwrites), and the `(field_idx, slot_x)` swap that two overlapping
    /// fields hit for real.
    #[test]
    fn cell_uuids_are_unique_across_the_identifying_tuple() {
        let mut seen: std::collections::HashMap<String, (usize, i32, i32, usize, usize)> =
            std::collections::HashMap::new();
        for field_idx in 0..4usize {
            for cell_gx in [-193, -1, 0, 1, 64, 65, 128, 192, 256] {
                for cell_gz in [-64, 0, 3, 64, 128] {
                    for slot_x in 0..4usize {
                        for slot_z in 0..4usize {
                            let key = (field_idx, cell_gx, cell_gz, slot_x, slot_z);
                            let uuid = deterministic_cell_uuid(
                                field_idx, cell_gx, cell_gz, slot_x, slot_z,
                            );
                            if let Some(other) = seen.insert(uuid.clone(), key) {
                                panic!("{key:?} and {other:?} share uuid {uuid}");
                            }
                        }
                    }
                }
            }
        }

        // Same rock, same name — the identity has to be stable, not merely
        // collision-free, or a respawned asteroid changes ledger row.
        assert_eq!(
            deterministic_cell_uuid(1, 7, -9, 2, 3),
            deterministic_cell_uuid(1, 7, -9, 2, 3)
        );
        // v4 formatting survives the hashing.
        let parsed =
            uuid::Uuid::parse_str(&deterministic_cell_uuid(0, 0, 0, 0, 0)).expect("valid uuid");
        assert_eq!(parsed.get_version_num(), 4);
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.add_message::<OutboundMessage>();
        app.init_resource::<SimOutbox>();
        app.init_resource::<AsteroidEntityMap>();
        app.init_resource::<WorldResource>();
        app.init_resource::<crate::core::broadcast::LastBroadcastEntityPositions>();
        app.init_resource::<crate::core::broadcast::LastBroadcastEntityHealth>();
        // Spawn a LocalShip entity with ShipPhysics so update_asteroid_window can query it.
        app.world_mut().spawn((
            crate::simulation::LocalShip,
            bevy::prelude::Transform::default(),
            crate::ship_state::ShipPhysics::default(),
        ));
        // (#475) Multi-field refactor: AsteroidWindow + PlayerGridPosition
        // are per-field components, attached by `attach_field_components`
        // to each `AsteroidFieldSection` entity. The systems are chained
        // so attach runs before update each frame.
        app.add_systems(
            Update,
            (attach_field_components, update_asteroid_window).chain(),
        );
        app
    }

    /// Helper: shallow-copy the first field entity's `AsteroidWindow`
    /// component for assertions. Per-field component replaces the old
    /// global resource (#475).
    fn first_field_window(app: &mut App) -> AsteroidWindow {
        let mut q = app
            .world_mut()
            .query::<(&AsteroidFieldSection, &AsteroidWindow)>();
        let (_, window) = q
            .iter(app.world())
            .next()
            .expect("at least one field entity must exist");
        AsteroidWindow {
            slots: window.slots.clone(),
            cosmetic_upper_slots: window.cosmetic_upper_slots.clone(),
            cosmetic_lower_slots: window.cosmetic_lower_slots.clone(),
            arena_gx: window.arena_gx,
            arena_gz: window.arena_gz,
            despawn_cells: window.despawn_cells,
            spawn_cells: window.spawn_cells,
            resolution: window.resolution,
            field_idx: window.field_idx,
            inner_radius: window.inner_radius,
            outer_radius: window.outer_radius,
            shape: window.shape,
            anchor_offset: window.anchor_offset,
            needs_init: window.needs_init,
        }
    }

    fn set_ship_pos(app: &mut App, x: f32, z: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_state::ShipPhysics, With<crate::simulation::LocalShip>>();
        let mut p = q
            .single_mut(app.world_mut())
            .expect("expected LocalShip with ShipPhysics");
        p.x = x;
        p.z = z;
    }

    fn grid(resolution: f32) -> GridConfig {
        GridConfig {
            resolution,
            fill_gameplay: 0.0,
            fill_cosmetic: 0.0,
            uniformity: 0.0,
            noise_freq: 0.02,
            noise_octaves: 3,
            density_noise_freq: 0.01,
            density_noise_octaves: 2,
            jitter: 0.0,
            cosmetic_y_offset: 0.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 2,
            despawn_cells: 3,
        }
    }

    fn field(grid_resolution: f32) -> AsteroidFieldConfig {
        AsteroidFieldConfig {
            inner_radius: 100.0,
            outer_radius: 200.0,
            density: 0.0,
            spawn_distance: 150.0,
            despawn_distance: 250.0,
            asteroid_type_paths: vec!["asteroid_small.toml".to_string()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(grid(grid_resolution)),
            shield_pierce: 0.0,
            shape: None,
            anchor: None,
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        }
    }

    #[test]
    fn window_initialises_from_spawned_asteroid_field_section() {
        let mut app = test_app();
        // WorldResource is init'd by test_app. The system should find the field.
        app.world_mut()
            .spawn((AsteroidFieldSection(field(15.0)), Transform::default()));
        app.update();

        let window = first_field_window(&mut app);
        assert_eq!(
            window.resolution, 15.0,
            "window.resolution should be sourced from the spawned AsteroidFieldSection"
        );
        assert_eq!(window.inner_radius, 100.0);
        assert_eq!(window.outer_radius, 200.0);
    }

    #[test]
    fn window_does_nothing_with_no_field_entity() {
        // (#475) With no field entity, no AsteroidWindow component exists.
        // The system is a no-op — verify by counting components.
        let mut app = test_app();
        app.update();
        let mut q = app.world_mut().query::<&AsteroidWindow>();
        let count = q.iter(app.world()).count();
        assert_eq!(
            count, 0,
            "no AsteroidFieldSection entity → no AsteroidWindow component"
        );
    }

    #[test]
    fn asteroid_field_shield_pierce_defaults_to_zero_in_toml() {
        // Pre-#414 behaviour: asteroid impacts are fully absorbed by shields.
        // A TOML file that does not mention shield_pierce must continue to
        // behave that way after the field is added.
        let toml = r#"
inner_radius = 100.0
outer_radius = 200.0
density = 0.5
asteroid_type_paths = ["x.toml"]
"#;
        let cfg: AsteroidFieldConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.shield_pierce, 0.0);
    }

    #[test]
    fn asteroid_field_shield_pierce_parses_when_present_in_toml() {
        let toml = r#"
inner_radius = 100.0
outer_radius = 200.0
density = 0.5
asteroid_type_paths = ["x.toml"]
shield_pierce = 0.4
"#;
        let cfg: AsteroidFieldConfig = toml::from_str(toml).unwrap();
        assert!((cfg.shield_pierce - 0.4).abs() < 1e-6);
    }

    #[test]
    fn window_propagates_torus_shape_from_field_section() {
        // The streaming lifecycle must thread the field's `shape` into the
        // AsteroidWindow resource so eligibility checks downstream see it.
        let mut app = test_app();
        let mut f = field(15.0);
        f.shape = Some(crate::entity_config::AsteroidFieldShape::Torus);
        app.world_mut()
            .spawn((AsteroidFieldSection(f), Transform::default()));
        app.update();

        let window = first_field_window(&mut app);
        assert_eq!(
            window.shape,
            Some(crate::entity_config::AsteroidFieldShape::Torus),
            "window.shape must be sourced from the spawned AsteroidFieldSection",
        );
        assert_eq!(window.inner_radius, 100.0);
        assert_eq!(window.outer_radius, 200.0);
    }

    #[test]
    fn window_shape_defaults_to_none_when_field_omits_it() {
        // Back-compat: an AsteroidFieldSection with `shape = None` must
        // leave the window in legacy (centre-distance) mode.
        let mut app = test_app();
        app.world_mut()
            .spawn((AsteroidFieldSection(field(15.0)), Transform::default()));
        app.update();

        let window = first_field_window(&mut app);
        assert!(window.shape.is_none(), "default field has no shape");
    }

    #[test]
    fn streaming_full_rebuild_with_torus_keeps_positions_near_annulus() {
        // Drive a full rebuild and assert that every spawned gameplay
        // asteroid sits near the annulus [inner_radius, outer_radius].
        // Torus eligibility admits cells whose bbox overlaps the annulus,
        // so positions may extend up to one cell diagonal beyond either
        // boundary. After the player departs, all asteroids must despawn.
        let mut app = test_app();
        // Field with a dense fill so cells actually spawn.
        let res = 15.0f32;
        let grid_cfg = GridConfig {
            resolution: res,
            fill_gameplay: 0.0, // admit every cell that passes density
            fill_cosmetic: 1.0,
            uniformity: 0.0,
            noise_freq: 0.02,
            noise_octaves: 1,
            density_noise_freq: 0.01,
            density_noise_octaves: 1,
            jitter: 0.0,
            cosmetic_y_offset: 0.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 20,
            despawn_cells: 22,
        };
        let f = AsteroidFieldConfig {
            inner_radius: 100.0,
            outer_radius: 200.0,
            density: 0.0,
            spawn_distance: 150.0,
            despawn_distance: 250.0,
            asteroid_type_paths: vec!["asteroid_small.toml".to_string()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(grid_cfg),
            shield_pierce: 0.0,
            shape: Some(crate::entity_config::AsteroidFieldShape::Torus),
            anchor: None,
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        };
        // Anchor the player on the belt so the spawn window covers it.
        set_ship_pos(&mut app, 150.0, 0.0);
        app.world_mut()
            .spawn((AsteroidFieldSection(f), Transform::default()));
        app.update();

        let tol = res * std::f32::consts::SQRT_2;
        let mut q = app.world_mut().query::<(&Transform, &Asteroid)>();
        let mut count = 0;
        for (t, _) in q.iter(app.world()) {
            let d = (t.translation.x.powi(2) + t.translation.z.powi(2)).sqrt();
            assert!(
                d >= 100.0 - tol && d <= 200.0 + tol,
                "asteroid at ({}, {}) dist={} outside [{}, {}]",
                t.translation.x,
                t.translation.z,
                d,
                100.0 - tol,
                200.0 + tol,
            );
            count += 1;
        }
        assert!(count > 0, "no asteroids spawned — test set-up is wrong");

        // Move the player far away → full rebuild should clear them.
        set_ship_pos(&mut app, 10_000.0, 10_000.0);
        app.update();
        let mut q = app.world_mut().query::<&Asteroid>();
        let remaining = q.iter(app.world()).count();
        assert_eq!(
            remaining, 0,
            "all asteroids must despawn after departing the belt"
        );
    }

    /// (#475) Multi-field: spawn two fields with disjoint annuli centred at
    /// the origin and assert both produce asteroids in their own bands when
    /// the player sits between them.
    #[test]
    fn two_fields_with_disjoint_annuli_each_spawn_asteroids() {
        let mut app = test_app();

        let grid_cfg = GridConfig {
            resolution: 25.0,
            fill_gameplay: 0.0,
            fill_cosmetic: 1.0,
            uniformity: 0.0,
            noise_freq: 0.02,
            noise_octaves: 1,
            density_noise_freq: 0.01,
            density_noise_octaves: 1,
            jitter: 0.0,
            cosmetic_y_offset: 0.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 30,
            despawn_cells: 32,
        };

        let inner = AsteroidFieldConfig {
            inner_radius: 100.0,
            outer_radius: 150.0,
            density: 0.0,
            spawn_distance: 250.0,
            despawn_distance: 350.0,
            asteroid_type_paths: vec!["asteroid_small.toml".to_string()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(grid_cfg.clone()),
            shield_pierce: 0.0,
            shape: Some(crate::entity_config::AsteroidFieldShape::Torus),
            anchor: None,
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        };
        let outer = AsteroidFieldConfig {
            inner_radius: 400.0,
            outer_radius: 500.0,
            density: 0.0,
            spawn_distance: 600.0,
            despawn_distance: 700.0,
            asteroid_type_paths: vec!["asteroid_small.toml".to_string()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(grid_cfg),
            shield_pierce: 0.0,
            shape: Some(crate::entity_config::AsteroidFieldShape::Torus),
            anchor: None,
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        };

        // Position the player between the two annuli so both spawn windows
        // overlap their respective belts.
        set_ship_pos(&mut app, 250.0, 0.0);
        app.world_mut()
            .spawn((AsteroidFieldSection(inner), Transform::default()));
        app.world_mut()
            .spawn((AsteroidFieldSection(outer), Transform::default()));
        app.update();

        // Both fields must have an AsteroidWindow component with distinct
        // FieldIndex values.
        let mut idx_q = app
            .world_mut()
            .query::<(&AsteroidFieldSection, &FieldIndex, &AsteroidWindow)>();
        let indices: Vec<usize> = idx_q.iter(app.world()).map(|(_, i, _)| i.0).collect();
        assert_eq!(
            indices.len(),
            2,
            "both field entities must carry FieldIndex + AsteroidWindow"
        );
        let mut sorted = indices.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1], "FieldIndex must be 0 and 1 (stable)");

        // Each spawned asteroid must lie within either the inner annulus
        // [100, 150] or the outer annulus [400, 500] (with the cell-diagonal
        // tolerance Torus eligibility introduces).
        let tol = 25.0 * std::f32::consts::SQRT_2;
        let mut asteroid_q = app.world_mut().query::<(&Transform, &Asteroid)>();
        let mut inner_count = 0;
        let mut outer_count = 0;
        for (t, _) in asteroid_q.iter(app.world()) {
            let d = (t.translation.x.powi(2) + t.translation.z.powi(2)).sqrt();
            if d >= 100.0 - tol && d <= 150.0 + tol {
                inner_count += 1;
            } else if d >= 400.0 - tol && d <= 500.0 + tol {
                outer_count += 1;
            } else {
                panic!(
                    "asteroid at dist={} fell outside both annuli [100..150] and [400..500]",
                    d
                );
            }
        }
        assert!(
            inner_count > 0,
            "inner belt must have spawned at least one asteroid (got 0)"
        );
        assert!(
            outer_count > 0,
            "outer belt must have spawned at least one asteroid \
             — proves the multi-field refactor (#475)"
        );
    }
}
