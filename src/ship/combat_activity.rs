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
        if current_hull < activity.prev_hull {
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
