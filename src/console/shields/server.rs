use bevy::prelude::*;

use crate::codec;
use crate::console_bridge::ConsoleStateChanged;
use crate::messages::{ShieldFacingStatus, ShieldsConsoleState};
use crate::ship_state::ShipState;
use crate::simulation::{AsteroidUuid, ShipHullIntegrity, ShipShields};
use crate::weapons_plugin::WeaponsTarget;

#[derive(Component, Clone, PartialEq)]
pub struct ShieldsConsoleStateComp(pub ShieldsConsoleState);

pub struct ShieldsConsolePlugin;

impl Plugin for ShieldsConsolePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ConsoleStateChanged>()
            .add_systems(Startup, spawn_shields_console_state_entity)
            .add_systems(
                Update,
                (
                    recompute_shields_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
                    push_shields_console_state
                        .in_set(crate::sim_sets::SimSet::Broadcast)
                        .after(recompute_shields_console_state),
                ),
            );
    }
}

fn spawn_shields_console_state_entity(mut commands: Commands) {
    commands.spawn(ShieldsConsoleStateComp(ShieldsConsoleState::default()));
}

fn recompute_shields_console_state(
    shields: Res<ShipShields>,
    hull: Res<ShipHullIntegrity>,
    ship: Res<ShipState>,
    weapons_target: Option<Res<WeaponsTarget>>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut comp_q: Query<&mut ShieldsConsoleStateComp>,
) {
    let facings: Vec<ShieldFacingStatus> = shields
        .0
        .snapshot()
        .into_iter()
        .map(|s| ShieldFacingStatus {
            label: s.label,
            hp: s.hp,
            max_hp: s.max_hp,
            online: s.online,
            offline_remaining: s.offline_remaining,
            is_focused: s.is_focused,
        })
        .collect();

    let total_hp = hull.0.total_max();
    let total_current = hull.0.total_current();
    let hull_integrity_pct = if total_hp > 0.0 {
        ((total_current / total_hp) * 100.0).clamp(0.0, 100.0)
    } else {
        100.0
    };

    let focused_facing = facings
        .iter()
        .find(|f| f.is_focused)
        .map(|f| f.label.clone());

    let all_online = facings.iter().all(|f| f.online);
    let any_offline = facings.iter().any(|f| !f.online);
    let grid_status = if any_offline {
        "EMITTER OFFLINE"
    } else if all_online {
        "GRID NOMINAL"
    } else {
        "GRID NOMINAL"
    }
    .to_string();

    let target_bearing = weapons_target.as_ref().and_then(|wt| {
        let uuid = wt.0.as_ref()?;
        let live = asteroid_q
            .iter()
            .find(|(u, _)| u.0 == *uuid)
            .map(|(_, t)| (t.translation.x, t.translation.z))
            .or_else(|| {
                entity_q
                    .iter()
                    .find(|(u, _)| u.0 == *uuid)
                    .map(|(_, t)| (t.translation.x, t.translation.z))
            })?;
        let dx = live.0 - ship.x;
        let dz = live.1 - ship.z;
        let bearing_rad =
            (dz.atan2(dx) - ship.yaw + std::f32::consts::PI) % (2.0 * std::f32::consts::PI);
        Some(bearing_rad.to_degrees())
    });

    let next = ShieldsConsoleState {
        facings,
        hull_integrity_pct,
        focused_facing,
        target_bearing,
        grid_status,
    };

    for mut comp in comp_q.iter_mut() {
        if comp.0 != next {
            comp.0 = next.clone();
        }
    }
}

fn push_shields_console_state(
    comp_q: Query<&ShieldsConsoleStateComp, Changed<ShieldsConsoleStateComp>>,
    mut writer: MessageWriter<ConsoleStateChanged>,
) {
    for comp in comp_q.iter() {
        if let Ok(json) = codec::encode_console_state(&comp.0) {
            writer.write(ConsoleStateChanged {
                name: "Shields".into(),
                json,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::messages::Console;
    use crate::ship_state::ShipState;
    use crate::simulation::ShipShields;

    fn test_app() -> App {
        let config = crate::shield::ShieldConfig {
            num_facings: 2,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                (Console::Helm, 25.0),
                (Console::Tactical, 25.0),
                (Console::Power, 25.0),
                (Console::Shields, 25.0),
            ])))
            .insert_resource(ShipShields(crate::shield::ShieldSystem::new(&config)))
            .add_message::<ConsoleStateChanged>()
            .add_plugins(ShieldsConsolePlugin);
        app
    }

    #[test]
    fn spawns_state_entity_after_update() {
        let mut app = test_app();
        app.update();
        let mut query = app.world_mut().query::<&ShieldsConsoleStateComp>();
        let entities = query.iter(app.world()).count();
        assert_eq!(entities, 1);
    }

    #[test]
    fn recompute_produces_hull_integrity_pct() {
        let mut app = test_app();
        app.update();
        let mut query = app.world_mut().query::<&ShieldsConsoleStateComp>();
        let state = query.single(app.world()).unwrap();
        assert!((state.0.hull_integrity_pct - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn recompute_produces_four_facings() {
        let config = crate::shield::ShieldConfig {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                (Console::Helm, 25.0),
                (Console::Tactical, 25.0),
            ])))
            .insert_resource(ShipShields(crate::shield::ShieldSystem::new(&config)))
            .add_message::<ConsoleStateChanged>()
            .add_plugins(ShieldsConsolePlugin);
        app.update();
        let mut query = app.world_mut().query::<&ShieldsConsoleStateComp>();
        let state = query.single(app.world()).unwrap();
        assert_eq!(state.0.facings.len(), 4);
    }

    #[test]
    fn recompute_shows_focused_facing() {
        let mut app = test_app();
        let mut shields = app.world_mut().resource_mut::<ShipShields>();
        shields.0.set_focused_facing(Some(0));
        app.update();
        let mut query = app.world_mut().query::<&ShieldsConsoleStateComp>();
        let state = query.single(app.world()).unwrap();
        // With 2 facings, index 0 is "Fore" by default.
        assert!(state.0.focused_facing.is_some());
    }

    #[test]
    fn recompute_clears_focused_facing() {
        let mut app = test_app();
        {
            let mut shields = app.world_mut().resource_mut::<ShipShields>();
            shields.0.set_focused_facing(Some(0));
        }
        {
            let mut shields = app.world_mut().resource_mut::<ShipShields>();
            shields.0.set_focused_facing(None);
        }
        app.update();
        let mut query = app.world_mut().query::<&ShieldsConsoleStateComp>();
        let state = query.single(app.world()).unwrap();
        assert_eq!(state.0.focused_facing, None);
    }

    #[test]
    fn recompute_grid_status_offline_when_any_facing_down() {
        let mut app = test_app();
        let mut shields = app.world_mut().resource_mut::<ShipShields>();
        // Apply heavy damage at bearing 0 to drain first facing
        shields.0.apply_damage(9999, 0.0);
        app.update();
        let mut query = app.world_mut().query::<&ShieldsConsoleStateComp>();
        let state = query.single(app.world()).unwrap();
        assert_eq!(state.0.grid_status, "EMITTER OFFLINE");
    }

    #[test]
    fn recompute_without_change_does_not_panic() {
        let mut app = test_app();
        app.update();
        app.update();
        let mut query = app.world_mut().query::<&ShieldsConsoleStateComp>();
        let state = query.single(app.world()).unwrap();
        assert!((state.0.hull_integrity_pct - 100.0).abs() < f32::EPSILON);
    }
}
