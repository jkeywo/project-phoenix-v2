use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::radar::WEAPONS_RADAR_RANGE;
use crate::radar::is_fire_ready;
use crate::asteroid_spawner::{spawn_asteroid_positions, spawn_asteroid_uuids};
use crate::breakdown::{breakdowns_from_damage, BreakdownQueue};
use crate::damage::{collision_damage, HullIntegrity};
use crate::lobby::{CurrentPhase, InboundMessage, OutboundMessage, Sessions, Target, WorldResource};
use crate::messages::{
    AsteroidInfo, ClientMessage, Console, GamePhase, ServerMessage, ViewMode,
};
use crate::ship_physics::{compute_physics, ShipPhysicsConfig, ShipPhysicsInput, ShipPhysicsState};
use crate::ship_state::ShipState;

// ── Beam constants ────────────
const BEAM_DURATION_SECS: f32 = 6.0;
const BEAM_DAMAGE_PER_SEC: f32 = 5.0;
const BEAM_COOLDOWN_SECS: f32 = 6.0;

// ── Marker Components ────────
#[derive(Component)]
pub struct Ship;

#[derive(Component)]
pub struct Asteroid;

/// Stable UUID string identifying this asteroid entity (for targeting).
#[derive(Component, Clone)]
pub struct AsteroidUuid(pub String);

/// Tracks remaining HP for an asteroid entity (max and current = 30).
#[derive(Component)]
pub struct AsteroidDamage {
    pub max_hp: i32,
    pub current_hp: i32,
}

// ── Resources ────────────────
#[derive(Resource)]
struct SimBroadcastTimer(Timer);

#[derive(Resource)]
struct HelmInputTimer(Timer);

/// Ship-wide Hull Integrity (0–100). Tracked as a Bevy resource so systems
/// can read/write it independently of `ShipState`.
#[derive(Resource)]
pub struct ShipHullIntegrity(pub HullIntegrity);

/// Tracks whether the initial WorldSetup broadcast has fired, so it only
/// goes out once per game.
#[derive(Resource, Default)]
struct WorldSetupBroadcast {
    sent: bool,
}

/// The currently locked target UUID on the Weapons console. `None` means no
/// lock is active.
#[derive(Resource, Default)]
pub struct WeaponsTarget(pub Option<String>);

/// Active phaser beam state. `target_uuid` is `Some` while a beam is firing.
/// `remaining_secs` counts down to 0. `damage_accumulator` tracks fractional
/// damage between ticks so 5 HP/s is applied accurately at any frame rate.
#[derive(Resource, Default)]
pub struct ActiveBeam {
    pub target_uuid: Option<String>,
    pub remaining_secs: f32,
    pub damage_accumulator: f32,
}

/// Post-beam cooldown. The weapons console is locked out for `BEAM_COOLDOWN_SECS`
/// after every beam end (natural, sever, or cancel).
#[derive(Resource, Default)]
pub struct PhaserCooldown {
    pub remaining_secs: f32,
}

/// Bevy message fired (with world-space position) when an asteroid is destroyed
/// by phaser fire. The renderer uses this to spawn a ripple VFX at the site.
#[derive(Message, Clone, Debug)]
pub struct AsteroidDestroyedVfx {
    pub x: f32,
    pub z: f32,
}

/// Bevy resource wrapping the breakdown queue.
#[derive(Resource)]
pub struct BreakdownQueueResource {
    pub queue: BreakdownQueue,
    /// Cumulative damage taken since game start (tracks 10-HP bucket crossings).
    pub cumulative_damage: i32,
    rng: rand::rngs::SmallRng,
}

impl Default for BreakdownQueueResource {
    fn default() -> Self {
        use rand::SeedableRng as _;
        Self {
            queue: BreakdownQueue::new(),
            cumulative_damage: 0,
            rng: rand::rngs::SmallRng::from_os_rng(),
        }
    }
}

impl PhaserCooldown {
    pub fn is_active(&self) -> bool {
        self.remaining_secs > 0.0
    }

    pub fn start(&mut self) {
        self.remaining_secs = BEAM_COOLDOWN_SECS;
    }

    pub fn tick(&mut self, dt: f32) {
        self.remaining_secs = (self.remaining_secs - dt).max(0.0);
    }
}

