// Asteroid lifecycle managed by a ring-buffer window.
//
// This module provides:
// - AsteroidWindow resource: the 2D ring-buffer tracking which lattice cells
//   of the world's ONE composed asteroid field are loaded
// - AsteroidEntityMap resource: UUID → Entity lookup for despawning (global,
//   keyed by globally-unique asteroid UUID)
// - check_destroyed_asteroids: despawns asteroids with HP ≤ 0, clears slot
// - update_asteroid_window: composes every `AsteroidFieldSection` entity into
//   one weighted density field and drives spawn/despawn for it based on
//   player movement
//
// History: pre-#475 the window was a global resource; #475 made it a
// per-field component so multiple fields could stream concurrently — which
// double-spawned rocks wherever two fields overlapped, because each field
// evaluated the shared space independently. #913 replaces the per-field
// windows with a single window over the composed density field
// (`asteroid_spawner::eval_cell_composed`): authored fields stay separate
// TOML entities, but every lattice cell is evaluated exactly once with all
// covering fields blended by `[asteroid_field] weight`.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::asteroid_spawner::{
    composed_lattice, eval_cell_composed, ComposedLattice, ComposedLayer, FieldContribution,
};
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

// ── Resources ────────────────────────────────────────────────────────────

/// The 2D ring-buffer window for the world's one composed asteroid field.
///
/// Indexed as [slot_z][slot_x] where (despawn_cells, despawn_cells) is the
/// player center. The lattice is world-anchored: cell `(gx, gz)` covers the
/// world position `(gx * resolution, gz * resolution)`. Per-field anchors
/// are applied inside the composed evaluator, not here.
#[derive(Resource)]
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
    /// Lattice resolution (world units per cell), derived from the composed
    /// contributions (`asteroid_spawner::composed_lattice`).
    pub resolution: f32,
    /// Player's lattice cell from the previous tick.
    pub player_grid: Option<(i32, i32)>,
    /// Fingerprint of the contribution set the current window contents were
    /// built from. When the live set of `AsteroidFieldSection` entities
    /// stops matching (a layered world loads or unloads a field), the next
    /// tick full-rebuilds against the new composition.
    pub composition_key: u64,
    /// `true` until the first `update_asteroid_window` tick has run a
    /// `full_rebuild`; the window resource exists before the player position
    /// has been observed.
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
            player_grid: None,
            composition_key: 0,
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

/// Maps asteroid UUID to spawned Entity for despawn and slot lookup.
#[derive(Resource, Default)]
pub struct AsteroidEntityMap(pub HashMap<String, Entity>);

// ── Systems ─────────────────────────────────────────────────────────────

