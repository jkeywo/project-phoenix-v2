use bevy::prelude::*;

use crate::console_bridge::ConsoleStateChanged;
use crate::messages::{HelmConsoleState, ViewMode};
use crate::server_app::{ShipBoost, ShipImpulse};
use crate::ship_plugin::BoostConfigResource;
use crate::ship_state::ShipState;

pub struct HelmPlugin;

impl Plugin for HelmPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ConsoleStateChanged>();
        app.add_systems(Startup, spawn_helm_console_state_entity);
        app.add_systems(
            Update,
            (
                recompute_helm_console_state,
                push_helm_console_state.after(recompute_helm_console_state),
            )
                .in_set(crate::sim_sets::SimSet::Broadcast),
        );
    }
}

#[derive(Component, Clone, PartialEq)]
pub struct HelmConsoleStateComp(pub HelmConsoleState);

fn spawn_helm_console_state_entity(mut commands: Commands) {
    commands.spawn(HelmConsoleStateComp(HelmConsoleState {
        heading: 0.0,
        speed: 0.0,
        x: 0.0,
        z: 0.0,
        yaw: 0.0,
        impulse_charge_progress: 0.0,
        on_screen: false,
        boost_battery: 0.0,
        boost_active: false,
        boost_enabled: false,
    }));
}

fn heading_from_yaw(yaw_rad: f32) -> f32 {
    (yaw_rad.to_degrees()).rem_euclid(360.0)
}

fn recompute_helm_console_state(
    ship: Res<ShipState>,
    impulse: Option<Res<ShipImpulse>>,
    boost: Option<Res<ShipBoost>>,
    boost_config: Option<Res<BoostConfigResource>>,
    mut comp_q: Query<&mut HelmConsoleStateComp>,
) {
    let heading = heading_from_yaw(ship.yaw);
    let on_screen = matches!(ship.view_mode, ViewMode::Radar);
    let charge_progress = impulse
        .as_ref()
        .map(|imp| imp.0.charge_progress)
        .unwrap_or(0.0);
    let boost_enabled = boost_config.as_ref().map(|c| c.enabled).unwrap_or(false);
    let boost_battery = boost.as_ref().map(|b| b.0.battery).unwrap_or(0.0);
    let boost_active = boost.as_ref().map(|b| b.0.is_active()).unwrap_or(false);

    let next = HelmConsoleState {
        heading,
        speed: ship.forward_speed,
        x: ship.x,
        z: ship.z,
        yaw: ship.yaw,
        impulse_charge_progress: charge_progress,
        on_screen,
        boost_battery,
        boost_active,
        boost_enabled,
    };

    for mut comp in comp_q.iter_mut() {
        if comp.0 != next {
            comp.0 = next.clone();
        }
    }
}