// ── Plugin ───────────────────
pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RapierPhysicsPlugin::<()>::default())
            .add_message::<AsteroidDestroyedVfx>()
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
            .init_resource::<WorldResource>()
            .init_resource::<WorldSetupBroadcast>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<BreakdownQueueResource>()
            .insert_resource(SimBroadcastTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .insert_resource(HelmInputTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .add_systems(Startup, setup_world)
            .add_systems(Update, (
                handle_toggle,
                handle_set_view,
                handle_set_target,
                handle_fire_phaser,
                tick_active_beam,
                process_helm_inputs,
                sync_ship_position,
                handle_collisions,
                broadcast_sim_state,
                broadcast_weapons_update.after(broadcast_sim_state),
                broadcast_world_setup_on_start.after(crate::lobby::process_lobby),
            ));
    }
}

// ── Systems ──────────────────
fn handle_toggle(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if matches!(ev.msg, ClientMessage::ToggleRedAlert)
            && sessions.0.console_holder(Console::CaptainChair) == Some(ev.token.as_str())
        {
            ship.toggle_red_alert();
        }
    }
}

fn handle_set_view(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if let ClientMessage::SetView { mode } = ev.msg.clone() {
            // Authorization is per-variant: Camera views are the captain's call,
            // Radar is the helm's call. A request from the wrong console is
            // silently ignored.
            let required = match &mode {
                ViewMode::Camera(_) => Console::CaptainChair,
                ViewMode::Radar => Console::Helm,
            };
            if sessions.0.console_holder(required) == Some(ev.token.as_str()) {
                ship.view_mode = mode;
            }
        }
    }
}

fn handle_set_target(
    mut reader: MessageReader<InboundMessage>,
    mut writer: MessageWriter<OutboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    mut weapons_target: ResMut<WeaponsTarget>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::SetTarget { uuid } = &ev.msg else { continue };

        // Only the Weapons console holder may lock a target.
        if sessions.0.console_holder(Console::Weapons) != Some(ev.token.as_str()) {
            continue;
        }

        // Validate: asteroid must exist in world data and be within WEAPONS_RADAR_RANGE.
        let asteroid = world.0.asteroids.iter().find(|a| &a.uuid == uuid);
        let locked = match asteroid {
            None => false,
            Some(a) => {
                let dx = a.x - ship.x;
                let dz = a.z - ship.z;
                dx * dx + dz * dz <= WEAPONS_RADAR_RANGE * WEAPONS_RADAR_RANGE
            }
        };

        if locked {
            weapons_target.0 = Some(uuid.clone());
        } else {
            // Rejection clears the visual lock.
            weapons_target.0 = None;
        }

        writer.write(OutboundMessage {
            target: Target::Token(ev.token.clone()),
            msg: ServerMessage::TargetLock { uuid: uuid.clone(), locked },
        });
    }
}

fn process_helm_inputs(
    time: Res<Time>,
    mut timer: ResMut<HelmInputTimer>,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut ship: ResMut<ShipState>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    // Only process if helm is occupied
    let helm_token = sessions.0.console_holder(Console::Helm);
    if helm_token.is_none() {
        return;
    }

    // Collect helm inputs; default to zero so the ship decelerates when no
    // messages arrive (joystick released or between network packets).
    let mut thrust: f32 = 0.0;
    let mut steering: f32 = 0.0;

    for ev in reader.read() {
        if ev.token != helm_token.unwrap() {
            continue;
        }
        if let ClientMessage::HelmInput { thrust: t, steering: s } = ev.msg {
            thrust = t;
            steering = s;
        }
    }

    // Compute physics — use the timer's nominal period, not the frame delta.
    // The timer fires every 100 ms; time.delta_secs() is only one frame (~16 ms).
    let dt = timer.0.duration().as_secs_f32();
    let state = ShipPhysicsState {
        x: ship.x,
        z: ship.z,
        yaw: ship.yaw,
        forward_speed: ship.forward_speed,
    };
    let input = ShipPhysicsInput { thrust, steering };
    let config = ShipPhysicsConfig::new();
    let result = compute_physics(state, input, dt, &config);

    ship.x = result.x;
    ship.z = result.z;
    ship.yaw = result.yaw;
    ship.forward_speed = result.forward_speed;
}

fn sync_ship_position(
    ship: Res<ShipState>,
    mut ship_query: Query<&mut Transform, With<Ship>>,
) {
    let Ok(mut transform) = ship_query.single_mut() else {
        return;
    };

    transform.translation.x = ship.x;
    transform.translation.z = ship.z;
    transform.rotation = Quat::from_axis_angle(Vec3::Y, ship.yaw);
}

