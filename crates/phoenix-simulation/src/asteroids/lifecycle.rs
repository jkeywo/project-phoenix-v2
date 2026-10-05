// Asteroid lifecycle managed by a ring-buffer window.
//
// This module provides:
// - AsteroidWindow resource: the 2D ring-buffer tracking which lattice cells
//   of the world's ONE composed asteroid field are loaded
// - AsteroidEntityMap resource: UUID → Entity lookup for despawning (global,
//   keyed by globally-unique asteroid UUID)
// - check_destroyed_asteroids: despawns asteroids with HP ≤ 0, clears slot
// - update_asteroid_window: composes every `AsteroidFieldSection` entity into
//   one weighted density field and drives spawn/despawn for it based on the
//   FLEET's streaming centre (the mean over every `FleetSlotOf` ship), not the
//   single `LocalShip` — see `fleet_stream_centre` and issue #1116
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

use crate::asteroids::spawner::{
    composed_lattice, eval_cell_composed, ComposedLattice, ComposedLayer, FieldContribution,
};
use crate::asteroids::window::{
    compute_player_grid_cell, compute_slot_for_world_cell, eval_on_player_move,
};
use crate::core::messages::{EntitySnapshot, ServerMessage};
use crate::entities::spawner::{AsteroidFieldSection, MeshSection};
use crate::lobby::Target;
use crate::lobby::WorldResource;
use crate::server_app::SimOutbox;
use crate::ship::state::ShipPhysics;

pub use crate::entities::spawner::EntitySystemHull;
pub use crate::server_app::{Asteroid, AsteroidShieldPierce, AsteroidUuid};

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
    mut positions_cache: ResMut<crate::server_app::LastBroadcastEntityPositions>,
    mut health_cache: ResMut<crate::server_app::LastBroadcastEntityHealth>,
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
        crate::server_app::prune_entity_replication_caches(
            &mut positions_cache,
            &mut health_cache,
            std::slice::from_ref(&uuid.0),
        );

        outbox.push_reliable((
            Target::All,
            ServerMessage::AsteroidDestroyed {
                uuid: uuid.0.clone(),
            },
        ));
        commands.entity(entity).try_despawn();
    }
}

