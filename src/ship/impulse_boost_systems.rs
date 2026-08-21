use bevy::prelude::*;

use crate::lobby::Sessions;
use crate::server_app::{LocalShip, Ship};
use crate::ship::components::{
    BoostConfigResource, ImpulseConfigResource, LastHelmInput, ShipConfigComponent,
    ShipSystemControlSources,
};
use crate::ship::helm::ImpulseCommand;
use crate::ship::helm_ai::helm_axes_operate_ai;
use crate::simulation::{ShipBoost, ShipImpulse};

/// Hull-damage impulse auto-cancel (issue #695, reshaped by #824): writes an
/// `Idle` `ImpulseCommand` intent when the LocalShip's hull took damage this
/// tick, rather than mutating `ShipImpulse` directly. The shared
/// `apply_helm_commands` system applies the actual `cancel_charge`
/// transition.
///
/// The admitted `StartImpulseCharge`/`CancelImpulse` loop this system used to
/// carry moved to `ship::helm_admission::process_helm_inputs` (issue #824),
/// which applies impulse commands for every ship — human-admitted and
/// AI-emitted alike — with the same `BlocksImpulse` region gate. This system
/// runs in `SimSet::Input`, before `process_helm_inputs` (`SimSet::Physics`),
/// so an admitted command can still override the hull-damage cancel within
/// the same tick — matching the old sequential direct-mutation order exactly.
pub fn handle_impulse_messages(
    impulse_q: Query<&ShipImpulse, With<LocalShip>>,
    mut impulse_cmd_q: Query<&mut ImpulseCommand, With<LocalShip>>,
    hull_q: Query<&crate::entity_spawner::EntitySystemHull, With<LocalShip>>,
    mut last_hull_hp: Local<f32>,
) {
    // Guard: only proceed when the LocalShip actually carries `ShipImpulse`
    // (matches the old direct-mutation code's implicit guard).
    if impulse_q.iter().next().is_none() {
        return;
    }
    let hull_total = hull_q
        .single()
        .map(|h| (h.0.total_current(), h.0.total_max()))
        .unwrap_or((100.0, 100.0));
    if *last_hull_hp == 0.0 && (hull_total.0 - hull_total.1).abs() < 1e-6 {
        *last_hull_hp = hull_total.1;
    }

    let current_hp = hull_total.0;
    let mut desired: Option<crate::impulse::ImpulsePhase> = None;
    if current_hp < *last_hull_hp {
        desired = Some(crate::impulse::ImpulsePhase::Idle);
    }
    *last_hull_hp = current_hp;

    if let Some(phase) = desired {
        if let Some(mut cmd_comp) = impulse_cmd_q.iter_mut().next() {
            cmd_comp.0 = phase;
        }
    }
}

pub(crate) fn tick_impulse(
    time: Res<Time>,
    mut ships_q: Query<(&mut ShipImpulse, Option<&ImpulseConfigResource>), With<Ship>>,
) {
    let dt = time.delta_secs();
    for (mut impulse, entity_cfg) in ships_q.iter_mut() {
        let charge_duration = entity_cfg.cloned().unwrap_or_default().charge_duration;
        impulse.0.tick(dt, charge_duration);
    }
}

// `handle_boost_messages` was retired by issue #881 into
// `ship::helm_admission::process_helm_inputs`, the shared per-entity applier
// that already carried the impulse payloads (issue #824 did the same
// retirement for `handle_impulse_messages`' admission loop). It was
// `With<LocalShip>` and read a single entity, so the admitted `SetBoost` that
// `ai_helm_boost` (#780) emits for a non-local `AiHighFidelity` NPC never
// reached `BoostCommand`. `BoostCommand` now has exactly one writer.

fn normalized_boost_drain_factor(thrust: f32, steering: f32) -> f32 {
    thrust.clamp(-1.0, 1.0).abs() + steering.clamp(-1.0, 1.0).abs()
}

