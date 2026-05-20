use bevy::prelude::*;

use crate::entity_spawner::RegionEffectsSection;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{ClientMessage, ModifierSlot};
use crate::modifiers::ShipModifiers;
use crate::region_effects::RegionEffectKind;
use crate::region_plugin::RegionMembership;
use crate::ship_physics::{compute_physics, ShipPhysicsConfig, ShipPhysicsInput, ShipPhysicsState};
use crate::ship_state::ShipState;
use crate::simulation::{Ship, ShipHullIntegrity, ShipImpulse};

// ── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource)]
struct HelmInputTimer(Timer);

#[derive(Resource, Default)]
pub struct LastHelmInput {
    pub thrust: f32,
    pub steering: f32,
}

/// Runtime ship physics config, loaded from `[helm_console]` in the entity TOML.
/// When absent, `ShipPhysicsConfig::new()` defaults are used.
#[derive(Resource, Clone)]
pub struct ShipPhysicsConfigResource(pub crate::ship_physics::ShipPhysicsConfig);

/// Runtime impulse drive config, loaded from `[helm_console]` in the entity TOML.
/// Charge duration and speed multiplier can be overridden per ship.
#[derive(Resource, Clone)]
pub struct ImpulseConfigResource {
    pub charge_duration: f32,
    pub speed_multiplier: f32,
}

impl Default for ImpulseConfigResource {
    fn default() -> Self {
        Self {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
        }
    }
}

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct ShipPlugin;

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmInputTimer(Timer::from_seconds(
            0.1,
            TimerMode::Repeating,
        )))
        .init_resource::<LastHelmInput>()
        .init_resource::<ImpulseConfigResource>()
        .add_systems(
            Update,
            (
                process_helm_inputs.in_set(crate::sim_sets::SimSet::Physics),
                tick_impulse.in_set(crate::sim_sets::SimSet::Physics),
                sync_ship_position.in_set(crate::sim_sets::SimSet::Physics),
                handle_impulse_messages.in_set(crate::sim_sets::SimSet::Input),
            )
                .after(crate::lobby::process_lobby),
        );
    }
}

// ── Systems ─────────────────────────────────────────────────────────────────

fn process_helm_inputs(
    time: Res<Time>,
    mut timer: ResMut<HelmInputTimer>,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut ship: ResMut<ShipState>,
    mut last_input: ResMut<LastHelmInput>,
    modifiers: Res<ShipModifiers>,
    ship_physics_config: Option<Res<ShipPhysicsConfigResource>>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let helm_token = sessions.0.console_holder(crate::messages::Console::Helm);
    if helm_token.is_none() {
        return;
    }

    for ev in reader.read() {
        if ev.token != helm_token.unwrap() {
            continue;
        }
        if let ClientMessage::HelmInput {
            thrust: t,
            steering: s,
        } = ev.msg
        {
            last_input.thrust = t;
            last_input.steering = s;
        }
    }

    let dt = timer.0.duration().as_secs_f32();
    let state = ShipPhysicsState {
        x: ship.x,
        z: ship.z,
        yaw: ship.yaw,
        forward_speed: ship.forward_speed,
    };
    let input = ShipPhysicsInput {
        thrust: last_input.thrust,
        steering: last_input.steering,
    };
    let mut config = match ship_physics_config.as_deref() {
        Some(cfg) => cfg.0.clone(),
        None => ShipPhysicsConfig::new(),
    };
    config.max_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
    config.max_reverse_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
    config.max_yaw_rate *= modifiers.get(&ModifierSlot::MaxYawRate);
    let result = compute_physics(state, input, dt, &config);

    ship.x = result.x;
    ship.z = result.z;
    ship.yaw = result.yaw;
    ship.forward_speed = result.forward_speed;
}

fn sync_ship_position(ship: Res<ShipState>, mut ship_query: Query<&mut Transform, With<Ship>>) {
    let Ok(mut transform) = ship_query.single_mut() else {
        return;
    };

    transform.translation.x = ship.x;
    transform.translation.z = ship.z;
    transform.rotation = Quat::from_axis_angle(Vec3::Y, ship.yaw);
}

pub fn handle_impulse_messages(
    mut reader: MessageReader<InboundMessage>,
    mut impulse: ResMut<ShipImpulse>,
    hull: Res<ShipHullIntegrity>,
    mut last_hull_hp: Local<f32>,
    membership: Option<Res<RegionMembership>>,
    region_query: Query<&RegionEffectsSection>,
    ship_query: Query<Entity, With<Ship>>,
) {
    if *last_hull_hp == 0.0 && (hull.0.total_current() - hull.0.total_max()).abs() < 1e-6 {
        *last_hull_hp = hull.0.total_max();
    }

    let current_hp = hull.0.total_current();
    if current_hp < *last_hull_hp {
        impulse.0.cancel_charge();
    }
    *last_hull_hp = current_hp;

    for msg in reader.read() {
        match &msg.msg {
            ClientMessage::StartImpulseCharge => {
                if !is_inside_blocks_impulse(&membership, &region_query, &ship_query) {
                    impulse.0.start_charge();
                }
            }
            ClientMessage::CancelImpulse => {
                impulse.0.cancel_charge();
            }
            _ => {}
        }
    }
}