fn handle_collisions(
    context: ReadRapierContext,
    ship_query: Query<Entity, With<Ship>>,
    mut ship: ResMut<ShipState>,
    mut hull: ResMut<ShipHullIntegrity>,
    mut breakdowns: ResMut<BreakdownQueueResource>,
) {
    let Ok(ctx) = context.single() else { return };
    let Ok(ship_entity) = ship_query.single() else { return };
    if ctx.contact_pairs_with(ship_entity).next().is_some() {
        let max_speed = ShipPhysicsConfig::new().max_speed;
        let damage = collision_damage(ship.forward_speed, max_speed);
        let before = breakdowns.cumulative_damage;
        hull.0.apply_damage(damage);
        breakdowns.cumulative_damage += damage;
        let new_count = breakdowns_from_damage(before, breakdowns.cumulative_damage);
        // Avoid double-borrow: split mutable access to queue and rng.
        let BreakdownQueueResource { queue, rng, .. } = &mut *breakdowns;
        for _ in 0..new_count {
            queue.push_random(rng);
        }
        ship.forward_speed = 0.0;
    }
}

/// Handle `FirePhaser` messages from the Weapons console.
///
/// Validates: sender is Weapons holder, game is in-progress, no active cooldown,
/// a locked target exists, and that target is currently fire-ready.
/// On success, starts a new beam (cancelling any active beam first) and broadcasts
/// `BeamStarted` to all players.
fn handle_fire_phaser(
    mut reader: MessageReader<InboundMessage>,
    mut writer: MessageWriter<OutboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    weapons_target: Res<WeaponsTarget>,
    mut beam: ResMut<ActiveBeam>,
    mut cooldown: ResMut<PhaserCooldown>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if !matches!(ev.msg, ClientMessage::FirePhaser) {
            continue;
        }
        // Only the Weapons console holder may fire.
        if sessions.0.console_holder(Console::Weapons) != Some(ev.token.as_str()) {
            continue;
        }
        // Reject if on cooldown.
        if cooldown.is_active() {
            continue;
        }
        // Need a locked target.
        let Some(target_uuid) = &weapons_target.0 else { continue };
        // Target must still exist in world data and be fire-ready.
        let Some(asteroid) = world.0.asteroids.iter().find(|a| &a.uuid == target_uuid) else {
            continue;
        };
        if !is_fire_ready(asteroid.x, asteroid.z, ship.x, ship.z, ship.yaw) {
            continue;
        }

        // If another beam was active (shouldn't happen with cooldown enforcement,
        // but guard defensively), end it first.
        if let Some(old_uuid) = beam.target_uuid.take() {
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            writer.write(OutboundMessage {
                target: Target::All,
                msg: ServerMessage::BeamEnded { target_uuid: old_uuid },
            });
        }

        // Start new beam.
        beam.target_uuid = Some(target_uuid.clone());
        beam.remaining_secs = BEAM_DURATION_SECS;
        beam.damage_accumulator = 0.0;

        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BeamStarted { target_uuid: target_uuid.clone() },
        });
    }
}

/// Tick the active beam each frame: apply damage, check sever conditions
/// (arc, range, target destroyed), and handle natural expiry.
///
/// When the beam ends (any cause), starts the post-beam cooldown and broadcasts
/// `BeamEnded`. If the target asteroid reaches 0 HP, also broadcasts
/// `AsteroidDestroyed` and removes it from `WorldData`.
fn tick_active_beam(
    time: Res<Time>,
    mut beam: ResMut<ActiveBeam>,
    mut cooldown: ResMut<PhaserCooldown>,
    mut writer: MessageWriter<OutboundMessage>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    ship: Res<ShipState>,
    mut world: ResMut<WorldResource>,
    mut asteroid_query: Query<(Entity, &AsteroidUuid, &mut AsteroidDamage)>,
    mut commands: Commands,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    let dt = time.delta_secs();

    // Tick cooldown regardless of beam state.
    cooldown.tick(dt);

    let Some(target_uuid) = beam.target_uuid.clone() else {
        return;
    };

    // Check sever: target no longer exists in world data.
    let asteroid_info = world.0.asteroids.iter().find(|a| a.uuid == target_uuid).cloned();
    let Some(info) = asteroid_info else {
        // Target was already destroyed (e.g., double-tick race). End beam silently.
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BeamEnded { target_uuid },
        });
        return;
    };

    // Check sever: out of range or out of arc.
    if !is_fire_ready(info.x, info.z, ship.x, ship.z, ship.yaw) {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BeamEnded { target_uuid },
        });
        return;
    }

    // Apply damage proportionally to elapsed time.
    beam.damage_accumulator += BEAM_DAMAGE_PER_SEC * dt;
    let damage_to_apply = beam.damage_accumulator.floor() as i32;
    if damage_to_apply > 0 {
        beam.damage_accumulator -= damage_to_apply as f32;

        // Find the asteroid entity and apply damage.
        let mut destroyed = false;
        for (entity, uuid_comp, mut dmg) in asteroid_query.iter_mut() {
            if uuid_comp.0 == target_uuid {
                dmg.current_hp = (dmg.current_hp - damage_to_apply).max(0);
                if dmg.current_hp == 0 {
                    destroyed = true;
                    commands.entity(entity).despawn();
                }
                break;
            }
        }

        if destroyed {
            // Remove from world data.
            world.0.asteroids.retain(|a| a.uuid != target_uuid);

            // Fire VFX event with the asteroid's last known position so the
            // renderer can play the destruction ripple. `info` holds the position
            // captured before the retain() call above.
            vfx_events.write(AsteroidDestroyedVfx { x: info.x, z: info.z });

            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start();

            writer.write(OutboundMessage {
                target: Target::All,
                msg: ServerMessage::AsteroidDestroyed { uuid: target_uuid.clone() },
            });
            writer.write(OutboundMessage {
                target: Target::All,
                msg: ServerMessage::BeamEnded { target_uuid },
            });
            return;
        }
    }

    // Tick beam duration.
    beam.remaining_secs -= dt;
    if beam.remaining_secs <= 0.0 {
        // Natural expiry.
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BeamEnded { target_uuid },
        });
    }
}

