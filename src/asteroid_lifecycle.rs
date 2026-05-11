// Asteroid spawning and despawning systems for lazy loading.
//
// This module provides:
// - DestroyedAsteroids resource to track destroyed asteroids
// - Lazy asteroid spawning based on ship position
// - Despawning of distant asteroids

use bevy::prelude::*;
use std::collections::HashSet;

use crate::asteroid_spawner::{generate_donut_field, generate_donut_uuids};
use crate::map_config::MapConfig;
use crate::messages::ServerMessage;
use crate::lobby::{OutboundMessage, Target};

// ── Resources ────────────────────────────────────────────────────────────

/// Tracks which asteroids have been destroyed to prevent respawning.
#[derive(Resource, Default, Debug)]
pub struct DestroyedAsteroids(pub HashSet<String>);

/// Timer for asteroid spawning.
#[derive(Resource)]
pub struct AsteroidSpawnTimer(pub Timer);

// ── Components ──────────────────────────────────────────────────────────
// Note: Asteroid, AsteroidUuid, and AsteroidDamage are defined in simulation.rs
// and re-exported here for convenience.

pub use crate::simulation::{Asteroid, AsteroidUuid, AsteroidDamage};

// ── Systems ─────────────────────────────────────────────────────────────

/// System to check for and handle destroyed asteroids.
/// When an asteroid's HP reaches 0, add its UUID to DestroyedAsteroids and despawn.
pub fn check_destroyed_asteroids(
    mut commands: Commands,
    mut destroyed: ResMut<DestroyedAsteroids>,
    mut writer: MessageWriter<OutboundMessage>,
    asteroid_query: Query<(Entity, &AsteroidUuid, &AsteroidDamage)>,
) {
 for (entity, uuid, damage) in asteroid_query.iter() {
 if damage.current_hp <= 0 {
 // Add to destroyed set
 destroyed.0.insert(uuid.0.clone());
 
 // Broadcast destruction
 writer.write(OutboundMessage { 
 target: Target::All, 
 msg: ServerMessage::AsteroidDestroyed { uuid: uuid.0.clone() },
 });
 
 // Despawn the entity
 commands.entity(entity).despawn();
 } 
 } 
 }

/// System to spawn asteroids lazily based on ship position.
/// Runs on a timer, spawns at most one asteroid per frame.
pub fn lazy_asteroid_spawn(
    commands: Commands,
    time: Res<Time>,
    mut spawn_timer: ResMut<AsteroidSpawnTimer>,
    destroyed: Res<DestroyedAsteroids>,
    ship_state: Res<crate::ship_state::ShipState>,
    map_config: Option<Res<MapConfig>>,
    existing_asteroids: Query<&AsteroidUuid>,
) {
    // Only run on timer tick
    let timer = &mut spawn_timer.0;
    if !timer.tick(time.delta()).just_finished() {
        return;
    }
    
    // Get map config
    let map_config = match map_config {
        Some(mc) => mc.into_inner(),
        None => return,
    };
    
    // Get existing asteroid UUIDs
    let existing_uuids: HashSet<String> = existing_asteroids
        .iter()
        .map(|uuid| uuid.0.clone())
        .collect();
    
 // Check each asteroid field
 for (field_idx, field) in map_config.asteroid_fields.iter().enumerate() {
 // Check if ship is within spawn distance of this field
 // For now, assume field is centered at origin
 let ship_dist = (ship_state.x * ship_state.x + ship_state.z * ship_state.z).sqrt();

 // Only process fields where ship is within spawn distance
 if ship_dist <= field.spawn_distance {
 // Generate candidate positions using donut model
 let seed_offset = field_idx as u64;
 let candidates = generate_donut_field(
 field.inner_radius,
 field.outer_radius,
 field.density,
 seed_offset,
 &field.asteroid_type_paths,
 &field.cosmetic_type_paths,
 );

 // Generate UUIDs for this field
 let uuids = generate_donut_uuids(
 field.inner_radius,
 field.outer_radius,
 field.density,
 seed_offset,
 candidates.spawns.len(),
 );

 // For each candidate, check if it should be spawned
 for (spawn_idx, spawn) in candidates.spawns.iter().enumerate() {
 // Use the proper UUID for this spawn
 let uuid = uuids[spawn_idx].clone();

 // Skip if already spawned or destroyed
 if existing_uuids.contains(&uuid) || destroyed.0.contains(&uuid) {
 continue;
 }

 // Spawn the asteroid
 spawn_asteroid_entity(
 commands,
 spawn.x,
 spawn.z,
 uuid.clone(),
 field.asteroid_type_paths.contains(&spawn.config_path),
 );

 // At most one spawn per frame
 return;
 } 
 } 
 }
}

/// System to despawn asteroids that are too far from the ship.
pub fn despawn_distant_asteroids(
    mut commands: Commands,
    ship_state: Res<crate::ship_state::ShipState>,
    map_config: Option<Res<MapConfig>>,
    destroyed: Res<DestroyedAsteroids>,
    asteroid_query: Query<(Entity, &Transform, &AsteroidUuid)>,
) {
 let map_config = match map_config {
 Some(mc) => mc.into_inner(),
 None => return,
 };

 for field in &map_config.asteroid_fields {
 let despawn_dist_sq = field.despawn_distance.powi(2);

 for (entity, transform, uuid) in asteroid_query.iter() {
 let dist_sq = (transform.translation.x - ship_state.x).powi(2) 
 + (transform.translation.z - ship_state.z).powi(2);

 // Only despawn if not already destroyed
 if dist_sq > despawn_dist_sq && !destroyed.0.contains(&uuid.0) {
 commands.entity(entity).despawn();
 } 
 } 
 } 
 }

/// Helper function to spawn an asteroid entity.
pub fn spawn_asteroid_entity(
    mut commands: Commands,
    x: f32,
    z: f32,
    uuid: String,
    is_gameplay: bool,
) {
    if is_gameplay {
        commands.spawn((
            Asteroid,
            AsteroidUuid(uuid),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
            Transform::from_xyz(x, 0.0, z),
            bevy_rapier3d::prelude::Collider::ball(2.0),
            bevy_rapier3d::prelude::RigidBody::Fixed,
        ));
    } else {
        // Cosmetic asteroid - no collider
        commands.spawn((
            Asteroid,
            AsteroidUuid(uuid),
            Transform::from_xyz(x, 0.0, z),
        ));
    }
}

// ── Plugin ──────────────────────────────────────────────────────────────

/// Plugin for asteroid spawning and despawning systems.
pub struct AsteroidLifecyclePlugin;

impl Plugin for AsteroidLifecyclePlugin {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<DestroyedAsteroids>()
            .insert_resource(AsteroidSpawnTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .add_systems(Update, (
                check_destroyed_asteroids,
                lazy_asteroid_spawn,
                despawn_distant_asteroids,
            ));
    }
}