fn tick_impulse(
    time: Res<Time>,
    mut impulse: ResMut<ShipImpulse>,
    config: Res<ImpulseConfigResource>,
) {
    impulse.0.tick(time.delta_secs(), config.charge_duration);
}

fn is_inside_blocks_impulse(
    membership: &Option<Res<RegionMembership>>,
    region_query: &Query<&RegionEffectsSection>,
    ship_query: &Query<Entity, With<Ship>>,
) -> bool {
    let Some(membership) = membership else {
        return false;
    };
    let Ok(ship_entity) = ship_query.single() else {
        return false;
    };
    let Some(inside) = membership.inside.get(&ship_entity) else {
        return false;
    };
    for &region_entity in inside {
        if let Ok(effects) = region_query.get(region_entity) {
            if effects
                .0
                .iter()
                .any(|e| *e == RegionEffectKind::BlocksImpulse)
            {
                return true;
            }
        }
    }
    false
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::LobbyPlugin;
    use crate::messages::ClientMessage;
    use crate::regions::server::RegionPlugin;
    use crate::entity_config::EntityConfig;
    use crate::region_shape::RegionShape;
    use crate::region_effects::{BlocksImpulseEffect, RegionEffectsConfig};
    use crate::entity_spawner::spawn_entity;
    use crate::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::ship_state::ShipState;
    use crate::modifiers::ShipModifiers;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(crate::damage::ConsoleHull::from_config(&[
                (crate::messages::Console::Helm, 25.0),
                (crate::messages::Console::Tactical, 25.0),
                (crate::messages::Console::Power, 25.0),
                (crate::messages::Console::Shields, 25.0),
            ])))
            .insert_resource(crate::simulation::ShipShields(crate::shield::ShieldSystem::default()))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(ShipModifiers::new())
            .add_plugins(ShipPlugin);
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) {
        app.update();
    }

    fn start_game_with_helm(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn start_game_with_helm_and_science(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Hikaru".into(),
            },
        );
        tick(app);
        push(
            app,
            "helm",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    // ── Impulse Drive / Damage Cancellation tests ──────────────────────────

    #[test]
    fn hull_damage_cancels_charging_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );

        {
            let mut rng = rand::rng();
            app.world_mut()
                .resource_mut::<ShipHullIntegrity>()
                .0
                .apply_damage(10.0, &mut rng);
        }
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
            "impulse charge should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn hull_damage_cancels_active_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        {
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
            imp.0.start_charge();
            imp.0.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        }
        assert!(
            app.world().resource::<ShipImpulse>().0.is_active(),
            "impulse should be active before damage"
        );

        {
            let mut rng = rand::rng();
            app.world_mut()
                .resource_mut::<ShipHullIntegrity>()
                .0
                .apply_damage(10.0, &mut rng);
        }
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
            "active impulse should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn no_hull_damage_does_not_cancel_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
            "impulse should still be charging when no damage occurred"
        );
    }

    #[test]
    fn start_impulse_charge_message_begins_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
        );
    }

    #[test]
    fn cancel_impulse_message_cancels_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);
        push(&mut app, "helm", ClientMessage::CancelImpulse);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
        );
    }

    // ── BlocksImpulse region gating tests ────────────────────────────

    fn blocks_impulse_test_app() -> App {
        let mut app = test_app();
        app.add_plugins(RegionPlugin);
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    fn spawn_blocks_impulse_region(app: &mut App, x: f32, z: f32, radius: f32) -> bevy::ecs::entity::Entity {
        let config = EntityConfig {
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: Some(RegionEffectsConfig {
                blocks_impulse: Some(BlocksImpulseEffect {}),
                ..Default::default()
            }),
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            science_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            sensors_console: None,
            star: None,
            planet: None,
            asteroid_field: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    #[test]
    fn start_impulse_charge_ignored_inside_blocks_impulse_region() {
        let mut app = blocks_impulse_test_app();

        let _region = spawn_blocks_impulse_region(&mut app, 0.0, 0.0, 50.0);

        start_game_with_helm_and_science(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
            "impulse should be idle before StartImpulseCharge"
        );

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Idle,
            "StartImpulseCharge should be ignored inside BlocksImpulse region"
        );
    }

    #[test]
    fn start_impulse_charge_works_outside_blocks_impulse_region() {
        let mut app = blocks_impulse_test_app();

        let _region = spawn_blocks_impulse_region(&mut app, 500.0, 0.0, 50.0);

        start_game_with_helm_and_science(&mut app);

        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
            "StartImpulseCharge should work when outside BlocksImpulse region"
        );
    }
}
