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
                handle_set_view,
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
        if let ClientMessage::SetView { direction } = ev.msg.clone() {
            if sessions.0.console_holder(Console::CaptainChair) == Some(ev.token.as_str()) {
                ship.view_direction = direction;
            }
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
) {
    let Ok(ctx) = context.single() else { return };
    let Ok(ship_entity) = ship_query.single() else { return };
    if ctx.contact_pairs_with(ship_entity).next().is_some() {
        ship.forward_speed = 0.0;
    }
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
        app.add_plugins(LobbyPlugin)
            .insert_resource(ShipState::new())
            .init_resource::<Outbox>()
            .add_systems(Update, handle_set_view)
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
        push(&mut app, "captain", ClientMessage::SetView { direction: ViewDirection::Starboard });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_direction,
            ViewDirection::Fore
        );
    }

    #[test]
    fn non_captain_set_view_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "crew", ClientMessage::Identify { token: "crew".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "crew", ClientMessage::SetView { direction: ViewDirection::Port });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_direction,
            ViewDirection::Fore
        );
    }

    #[test]
    fn captain_set_view_changes_direction() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { direction: ViewDirection::Aft });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_direction,
            ViewDirection::Aft
        );
    }
}
