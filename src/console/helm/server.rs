use bevy::prelude::*;

use crate::damage::DamageTier;
use crate::messages::{
    HelmBlackboard, HelmEngineBlackboard, InterSystemPayload, InterSystemQueue,
    SystemBlackboard, SystemId,
};
use crate::server_app::{ShipBoost, ShipImpulse};
use crate::ship_plugin::BoostConfigResource;
use crate::ship_state::ShipPhysics;
use crate::system_registry::{
    helm_engine_port_system_id, helm_engine_starboard_system_id, HELM_SYSTEM_ID,
};

pub struct HelmPlugin;

impl Plugin for HelmPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            publish_helm_blackboard.in_set(crate::sim_sets::SimSet::Publish),
        );
    }
}

/// Publish the Helm system's blackboard from current sim state.
/// Runs in `SimSet::Publish` (phase 1a) so downstream Broadcast systems
/// see fully-updated values. The component-change dirty-tracking is done
/// globally by `broadcast_blackboard_updates` in `SimSet::Broadcast`.
///
/// Also publishes per-engine `HelmEngine` blackboard entries (issue #511).
fn publish_helm_blackboard(
    physics_q: Query<
        (&ShipPhysics, Option<&BoostConfigResource>),
        With<crate::simulation::LocalShip>,
    >,
    impulse_q: Query<&ShipImpulse, With<crate::simulation::LocalShip>>,
    impulse_res: Option<Res<ShipImpulse>>,
    boost_q: Query<&ShipBoost, With<crate::simulation::LocalShip>>,
    boost_res: Option<Res<ShipBoost>>,
    boost_config_res: Option<Res<BoostConfigResource>>,
    hull_q: Query<&crate::entity_spawner::EntitySystemHull, With<crate::simulation::LocalShip>>,
    last_input_q: Query<&crate::ship_plugin::LastHelmInput, With<crate::simulation::LocalShip>>,
    queue: Res<InterSystemQueue>,
    mut ship_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::simulation::LocalShip>,
    >,
) {
    let (physics, entity_boost_cfg) = physics_q
        .single()
        .ok()
        .map(|(p, c)| (*p, c.cloned()))
        .unwrap_or_default();
    // Prefer per-entity Component on LocalShip; fall back to Resource for
    // legacy test paths that still insert a global ShipImpulse.
    let impulse_charge = impulse_q
        .single()
        .ok()
        .map(|i| i.0.charge_progress)
        .or_else(|| impulse_res.as_deref().map(|r| r.0.charge_progress))
        .unwrap_or(0.0);
    // Per-entity component takes priority over the Resource fallback.
    let boost_config = entity_boost_cfg.or_else(|| boost_config_res.as_deref().cloned());
    let boost_enabled = boost_config.as_ref().map(|c| c.enabled).unwrap_or(false);
    // Prefer per-entity ShipBoost on LocalShip; fall back to Resource.
    let boost_state = boost_q
        .single()
        .ok()
        .map(|b| b.0.clone())
        .or_else(|| boost_res.as_deref().map(|r| r.0.clone()));
    let boost_battery = boost_state.as_ref().map(|b| b.battery).unwrap_or(0.0);
    let boost_active = boost_state.as_ref().map(|b| b.is_active()).unwrap_or(false);
    // view_mode is not raw sim truth; helm blackboard omits it

    let bb = HelmBlackboard {
        yaw: physics.yaw,
        forward_speed: physics.forward_speed,
        x: physics.x,
        z: physics.z,
        impulse_charge,
        boost_battery,
        boost_active,
        boost_enabled,
    };

    // Read last helm input for engine thrust fraction.
    let last_input = last_input_q.iter().next().copied().unwrap_or_default();

    // Per-engine blackboard (issue #511): one entry per fine engine system.
    let hull = hull_q.single().ok();
    let engine_entries = [
        (
            helm_engine_port_system_id(),
            SystemId("helm-engine-port".into()),
        ),
        (
            helm_engine_starboard_system_id(),
            SystemId("helm-engine-starboard".into()),
        ),
    ];

    if let Some(mut bbs) = ship_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(HELM_SYSTEM_ID.to_string()),
            SystemBlackboard::Helm(bb),
        );

        // Publish per-engine entries.
        for (system_id, engine_sid) in engine_entries {
            let tier = hull
                .as_ref()
                .map(|h| h.0.tier_for(&engine_sid))
                .unwrap_or(DamageTier::Operational);
            let is_online = !matches!(tier, DamageTier::Disabled | DamageTier::Destroyed);
            // Prefer the JoystickState from the InterSystemQueue (written by
            // `publish_joystick_to_engines` in SimSet::Physics, which runs
            // before SimSet::Publish). Fall back to LastHelmInput if no
            // channel-1 message targeted this engine this tick.
            let last_input_thrust = last_input.thrust;
            let joystick_thrust = queue
                .0
                .iter()
                .filter(|m| m.target == system_id)
                .filter_map(|m| {
                    if let InterSystemPayload::JoystickState { thrust, .. } = &m.payload {
                        Some(*thrust)
                    } else {
                        None
                    }
                })
                .last()
                .unwrap_or(last_input_thrust);
            let thrust_fraction = if is_online {
                joystick_thrust.abs()
            } else {
                0.0
            };
            bbs.0.insert(
                system_id,
                SystemBlackboard::HelmEngine(HelmEngineBlackboard {
                    thrust_fraction,
                    is_online,
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boost::BoostState;
    use crate::messages::SystemBlackboard;
    use crate::server_app::ShipSystemBlackboards;

    fn base_app() -> App {
        let mut app = App::new();
        app.add_systems(Update, publish_helm_blackboard);
        // Initialise InterSystemQueue so the system parameter is satisfied.
        app.init_resource::<InterSystemQueue>();
        // Spawn a LocalShip entity with ShipPhysics and ShipSystemBlackboards so the system can query it.
        app.world_mut().spawn((
            crate::simulation::LocalShip,
            ShipPhysics::default(),
            ShipSystemBlackboards::default(),
            ShipImpulse::default(),
            crate::ship_plugin::LastHelmInput::default(),
        ));
        app
    }

    /// Helper: read the helm blackboard from the LocalShip entity's ShipSystemBlackboards component.
    fn get_helm_blackboard(app: &mut App) -> crate::messages::HelmBlackboard {
        let key = SystemId(HELM_SYSTEM_ID.to_string());
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::simulation::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::Helm(bb) = bbs
            .0
            .get(&key)
            .expect("expected helm entry in blackboards")
            .clone()
        else {
            panic!("expected Helm blackboard")
        };
        bb
    }

    // ── Publish tests ──────────────────────────────────────────────────────

    #[test]
    fn publish_writes_helm_entry_to_blackboards() {
        let mut app = base_app();
        app.update();

        let key = SystemId(HELM_SYSTEM_ID.to_string());
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::simulation::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        assert!(
            bbs.0.contains_key(&key),
            "expected helm entry in blackboards"
        );
    }

    #[test]
    fn publish_reflects_ship_position_and_yaw() {
        let mut app = base_app();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipPhysics, With<crate::simulation::LocalShip>>();
            let mut physics = q.single_mut(app.world_mut()).unwrap();
            physics.x = 100.0;
            physics.z = -200.0;
            physics.yaw = std::f32::consts::FRAC_PI_4;
            physics.forward_speed = 50.0;
        }
        app.update();

        let bb = get_helm_blackboard(&mut app);
        assert!((bb.x - 100.0).abs() < 0.001);
        assert!((bb.z - (-200.0)).abs() < 0.001);
        assert!((bb.forward_speed - 50.0).abs() < 0.001);
        assert!((bb.yaw - std::f32::consts::FRAC_PI_4).abs() < 0.001);
    }

    #[test]
    fn publish_reflects_impulse_charge() {
        let mut app = base_app();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipImpulse, With<crate::simulation::LocalShip>>();
            let mut imp = q
                .single_mut(app.world_mut())
                .expect("LocalShip must have ShipImpulse");
            imp.0.charge_progress = 0.5;
        }
        app.update();

        let bb = get_helm_blackboard(&mut app);
        assert!((bb.impulse_charge - 0.5).abs() < 0.001);
    }

    #[test]
    fn publish_reflects_boost_state() {
        let mut app = base_app();
        app.insert_resource(BoostConfigResource {
            enabled: true,
            multiplier: 3.0,
            steering_multiplier: 2.0,
            active_duration: 4.0,
            recharge_duration: 20.0,
        });
        app.insert_resource(ShipBoost(BoostState {
            active: true,
            battery: 0.75,
        }));
        app.update();

        let bb = get_helm_blackboard(&mut app);
        assert!(bb.boost_enabled);
        assert!(bb.boost_active);
        assert!((bb.boost_battery - 0.75).abs() < 0.001);
    }

    #[test]
    fn publish_boost_disabled_when_no_config() {
        let mut app = base_app();
        app.update();

        let bb = get_helm_blackboard(&mut app);
        assert!(!bb.boost_enabled);
        assert!(!bb.boost_active);
    }

    // ── Per-engine blackboard tests (issue #511) ───────────────────────────

    #[test]
    fn publish_writes_engine_port_entry_to_blackboards() {
        let mut app = base_app();
        app.update();

        let key = helm_engine_port_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::simulation::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        assert!(
            bbs.0.contains_key(&key),
            "expected helm-engine-port in blackboards"
        );
    }

    #[test]
    fn publish_writes_engine_starboard_entry_to_blackboards() {
        let mut app = base_app();
        app.update();

        let key = helm_engine_starboard_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::simulation::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        assert!(
            bbs.0.contains_key(&key),
            "expected helm-engine-starboard in blackboards"
        );
    }

    #[test]
    fn engine_is_online_when_no_hull_damage() {
        let mut app = base_app();
        app.update();

        let key = helm_engine_port_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::simulation::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::HelmEngine(engine_bb) = bbs
            .0
            .get(&key)
            .expect("expected helm-engine-port in blackboards")
            .clone()
        else {
            panic!("expected HelmEngine blackboard");
        };
        assert!(
            engine_bb.is_online,
            "engine should be online when no hull damage"
        );
    }

    #[test]
    fn engine_thrust_fraction_reflects_last_input() {
        let mut app = base_app();
        // Set helm input to 0.8 thrust.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<crate::simulation::LocalShip>>()
                .single(app.world())
                .unwrap();
            app.world_mut()
                .entity_mut(ship)
                .insert(crate::ship_plugin::LastHelmInput {
                    thrust: 0.8,
                    steering: 0.0,
                });
        }
        app.update();

        let key = helm_engine_port_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::simulation::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::HelmEngine(engine_bb) = bbs
            .0
            .get(&key)
            .expect("expected helm-engine-port in blackboards")
            .clone()
        else {
            panic!("expected HelmEngine blackboard");
        };
        assert!(
            (engine_bb.thrust_fraction - 0.8).abs() < 0.001,
            "thrust_fraction should match last helm input"
        );
    }
}
