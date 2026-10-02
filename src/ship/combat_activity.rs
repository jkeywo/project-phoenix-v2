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

impl RecentCombatActivity {
    /// Read the authoritative activity memory for continuation projection.
    pub(crate) fn continuation(&self) -> (Option<f32>, Option<f32>, Option<f32>, f32) {
        (
            self.last_damage_taken,
            self.last_hostile_fire_taken,
            self.last_weapon_fired,
            self.prev_hull,
        )
    }

    /// Replace bootstrap memory with a restored continuation.
    pub(crate) fn replace_continuation(
        &mut self,
        last_damage_taken: Option<f32>,
        last_hostile_fire_taken: Option<f32>,
        last_weapon_fired: Option<f32>,
        prev_hull: f32,
    ) {
        self.last_damage_taken = last_damage_taken;
        self.last_hostile_fire_taken = last_hostile_fire_taken;
        self.last_weapon_fired = last_weapon_fired;
        self.prev_hull = prev_hull;
    }
}

/// Update every ship's `RecentCombatActivity` and clear its per-tick
/// `WeaponFiredThisTick`/`ShipAttackedThisTick` markers. Runs in
/// `SimSet::Broadcast` so damage systems in earlier sets have already
/// mutated hull/attacker/weapon-fired.
pub fn update_combat_activity(
    time: Res<Time>,
    mut ships: Query<
        (
            &crate::entities::spawner::EntitySystemHull,
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
#[path = "combat_activity_tests.rs"]
mod tests;