/// Check for destroyed asteroids, clear their window slot, broadcast, and
/// despawn the entity.
///
/// With one composed window per world (#913) every streamed rock belongs to
/// the same window; slot clearing is guarded by UUID equality so asteroids
/// spawned outside the window (hand-placed test rocks) despawn correctly
/// without disturbing a slot they never owned.
pub fn check_destroyed_asteroids(
    mut commands: Commands,
    mut window: ResMut<AsteroidWindow>,
    mut entity_map: ResMut<AsteroidEntityMap>,
    mut world: ResMut<WorldResource>,
    asteroid_query: Query<(Entity, &Transform, &AsteroidUuid, &EntitySystemHull)>,
    mut outbox: ResMut<SimOutbox>,
    mut positions_cache: ResMut<crate::core::broadcast::LastBroadcastEntityPositions>,
    mut health_cache: ResMut<crate::core::broadcast::LastBroadcastEntityHealth>,
) {
    for (entity, transform, uuid, hull_comp) in asteroid_query.iter() {
        if !hull_comp.0.is_destroyed() {
            continue;
        }
        let (cell_gx, cell_gz) = compute_player_grid_cell(
            transform.translation.x,
            transform.translation.z,
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

/// Update the composed asteroid field's ring-buffer window when the player
/// moves. Runs every frame; a no-op if the player has not crossed a lattice
/// cell boundary since the previous tick and the authored field set is
/// unchanged.
///
/// Every `AsteroidFieldSection` entity contributes to ONE evaluator: the
/// contributions are gathered each tick (in spawn order, which follows the
/// world TOML's author order), fingerprinted, and any change to the set —
/// a layered world loading or unloading a field entity — forces a full
/// rebuild against the new composition.
pub fn update_asteroid_window(
    mut commands: Commands,
    physics_q: Query<&ShipPhysics, With<crate::simulation::LocalShip>>,
    fields: Query<(Entity, &AsteroidFieldSection)>,
    mut window: ResMut<AsteroidWindow>,
    mut world: ResMut<WorldResource>,
    mut entity_map: ResMut<AsteroidEntityMap>,
    mut outbox: ResMut<SimOutbox>,
    mut positions_cache: ResMut<crate::core::broadcast::LastBroadcastEntityPositions>,
    mut health_cache: ResMut<crate::core::broadcast::LastBroadcastEntityHealth>,
) {
    // Deterministic composition order: Bevy allocates Entity ids in spawn
    // order and world spawning walks the TOML in author order, so sorting by
    // Entity reproduces the authored field order run over run.
    let mut sections: Vec<(Entity, &AsteroidFieldSection)> = fields.iter().collect();
    sections.sort_by_key(|(e, _)| *e);
    let contributions: Vec<FieldContribution> = sections
        .iter()
        .filter_map(|(_, s)| FieldContribution::from_config(&s.0))
        .collect();

    let key = composition_key(&contributions);

    let Some(lattice) = composed_lattice(&contributions) else {
        // No streaming fields — despawn anything a previous composition left.
        if window.composition_key != key {
            clear_window_contents(
                &mut commands,
                &mut window,
                &mut entity_map,
                &mut world,
                &mut positions_cache,
                &mut health_cache,
            );
            window.player_grid = None;
            window.needs_init = true;
            window.composition_key = key;
        }
        return;
    };

    let physics = physics_q.single().ok().copied().unwrap_or_default();
    let (gx, gz) = compute_player_grid_cell(physics.x, physics.z, lattice.resolution);

    let needs_init =
        window.needs_init || window.composition_key != key || window.player_grid.is_none();
    let (old_gx, old_gz) = window.player_grid.unwrap_or((gx, gz));

    if !needs_init && old_gx == gx && old_gz == gz {
        return;
    }

    let delta = eval_on_player_move(
        old_gx,
        old_gz,
        gx,
        gz,
        lattice.spawn_cells,
        lattice.despawn_cells,
    );

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
            &contributions,
            &lattice,
        );
        window.needs_init = false;
        window.composition_key = key;
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
                    &contributions,
                    lattice.resolution,
                );
                try_spawn_cosmetic_cell(
                    &mut commands,
                    &mut window,
                    *cell_gx,
                    *cell_gz,
                    sx,
                    sz,
                    &contributions,
                    lattice.resolution,
                );
            }
        }
    }

    window.player_grid = Some((gx, gz));
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Order-sensitive fingerprint of the live contribution set, used to detect
/// mid-run composition changes (world layers loading or unloading a field).
/// Debug formatting is stable within a run, which is all the key needs —
/// it never has to survive a process restart.
fn composition_key(fields: &[FieldContribution]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{fields:?}").bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Despawn every entity the window currently tracks — gameplay rocks via the
/// global map, cosmetics via their slot handles — and clear every slot.
/// Slot dimensions are left alone; `full_rebuild` resizes afterwards.
fn clear_window_contents(
    commands: &mut Commands,
    window: &mut AsteroidWindow,
    entity_map: &mut AsteroidEntityMap,
    world: &mut ResMut<WorldResource>,
    positions_cache: &mut crate::core::broadcast::LastBroadcastEntityPositions,
    health_cache: &mut crate::core::broadcast::LastBroadcastEntityHealth,
) {
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

    for row in window.slots.iter_mut() {
        for slot in row.iter_mut() {
            *slot = None;
        }
    }
    for row in window.cosmetic_upper_slots.iter_mut() {
        for slot in row.iter_mut() {
            if let Some(entity) = slot.take() {
                commands.entity(entity).try_despawn();
            }
        }
    }
    for row in window.cosmetic_lower_slots.iter_mut() {
        for slot in row.iter_mut() {
            if let Some(entity) = slot.take() {
                commands.entity(entity).try_despawn();
            }
        }
    }
}

/// Full rebuild: despawn all tracked entities, clear the window, size it to
/// the composed lattice, and re-evaluate every cell within the spawn window
/// against the composed density field.
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
    contributions: &[FieldContribution],
    lattice: &ComposedLattice,
) {
    clear_window_contents(
        commands,
        window,
        entity_map,
        world,
        positions_cache,
        health_cache,
    );

    // Sync window extents from the composed lattice so TOML-specified values
    // take effect.
    window.spawn_cells = lattice.spawn_cells;
    window.despawn_cells = lattice.despawn_cells;

    let size = (2 * window.despawn_cells + 1) as usize;
    window.slots = vec![vec![None; size]; size];
    window.cosmetic_upper_slots = vec![vec![None; size]; size];
    window.cosmetic_lower_slots = vec![vec![None; size]; size];
    window.arena_gx = gx;
    window.arena_gz = gz;
    window.resolution = lattice.resolution;

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
                    contributions,
                    lattice.resolution,
                );
                try_spawn_cosmetic_cell(
                    commands,
                    window,
                    cx,
                    cz,
                    sx,
                    sz,
                    contributions,
                    lattice.resolution,
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
///
/// Since #913 there is exactly one composed field per world, so callers pin
/// `field_idx` to 0; the parameter stays so historical uuids for field 0
/// remain unchanged and the aliasing regression test keeps its coverage.
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

/// SplitMix64's finaliser. Local twin of the one [`crate::sim_rng`] reaches
/// through `vellum_rng::split_mix_64`: this path deliberately does not depend
/// on the master seed (a rock's identity is a pure function of its cell), so it
/// does not reach for that module's state — and keeping the constants here
/// rather than calling the crate's copy keeps the asteroid field's recorded
/// values independent of a fleet-wide RNG decision.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Evaluate a single cell of the composed density field for gameplay
/// asteroid spawning. If the cell passes the weighted density check, spawn
/// a gameplay asteroid entity and populate the window slot. The selected
/// contribution (the field the composed evaluator picked by weight) supplies
/// the spawn tuning: shield pierce and random rotation.
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
    contributions: &[FieldContribution],
    lattice_resolution: f32,
) {
    if window.slots[slot_z][slot_x].is_some() {
        return;
    }

    let Some((spawn, sel_idx)) = eval_cell_composed(
        contributions,
        lattice_resolution,
        cell_gx,
        cell_gz,
        ComposedLayer::Gameplay,
    ) else {
        return;
    };
    let selected = &contributions[sel_idx];

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
    // already a pure function of the cell (Key Constraint 8), and a random
    // uuid was the one thing making two identical runs report different
    // `damage_by_ship` keys once a torpedo hit one. Deliberately independent of
    // the `SimRng` master seed: the rock's own identity does not vary with it,
    // and threading the resource through here would mean plumbing it into the
    // whole streaming spawner. `field_idx` is pinned to 0 — one composed
    // field per world.
    let uuid = deterministic_cell_uuid(0, cell_gx, cell_gz, slot_x, slot_z);

    // The composed evaluator returns world-space positions (per-field anchors
    // are applied inside it).
    let world_x = spawn.x;
    let world_z = spawn.z;

    // Deterministic random rotation seeded from the cell coordinates, using
    // the selected contribution's authored maxima. Same local-seeding policy
    // as the density evaluator; the leading 0 is the pinned composed-field
    // index (formerly the per-field index).
    let rotation = if let Some(max_deg) = selected.random_rotation {
        use rand::SeedableRng;
        let rot_seed = {
            let mut s: u64 = 0;
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
        AsteroidShieldPierce(selected.shield_pierce),
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
) -> Entity {
    let config_cache = crate::config_cache::get_config_cache();
    let entity_config = config_cache.get(&spawn.config_path);

    let mut entity_cmd = commands.spawn((
        Transform::from_xyz(spawn.x, y, spawn.z),
        Visibility::default(),
    ));

    if let Some(cfg) = entity_config {
        if let Some(mesh) = &cfg.mesh {
            entity_cmd.insert(MeshSection(mesh.clone()));
        }
    }

    entity_cmd.id()
}

/// Evaluate and spawn cosmetic asteroids (upper and lower) for a single
/// lattice cell of the composed field. The per-layer seed salts keep the
/// two cosmetic layers independent of each other and of the gameplay layer.
#[allow(clippy::too_many_arguments)]
fn try_spawn_cosmetic_cell(
    commands: &mut Commands,
    window: &mut AsteroidWindow,
    cell_gx: i32,
    cell_gz: i32,
    slot_x: usize,
    slot_z: usize,
    contributions: &[FieldContribution],
    lattice_resolution: f32,
) {
    if window.cosmetic_upper_slots[slot_z][slot_x].is_none() {
        if let Some((spawn, _)) = eval_cell_composed(
            contributions,
            lattice_resolution,
            cell_gx,
            cell_gz,
            ComposedLayer::CosmeticUpper,
        ) {
            let entity = spawn_cosmetic_entity(commands, &spawn, spawn.y);
            window.cosmetic_upper_slots[slot_z][slot_x] = Some(entity);
        }
    }

    if window.cosmetic_lower_slots[slot_z][slot_x].is_none() {
        if let Some((spawn, _)) = eval_cell_composed(
            contributions,
            lattice_resolution,
            cell_gx,
            cell_gz,
            ComposedLayer::CosmeticLower,
        ) {
            let entity = spawn_cosmetic_entity(commands, &spawn, -spawn.y);
            window.cosmetic_lower_slots[slot_z][slot_x] = Some(entity);
        }
    }
}

// ── Plugin ──────────────────────────────────────────────────────────────

/// Plugin for the ring-buffer asteroid window lifecycle.
pub struct AsteroidLifecyclePlugin;

impl Plugin for AsteroidLifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AsteroidWindow>()
            .init_resource::<AsteroidEntityMap>()
            .init_resource::<crate::core::broadcast::LastBroadcastEntityPositions>()
            .init_resource::<crate::core::broadcast::LastBroadcastEntityHealth>()
            // `FixedUpdate` (issue #895): the window tracks the ship the sim
            // moves, spawns/despawns are sim state, and destroyed-asteroid
            // respawn bookkeeping must count in ticks, not frames.
            //
            // `.before(PhysicsSet::SyncBackend)` (issue #896 follow-up): both
            // systems spawn asteroid colliders via `Commands`, and now that
            // rapier's `PhysicsSet` chain shares `FixedUpdate` with the rest of
            // the sim (see `server_app::register_physics`), those two sets are
            // otherwise free to interleave in either order. Left unordered, a
            // collider spawned here lands before rapier's `SyncBackend` copies
            // it in — or a tick late — depending on how the multithreaded
            // executor happens to schedule `ApplyDeferred` that run. Ordering
            // before `SyncBackend` removes that ambiguity the same way
            // `register_physics` orders `sync_ship_position` before it: a
            // spawned rock is visible to rapier the same tick it appears.
            .add_systems(
                FixedUpdate,
                (check_destroyed_asteroids, update_asteroid_window)
                    .chain()
                    .before(bevy_rapier3d::plugin::PhysicsSet::SyncBackend),
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
        app.init_resource::<AsteroidWindow>();
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
        // (#913) One composed window per world: every AsteroidFieldSection
        // entity feeds the same evaluator and the same window resource.
        app.add_systems(Update, update_asteroid_window);
        app
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
            weight: 1.0,
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

    /// Helper: an annulus field with a torus shape and a dense fill, so
    /// every eligible cell spawns and assertions are exact.
    fn torus_field(
        inner_radius: f32,
        outer_radius: f32,
        resolution: f32,
        spawn_cells: u32,
        despawn_cells: u32,
    ) -> AsteroidFieldConfig {
        AsteroidFieldConfig {
            inner_radius,
            outer_radius,
            density: 0.0,
            weight: 1.0,
            spawn_distance: 150.0,
            despawn_distance: 250.0,
            asteroid_type_paths: vec!["asteroid_small.toml".to_string()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(GridConfig {
                resolution,
                fill_gameplay: 0.0, // admit every covered cell
                fill_cosmetic: 1.0,
                uniformity: 0.0,
                noise_freq: 0.02,
                noise_octaves: 1,
                density_noise_freq: 0.01,
                density_noise_octaves: 1,
                jitter: 0.0,
                cosmetic_y_offset: 0.0,
                gameplay_y_variance: 0.0,
                spawn_cells,
                despawn_cells,
            }),
            shield_pierce: 0.0,
            shape: Some(crate::entity_config::AsteroidFieldShape::Torus),
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

        let window = app.world().resource::<AsteroidWindow>();
        assert_eq!(
            window.resolution, 15.0,
            "window.resolution should be sourced from the composed lattice"
        );
        assert_eq!(window.spawn_cells, 2);
        assert_eq!(window.despawn_cells, 3);
        assert!(!window.needs_init, "first tick must run the full rebuild");
    }

    #[test]
    fn window_does_nothing_with_no_field_entity() {
        // (#913) With no field entity there is nothing to compose; the
        // window resource stays untouched and no asteroid spawns.
        let mut app = test_app();
        app.update();
        let window = app.world().resource::<AsteroidWindow>();
        assert!(
            window.player_grid.is_none(),
            "no AsteroidFieldSection entity → the window never initialises"
        );
        let mut q = app.world_mut().query::<&Asteroid>();
        assert_eq!(q.iter(app.world()).count(), 0);
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
    fn asteroid_field_weight_defaults_to_one_in_toml() {
        // (#913) A field that does not author a weight is an equal partner
        // in the composed density blend.
        let toml = r#"
inner_radius = 100.0
outer_radius = 200.0
density = 0.5
asteroid_type_paths = ["x.toml"]
"#;
        let cfg: AsteroidFieldConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.weight, 1.0);
    }

    #[test]
    fn asteroid_field_weight_parses_when_present_in_toml() {
        let toml = r#"
inner_radius = 100.0
outer_radius = 200.0
density = 0.5
weight = 2.5
asteroid_type_paths = ["x.toml"]
"#;
        let cfg: AsteroidFieldConfig = toml::from_str(toml).unwrap();
        assert!((cfg.weight - 2.5).abs() < 1e-6);
    }

    #[test]
    fn streaming_full_rebuild_with_torus_keeps_positions_near_annulus() {
        // Drive a full rebuild and assert that every spawned gameplay
        // asteroid sits near the annulus [inner_radius, outer_radius].
        // Torus eligibility admits cells whose bbox overlaps the annulus,
        // so positions may extend up to one cell diagonal beyond either
        // boundary. After the player departs, all asteroids must despawn.
        let mut app = test_app();
        let res = 15.0f32;
        let f = torus_field(100.0, 200.0, res, 20, 22);
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

    /// (#475 → #913) Multi-field: two fields with disjoint annuli compose
    /// into one density field whose support is the union of the two bands.
    /// Both bands must produce asteroids when the player sits between them.
    #[test]
    fn two_fields_with_disjoint_annuli_each_spawn_asteroids() {
        let mut app = test_app();

        let inner = torus_field(100.0, 150.0, 25.0, 30, 32);
        let outer = torus_field(400.0, 500.0, 25.0, 30, 32);

        // Position the player between the two annuli so the composed spawn
        // window overlaps both belts.
        set_ship_pos(&mut app, 250.0, 0.0);
        app.world_mut()
            .spawn((AsteroidFieldSection(inner), Transform::default()));
        app.world_mut()
            .spawn((AsteroidFieldSection(outer), Transform::default()));
        app.update();

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
             — the composed field's support is the union of the authored bands"
        );
    }

    /// (#913) The headline regression: two OVERLAPPING fields must not
    /// double-spawn in the overlap band. With the per-field windows each
    /// field evaluated the shared cells independently and both spawned;
    /// the composed evaluator runs each lattice cell exactly once.
    #[test]
    fn overlapping_fields_spawn_each_cell_at_most_once() {
        let mut app = test_app();
        let res = 25.0f32;

        // Annuli [100, 200] and [150, 250] — the band [150, 200] is covered
        // by both. fill 0.0 + jitter 0.0 → every covered cell spawns exactly
        // at its centre, so cell occupancy is exact.
        let a = torus_field(100.0, 200.0, res, 12, 14);
        let b = torus_field(150.0, 250.0, res, 12, 14);

        set_ship_pos(&mut app, 175.0, 0.0);
        app.world_mut()
            .spawn((AsteroidFieldSection(a), Transform::default()));
        app.world_mut()
            .spawn((AsteroidFieldSection(b), Transform::default()));
        app.update();

        let mut q = app.world_mut().query::<(&Transform, &Asteroid)>();
        let mut cells = std::collections::HashSet::new();
        let mut overlap_band_count = 0;
        let mut total = 0;
        for (t, _) in q.iter(app.world()) {
            let cell = (
                (t.translation.x / res).round() as i32,
                (t.translation.z / res).round() as i32,
            );
            assert!(
                cells.insert(cell),
                "cell {cell:?} spawned more than one asteroid — overlapping \
                 fields must compose, not double-spawn"
            );
            let d = (t.translation.x.powi(2) + t.translation.z.powi(2)).sqrt();
            if (160.0..=190.0).contains(&d) {
                overlap_band_count += 1;
            }
            total += 1;
        }
        assert!(total > 0, "no asteroids spawned — test set-up is wrong");
        assert!(
            overlap_band_count > 0,
            "the overlap band [160, 190] must contain asteroids — otherwise \
             this test never exercised the composed path"
        );
    }

    /// (#913) Same authored fields → identical composed field, run over run:
    /// every rock at the same position with the same uuid.
    #[test]
    fn composed_field_is_deterministic_across_runs() {
        let build = || {
            let mut app = test_app();
            let a = torus_field(100.0, 200.0, 25.0, 12, 14);
            let b = torus_field(150.0, 250.0, 25.0, 12, 14);
            set_ship_pos(&mut app, 175.0, 0.0);
            app.world_mut()
                .spawn((AsteroidFieldSection(a), Transform::default()));
            app.world_mut()
                .spawn((AsteroidFieldSection(b), Transform::default()));
            app.update();
            let mut q = app.world_mut().query::<(&Transform, &AsteroidUuid)>();
            let mut rocks: Vec<(String, [i64; 3])> = q
                .iter(app.world())
                .map(|(t, u)| {
                    (
                        u.0.clone(),
                        [
                            (t.translation.x * 1000.0) as i64,
                            (t.translation.y * 1000.0) as i64,
                            (t.translation.z * 1000.0) as i64,
                        ],
                    )
                })
                .collect();
            rocks.sort();
            rocks
        };
        let run_a = build();
        let run_b = build();
        assert!(!run_a.is_empty(), "no asteroids spawned");
        assert_eq!(run_a, run_b, "same fields must produce the same rocks");
    }

    /// (#913) Adding a field entity mid-run changes the composition key and
    /// forces a full rebuild against the new composed field — with no
    /// leftover duplicates from the old composition.
    #[test]
    fn adding_a_field_recomposes_without_duplicates() {
        let mut app = test_app();
        let res = 25.0f32;
        set_ship_pos(&mut app, 175.0, 0.0);
        app.world_mut().spawn((
            AsteroidFieldSection(torus_field(100.0, 200.0, res, 12, 14)),
            Transform::default(),
        ));
        app.update();

        let mut q = app.world_mut().query::<&Asteroid>();
        let single_field_count = q.iter(app.world()).count();
        assert!(single_field_count > 0, "first field spawned nothing");

        // A second, overlapping field arrives (e.g. a world layer loads).
        app.world_mut().spawn((
            AsteroidFieldSection(torus_field(150.0, 250.0, res, 12, 14)),
            Transform::default(),
        ));
        app.update();

        let mut q = app.world_mut().query::<(&Transform, &Asteroid)>();
        let mut cells = std::collections::HashSet::new();
        let mut new_band = 0;
        for (t, _) in q.iter(app.world()) {
            let cell = (
                (t.translation.x / res).round() as i32,
                (t.translation.z / res).round() as i32,
            );
            assert!(
                cells.insert(cell),
                "cell {cell:?} holds more than one rock after recomposition"
            );
            let d = (t.translation.x.powi(2) + t.translation.z.powi(2)).sqrt();
            // Beyond the first field's reach even with the torus cell-diagonal
            // slack (200 + 25·√2 ≈ 235) — only the new field spawns out here.
            if d > 240.0 {
                new_band += 1;
            }
        }
        assert!(
            new_band > 0,
            "the new field's outer band (dist > 240) must have spawned rocks"
        );
    }

    /// (#924) A single-cell crossing must spawn exactly the newly-entered
    /// edge cells, despawn exactly the newly-exited trailing cells, and
    /// leave every interior survivor's entity untouched. Before the fix,
    /// slot addressing was keyed by offset-from-player: despawn slots were
    /// computed against the OLD arena origin, the arena was updated, spawn
    /// slots were computed against the NEW origin, and every surviving
    /// slot's contents were left addressed by the stale offset — so a
    /// one-cell move silently skipped newly-entered edge cells (their slot
    /// looked occupied) and left trailing rocks stranded (never despawned).
    /// Ring addressing (`cell.rem_euclid(size)`) makes a cell's slot
    /// independent of the player's position, so this test would have caught
    /// the bug: interior cells must keep the exact same `Entity`, not a
    /// respawned one.
    #[test]
    fn single_cell_crossing_reindexes_only_entered_and_exited_cells() {
        let mut app = test_app();
        let res = 10.0f32;
        let spawn_cells = 4u32;
        let despawn_cells = 4u32;
        // inner_radius 0, huge outer_radius: every cell near the player is
        // eligible, so occupancy across the small area under test is exact
        // and predictable.
        let f = torus_field(0.0, 100_000.0, res, spawn_cells, despawn_cells);

        set_ship_pos(&mut app, 0.0, 0.0);
        app.world_mut()
            .spawn((AsteroidFieldSection(f), Transform::default()));
        app.update(); // full rebuild at grid cell (0, 0)

        let cell_of = |t: &Transform| -> (i32, i32) {
            (
                (t.translation.x / res).round() as i32,
                (t.translation.z / res).round() as i32,
            )
        };

        let before: std::collections::HashMap<(i32, i32), Entity> = {
            let mut q = app.world_mut().query::<(Entity, &Transform, &Asteroid)>();
            q.iter(app.world())
                .map(|(e, t, _)| (cell_of(t), e))
                .collect()
        };
        assert!(!before.is_empty(), "no asteroids spawned before the move");

        // Cross exactly one cell boundary: grid cell (0, 0) -> (1, 0).
        set_ship_pos(&mut app, res, 0.0);
        app.update();

        let after: std::collections::HashMap<(i32, i32), Entity> = {
            let mut q = app.world_mut().query::<(Entity, &Transform, &Asteroid)>();
            q.iter(app.world())
                .map(|(e, t, _)| (cell_of(t), e))
                .collect()
        };

        let dc = despawn_cells as i32;

        // Interior survivors: cells within both the old and new despawn
        // window must be the exact same Entity — no despawn/respawn churn.
        let mut interior_checked = 0;
        for (&(cx, cz), &entity) in &before {
            let in_old_window = cx.abs().max(cz.abs()) <= dc;
            let in_new_window = (cx - 1).abs().max(cz.abs()) <= dc;
            if in_old_window && in_new_window {
                interior_checked += 1;
                assert_eq!(
                    after.get(&(cx, cz)),
                    Some(&entity),
                    "interior survivor cell {:?} churned (despawned/respawned) \
                     across a one-cell move",
                    (cx, cz)
                );
            }
        }
        assert!(
            interior_checked > 0,
            "test set-up produced no interior survivor cells to check"
        );

        // Trailing despawn: the old window's leftmost column exits the new
        // window and must be gone.
        let trailing_col = -dc;
        let mut trailing_checked = 0;
        for &(cx, cz) in before.keys() {
            if cx == trailing_col {
                trailing_checked += 1;
                assert!(
                    !after.contains_key(&(cx, cz)),
                    "trailing cell {:?} should have despawned after the crossing",
                    (cx, cz)
                );
            }
        }
        assert!(
            trailing_checked > 0,
            "test set-up produced no trailing cells to check"
        );

        // Edge spawn: the newly-entered spawn column (beyond the old spawn
        // window) must now hold asteroids that did not exist before.
        let entered_col = 1 + spawn_cells as i32;
        let mut edge_checked = 0;
        for &(cx, cz) in after.keys() {
            if cx == entered_col {
                edge_checked += 1;
                assert!(
                    !before.contains_key(&(cx, cz)),
                    "edge cell {:?} existed before the move — test set-up is wrong",
                    (cx, cz)
                );
            }
        }
        assert!(
            edge_checked > 0,
            "no asteroids spawned in the newly-entered edge column"
        );
    }

    /// (#924) Several sequential one-cell moves must land in the same state
    /// as a single full rebuild at the destination. Ring addressing means a
    /// cell's slot never depends on the path taken to reach the current
    /// player position, so a walk of individual steps and one big jump to
    /// the same place must agree exactly — no drift accumulates from
    /// repeated incremental deltas.
    #[test]
    fn multi_step_walk_matches_full_rebuild_at_destination() {
        let res = 10.0f32;
        let spawn_cells = 4u32;
        let despawn_cells = 4u32;
        let field = || torus_field(0.0, 100_000.0, res, spawn_cells, despawn_cells);

        let cells = |app: &mut App| -> std::collections::HashSet<(i32, i32)> {
            let mut q = app.world_mut().query::<(&Transform, &Asteroid)>();
            q.iter(app.world())
                .map(|(t, _)| {
                    (
                        (t.translation.x / res).round() as i32,
                        (t.translation.z / res).round() as i32,
                    )
                })
                .collect()
        };

        // Walk: five sequential one-cell moves along +x, each within
        // spawn_cells so none of them force a full rebuild on their own.
        let mut walked = test_app();
        set_ship_pos(&mut walked, 0.0, 0.0);
        walked
            .world_mut()
            .spawn((AsteroidFieldSection(field()), Transform::default()));
        walked.update(); // full rebuild at (0, 0)
        for step in 1..=5 {
            set_ship_pos(&mut walked, step as f32 * res, 0.0);
            walked.update();
        }

        // Direct: a single full rebuild landing at the same destination.
        let mut direct = test_app();
        set_ship_pos(&mut direct, 5.0 * res, 0.0);
        direct
            .world_mut()
            .spawn((AsteroidFieldSection(field()), Transform::default()));
        direct.update();

        let walked_cells = cells(&mut walked);
        let direct_cells = cells(&mut direct);
        assert!(!walked_cells.is_empty(), "walk produced no asteroids");
        assert_eq!(
            walked_cells.len(),
            direct_cells.len(),
            "walked population size must match a direct full rebuild at the destination"
        );
        assert_eq!(
            walked_cells, direct_cells,
            "walked cell occupancy must match a direct full rebuild at the destination"
        );
    }
}
