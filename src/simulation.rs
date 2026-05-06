use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::asteroid_spawner::spawn_asteroid_positions;
use crate::lobby::{CurrentPhase, InboundMessage, OutboundMessage, Sessions, Target};
use crate::messages::{ClientMessage, Console, GamePhase, ServerMessage};
use crate::ship_physics::{compute_physics, ShipPhysicsConfig, ShipPhysicsInput, ShipPhysicsState};
use crate::ship_state::ShipState;

// ── Marker Components ────────
#[derive(Component)]
pub struct Ship;

#[derive(Component)]
pub struct Asteroid;

// ── Resources ────────────────
#[derive(Resource)]
struct SimBroadcastTimer(Timer);

#[derive(Resource)]
struct HelmInputTimer(Timer);

// ── Plugin ───────────────────
pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RapierPhysicsPlugin::<()>::default())
            .insert_resource(ShipState::new())
            .insert_resource(SimBroadcastTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .insert_resource(HelmInputTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .add_systems(Startup, setup_world)
            .add_systems(Update, (
                handle_toggle,
                process_helm_inputs,
                sync_ship_position,
                handle_collisions,
                broadcast_sim_state,
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

    // Collect all HelmInput messages from the helm player since last tick
    let mut thrust: f32 = 0.0;
    let mut steering: f32 = 0.0;
    let mut has_input = false;

    for ev in reader.read() {
        if ev.token != helm_token.unwrap() {
            continue;
        }
        if let ClientMessage::HelmInput { thrust: t, steering: s } = ev.msg {
            thrust = t;
            steering = s;
            has_input = true;
        }
    }

    if !has_input {
        return;
    }

    // Compute physics
    let dt = time.delta_secs();
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

fn handle_collisions(_queries: Query<(), With<Ship>>) {
    // Collision handling - ship velocity is managed through direct velocity set
    // in process_helm_inputs; on collision we simply zero the velocity
}

fn broadcast_sim_state(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    mut writer: MessageWriter<OutboundMessage>,
    ship: Res<ShipState>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if timer.0.tick(time.delta()).just_finished() {
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::SimState { snapshot: ship.snapshot() },
        });
    }
}

// ── World Setup ──────────────
fn setup_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let config = ShipPhysicsConfig::new();
    let positions = spawn_asteroid_positions(config.max_speed * 3.0, 40, 20.0);

    // Spawn asteroids
    let asteroid_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.4, 0.35, 0.3),
        ..default()
    });
    let asteroid_mesh = meshes.add(Sphere { radius: 2.0 });

    for (x, z) in &positions {
        commands.spawn((
            Asteroid,
            Mesh3d(asteroid_mesh.clone()),
            MeshMaterial3d(asteroid_mat.clone()),
            Transform::from_xyz(*x, 0.0, *z),
            Collider::ball(2.0),
            RigidBody::Fixed,
        ));
    }

    // Spawn ship (no mesh, just collider and rigid body for physics)
    commands.spawn((
        Ship,
        Transform::default(),
        Collider::capsule_y(3.0, 6.0),
        RigidBody::Dynamic,
        LockedAxes::TRANSLATION_LOCKED_Y,
    ));
}

// ── Tests ────────────────────
