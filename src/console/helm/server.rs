use bevy::prelude::*;

use crate::messages::{HelmBlackboard, SystemBlackboard, SystemId};
use crate::server_app::{ShipBoost, ShipImpulse, ShipSystemBlackboards};
use crate::ship_plugin::BoostConfigResource;
use crate::ship_state::ShipPhysics;
use crate::system_registry::HELM_SYSTEM_ID;

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
fn publish_helm_blackboard(
    physics_q: Query<&ShipPhysics, With<crate::simulation::LocalShip>>,
    impulse: Option<Res<ShipImpulse>>,
    boost: Option<Res<ShipBoost>>,
    boost_config: Option<Res<BoostConfigResource>>,
    mut ship_q: Query<&mut crate::server_app::ShipSystemBlackboards, With<crate::simulation::LocalShip>>,
) {
    let physics = physics_q.single().ok().copied().unwrap_or_default();
    let impulse_charge = impulse
        .as_ref()
        .map(|imp| imp.0.charge_progress)
        .unwrap_or(0.0);
    let boost_enabled = boost_config.as_ref().map(|c| c.enabled).unwrap_or(false);
    let boost_battery = boost.as_ref().map(|b| b.0.battery).unwrap_or(0.0);
    let boost_active = boost.as_ref().map(|b| b.0.is_active()).unwrap_or(false);
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

    if let Ok(mut bbs) = ship_q.single_mut() {
        bbs.0.insert(
            SystemId(HELM_SYSTEM_ID.to_string()),
            SystemBlackboard::Helm(bb),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boost::BoostState;
    use crate::impulse::ImpulseState;
    use crate::messages::SystemBlackboard;
    use crate::server_app::ShipSystemBlackboards;

    fn base_app() -> App {
        let mut app = App::new();
        app            .insert_resource(ShipImpulse(ImpulseState::new()))
            .add_systems(Update, publish_helm_blackboard);
        // Spawn a LocalShip entity with ShipPhysics and ShipSystemBlackboards so the system can query it.
        app.world_mut().spawn((
            crate::simulation::LocalShip,
            ShipPhysics::default(),
            ShipSystemBlackboards::default(),
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
        let SystemBlackboard::Helm(bb) = bbs.0.get(&key).expect("expected helm entry in blackboards").clone() else {
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
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
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

}