fn broadcast_sim_state(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    mut writer: MessageWriter<OutboundMessage>,
    ship: Res<ShipState>,
    hull: Res<ShipHullIntegrity>,
    breakdowns: Res<BreakdownQueueResource>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if timer.0.tick(time.delta()).just_finished() {
        let authorized = breakdowns.queue.front().cloned();
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::SimState { snapshot: ship.snapshot(hull.0.current(), authorized) },
        });
    }
}

/// Broadcast `WeaponsUpdate` to the Weapons console player at 10 Hz.
///
/// Reuses `SimBroadcastTimer`; after the timer ticks in `broadcast_sim_state`
/// the `just_finished()` flag is still `true` for the remainder of the frame
/// because `Repeating` timers latch it until the next `tick`.
fn broadcast_weapons_update(
    timer: Res<SimBroadcastTimer>,
    mut writer: MessageWriter<OutboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    weapons_target: Res<WeaponsTarget>,
    cooldown: Res<PhaserCooldown>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if !timer.0.just_finished() {
        return;
    }
    let Some(weapons_token) = sessions.0.console_holder(Console::Weapons) else {
        return;
    };

    let fire_ready = match &weapons_target.0 {
        None => false,
        Some(uuid) => {
            world.0.asteroids.iter()
                .find(|a| &a.uuid == uuid)
                .map(|a| is_fire_ready(a.x, a.z, ship.x, ship.z, ship.yaw))
                .unwrap_or(false)
        }
    };

    writer.write(OutboundMessage {
        target: Target::Token(weapons_token.to_string()),
        msg: ServerMessage::WeaponsUpdate {
            target_uuid: weapons_target.0.clone(),
            fire_ready,
            on_cooldown: cooldown.is_active(),
        },
    });
}

/// Emit a single `WorldSetup` broadcast the first frame the game enters
/// `InProgress`. Stays silent in Lobby and on subsequent in-game ticks.
fn broadcast_world_setup_on_start(
    mut writer: MessageWriter<OutboundMessage>,
    world: Res<WorldResource>,
    phase: Res<CurrentPhase>,
    mut state: ResMut<WorldSetupBroadcast>,
) {
    if phase.0 != GamePhase::InProgress || state.sent {
        return;
    }
    writer.write(OutboundMessage {
        target: Target::All,
        msg: ServerMessage::WorldSetup { world: world.0.clone() },
    });
    state.sent = true;
}