fn push_helm_console_state(
    comp_q: Query<&HelmConsoleStateComp, Changed<HelmConsoleStateComp>>,
    mut writer: MessageWriter<ConsoleStateChanged>,
) {
    for comp in comp_q.iter() {
        if let Ok(json) = crate::core::codec::encode_console_state(&comp.0) {
            writer.write(ConsoleStateChanged {
                name: "Helm".into(),
                json,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::impulse::ImpulseState;

    #[derive(Resource, Default)]
    struct Outbox(Vec<ConsoleStateChanged>);

    fn collect(mut reader: MessageReader<ConsoleStateChanged>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn recompute_test_app() -> App {
        let mut app = App::new();
        app.add_systems(Startup, spawn_helm_console_state_entity);
        app.add_systems(Update, recompute_helm_console_state);
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipImpulse(ImpulseState::new()));
        app
    }

    fn push_test_app() -> App {
        let mut app = App::new();
        app.add_message::<ConsoleStateChanged>()
            .init_resource::<Outbox>()
            .add_systems(
                Update,
                (
                    push_helm_console_state,
                    collect.after(push_helm_console_state),
                ),
            );
        app.world_mut()
            .spawn(HelmConsoleStateComp(HelmConsoleState {
                heading: 0.0,
                speed: 0.0,
                x: 0.0,
                z: 0.0,
                yaw: 0.0,
                impulse_charge_progress: 0.0,
                on_screen: false,
                boost_battery: 0.0,
                boost_active: false,
                boost_enabled: false,
            }));
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipImpulse(ImpulseState::new()));
        app
    }

    // ── Spawn tests ──

    #[test]
    fn spawn_entity_exists_with_defaults() {
        let mut app = App::new();
        app.add_systems(Startup, spawn_helm_console_state_entity);
        app.update();

        let mut q = app.world_mut().query::<&HelmConsoleStateComp>();
        let state = q.single(app.world()).unwrap();
        assert_eq!(state.0.heading, 0.0);
        assert_eq!(state.0.speed, 0.0);
        assert!(!state.0.on_screen);
    }

    // ── Recompute tests ──

    #[test]
    fn recompute_reflects_ship_position_and_yaw() {
        let mut app = recompute_test_app();
        {
            let mut ship = app.world_mut().resource_mut::<ShipState>();
            ship.x = 100.0;
            ship.z = -200.0;
            ship.yaw = std::f32::consts::FRAC_PI_4;
            ship.forward_speed = 50.0;
        }
        app.update();

        let mut q = app.world_mut().query::<&HelmConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!((comp.0.x - 100.0).abs() < 0.001);
        assert!((comp.0.z - (-200.0)).abs() < 0.001);
        assert!((comp.0.speed - 50.0).abs() < 0.001);
        assert!((comp.0.heading - 45.0).abs() < 0.1);
    }

    #[test]
    fn recompute_reflects_on_screen() {
        let mut app = recompute_test_app();
        {
            let mut ship = app.world_mut().resource_mut::<ShipState>();
            ship.view_mode = ViewMode::Radar;
        }
        app.update();

        let mut q = app.world_mut().query::<&HelmConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(comp.0.on_screen);
    }

    #[test]
    fn recompute_reflects_impulse_charge_progress() {
        let mut app = recompute_test_app();
        {
            let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
            imp.0.charge_progress = 0.5;
        }
        app.update();

        let mut q = app.world_mut().query::<&HelmConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!((comp.0.impulse_charge_progress - 0.5).abs() < 0.001);
    }

    #[test]
    fn recompute_reflects_boost_state_and_enabled() {
        let mut app = recompute_test_app();
        app.insert_resource(BoostConfigResource {
            enabled: true,
            multiplier: 3.0,
            steering_multiplier: 2.0,
            active_duration: 4.0,
            recharge_duration: 20.0,
        });
        app.insert_resource(ShipBoost(crate::boost::BoostState {
            active: true,
            battery: 0.75,
        }));
        app.update();

        let mut q = app.world_mut().query::<&HelmConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(comp.0.boost_enabled);
        assert!(comp.0.boost_active);
        assert!((comp.0.boost_battery - 0.75).abs() < 0.001);
    }

    #[test]
    fn recompute_boost_disabled_when_no_config() {
        let mut app = recompute_test_app();
        app.update();

        let mut q = app.world_mut().query::<&HelmConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(!comp.0.boost_enabled);
        assert!(!comp.0.boost_active);
    }

    // ── Push tests ──

    #[test]
    fn push_emits_one_message_with_expected_values() {
        let mut app = push_test_app();

        // First update: freshly spawned component is Changed -> push fires.
        app.update();
        app.world_mut().resource_mut::<Outbox>().0.clear();

        {
            let mut q = app.world_mut().query::<&mut HelmConsoleStateComp>();
            let mut comp = q.single_mut(app.world_mut()).unwrap();
            comp.0 = HelmConsoleState {
                heading: 180.0,
                speed: 100.0,
                x: 500.0,
                z: -300.0,
                yaw: std::f32::consts::PI,
                impulse_charge_progress: 0.0,
                on_screen: true,
                boost_battery: 0.0,
                boost_active: false,
                boost_enabled: false,
            };
        }
        app.update();

        let pushes = &app.world().resource::<Outbox>().0;
        assert_eq!(pushes.len(), 1, "expected exactly one push after a change");
        let push = &pushes[0];
        assert_eq!(push.name, "Helm");
        assert!(
            push.json.contains("\"heading\":180.0"),
            "json: {}",
            push.json
        );
        assert!(push.json.contains("\"speed\":100.0"), "json: {}", push.json);
        assert!(
            push.json.contains("\"on_screen\":true"),
            "json: {}",
            push.json
        );

        // No further change -> no further pushes.
        app.world_mut().resource_mut::<Outbox>().0.clear();
        app.update();
        assert!(app.world().resource::<Outbox>().0.is_empty());
    }
}
