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

// Ã¢â€â‚¬Ã¢â€â‚¬ Resources Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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
    pub acceleration_multiplier: f32,
}

impl Default for ImpulseConfigResource {
    fn default() -> Self {
        Self {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER,
        }
    }
}

/// Runtime banking config, loaded from `[helm_console] max_bank_deg` in the entity TOML.
#[derive(Resource, Clone)]
pub struct BankConfigResource {
    pub max_bank_deg: f32,
}

impl Default for BankConfigResource {
    fn default() -> Self {
        Self { max_bank_deg: 0.0 }
    }
}

/// How quickly the ship's visual roll lerps toward the target bank angle.
const BANK_LERP_RATE: f32 = 5.0;

// Ã¢â€â‚¬Ã¢â€â‚¬ Plugin Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

pub struct ShipPlugin;

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmInputTimer(Timer::from_seconds(
            0.1,
            TimerMode::Repeating,
        )))
        .init_resource::<LastHelmInput>()
        .init_resource::<ImpulseConfigResource>()
        .init_resource::<BankConfigResource>()
        .add_systems(
            Update,
            (
                process_helm_inputs.in_set(crate::sim_sets::SimSet::Physics),
                tick_impulse.in_set(crate::sim_sets::SimSet::Physics),
                sync_ship_position
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .before(crate::sim_sets::AiTickLabel),
                handle_impulse_messages.in_set(crate::sim_sets::SimSet::Input),
            )
                .after(crate::lobby::process_lobby),
        );
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Systems Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

fn process_helm_inputs(
    time: Res<Time>,
    mut timer: ResMut<HelmInputTimer>,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut ship: ResMut<ShipState>,
    mut last_input: ResMut<LastHelmInput>,
    modifiers: Res<ShipModifiers>,
    ship_physics_config: Option<Res<ShipPhysicsConfigResource>>,
    impulse: Res<ShipImpulse>,
    impulse_config: Res<ImpulseConfigResource>,
    bank_config: Res<BankConfigResource>,
    mut prev_phase: Local<Option<crate::impulse::ImpulsePhase>>,
) {
    // Edge-detect Idle → Charging (or any → Charging) and zero out the
    // last cached helm input so a stale steering/thrust value can't
    // resurface the moment impulse cancels or the autopilot disengages.
    // Mirrors the `prev_phase` Local pattern in
    // `modifiers/coordination.rs::translate_impulse_modifiers`.
    let current_phase = impulse.0.phase;
    if Some(current_phase) != *prev_phase {
        if current_phase == crate::impulse::ImpulsePhase::Charging {
            last_input.thrust = 0.0;
            last_input.steering = 0.0;
        }
        *prev_phase = Some(current_phase);
    }

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
    let impulse_active = impulse.0.is_active();
    let input = if impulse_active {
        // Autopilot: full forward thrust, zero steering. Player input is ignored.
        ShipPhysicsInput {
            thrust: 1.0,
            steering: 0.0,
        }
    } else {
        ShipPhysicsInput {
            thrust: last_input.thrust,
            steering: last_input.steering,
        }
    };
    let mut config = match ship_physics_config.as_deref() {
        Some(cfg) => cfg.0,
        None => ShipPhysicsConfig::new(),
    };
    config.max_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
    config.max_reverse_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
    config.max_yaw_rate *= modifiers.get(&ModifierSlot::MaxYawRate);
    if impulse_active {
        // Mirror `ship/impulse.rs::apply_to_physics`: a non-positive
        // multiplier (e.g. an unset TOML field defaulting to 0) falls
        // back to the const instead of nuking acceleration entirely.
        let mult = if impulse_config.acceleration_multiplier > 0.0 {
            impulse_config.acceleration_multiplier
        } else {
            crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
        };
        config.acceleration *= mult;
    }
    let result = compute_physics(state, input, dt, &config);

    ship.x = result.x;
    ship.z = result.z;
    ship.yaw = result.yaw;
    ship.forward_speed = result.forward_speed;

    // Visual banking: lerp roll toward target based on steering
    let max_bank_rad = bank_config.max_bank_deg.to_radians();
    let target_roll = if impulse_active {
        0.0
    } else {
        -input.steering * max_bank_rad
    };
    let lerp_factor = (BANK_LERP_RATE * dt).min(1.0);
    ship.roll = ship.roll + (target_roll - ship.roll) * lerp_factor;
}

