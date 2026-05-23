// Asteroid lifecycle managed by a ring-buffer window.
//
// This module provides:
// - AsteroidWindow resource: 2D ring-buffer tracking which grid cells are loaded
// - PlayerGridPosition: last known player grid cell (for movement detection)
// - AsteroidEntityMap: UUID → Entity lookup for despawning
// - check_destroyed_asteroids: despawns asteroids with HP ≤ 0, clears slot
// - update_asteroid_window: drives spawn/despawn based on player movement

use bevy::prelude::*;
use std::collections::HashMap;

use crate::asteroid_spawner::eval_cell;
use crate::asteroid_window::{compute_player_grid_cell, compute_slot_for_world_cell, eval_on_player_move};
use crate::entity_spawner::{AsteroidFieldSection, MeshSection};
use crate::lobby::Target;
use crate::lobby::WorldResource;
use crate::messages::{EntitySnapshot, ServerMessage};
use crate::simulation::SimOutbox;
use crate::ship_state::ShipState;

pub use crate::simulation::{Asteroid, AsteroidUuid};
pub use crate::entity_spawner::EntityConsoleHull;

// ── Resources ────────────────────────────────────────────────────────────

/// The 2D ring-buffer window. Each slot holds the asteroid data or None.
/// Indexed as [slot_z][slot_x] where (despawn_cells, despawn_cells) is the
/// player center.
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
    /// Grid resolution (world units per cell).
    pub resolution: f32,
    /// Index of the asteroid field this window manages.
    pub field_idx: usize,
    /// Inner radius of the torus (cells closer than this have no asteroids).
    pub inner_radius: f32,
    /// Outer radius of the torus (cells farther than this have no asteroids).
    pub outer_radius: f32,
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

/// Player grid position from the previous frame.
#[derive(Resource, Default)]
pub struct PlayerGridPosition(pub Option<(i32, i32)>);

/// Maps asteroid UUID to spawned Entity for despawn and slot lookup.
#[derive(Resource, Default)]
pub struct AsteroidEntityMap(pub HashMap<String, Entity>);

// ── Systems ─────────────────────────────────────────────────────────────

/// Check for destroyed asteroids, clear their window slot, broadcast, and
/// despawn the entity.
pub fn check_destroyed_asteroids(
    mut commands: Commands,
    mut window: ResMut<AsteroidWindow>,
    mut entity_map: ResMut<AsteroidEntityMap>,
    mut world: ResMut<WorldResource>,
    asteroid_query: Query<(Entity, &Transform, &AsteroidUuid, &EntityConsoleHull)>,
    mut outbox: ResMut<SimOutbox>,
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
            window.arena_gx, window.arena_gz,
            cell_gx, cell_gz,
            window.despawn_cells,
        ) {
            if let Some(row) = window.slots.get_mut(sz) {
                if let Some(slot) = row.get_mut(sx) {
                    if slot.as_ref().map_or(false, |d| d.uuid == uuid.0) {
                        *slot = None;
                    }
                }
            }
        }
        entity_map.0.remove(&uuid.0);
        world.0.entities.retain(|e| e.uuid != uuid.0);

        outbox.0.push((Target::All, ServerMessage::AsteroidDestroyed { uuid: uuid.0.clone() }));
        commands.entity(entity).despawn();
    }
}