// ── World Setup ──────────────
fn setup_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut world: ResMut<WorldResource>,
) {
    let config = ShipPhysicsConfig::new();
    let positions = spawn_asteroid_positions(config.max_speed * 3.0, 40, 20.0);
    let uuids = spawn_asteroid_uuids(config.max_speed * 3.0, 40, 20.0);

    // Record asteroid layout so it can be broadcast as WorldSetup and
    // included in Welcome for reconnecting clients.
    world.0.asteroids = positions
        .iter()
        .zip(uuids.iter())
        .map(|((x, z), uuid)| AsteroidInfo { uuid: uuid.clone(), x: *x, z: *z, radius: 2.0 })
        .collect();

    // Spawn asteroids
    let asteroid_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.4, 0.35, 0.3),
        ..default()
    });
    let asteroid_mesh = meshes.add(Sphere { radius: 2.0 });

    for ((x, z), uuid) in positions.iter().zip(uuids.iter()) {
        commands.spawn((
            Asteroid,
            AsteroidUuid(uuid.clone()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
            Mesh3d(asteroid_mesh.clone()),
            MeshMaterial3d(asteroid_mat.clone()),
            Transform::from_xyz(*x, 0.0, *z),
            Collider::ball(2.0),
            RigidBody::Fixed,
        ));
    }

    // ── Cosmetic asteroids above/below the play plane ──────────────────
    // Pure decoration — no colliders. Distributed in two slab regions
    // sandwiching the play plane.
    let cosmetic_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.3, 0.28),
        perceptual_roughness: 0.95,
        ..default()
    });
    let cosmetic_positions = spawn_asteroid_positions(config.max_speed * 4.0, 80, 10.0);
    for (i, (x, z)) in cosmetic_positions.iter().enumerate() {
        // Pseudo-random Y in [-60,-10] ∪ [10,60] using index hashing
        let h = ((i as u32).wrapping_mul(2654435761)) ^ 0x9E3779B9;
        let above = (h & 1) == 0;
        let mag = 10.0 + ((h >> 1) % 5000) as f32 / 100.0; // 10..60
        let y = if above { mag } else { -mag };
        let radius = 0.5 + ((h >> 13) % 250) as f32 / 100.0; // 0.5..3.0
        let mesh = meshes.add(Sphere { radius });
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(cosmetic_mat.clone()),
            Transform::from_xyz(*x, y, *z),
        ));
    }

    // ── Starfield skybox ───────────────────────────────────────────────
    // Procedural points: many small unlit white spheres at radius ~2000
    // around the origin. Cheap and works on WebGL2.
    let star_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        unlit: true,
        ..default()
    });
    let star_mesh = meshes.add(Sphere { radius: 1.0 });
    let star_count = 400u32;
    let radius = 2000.0_f32;
    for i in 0..star_count {
        // Deterministic pseudo-random unit vector via golden-spiral on a sphere.
        let frac = (i as f32 + 0.5) / star_count as f32;
        let phi = (1.0 - 2.0 * frac).acos();
        let theta = std::f32::consts::PI * (1.0 + 5_f32.sqrt()) * i as f32;
        let x = phi.sin() * theta.cos() * radius;
        let y = phi.sin() * theta.sin() * radius;
        let z = phi.cos() * radius;
        // Hash for size variation
        let h = ((i.wrapping_mul(2654435761)) ^ 0xDEADBEEF) % 100;
        let scale = 1.5 + (h as f32) / 25.0; // 1.5..5.5
        commands.spawn((
            Mesh3d(star_mesh.clone()),
            MeshMaterial3d(star_mat.clone()),
            Transform::from_xyz(x, y, z).with_scale(Vec3::splat(scale)),
        ));
    }

    // Spawn ship — kinematic so we drive position directly from ShipState;
    // collision events fire so handle_collisions can zero velocity on impact.
    commands.spawn((
        Ship,
        Transform::default(),
        RigidBody::KinematicPositionBased,
        Collider::capsule_y(3.0, 6.0),
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));
}