pub(crate) fn tick_boost(
    time: Res<Time>,
    mut boost_entity_q: Query<(Option<&BoostConfigResource>, &mut ShipBoost), With<LocalShip>>,
    last_input_q: Query<&LastHelmInput, With<LocalShip>>,
    sessions: Res<Sessions>,
    impulse_q: Query<&ShipImpulse, With<LocalShip>>,
    ship_components: Query<(&ShipConfigComponent, &ShipSystemControlSources), With<LocalShip>>,
) {
    let Some((_ship_config, control_sources)) = ship_components.iter().next() else {
        return;
    };
    let Some((entity_cfg, mut entity_boost)) = boost_entity_q.iter_mut().next() else {
        return;
    };
    let config = entity_cfg.cloned().unwrap_or_default();
    if !config.enabled {
        return;
    }
    let last_input = last_input_q.single().copied().unwrap_or_default();
    let has_helm = sessions
        .0
        .holder_for_station(&crate::messages::StationId(
            crate::system_registry::HELM_STATION_ID.into(),
        ))
        .is_some()
        || helm_axes_operate_ai(control_sources);
    let impulse_active = impulse_q
        .iter()
        .next()
        .map(|i| i.0.is_active())
        .unwrap_or(false);
    let drain_factor = if !has_helm {
        0.0
    } else if impulse_active {
        normalized_boost_drain_factor(1.0, 0.0)
    } else {
        normalized_boost_drain_factor(last_input.thrust, last_input.steering)
    };
    entity_boost.0.tick_with_drain_factor(
        time.delta_secs(),
        config.active_duration,
        config.recharge_duration,
        drain_factor,
    );
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::entity_config::EntityConfig;
    use crate::entity_spawner::spawn_entity;
    use crate::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::messages::{ClientMessage, SystemControlPayload};
    use crate::region_effects::{BlocksImpulseEffect, RegionEffectsConfig};
    use crate::region_shape::RegionShape;
    use crate::regions::server::RegionPlugin;
    use crate::ship::test_support::*;

    fn apply_hull_damage(app: &mut App, amount: f32) {
        let mut rng = crate::sim_rng::unseeded_test_rng();
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
            .unwrap()
            .0
            .apply_damage(amount, &mut rng);
    }

    fn toggle_boost(app: &mut App) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ShipBoost>()
            .unwrap()
            .0
            .toggle();
    }

    fn boost_is_active(app: &mut App) -> bool {
        let ship = find_ship_entity(app);
        app.world()
            .entity(ship)
            .get::<ShipBoost>()
            .map(|b| b.0.is_active())
            .unwrap_or(false)
    }

    fn boost_battery(app: &mut App) -> f32 {
        let ship = find_ship_entity(app);
        app.world()
            .entity(ship)
            .get::<ShipBoost>()
            .map(|b| b.0.battery)
            .unwrap_or(0.0)
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Impulse Drive / Damage Cancellation tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn hull_damage_cancels_charging_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );

        apply_hull_damage(&mut app, 10.0);
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "impulse charge should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn hull_damage_cancels_active_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        // One tick to let handle_impulse_messages initialise last_hull_hp from the
        // current (undamaged) hull, so a subsequent damage event is detected.
        tick(&mut app);

        {
            let active = {
                let mut s = crate::impulse::ImpulseState::new();
                s.start_charge();
                s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
                s
            };
            set_ship_impulse(&mut app, active);
        }
        assert!(
            get_ship_impulse(&mut app).is_active(),
            "impulse should be active before damage"
        );

        apply_hull_damage(&mut app, 10.0);
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "active impulse should be cancelled when hull damage is taken"
        );
    }

    #[test]
    fn no_hull_damage_does_not_cancel_impulse() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "impulse should still be charging when no damage occurred"
        );
    }

    #[test]
    fn start_impulse_charge_message_begins_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Charging,);
    }

    #[test]
    fn control_system_start_impulse_charge_begins_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Charging,);
    }

    #[test]
    fn cancel_impulse_message_cancels_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::CancelImpulse,
            },
        );
        tick(&mut app);

        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Idle,);
    }

    #[test]
    fn control_system_cancel_impulse_cancels_charge() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::CancelImpulse,
            },
        );
        tick(&mut app);

        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Idle,);
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ BlocksImpulse region gating tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn blocks_impulse_test_app() -> App {
        let mut app = test_app();
        app.add_plugins(RegionPlugin);
        app
    }

    fn spawn_blocks_impulse_region(
        app: &mut App,
        x: f32,
        z: f32,
        radius: f32,
    ) -> bevy::ecs::entity::Entity {
        let config = EntityConfig {
            reference_grid: None,
            name: None,
            display_name: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
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
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            sensors_console: None,
            navigation_console: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
            css: None,
            target: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
            infrastructure: None,
            operations: None,
            scan: None,
            tractor: None,
            held_response: None,
            dock: None,
            umbilical: None,
            civilian: None,
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
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "impulse should be idle before StartImpulseCharge"
        );

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "StartImpulseCharge should be ignored inside BlocksImpulse region"
        );
    }

    #[test]
    fn start_impulse_charge_works_outside_blocks_impulse_region() {
        let mut app = blocks_impulse_test_app();

        let _region = spawn_blocks_impulse_region(&mut app, 500.0, 0.0, 50.0);

        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);

        assert_eq!(
            get_ship_impulse(&mut app).phase,
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
        // Timer fires at 30 Hz (dt ≈ 1/30 s), so the first tick gives
        // forward_speed ≈ 41.67/30 ≈ 1.39.
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource {
                charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
                speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
                acceleration_multiplier: 5.0,
                engage_distance: 200.0,
                cancel_distance: 40.0,
                steering_multiplier: 0.0,
            });
        start_game_with_helm_and_science(&mut app);

        // Activate impulse directly (bypass charge).
        {
            let mut s = crate::impulse::ImpulseState::new();
            s.start_charge();
            s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
            set_ship_impulse(&mut app, s);
        }

        // Player tries to fight the autopilot: zero thrust, hard right steer.
        // The server must ignore both and force thrust=1.0, steering=0.0.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
            },
        );
        tick(&mut app);

        let physics = get_ship_physics(&mut app);
        // With 5x boost, expect ≈1.39; without boost ≈0.28. Require >=1.0 to
        // clearly distinguish the boosted path.
        assert!(
            physics.forward_speed >= 1.0,
            "active impulse should autopilot with boosted accel; got forward_speed={}",
            physics.forward_speed
        );
        // Steering must be ignored — yaw should be essentially unchanged.
        assert!(
            physics.yaw.abs() < 1e-3,
            "active impulse must zero steering; got yaw={}",
            physics.yaw
        );
    }

    /// While the impulse drive is Idle, the configured
    /// `acceleration_multiplier` must have no effect — it applies only
    /// during the Active phase.
    #[test]
    fn idle_impulse_does_not_boost_acceleration() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource {
                charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
                speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
                acceleration_multiplier: 5.0,
                engage_distance: 200.0,
                cancel_distance: 40.0,
                steering_multiplier: 0.0,
            });
        start_game_with_helm_and_science(&mut app);

        // Impulse stays Idle; helm asks for full thrust.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.0 },
            },
        );
        tick(&mut app);

        let physics = get_ship_physics(&mut app);
        // Base accel ≈ 8.33, dt = 1/30 → expected ≈ 0.28. Cap at 2.0 to
        // catch any accidental boost.
        assert!(
            physics.forward_speed < 2.0,
            "idle impulse must not boost accel; got forward_speed={}",
            physics.forward_speed
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
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(ImpulseConfigResource {
                charge_duration: crate::impulse::IMPULSE_CHARGE_DURATION,
                speed_multiplier: crate::impulse::IMPULSE_SPEED_MULTIPLIER,
                acceleration_multiplier: 0.0,
                engage_distance: 200.0,
                cancel_distance: 40.0,
                steering_multiplier: 0.0,
            });
        start_game_with_helm_and_science(&mut app);

        // Activate impulse directly (bypass charge).
        {
            let mut s = crate::impulse::ImpulseState::new();
            s.start_charge();
            s.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
            set_ship_impulse(&mut app, s);
        }
        tick(&mut app);

        let physics = get_ship_physics(&mut app);
        // Const is 5.0 → expect ≈ 1.39/tick (dt=1/30). Without the fallback,
        // forward_speed would be ~0 (0× accel during impulse).
        assert!(
            physics.forward_speed >= 1.0,
            "zero acceleration_multiplier must fall back to const; \
             got forward_speed={}",
            physics.forward_speed
        );
    }

    // ── Boost drive tests ─────────────────────────────────────────────

    fn enabled_boost_config() -> BoostConfigResource {
        BoostConfigResource {
            enabled: true,
            multiplier: 3.0,
            steering_multiplier: 2.0,
            active_duration: 4.0,
            recharge_duration: 20.0,
        }
    }

    /// With boost enabled and engaged, the ship accelerates ~3× faster than the
    /// un-boosted baseline (multiplier applies to both accel and max speed).
    #[test]
    fn active_boost_triples_acceleration() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);
        toggle_boost(&mut app); // engage
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.0 },
            },
        );
        tick(&mut app);
        let boosted = get_ship_physics(&mut app).forward_speed;

        // Baseline: identical run with boost left disabled.
        let mut base = test_app();
        start_game_with_helm_and_science(&mut base);
        push(
            &mut base,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut base,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.0 },
            },
        );
        tick(&mut base);
        let baseline = get_ship_physics(&mut base).forward_speed;

        assert!(baseline > 0.0, "baseline should move; got {baseline}");
        assert!(
            (boosted - baseline * 3.0).abs() < baseline * 0.1,
            "boosted ({boosted}) should be ~3× baseline ({baseline})"
        );
    }

    /// With boost enabled and engaged, steering uses the separate configured
    /// yaw-rate multiplier.
    #[test]
    fn active_boost_multiplies_steering_rate() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);
        toggle_boost(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
            },
        );
        tick(&mut app);
        let boosted_yaw = get_ship_physics(&mut app).yaw;

        let mut base = test_app();
        start_game_with_helm_and_science(&mut base);
        push(
            &mut base,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.0 },
            },
        );
        push(
            &mut base,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
            },
        );
        tick(&mut base);
        let baseline_yaw = get_ship_physics(&mut base).yaw;

        assert!(
            baseline_yaw > 0.0,
            "baseline should turn; got {baseline_yaw}"
        );
        assert!(
            (boosted_yaw - baseline_yaw * 2.0).abs() < baseline_yaw * 0.1,
            "boosted yaw ({boosted_yaw}) should be ~2× baseline ({baseline_yaw})"
        );
    }

    #[test]
    fn active_boost_battery_drain_scales_with_helm_demand() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        toggle_boost(&mut app);
        {
            set_last_helm_input(
                &mut app,
                LastHelmInput {
                    thrust: 1.0,
                    steering: 1.0,
                    lateral: 0.0,
                },
            );
        }

        tick(&mut app);

        let battery = boost_battery(&mut app);
        assert!(
            (battery - 0.9).abs() < 0.001,
            "full thrust + full steering should drain twice the base rate; got {battery}"
        );
    }

    #[test]
    fn active_boost_battery_does_not_drain_with_idle_helm() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        toggle_boost(&mut app);

        tick(&mut app);

        let battery = boost_battery(&mut app);
        assert!(
            (battery - 1.0).abs() < f32::EPSILON,
            "idle helm should not spend boost battery; got {battery}"
        );
    }

    /// A `ToggleBoost` message engages the drive only when the feature is
    /// enabled for this ship.
    #[test]
    fn toggle_boost_engages_when_enabled() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_boost_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(boost_is_active(&mut app));

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_boost_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(!boost_is_active(&mut app));
    }

    #[test]
    fn control_system_toggle_boost_engages_when_enabled() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_boost_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(boost_is_active(&mut app));
    }

    #[test]
    fn control_system_set_boost_sets_active_state() {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(enabled_boost_config());
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_boost_system_id(),
                payload: SystemControlPayload::SetBoost { active: true },
            },
        );
        tick(&mut app);
        assert!(boost_is_active(&mut app));

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_boost_system_id(),
                payload: SystemControlPayload::SetBoost { active: false },
            },
        );
        tick(&mut app);
        assert!(!boost_is_active(&mut app));
    }

    /// When boost is disabled (no TOML), `ToggleBoost` is ignored and the
    /// multiplier never applies even if the state were somehow active.
    #[test]
    fn toggle_boost_ignored_when_disabled() {
        let mut app = test_app(); // BoostConfigResource defaults to disabled
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_boost_system_id(),
                payload: SystemControlPayload::ToggleBoost,
            },
        );
        tick(&mut app);
        assert!(
            !boost_is_active(&mut app),
            "ToggleBoost must be a no-op when boost is disabled"
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
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
            },
        );
        tick(&mut app);

        // Press IMPULSE → starts charging. `LastHelmInput` must be cleared.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
            },
        );
        tick(&mut app);
        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "impulse should be charging after StartImpulseCharge"
        );
        let last = get_last_helm_input(&mut app);
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
        let yaw_before = get_ship_physics(&mut app).yaw;
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::CancelImpulse,
            },
        );
        tick(&mut app);
        let yaw_after = get_ship_physics(&mut app).yaw;
        assert!(
            (yaw_after - yaw_before).abs() < 1e-3,
            "post-cancel tick must not autopilot a phantom turn; \
             yaw drifted by {}",
            yaw_after - yaw_before
        );
    }
}