/// Update the ring-buffer window when the player moves to a new grid cell.
/// Runs every frame; no-ops if the player has not crossed a cell boundary.
///
/// Sources its field configuration from a spawned entity carrying an
/// `AsteroidFieldSection` component (the first one found whose `grid` is set).
pub fn update_asteroid_window(
    mut commands: Commands,
    ship: Res<ShipState>,
    fields: Query<&AsteroidFieldSection>,
    mut window: ResMut<AsteroidWindow>,
    mut world: ResMut<WorldResource>,
    mut player_grid: ResMut<PlayerGridPosition>,
    mut entity_map: ResMut<AsteroidEntityMap>,
    mut outbox: ResMut<SimOutbox>,
) {
    let (field_idx, field) = match fields.iter().enumerate().find(|(_, f)| f.0.grid.is_some()) {
        Some((idx, f)) => (idx, f.0.clone()),
        None => return,
    };
    let grid = match &field.grid {
        Some(g) => g,
        None => return,
    };

    let (gx, gz) = compute_player_grid_cell(ship.x, ship.z, grid.resolution);

    let needs_init = player_grid.0.is_none();
    let (old_gx, old_gz) = player_grid.0.unwrap_or((gx, gz));

    if !needs_init && old_gx == gx && old_gz == gz {
        return;
    }

    let delta = eval_on_player_move(old_gx, old_gz, gx, gz, grid.spawn_cells, grid.despawn_cells);

    if needs_init || delta.full_rebuild {
        full_rebuild(
            &mut commands, &mut window, &mut entity_map, &mut world, &mut outbox,
            gx, gz, field_idx, &grid, field.inner_radius, field.outer_radius,
            &field.asteroid_type_paths,
            &field.cosmetic_type_paths,
        );
    } else {
        for (cell_gx, cell_gz) in &delta.cells_to_despawn {
            if let Some((sx, sz)) = compute_slot_for_world_cell(
                window.arena_gx, window.arena_gz, *cell_gx, *cell_gz, window.despawn_cells,
            ) {
                clear_slot(&mut window, &mut commands, &mut entity_map, &mut world, sx, sz);
            }
        }

        window.arena_gx = gx;
        window.arena_gz = gz;

        for (cell_gx, cell_gz) in &delta.cells_to_spawn {
            if let Some((sx, sz)) = compute_slot_for_world_cell(
                window.arena_gx, window.arena_gz, *cell_gx, *cell_gz, window.despawn_cells,
            ) {
                try_spawn_cell(
                    &mut commands, &mut window, &mut entity_map, &mut world, &mut outbox,
                    *cell_gx, *cell_gz, sx, sz, field_idx, &grid,
                    field.inner_radius, field.outer_radius, &field.asteroid_type_paths,
                );
                try_spawn_cosmetic_cell(
                    &mut commands, &mut window,
                    *cell_gx, *cell_gz, sx, sz, field_idx, &grid,
                    field.inner_radius, field.outer_radius, &field.cosmetic_type_paths,
                );
            }
        }
    }

    player_grid.0 = Some((gx, gz));
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Full rebuild: despawn all tracked entities, clear the window, re-evaluate
/// every cell within the spawn window.
fn full_rebuild(
    commands: &mut Commands,
    window: &mut AsteroidWindow,
    entity_map: &mut AsteroidEntityMap,
    world: &mut ResMut<WorldResource>,
    outbox: &mut ResMut<SimOutbox>,
    gx: i32, gz: i32,
    field_idx: usize,
    grid: &crate::entity_config::GridConfig,
    inner_radius: f32, outer_radius: f32,
    gameplay_type_paths: &[String],
    cosmetic_type_paths: &[String],
) {
    for (_uuid, &entity) in entity_map.0.iter() {
        commands.entity(entity).despawn();
    }
    entity_map.0.clear();

    // Despawn all existing cosmetic entities before resizing the slot arrays.
    for row in &window.cosmetic_upper_slots {
        for slot in row {
            if let Some(&entity) = slot.as_ref() {
                commands.entity(entity).despawn();
            }
        }
    }
    for row in &window.cosmetic_lower_slots {
        for slot in row {
            if let Some(&entity) = slot.as_ref() {
                commands.entity(entity).despawn();
            }
        }
    }

    // Only remove asteroid snapshots from WorldResource; preserve named entities
    // (stations, raiders, player ship) spawned by world/server.rs and game-start systems.
    world.0.entities.retain(|e| !e.tags.iter().any(|t| t == "asteroid"));

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

    let s_cells = window.spawn_cells as i32;
    for cx in (gx - s_cells)..=(gx + s_cells) {
        for cz in (gz - s_cells)..=(gz + s_cells) {
            if let Some((sx, sz)) = compute_slot_for_world_cell(
                gx, gz, cx, cz, window.despawn_cells,
            ) {
                try_spawn_cell(
                    commands, window, entity_map, world, outbox,
                    cx, cz, sx, sz, field_idx, grid,
                    inner_radius, outer_radius, gameplay_type_paths,
                );
                try_spawn_cosmetic_cell(
                    commands, window,
                    cx, cz, sx, sz, field_idx, grid,
                    inner_radius, outer_radius, cosmetic_type_paths,
                );
            }
        }
    }
}

/// Evaluate a single cell for asteroid spawning. If the cell passes the
/// density check and is within the torus, spawn a gameplay asteroid entity
/// and populate the window slot.
fn try_spawn_cell(
    commands: &mut Commands,
    window: &mut AsteroidWindow,
    entity_map: &mut AsteroidEntityMap,
    world: &mut ResMut<WorldResource>,
    outbox: &mut ResMut<SimOutbox>,
    cell_gx: i32, cell_gz: i32,
    slot_x: usize, slot_z: usize,
    field_idx: usize,
    grid: &crate::entity_config::GridConfig,
    inner_radius: f32, outer_radius: f32,
    gameplay_type_paths: &[String],
) {
    if window.slots[slot_z][slot_x].is_some() {
        return;
    }

    let cell_cx = cell_gx as f32 * grid.resolution;
    let cell_cz = cell_gz as f32 * grid.resolution;
    let dist = (cell_cx * cell_cx + cell_cz * cell_cz).sqrt();
    if dist < inner_radius || dist > outer_radius {
        return;
    }

    if gameplay_type_paths.is_empty() {
        return;
    }

    let Some(spawn) = eval_cell(
        field_idx as u64, cell_gx, cell_gz, grid,
        inner_radius, outer_radius,
        gameplay_type_paths, &[],
    ) else { return };

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
        .map(|h| if h.hull_integrity > 0.0 { h.hull_integrity } else { h.captain_chair.unwrap_or(30.0) })
        .unwrap_or(30.0);
    let snapshot_tags = entity_config
        .map(|c| c.tags.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| vec!["asteroid".into()]);

    let uuid = uuid::Uuid::new_v4().to_string();

    let asteroid_hull = EntityConsoleHull(
        crate::damage::ConsoleHull::from_config(&[(crate::messages::Console::CaptainChair, max_hp)])
    );

    let mut entity_cmd = commands.spawn((
        Asteroid,
        AsteroidUuid(uuid.clone()),
        asteroid_hull,
        Transform::from_xyz(spawn.x, spawn.y, spawn.z),
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
        position: Some([spawn.x, spawn.y, spawn.z]),
        tags: snapshot_tags,
        radius: Some(collider_radius),
        ..EntitySnapshot::default()
    });

    outbox.0.push((Target::All, ServerMessage::AsteroidSpawned {
        uuid,
        x: spawn.x,
        y: spawn.y,
        z: spawn.z,
        config_path: spawn.config_path,
        max_hp: max_hp as i32,
        current_hp: max_hp as i32,
        radius: collider_radius,
    }));
}

/// Clear a single window slot: remove data and despawn the associated entity.
fn clear_slot(
    window: &mut AsteroidWindow,
    commands: &mut Commands,
    entity_map: &mut AsteroidEntityMap,
    world: &mut ResMut<WorldResource>,
    slot_x: usize, slot_z: usize,
) {
    if let Some(slot) = window.slots.get_mut(slot_z).and_then(|row| row.get_mut(slot_x)) {
        if let Some(data) = slot.take() {
            if let Some(&entity) = entity_map.0.get(&data.uuid) {
                commands.entity(entity).despawn();
            }
            entity_map.0.remove(&data.uuid);
            world.0.entities.retain(|e| e.uuid != data.uuid);
        }
    }
    if let Some(entity) = window.cosmetic_upper_slots
        .get_mut(slot_z).and_then(|row| row.get_mut(slot_x)).and_then(|s| s.take())
    {
        commands.entity(entity).despawn();
    }
    if let Some(entity) = window.cosmetic_lower_slots
        .get_mut(slot_z).and_then(|row| row.get_mut(slot_x)).and_then(|s| s.take())
    {
        commands.entity(entity).despawn();
    }
}

/// Spawn a single cosmetic asteroid entity (no hull, no UUID tracking).
/// Returns the spawned `Entity` so the caller can store it in a cosmetic slot.
fn spawn_cosmetic_entity(
    commands: &mut Commands,
    spawn: &crate::asteroid_spawner::AsteroidSpawn,
    y: f32,
) -> Entity {
    let config_cache = crate::config_cache::get_config_cache();
    let entity_config = config_cache.get(&spawn.config_path);
    let collider_radius = entity_config
        .and_then(|c| c.collider.as_ref())
        .map(|c| c.radius)
        .unwrap_or(1.0);

    let mut entity_cmd = commands.spawn((
        Transform::from_xyz(spawn.x, y, spawn.z),
        bevy_rapier3d::prelude::Collider::ball(collider_radius),
        bevy_rapier3d::prelude::RigidBody::Fixed,
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
    cell_gx: i32, cell_gz: i32,
    slot_x: usize, slot_z: usize,
    field_idx: usize,
    grid: &crate::entity_config::GridConfig,
    inner_radius: f32, outer_radius: f32,
    cosmetic_type_paths: &[String],
) {
    if cosmetic_type_paths.is_empty() {
        return;
    }

    let cell_cx = cell_gx as f32 * grid.resolution;
    let cell_cz = cell_gz as f32 * grid.resolution;
    let dist = (cell_cx * cell_cx + cell_cz * cell_cz).sqrt();
    if dist < inner_radius || dist > outer_radius {
        return;
    }

    // Upper layer — large seed offset keeps this independent from gameplay seeds.
    if window.cosmetic_upper_slots[slot_z][slot_x].is_none() {
        if let Some(spawn) = eval_cell(
            field_idx as u64 + 0x0001_0000_0000,
            cell_gx, cell_gz, grid,
            inner_radius, outer_radius,
            &[], cosmetic_type_paths,
        ) {
            let entity = spawn_cosmetic_entity(commands, &spawn, spawn.y);
            window.cosmetic_upper_slots[slot_z][slot_x] = Some(entity);
        }
    }

    // Lower layer — separate seed offset so upper and lower differ.
    if window.cosmetic_lower_slots[slot_z][slot_x].is_none() {
        if let Some(spawn) = eval_cell(
            field_idx as u64 + 0x0002_0000_0000,
            cell_gx, cell_gz, grid,
            inner_radius, outer_radius,
            &[], cosmetic_type_paths,
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
        app
            .init_resource::<AsteroidWindow>()
            .init_resource::<PlayerGridPosition>()
            .init_resource::<AsteroidEntityMap>()
            .add_systems(Update, (
                check_destroyed_asteroids,
                update_asteroid_window,
            ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_spawner::AsteroidFieldSection;
    use crate::lobby::OutboundMessage;
    use crate::simulation::SimOutbox;
    use crate::entity_config::{AsteroidFieldConfig, GridConfig};

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.add_message::<OutboundMessage>();
        app.init_resource::<SimOutbox>();
        app.init_resource::<AsteroidWindow>();
        app.init_resource::<PlayerGridPosition>();
        app.init_resource::<AsteroidEntityMap>();
        app.init_resource::<WorldResource>();
        app.insert_resource(ShipState::new());
        app.add_systems(Update, update_asteroid_window);
        app
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
        }
    }

    #[test]
    fn window_initialises_from_spawned_asteroid_field_section() {
        let mut app = test_app();
        // WorldResource is init'd by test_app. The system should find the field.
        app.world_mut().spawn((
            AsteroidFieldSection(field(15.0)),
            Transform::default(),
        ));
        app.update();

        let window = app.world().resource::<AsteroidWindow>();
        assert_eq!(window.resolution, 15.0,
            "window.resolution should be sourced from the spawned AsteroidFieldSection");
        assert_eq!(window.inner_radius, 100.0);
        assert_eq!(window.outer_radius, 200.0);
    }

    #[test]
    fn window_does_nothing_with_no_field_entity() {
        let mut app = test_app();
        app.update();
        let window = app.world().resource::<AsteroidWindow>();
        // Default resolution from AsteroidWindow::default() is 10.0.
        assert_eq!(window.resolution, 10.0);
    }
}
