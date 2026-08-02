use bevy::prelude::*;

/// Per-ship tracker of recent combat activity for the Captain AI to decide on
/// red alert. Every ship (player + NPC) carries its own component; no global
/// resource. Updated by `update_combat_activity` (SimSet::Broadcast).
#[derive(Component, Clone, Debug, Default)]
pub struct RecentCombatActivity {
    /// Simulation time (elapsed_secs) when damage was last taken, if any.
    pub last_damage_taken: Option<f32>,
    /// Simulation time (elapsed_secs) when hostile fire last targeted this
    /// ship, even if shields absorbed the hit before hull damage leaked
    /// through.
    pub last_hostile_fire_taken: Option<f32>,
    /// Simulation time (elapsed_secs) when a weapon was last fired, if any.
    pub last_weapon_fired: Option<f32>,
    /// Hull total at the end of the previous tick, used to detect damage.
    pub prev_hull: f32,
}

/// Update every ship's `RecentCombatActivity` and clear its per-tick
/// `WeaponFiredThisTick`/`ShipAttackedThisTick` markers. Runs in
/// `SimSet::Broadcast` so damage systems in earlier sets have already
/// mutated hull/attacker/weapon-fired.
pub fn update_combat_activity(
    time: Res<Time>,
    mut ships: Query<
        (
            &crate::entity_spawner::EntitySystemHull,
            &mut RecentCombatActivity,
            &mut crate::server_app::WeaponFiredThisTick,
            &mut crate::server_app::ShipAttackedThisTick,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let now = time.elapsed_secs();

    for (hull_comp, mut activity, mut weapon_fired, mut attacked) in ships.iter_mut() {
        // Check for hull decrease against prev_hull snapshot.
        let current_hull = hull_comp.0.total_current();
        let previous_hull = if activity.prev_hull > 0.0 {
            activity.prev_hull
        } else {
            hull_comp.0.total_max()
        };
        if current_hull < previous_hull {
            activity.last_damage_taken = Some(now);
        }
        activity.prev_hull = current_hull;

        if attacked.0 {
            activity.last_hostile_fire_taken = Some(now);
            attacked.0 = false;
        }

        if weapon_fired.0 {
            activity.last_weapon_fired = Some(now);
            weapon_fired.0 = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::SystemHull;
    use crate::messages::SystemId;
    use crate::server_app::{Ship, ShipAttackedThisTick, WeaponFiredThisTick};

    fn app_with_hull(hull: SystemHull) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_systems(Update, update_combat_activity);
        let ship = app
            .world_mut()
            .spawn((
                Ship,
                crate::entity_spawner::EntitySystemHull(hull),
                RecentCombatActivity::default(),
                WeaponFiredThisTick::default(),
                ShipAttackedThisTick::default(),
            ))
            .id();
        (app, ship)
    }

    fn activity_for(app: &mut App, ship: Entity) -> RecentCombatActivity {
        app.world()
            .get::<RecentCombatActivity>(ship)
            .unwrap()
            .clone()
    }

    fn test_hull(current_damage: f32) -> SystemHull {
        let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);
        if current_damage > 0.0 {
            let mut rng = crate::sim_rng::unseeded_test_rng();
            hull.apply_damage(current_damage, &mut rng);
        }
        hull
    }

    #[test]
    fn first_update_with_full_hull_does_not_record_damage() {
        let (mut app, ship) = app_with_hull(test_hull(0.0));

        app.update();

        assert_eq!(activity_for(&mut app, ship).last_damage_taken, None);
    }

    #[test]
    fn first_update_with_damaged_hull_records_damage() {
        let (mut app, ship) = app_with_hull(test_hull(10.0));

        app.update();

        assert_eq!(activity_for(&mut app, ship).last_damage_taken, Some(0.0));
    }

    #[test]
    fn weapon_fire_records_activity_and_resets_flag() {
        let (mut app, ship) = app_with_hull(test_hull(0.0));
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<WeaponFiredThisTick>()
            .unwrap()
            .0 = true;

        app.update();

        assert_eq!(activity_for(&mut app, ship).last_weapon_fired, Some(0.0));
        assert!(!app.world().get::<WeaponFiredThisTick>(ship).unwrap().0);
    }

    #[test]
    fn hostile_fire_records_activity_and_resets_flag() {
        let (mut app, ship) = app_with_hull(test_hull(0.0));
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ShipAttackedThisTick>()
            .unwrap()
            .0 = true;

        app.update();

        assert_eq!(
            activity_for(&mut app, ship).last_hostile_fire_taken,
            Some(0.0)
        );
        assert!(!app.world().get::<ShipAttackedThisTick>(ship).unwrap().0);
    }
}
