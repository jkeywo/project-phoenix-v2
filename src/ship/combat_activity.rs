use bevy::prelude::*;

/// Tracks recent combat activity for the Captain AI to decide on red alert.
#[derive(Resource, Clone, Debug, Default)]
pub struct RecentCombatActivity {
    /// Simulation time (elapsed_secs) when damage was last taken, if any.
    pub last_damage_taken: Option<f32>,
    /// Simulation time (elapsed_secs) when a weapon was last fired, if any.
    pub last_weapon_fired: Option<f32>,
    /// Hull total at the end of the previous tick, used to detect damage.
    pub prev_hull: f32,
}

/// Updates `RecentCombatActivity` and resets `WeaponFiredThisTick`.
/// Runs in `SimSet::Broadcast`.
pub fn update_combat_activity(
    time: Res<Time>,
    hull: Option<Res<crate::server_app::ShipHullIntegrity>>,
    mut weapon_fired: ResMut<crate::server_app::WeaponFiredThisTick>,
    mut activity: ResMut<RecentCombatActivity>,
) {
    let now = time.elapsed_secs();

    // Check for hull decrease.
    if let Some(hull) = hull {
        let current_hull = hull.0.total_current();
        let previous_hull = if activity.prev_hull > 0.0 {
            activity.prev_hull
        } else {
            hull.0.total_max()
        };
        if current_hull < previous_hull {
            activity.last_damage_taken = Some(now);
        }
        activity.prev_hull = current_hull;
    }

    // Check for weapon fire.
    if weapon_fired.0 {
        activity.last_weapon_fired = Some(now);
        weapon_fired.0 = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::messages::Console;
    use crate::server_app::{ShipHullIntegrity, WeaponFiredThisTick};

    fn app_with_hull(hull: ConsoleHull) -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(ShipHullIntegrity(hull))
            .init_resource::<WeaponFiredThisTick>()
            .init_resource::<RecentCombatActivity>()
            .add_systems(Update, update_combat_activity);
        app
    }

    fn test_hull(current_damage: f32) -> ConsoleHull {
        let mut hull = ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)]);
        if current_damage > 0.0 {
            let mut rng = rand::rng();
            hull.apply_damage(current_damage, &mut rng);
        }
        hull
    }

    #[test]
    fn first_update_with_full_hull_does_not_record_damage() {
        let mut app = app_with_hull(test_hull(0.0));

        app.update();

        assert_eq!(
            app.world()
                .resource::<RecentCombatActivity>()
                .last_damage_taken,
            None
        );
    }

    #[test]
    fn first_update_with_damaged_hull_records_damage() {
        let mut app = app_with_hull(test_hull(10.0));

        app.update();

        assert_eq!(
            app.world()
                .resource::<RecentCombatActivity>()
                .last_damage_taken,
            Some(0.0)
        );
    }

    #[test]
    fn weapon_fire_records_activity_and_resets_flag() {
        let mut app = app_with_hull(test_hull(0.0));
        app.world_mut().resource_mut::<WeaponFiredThisTick>().0 = true;

        app.update();

        let activity = app.world().resource::<RecentCombatActivity>();
        assert_eq!(activity.last_weapon_fired, Some(0.0));
        assert!(!app.world().resource::<WeaponFiredThisTick>().0);
    }
}