/// Update the composed asteroid field's ring-buffer window when the fleet
/// moves. Runs every fixed step; a no-op if the streaming centre has not crossed
/// a lattice cell boundary since the previous tick and the authored field set is
/// unchanged.
///
/// The window centre is [`fleet_stream_centre`] — the geometric mean over every
/// `With<FleetSlotOf>` ship — NOT the single `LocalShip`. `LocalShip` marks a
/// different ship on each host of a two-host mission, and the loaded cell set
/// folds into `sim_digest` (a rock's position is what a collision resolves
/// against), so a `LocalShip`-centred window streamed a different belt on each
/// host and the two diverged from tick zero on any asteroid world (issue #1116,
/// `tests/local_ship_neutrality.rs` guards it). A fleet-wide centre is the
/// identical point on every host because every host simulates every fleet ship;
/// for a fleet of one (every solo mission) it is that ship's exact position, so
/// solo streaming is unchanged.
///
/// Every `AsteroidFieldSection` entity contributes to ONE evaluator: the
/// contributions are gathered each tick (in spawn order, which follows the
/// world TOML's author order), fingerprinted, and any change to the set —
/// a layered world loading or unloading a field entity — forces a full
/// rebuild against the new composition.
pub fn update_asteroid_window(
    mut commands: Commands,
    fleet_q: Query<(&ShipPhysics, &crate::lockstep::FleetSlotOf)>,
    fields: Query<(Entity, &AsteroidFieldSection)>,
    mut window: ResMut<AsteroidWindow>,
    mut world: ResMut<WorldResource>,
    mut entity_map: ResMut<AsteroidEntityMap>,
    mut outbox: ResMut<SimOutbox>,
    mut positions_cache: ResMut<crate::server_app::LastBroadcastEntityPositions>,
    mut health_cache: ResMut<crate::server_app::LastBroadcastEntityHealth>,
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

    let (cx, cz) = fleet_stream_centre(&fleet_q).unwrap_or_default();
    let (gx, gz) = compute_player_grid_cell(cx, cz, lattice.resolution);

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

/// The fleet's asteroid-streaming centre: the geometric mean of every ship any
/// host in the fleet flies (`With<FleetSlotOf>`), or `None` when no such ship
/// exists (a bare fixture, or before the fleet has spawned).
///
/// This is what drives the streaming window off FLEET-WIDE geometry rather than
/// off the single `LocalShip`, so every host loads the identical cell set no
/// matter which ship is local (issue #1116). The `AsteroidWindow` is one
/// ring-buffer arena centred on one lattice cell — its slot addressing is
/// `rem_euclid(2*despawn_cells+1)`, which aliases two cells more than the arena
/// side apart — so it cannot hold a literal per-cell union of windows centred on
/// two widely separated ships. The mean is the deterministic fleet point it CAN
/// centre on: for a fleet whose ships share the streamed window (the mission
/// case — a fleet closes on one objective, not scatters) it covers every ship,
/// and it is identical on every host because every host simulates every fleet
/// ship from the same ticks.
///
/// Summed in slot order so the floating-point mean is bit-identical across
/// hosts. For a fleet of ONE — every solo mission, whose single ship still
/// carries `FleetSlotOf` — the mean is that ship's exact position, so the cell
/// the window centres on is byte-for-byte the one the old `LocalShip` query
/// produced and solo streaming does not move.
fn fleet_stream_centre(
    fleet: &Query<(&ShipPhysics, &crate::lockstep::FleetSlotOf)>,
) -> Option<(f32, f32)> {
    let mut ships: Vec<_> = fleet.iter().map(|(p, slot)| (slot.0, p.x, p.z)).collect();
    if ships.is_empty() {
        return None;
    }
    ships.sort_by_key(|entry| entry.0);
    let (mut sum_x, mut sum_z) = (0.0f32, 0.0f32);
    for (_, x, z) in &ships {
        sum_x += *x;
        sum_z += *z;
    }
    let n = ships.len() as f32;
    Some((sum_x / n, sum_z / n))
}

/// Order-sensitive fingerprint of the live contribution set, used to detect
/// mid-run composition changes (world layers loading or unloading a field).
/// Debug formatting is stable within a run, which is all the key needs —
/// it never has to survive a process restart.
pub(crate) fn composition_key(fields: &[FieldContribution]) -> u64 {
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
    positions_cache: &mut crate::server_app::LastBroadcastEntityPositions,
    health_cache: &mut crate::server_app::LastBroadcastEntityHealth,
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
    crate::server_app::prune_entity_replication_caches(positions_cache, health_cache, &owned_uuids);

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
    positions_cache: &mut crate::server_app::LastBroadcastEntityPositions,
    health_cache: &mut crate::server_app::LastBroadcastEntityHealth,
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

// ── One rock, built in exactly one place ────────────────────────────────
//
// The streamer is no longer the only thing that spawns a gameplay rock: a
// snapshot restore (issue #862) has to put back the belt the capture was taken
// against, because the fresh app it restores into streamed a *different* one on
// its way to the restore point. Two spawn sites building the same entity by
// hand is how a restored rock ends up subtly unlike a streamed one — a missing
// `ColliderSection` here reads as radius 0.0 to collision avoidance, and the
// bug is invisible until a ship flies through a rock it could see.
//
// So the authored half is read once ([`RockConfig`]) and the component set is
// written once ([`rock_bundle`]); `try_spawn_cell` and `snapshot::restore` are
// both callers.

/// The authored facts a streamed rock's entity is built from.
///
/// Read from the config cache rather than stored in a save: this is TOML, and a
/// scenario whose TOML moved is one the content-version gate refuses outright.
#[derive(Clone, Debug)]
pub struct RockConfig {
    pub collider: crate::entities::config::ColliderConfig,
    pub max_hp: f32,
    pub tags: Vec<String>,
    pub mesh: Option<crate::entities::config::MeshConfig>,
    pub radar_icon: Option<String>,
    pub radar_colour: Option<[f32; 3]>,
    pub radar_size: Option<f32>,
}

/// Resolve one rock config path against the config cache, with the same
/// fallbacks the streamer has always used.
pub fn rock_config(config_path: &str) -> RockConfig {
    // A streaming boundary can spawn many rocks. Read only the selected
    // template, rather than cloning every cached hull/template for each rock.
    // Keep this lookup per spawn so subsequent cache replacements are visible.
    let cached = crate::entities::config_cache::get_cached_entity_config(config_path);
    let entity_config = cached.as_ref();
    let collider_radius = entity_config
        .and_then(|c| c.collider.as_ref())
        .map(|c| c.radius)
        .unwrap_or(2.0);
    // Radar appearance comes straight from the rock's own TOML, exactly like
    // collider/hull/tags. Cosmetic variants have no [radar_appearance] section
    // at all, so these stay None and the rock never appears on radar.
    let radar_appearance = entity_config.and_then(|c| c.radar_appearance.as_ref());
    RockConfig {
        collider: entity_config.and_then(|c| c.collider.clone()).unwrap_or(
            crate::entities::config::ColliderConfig {
                shape: crate::entities::config::ColliderShape::Ball,
                radius: collider_radius,
                length: 0.0,
                half_height: None,
                // A rock is terrain: it never manoeuvres, so it is never
                // size-ignored by another ship's hazard rule (issue #958).
                movable: false,
            },
        ),
        max_hp: entity_config
            .and_then(|c| c.hull.as_ref())
            .map(|h| {
                if h.hull_integrity > 0.0 {
                    h.hull_integrity
                } else {
                    30.0
                }
            })
            .unwrap_or(30.0),
        tags: entity_config
            .map(|c| c.tags.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| vec!["asteroid".into()]),
        mesh: entity_config.and_then(|c| c.mesh.clone()),
        radar_icon: radar_appearance.and_then(|r| r.icon.clone()),
        radar_colour: radar_appearance.and_then(|r| {
            r.colour
                .as_ref()
                .filter(|c| c.len() >= 3)
                .map(|c| [c[0], c[1], c[2]])
        }),
        radar_size: radar_appearance.and_then(|r| r.size),
    }
}

/// The component set that makes a gameplay rock a rock.
///
/// `ColliderSection` rides alongside the Rapier collider because two consumers
/// read the radius off the *component* rather than the physics body and got 0.0
/// from a rock that only had the latter: `handle_collisions`, whose de-overlap
/// then left the ship sitting inside the asteroid, and the AI `WorldSnapshot`,
/// which is how collision *avoidance* learns an obstacle's size. Field
/// asteroids bypass `spawn_entity` (which inserts this for every other entity),
/// so it has to be added here.
pub type RockBundle = (
    Asteroid,
    AsteroidUuid,
    AsteroidShieldPierce,
    EntitySystemHull,
    crate::entities::spawner::ColliderSection,
    Transform,
    bevy_rapier3d::prelude::Collider,
    // Pin the physics shape to the authored radius regardless of the rock's
    // canonical `Transform.scale`, which carries its authored `[mesh].scale` in
    // every profile. Camera-selected LOD compensation lives below it on a
    // presentation-only child. `Absolute(ONE)` therefore keeps the collision
    // radius equal to `config.collider.radius` in rendered and headless runs,
    // independent of both authored model scale and visual distance.
    bevy_rapier3d::prelude::ColliderScale,
    bevy_rapier3d::prelude::RigidBody,
);

/// Build the component set for one rock at `current_hp` of `config.max_hp`.
pub fn rock_bundle(
    uuid: &str,
    config: &RockConfig,
    translation: Vec3,
    rotation: Quat,
    shield_pierce: f32,
    current_hp: f32,
) -> RockBundle {
    let mut hull = crate::ship::damage::SystemHull::from_config(&[(
        crate::core::messages::SystemId("captain".into()),
        config.max_hp,
    )]);
    hull.set_hp(
        &crate::core::messages::SystemId("captain".into()),
        current_hp,
    );
    (
        Asteroid,
        AsteroidUuid(uuid.to_string()),
        AsteroidShieldPierce(shield_pierce),
        EntitySystemHull(hull),
        crate::entities::spawner::ColliderSection(config.collider.clone()),
        Transform::from_translation(translation).with_rotation(rotation),
        bevy_rapier3d::prelude::Collider::ball(config.collider.radius),
        bevy_rapier3d::prelude::ColliderScale::Absolute(bevy_rapier3d::prelude::Vect::ONE),
        bevy_rapier3d::prelude::RigidBody::Fixed,
    )
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
    // visual mesh, HP, and tags come from the TOML rather than hard-coded
    // values — through the same reader a snapshot restore uses.
    let rock = rock_config(&spawn.config_path);
    let collider_radius = rock.collider.radius;
    let max_hp = rock.max_hp;
    let snapshot_tags = rock.tags.clone();
    let radar_icon = rock.radar_icon.clone();
    let radar_colour = rock.radar_colour;
    let radar_size = rock.radar_size;

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

    let mut entity_cmd = commands.spawn(rock_bundle(
        &uuid,
        &rock,
        Vec3::new(world_x, spawn.y, world_z),
        rotation,
        selected.shield_pierce,
        max_hp,
    ));

    // Attach MeshSection so render_spawned_entities can add a 3-D visual mesh.
    if let Some(mesh) = &rock.mesh {
        entity_cmd.insert(MeshSection(mesh.clone()));
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

    outbox.push_reliable((
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
    positions_cache: &mut crate::server_app::LastBroadcastEntityPositions,
    health_cache: &mut crate::server_app::LastBroadcastEntityHealth,
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
            crate::server_app::prune_entity_replication_caches(
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
    spawn: &crate::asteroids::spawner::AsteroidSpawn,
    y: f32,
) -> Entity {
    let entity_config = crate::entities::config_cache::get_cached_entity_config(&spawn.config_path);

    let mut entity_cmd = commands.spawn((Transform::from_xyz(spawn.x, y, spawn.z),));

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
                (
                    check_destroyed_asteroids
                        .in_set(crate::sim_sets::FixedStep::CheckDestroyedAsteroids),
                    update_asteroid_window.in_set(crate::sim_sets::FixedStep::UpdateAsteroidWindow),
                )
                    .chain()
                    .before(bevy_rapier3d::plugin::PhysicsSet::SyncBackend),
            );
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