fn sync_ship_position(ship: Res<ShipState>, mut ship_query: Query<&mut Transform, With<Ship>>) {
    let Ok(mut transform) = ship_query.single_mut() else {
        return;
    };

    transform.translation.x = ship.x;
    transform.translation.z = ship.z;
    transform.rotation = Quat::from_euler(EulerRot::YXZ, ship.yaw, 0.0, ship.roll);
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
            ClientMessage::StartImpulseCharge
                if !is_inside_blocks_impulse(&membership, &region_query, &ship_query) =>
            {
                impulse.0.start_charge();
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
            if effects.0.contains(&RegionEffectKind::BlocksImpulse) {
                return true;
            }
        }
    }
    false
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::EntityConfig;
    use crate::entity_spawner::spawn_entity;
    use crate::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::lobby::LobbyPlugin;
    use crate::messages::ClientMessage;
    use crate::modifiers::ShipModifiers;
    use crate::region_effects::{BlocksImpulseEffect, RegionEffectsConfig};
    use crate::region_shape::RegionShape;
    use crate::regions::server::RegionPlugin;
    use crate::ship_state::ShipState;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(crate::damage::ConsoleHull::from_config(
                &[
                    (crate::messages::Console::Helm, 25.0),
                    (crate::messages::Console::Tactical, 25.0),
                    (crate::messages::Console::Power, 25.0),
                    (crate::messages::Console::Shields, 25.0),
                ],
            )))
            .insert_resource(crate::simulation::ShipShields(
                crate::shield::ShieldSystem::default(),
            ))
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Impulse Drive / Damage Cancellation tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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

    // Ã¢â€â‚¬Ã¢â€â‚¬ BlocksImpulse region gating tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn blocks_impulse_test_app() -> App {
        let mut app = test_app();
        app.add_plugins(RegionPlugin);
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    fn spawn_blocks_impulse_region(
        app: &mut App,
        x: f32,
        z: f32,
        radius: f32,
    ) -> bevy::ecs::entity::Entity {
        let config = EntityConfig {
            name: None,
            light: Vec::new(),
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
            shields_console: None,
            torpedoes: None,
            repair: None,
            comms: None,
            sensors_console: None,
            navigation_console: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            target: None,
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

    // ── Impulse autopilot tests ───────────────────────────────────────

    /// While the impulse drive is Active, the server should ignore any helm
    /// input from the player and autopilot the ship: full forward thrust,
    /// zero steering. The configured `acceleration_multiplier` boosts the
    /// base acceleration so the ship ramps up to the boosted top speed.
    #[test]
    fn active_impulse_autopilots_with_boosted_acceleration() {
        let mut app = test_app();
        // 5x boost: base accel = 25/3 ≈ 8.33; boosted = ~41.67 per second.
        // Timer fires every 100ms, so the very first tick advances physics by
        // dt = 0.1s → expected forward_speed ≈ 4.17.
        app.insert_resource(ImpulseConfigResource {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: 5.0,
        });
        start_game_with_helm_and_science(&mut app);

        // Activate impulse directly (bypass charge).
        {
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
            imp.0.start_charge();
            imp.0.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        }

        // Player tries to fight the autopilot: zero thrust, hard right steer.
        // The server must ignore both and force thrust=1.0, steering=0.0.
        push(
            &mut app,
            "helm",
            ClientMessage::HelmInput {
                thrust: 0.0,
                steering: 1.0,
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipState>();
        // With a 5x boost and dt=0.1s, expect roughly +4.17 forward.
        // Without the boost we'd expect ~+0.83; require >=3.0 to clearly
        // distinguish the boosted path.
        assert!(
            ship.forward_speed >= 3.0,
            "active impulse should autopilot with boosted accel; got forward_speed={}",
            ship.forward_speed
        );
        // Steering must be ignored — yaw should be essentially unchanged.
        assert!(
            ship.yaw.abs() < 1e-3,
            "active impulse must zero steering; got yaw={}",
            ship.yaw
        );
    }

    /// While the impulse drive is Idle, the configured
    /// `acceleration_multiplier` must have no effect — it applies only
    /// during the Active phase.
    #[test]
    fn idle_impulse_does_not_boost_acceleration() {
        let mut app = test_app();
        app.insert_resource(ImpulseConfigResource {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: 5.0,
        });
        start_game_with_helm_and_science(&mut app);

        // Impulse stays Idle; helm asks for full thrust.
        push(
            &mut app,
            "helm",
            ClientMessage::HelmInput {
                thrust: 1.0,
                steering: 0.0,
            },
        );
        tick(&mut app);

        let ship = app.world().resource::<ShipState>();
        // Base accel ≈ 8.33, dt = 0.1 → expected ≈ 0.83. Cap at 2.0 to
        // catch any accidental boost.
        assert!(
            ship.forward_speed < 2.0,
            "idle impulse must not boost accel; got forward_speed={}",
            ship.forward_speed
        );
    }

    /// A non-positive `acceleration_multiplier` (e.g. an unconfigured TOML
    /// field that defaults to 0.0) must fall back to the const
    /// `IMPULSE_ACCELERATION_MULTIPLIER` instead of nuking acceleration
    /// during impulse. Mirrors the `speed_multiplier <= 0` fallback in
    /// `ship/impulse.rs::apply_to_physics`.
    #[test]
    fn zero_acceleration_multiplier_falls_back_to_const() {
        let mut app = test_app();
        app.insert_resource(ImpulseConfigResource {
            charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: 0.0,
        });
        start_game_with_helm_and_science(&mut app);

        // Activate impulse directly (bypass charge).
        {
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
            imp.0.start_charge();
            imp.0.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
        }
        tick(&mut app);

        let ship = app.world().resource::<ShipState>();
        // Const is 5.0 → expect ≈ +4.17/tick. Without the fallback,
        // forward_speed would be ~0 (0× accel during impulse).
        assert!(
            ship.forward_speed >= 3.0,
            "zero acceleration_multiplier must fall back to const; \
             got forward_speed={}",
            ship.forward_speed
        );
    }

    /// REGRESSION (review finding #1 / #5): when impulse starts charging,
    /// the server's `LastHelmInput` must be cleared so a stale steering
    /// value can't immediately fly the ship the moment impulse cancels
    /// (or in the post-active autopilot-disengage frame).
    ///
    /// Reproduce: send `HelmInput { steering: 1.0 }`, then
    /// `StartImpulseCharge`, then `CancelImpulse`, then tick. Without the
    /// fix the post-cancel tick will read the stale `steering=1.0` and
    /// rotate the ship.
    #[test]
    fn stale_helm_steering_cleared_when_impulse_starts_charging() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        // Player jams the steering hard right before pressing IMPULSE.
        push(
            &mut app,
            "helm",
            ClientMessage::HelmInput {
                thrust: 0.0,
                steering: 1.0,
            },
        );
        tick(&mut app);

        // Press IMPULSE → starts charging. `LastHelmInput` must be cleared.
        push(&mut app, "helm", ClientMessage::StartImpulseCharge);
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipImpulse>().0.phase,
            ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );
        let last = app.world().resource::<LastHelmInput>();
        assert_eq!(
            (last.thrust, last.steering),
            (0.0, 0.0),
            "LastHelmInput must be zeroed on Charging transition; got \
             thrust={}, steering={}",
            last.thrust,
            last.steering
        );

        // Snapshot yaw, then cancel and tick once. With the bug, the
        // post-cancel tick replays steering=1.0 and yaw changes.
        let yaw_before = app.world().resource::<ShipState>().yaw;
        push(&mut app, "helm", ClientMessage::CancelImpulse);
        tick(&mut app);
        let yaw_after = app.world().resource::<ShipState>().yaw;
        assert!(
            (yaw_after - yaw_before).abs() < 1e-3,
            "post-cancel tick must not autopilot a phantom turn; \
             yaw drifted by {}",
            yaw_after - yaw_before
        );
    }
}