// ── Tests ────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{LobbyPlugin, InboundMessage, OutboundMessage};
    use crate::messages::*;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        // Use a 1-nanosecond timer so that any non-zero time delta finishes
        // the broadcast cycle, letting tests observe the snapshot after a
        // couple of update ticks.
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
            .init_resource::<WorldResource>()
            .init_resource::<WorldSetupBroadcast>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .add_message::<AsteroidDestroyedVfx>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<BreakdownQueueResource>()
            .insert_resource(SimBroadcastTimer(Timer::new(
                std::time::Duration::from_nanos(1), TimerMode::Repeating)))
            .init_resource::<Outbox>()
            .add_systems(Update, (handle_set_view, handle_set_target, handle_fire_phaser, tick_active_beam, broadcast_sim_state, broadcast_weapons_update.after(broadcast_sim_state), broadcast_world_setup_on_start.after(crate::lobby::process_lobby)))
            .add_systems(PostUpdate, collect);
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage { token: token.into(), msg });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let msgs = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        msgs
    }

    fn start_game(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn set_view_during_lobby_is_ignored() {
        let mut app = test_app();
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(&mut app);
        // Still in Lobby — game not started
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn non_captain_set_view_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "crew", ClientMessage::Identify { token: "crew".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "crew", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Port) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn captain_set_view_changes_direction() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Aft)
        );
    }

    fn start_game_with_helm(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "helm", ClientMessage::Identify { token: "helm".into(), name: "Bob".into() });
        tick(app);
        push(app, "helm", ClientMessage::SelectConsole { console: Console::Helm });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn helm_can_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Radar
        );
    }

    #[test]
    fn captain_cannot_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        // Captain has no authority over Radar; request is silently dropped.
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn helm_cannot_switch_view_to_camera() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn sim_state_broadcast_carries_ship_position_and_view_mode() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        // Move the ship and switch to radar
        {
            let mut ship = app.world_mut().resource_mut::<ShipState>();
            ship.x = 12.0;
            ship.z = -3.5;
            ship.yaw = 1.25;
        }
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        // Ensure Time has accumulated some real delta and the broadcast fires.
        // Two prior ticks have already advanced TimePlugin's clock; a fresh
        // tick now sees a non-zero delta, finishing the 1-ns broadcast timer.
        let out = tick(&mut app);

        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");

        assert_eq!(snap.ship_x, 12.0);
        assert_eq!(snap.ship_z, -3.5);
        assert_eq!(snap.ship_yaw, 1.25);
        assert_eq!(snap.view_mode, ViewMode::Radar);
    }

    #[test]
    fn world_setup_is_broadcast_once_after_start_game() {
        let mut app = test_app();
        // Pre-populate world data so the broadcast has something to emit.
        app.world_mut().insert_resource(WorldResource(WorldData {
            asteroids: vec![AsteroidInfo { uuid: "test-uuid".into(), x: 5.0, z: -1.0, radius: 2.0 }],
        }));

        // Bring the game up to the point of pressing StartGame
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "A".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(&mut app);
        // The StartGame tick should produce the WorldSetup broadcast
        push(&mut app, "captain", ClientMessage::StartGame);
        let start_out = tick(&mut app);

        let world_setups: Vec<_> = start_out.iter().filter(|m|
            matches!(&m.msg, ServerMessage::WorldSetup { .. })
        ).collect();
        assert_eq!(world_setups.len(), 1, "expected exactly one WorldSetup on the StartGame tick");
        match &world_setups[0].msg {
            ServerMessage::WorldSetup { world } => {
                assert_eq!(world.asteroids.len(), 1);
                assert_eq!(world.asteroids[0].x, 5.0);
            }
            _ => unreachable!(),
        }
        match &world_setups[0].target {
            crate::lobby::Target::All => {}
            t => panic!("WorldSetup should target All, got {:?}", t),
        }

        // Subsequent ticks must not re-broadcast WorldSetup
        let later = tick(&mut app);
        assert!(!later.iter().any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should only fire once per game");
    }

    #[test]
    fn world_setup_is_not_broadcast_during_lobby() {
        let mut app = test_app();
        app.world_mut().insert_resource(WorldResource(WorldData {
            asteroids: vec![AsteroidInfo { uuid: "test-uuid".into(), x: 0.0, z: 0.0, radius: 2.0 }],
        }));
        // Identify and select a console but don't start the game.
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "A".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        let out = tick(&mut app);
        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should not be broadcast in the Lobby phase");
    }

    #[test]
    fn hull_integrity_starts_at_100_and_appears_in_sim_snapshot() {
        let mut app = test_app();
        start_game(&mut app);
        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");
        assert_eq!(snap.hull_integrity, 100);
    }

    #[test]
    fn direct_damage_reduces_hull_integrity_in_broadcast() {
        let mut app = test_app();
        start_game(&mut app);

        // Directly apply damage to the resource (simulates collision at ~half speed).
        app.world_mut()
            .resource_mut::<ShipHullIntegrity>()
            .0.apply_damage(10);

        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");
        assert_eq!(snap.hull_integrity, 90);
    }

    #[test]
    fn taking_25hp_damage_enqueues_2_breakdowns_and_snapshot_shows_first() {
        let mut app = test_app();
        start_game(&mut app);

        // Apply 25 HP of damage directly in 10-HP bucket tracking terms,
        // mimicking how handle_collisions would do it via breakdowns_from_damage.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            let before = bd.cumulative_damage; // 0
            bd.cumulative_damage += 25;
            let new_count = breakdowns_from_damage(before, bd.cumulative_damage);
            assert_eq!(new_count, 2, "25 HP should create exactly 2 breakdowns");
            let BreakdownQueueResource { queue, rng, .. } = &mut *bd;
            for _ in 0..new_count {
                queue.push_random(rng);
            }
        }

        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");

        // Queue has 2 entries; snapshot shows the front (not None).
        assert!(
            snap.authorized_repair_console.is_some(),
            "snapshot should show the authorized repair console"
        );
        // Verify queue length via resource.
        let bd = app.world().resource::<BreakdownQueueResource>();
        assert_eq!(bd.queue.len(), 2, "2 breakdowns should be queued");
        assert_eq!(
            snap.authorized_repair_console.as_ref(),
            bd.queue.front(),
            "snapshot authorized_repair_console matches queue front"
        );
    }

    #[test]
    fn advancing_queue_exposes_next_breakdown() {
        let mut app = test_app();
        start_game(&mut app);

        // Seed 2 breakdowns.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.cumulative_damage = 25;
            let BreakdownQueueResource { queue, rng, .. } = &mut *bd;
            queue.push_random(rng);
            queue.push_random(rng);
        }

        // Capture the first (front) entry.
        let first = app.world().resource::<BreakdownQueueResource>().queue.front().cloned();

        // Pop the front (simulating a successful repair).
        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();

        // Second entry is now the front.
        let second = app.world().resource::<BreakdownQueueResource>().queue.front().cloned();

        assert!(second.is_some(), "second breakdown should now be front");
        assert_ne!(first, second, "consecutive entries are different consoles");
    }

    // ── SetTarget / TargetLock tests ──────────────────────────────────

    fn setup_weapons_world(app: &mut App, asteroid_x: f32, asteroid_z: f32) {
        app.world_mut().insert_resource(WorldResource(WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "target-uuid".into(),
                x: asteroid_x,
                z: asteroid_z,
                radius: 2.0,
            }],
        }));
    }

    fn start_game_with_weapons(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectConsole { console: Console::CaptainChair });
        tick(app);
        push(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(app);
        push(app, "weapons", ClientMessage::SelectConsole { console: Console::Weapons });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
        // Asteroid at (30, 0) — 30 units from ship origin, within 60-unit range.
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert_eq!(lock.0, "target-uuid");
        assert!(lock.1, "expected locked=true for in-range asteroid");

        // Server state should record the lock.
        assert_eq!(
            app.world().resource::<WeaponsTarget>().0.as_deref(),
            Some("target-uuid")
        );
    }

    #[test]
    fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
        let mut app = test_app();
        // Asteroid at (80, 0) — 80 units away, outside 60-unit Weapons range.
        setup_weapons_world(&mut app, 80.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for out-of-range asteroid");
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
    }

    #[test]
    fn unknown_uuid_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 10.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "no-such-asteroid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for unknown UUID");
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
    }

    // ── WeaponsUpdate / fire_ready tests ──────────────────────────────────────

    /// Target locked, within 40-unit phaser range, in forward arc → fire_ready = true.
    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        // Ship at origin, yaw=0 (facing -Z). Asteroid at (0, -20): directly ahead, 20 units away.
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        // Lock the target
        let _ = tick(&mut app);
        // Now run another tick to get a WeaponsUpdate
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, fire_ready, .. } =>
                Some((target_uuid.clone(), *fire_ready)),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(update.1, "expected fire_ready=true for in-range, forward-arc target");
    }

    /// Target locked but beyond 40-unit phaser range (within 60u lock range) → fire_ready = false.
    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        // Ship at origin, yaw=0. Asteroid at (0, -50): directly ahead, 50 units — within lock range
        // (60u) but outside phaser range (40u).
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, fire_ready, .. } =>
                Some((target_uuid.clone(), *fire_ready)),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(!update.1, "expected fire_ready=false for beyond-phaser-range target");
    }

    // ── FirePhaser / beam lifecycle tests ──────────────────────────────────────

    /// Helper: lock target then fire phaser; returns messages from the fire tick.
    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        // Lock
        push(app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(app);
        // Fire
        push(app, "weapons", ClientMessage::FirePhaser);
        tick(app)
    }

    /// Firing at a fire-ready target broadcasts BeamStarted to all.
    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        // Asteroid directly ahead at 20 units (yaw=0 → facing -Z → asteroid at (0,-20)).
        let out = lock_and_fire(&mut app, 0.0, -20.0);

        let beam_started = out.iter().find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
        assert!(beam_started.is_some(), "expected BeamStarted after firing at fire-ready target");
        match &beam_started.unwrap().msg {
            ServerMessage::BeamStarted { target_uuid } => assert_eq!(target_uuid, "target-uuid"),
            _ => unreachable!(),
        }
        match &beam_started.unwrap().target {
            Target::All => {}
            t => panic!("BeamStarted should target All, got {:?}", t),
        }

        // ActiveBeam resource should be populated.
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );
    }

    /// FirePhaser is silently ignored when the phaser is on cooldown.
    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Manually put the cooldown into active state (simulating a beam just ended).
        app.world_mut().resource_mut::<ActiveBeam>().target_uuid = None;
        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 3.0;

        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown");
    }

    /// Non-weapons player cannot fire.
    #[test]
    fn fire_phaser_ignored_from_non_weapons_player() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game(&mut app);

        push(&mut app, "captain", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "captain should not be able to fire phaser");
    }

    /// When the beam fires at a target outside the 180° arc, it is rejected.
    #[test]
    fn fire_phaser_rejected_when_target_behind_ship() {
        let mut app = test_app();
        // Yaw=0 means ship faces -Z. Asteroid at (0, +20) is directly behind — in rear arc.
        setup_weapons_world(&mut app, 0.0, 20.0);
        start_game_with_weapons(&mut app);
        // Lock (within 60u range) — lock doesn't require arc.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        // Fire — rejected because target is behind.
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is in rear arc");
    }

    /// A 6-second natural beam kills the asteroid (5 HP/s × 6s = 30 HP total).
    ///
    /// The test accelerates time by manipulating the beam state directly
    /// after confirming the beam started, then runs ticks with large deltas.
    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();

        // Spawn an asteroid entity with full HP so tick_active_beam can find it.
        let asteroid_entity = app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
        )).id();

        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Verify beam started.
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );

        // Fast-forward: accumulate 30 damage via the damage_accumulator.
        // Set accumulator to 30.0 so all damage applies in one tick.
        {
            let mut b = app.world_mut().resource_mut::<ActiveBeam>();
            b.damage_accumulator = 30.0;
            b.remaining_secs = 5.0; // still "ongoing"
        }

        let out = tick(&mut app);

        // Asteroid destroyed message should be present.
        let destroyed = out.iter().find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(destroyed.is_some(), "expected AsteroidDestroyed when asteroid HP reaches 0");
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        // BeamEnded also broadcast.
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction");

        // Asteroid no longer in world data.
        assert!(
            !app.world().resource::<WorldResource>().0.asteroids.iter().any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        // Beam resource cleared.
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());

        // Cooldown started.
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after beam end");

        // The entity should be despawned.
        assert!(app.world().get::<AsteroidDamage>(asteroid_entity).is_none(),
            "asteroid entity should be despawned");
    }

    /// Beam severs when ship rotates target out of the 180° forward arc.
    #[test]
    fn beam_severs_when_target_leaves_forward_arc() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Now rotate ship so the asteroid is behind it (yaw = π → facing +Z, asteroid at (0,-20) is behind).
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves forward arc");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-arc");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after arc sever");
    }

    /// Beam severs when the target moves beyond 40-unit phaser range.
    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move asteroid position in WorldData to 50 units away (out of 40u range).
        app.world_mut().resource_mut::<WorldResource>().0.asteroids[0].z = -50.0;

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-range");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after range sever");
    }

    /// No damage refund on sever — whatever HP was dealt is permanent.
    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        let asteroid_entity = app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
        )).id();

        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Apply partial damage via accumulator.
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 10.0;
        let _ = tick(&mut app);

        // Now sever by rotating ship.
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;
        let _ = tick(&mut app);

        let hp = app.world().get::<AsteroidDamage>(asteroid_entity)
            .map(|d| d.current_hp);
        assert!(
            hp.is_some() && hp.unwrap() < 30,
            "asteroid should retain damage after sever (no refund), hp={:?}",
            hp
        );
    }

    /// A fresh FirePhaser after cooldown on a new locked target cancels any
    /// active beam and starts a new one.
    #[test]
    fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
        let mut app = test_app();

        // Set up two asteroids.
        app.world_mut().insert_resource(WorldResource(WorldData {
            asteroids: vec![
                AsteroidInfo { uuid: "t1".into(), x: 0.0, z: -20.0, radius: 2.0 },
                AsteroidInfo { uuid: "t2".into(), x: 0.0, z: -15.0, radius: 2.0 },
            ],
        }));
        start_game_with_weapons(&mut app);

        // Lock and fire at t1.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t1".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let _ = tick(&mut app);
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t1"));

        // Natural beam expiry: set remaining to 0.
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 0.0;
        // Zero damage accumulator so no destruction fires.
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 0.0;
        let _ = tick(&mut app); // beam ends, cooldown starts

        // Cooldown should be active.
        assert!(app.world().resource::<PhaserCooldown>().is_active());

        // Force cooldown to expire.
        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 0.0;

        // Lock and fire at t2.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t2".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown");
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t2"));
    }
}
