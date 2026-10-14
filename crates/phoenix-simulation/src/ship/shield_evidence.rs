//! Damage evidence has a physical producer independent of delegation.
use super::{ShieldsAiConfigResource, ShieldsDamageHistory, ShipShields};
use bevy::prelude::*;

pub(crate) fn track_damage(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    world: Option<Res<crate::world::config::WorldConfig>>,
    mut entities: Query<(
        &ShipShields,
        &mut ShieldsDamageHistory,
        Option<&ShieldsAiConfigResource>,
    )>,
) {
    let (Some(tick), Some(world)) = (tick, world) else {
        return;
    };
    let defaults = ShieldsAiConfigResource::default();
    for (shields, mut history, config) in &mut entities {
        let config = config.unwrap_or(&defaults);
        let seconds = config.damage_window_secs.max(config.min_damage_window_secs);
        let window = (seconds * world.global.sim_tick_hz).ceil().max(1.0) as u64;
        history.ensure_len(shields.0.facings.len());
        for (index, facing) in shields.0.facings.iter().enumerate() {
            let previous = history.last_observed_hp(index, facing.hp);
            if facing.hp < previous {
                history.record_damage(index, tick.0, previous - facing.hp);
            }
            history.observe_hp(index, facing.hp);
        }
        history.prune_old(tick.0, window);
    }
}

#[cfg(test)]
#[path = "shield_evidence_tests.rs"]
mod tests;
