use bevy::prelude::*;

use crate::messages::{HelmBlackboard, SystemBlackboard, SystemId, ViewMode};
use crate::server_app::{ShipBoost, ShipImpulse, SystemBlackboards};
use crate::ship_plugin::BoostConfigResource;
use crate::ship_state::ShipState;
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
    ship: Res<ShipState>,
    impulse: Option<Res<ShipImpulse>>,
    boost: Option<Res<ShipBoost>>,
    boost_config: Option<Res<BoostConfigResource>>,
    mut blackboards: ResMut<SystemBlackboards>,
) {
    let impulse_charge = impulse
        .as_ref()
        .map(|imp| imp.0.charge_progress)
        .unwrap_or(0.0);
    let boost_enabled = boost_config.as_ref().map(|c| c.enabled).unwrap_or(false);
    let boost_battery = boost.as_ref().map(|b| b.0.battery).unwrap_or(0.0);
    let boost_active = boost.as_ref().map(|b| b.0.is_active()).unwrap_or(false);
    let on_screen = matches!(ship.view_mode, ViewMode::Radar);
    let _ = on_screen; // view mode is not raw sim truth; kept in SimSnapshot

    let bb = HelmBlackboard {
        yaw: ship.yaw,
        forward_speed: ship.forward_speed,
        x: ship.x,
        z: ship.z,
        impulse_charge,
        boost_battery,
        boost_active,
        boost_enabled,
    };

    blackboards
        .0
        .insert(SystemId(HELM_SYSTEM_ID.to_string()), SystemBlackboard::Helm(bb));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boost::BoostState;
    use crate::impulse::ImpulseState;
    use crate::messages::SystemBlackboard;
    use crate::server_app::{FrozenBlackboards, SystemBlackboards};

    fn base_app() -> App {
        let mut app = App::new();
        app.insert_resource(ShipState::new())
            .insert_resource(ShipImpulse(ImpulseState::new()))
            .init_resource::<SystemBlackboards>()
            .init_resource::<FrozenBlackboards>()
            .add_systems(Update, publish_helm_blackboard);
        app
    }

    // ── Publish tests ──────────────────────────────────────────────────────

    #[test]
    fn publish_writes_helm_entry_to_blackboards() {
        let mut app = base_app();
        app.update();

        let bbs = app.world().resource::<SystemBlackboards>();
        let key = SystemId(HELM_SYSTEM_ID.to_string());
        assert!(bbs.0.contains_key(&key), "expected helm entry in blackboards");
    }

    #[test]
    fn publish_reflects_ship_position_and_yaw() {
        let mut app = base_app();
        {
            let mut ship = app.world_mut().resource_mut::<ShipState>();
            ship.x = 100.0;
            ship.z = -200.0;
            ship.yaw = std::f32::consts::FRAC_PI_4;
            ship.forward_speed = 50.0;
        }
        app.update();

        let bbs = app.world().resource::<SystemBlackboards>();
        let key = SystemId(HELM_SYSTEM_ID.to_string());
        let SystemBlackboard::Helm(bb) = bbs.0.get(&key).unwrap() else { panic!("expected Helm blackboard") };
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

        let bbs = app.world().resource::<SystemBlackboards>();
        let key = SystemId(HELM_SYSTEM_ID.to_string());
        let SystemBlackboard::Helm(bb) = bbs.0.get(&key).unwrap() else { panic!("expected Helm blackboard") };
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
        app.insert_resource(ShipBoost(BoostState { active: true, battery: 0.75 }));
        app.update();

        let bbs = app.world().resource::<SystemBlackboards>();
        let key = SystemId(HELM_SYSTEM_ID.to_string());
        let SystemBlackboard::Helm(bb) = bbs.0.get(&key).unwrap() else { panic!("expected Helm blackboard") };
        assert!(bb.boost_enabled);
        assert!(bb.boost_active);
        assert!((bb.boost_battery - 0.75).abs() < 0.001);
    }

    #[test]
    fn publish_boost_disabled_when_no_config() {
        let mut app = base_app();
        app.update();

        let bbs = app.world().resource::<SystemBlackboards>();
        let key = SystemId(HELM_SYSTEM_ID.to_string());
        let SystemBlackboard::Helm(bb) = bbs.0.get(&key).unwrap() else { panic!("expected Helm blackboard") };
        assert!(!bb.boost_enabled);
        assert!(!bb.boost_active);
    }

    // ── Determinism test ───────────────────────────────────────────────────
    // Cross-system reads during Simulate must use FrozenBlackboards (last
    // tick's snapshot), not the live SystemBlackboards. This test proves the
    // snapshot is one tick behind.

    fn app_with_snapshot() -> App {
        let mut app = App::new();
        app.insert_resource(ShipState::new())
            .insert_resource(ShipImpulse(ImpulseState::new()))
            .init_resource::<SystemBlackboards>()
            .init_resource::<FrozenBlackboards>()
            // snapshot runs BEFORE publish so FrozenBlackboards lags by one tick
            .add_systems(
                Update,
                (
                    crate::server_app::snapshot_blackboards_for_test,
                    publish_helm_blackboard,
                )
                    .chain(),
            );
        app
    }

    #[test]
    fn frozen_blackboard_lags_by_one_tick() {
        let mut app = app_with_snapshot();

        // Tick 1: ship at yaw=1.0
        app.world_mut().resource_mut::<ShipState>().yaw = 1.0;
        app.update();

        // After tick 1: SystemBlackboards has yaw=1.0;
        // FrozenBlackboards was snapshotted BEFORE publish, so it still has the empty default.
        {
            let bbs = app.world().resource::<SystemBlackboards>();
            let key = SystemId(HELM_SYSTEM_ID.to_string());
            let SystemBlackboard::Helm(bb) = bbs.0.get(&key).unwrap() else { panic!("expected Helm blackboard") };
            assert!((bb.yaw - 1.0).abs() < 0.001, "SystemBlackboards should have yaw=1.0");
        }

        // Tick 2: ship moves to yaw=2.0
        app.world_mut().resource_mut::<ShipState>().yaw = 2.0;
        app.update();

        // FrozenBlackboards (snapshotted before tick 2's publish) should have yaw=1.0
        {
            let frozen = app.world().resource::<FrozenBlackboards>();
            let key = SystemId(HELM_SYSTEM_ID.to_string());
            let SystemBlackboard::Helm(bb) = frozen.0.get(&key).unwrap() else { panic!("expected Helm blackboard") };
            assert!(
                (bb.yaw - 1.0).abs() < 0.001,
                "FrozenBlackboards should still have last tick's yaw=1.0, got {}",
                bb.yaw
            );
        }
    }
}
