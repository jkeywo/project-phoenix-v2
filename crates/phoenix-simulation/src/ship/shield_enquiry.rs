//! Actual frequency remains ShipShields.1. Only causal cycle age is additional state.
use bevy::prelude::*;
use phoenix_model::shield_enquiry::{ShieldFrequencyCycle, ShieldFrequencyResponse};

#[derive(Component, Clone, Debug)]
pub struct ShieldProbeProfile {
    pub initial_frequency: f32,
    pub response: ShieldFrequencyResponse,
    pub cycle: Option<ShieldFrequencyCycle>,
}
#[derive(Component, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShieldCycleAnchor(pub u64);

pub(crate) fn install(
    initial_frequency: f32,
    console: &crate::entities::config::ShieldsConsoleConfig,
    cmds: &mut EntityCommands,
) {
    let profile = ShieldProbeProfile {
        initial_frequency,
        response: console.frequency_response.unwrap_or_default(),
        cycle: console.frequency_cycle.clone(),
    };
    let cycle = profile.cycle.clone();
    cmds.insert(profile);
    if let Some(cycle) = cycle {
        let entity = cmds.id();
        cmds.commands().queue(move |world: &mut World| {
            let tick = world
                .get_resource::<crate::sim_tick::SimTick>()
                .map_or(0, |tick| tick.0);
            // The first configuration exists immediately at spawn, not on a later scheduler pass.
            if let Some(mut shield) = world.get_mut::<crate::ship::shields::ShipShields>(entity) {
                shield.1 = cycle.frequencies[0];
            }
            world.entity_mut(entity).insert(ShieldCycleAnchor(tick));
        });
    }
}

pub(crate) fn tick(
    time: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    mut shields: Query<(
        &ShieldProbeProfile,
        &ShieldCycleAnchor,
        &mut crate::ship::shields::ShipShields,
    )>,
) {
    for (profile, anchor, mut shield) in &mut shields {
        if let Some(cycle) = &profile.cycle {
            shield.1 = cycle
                .frequency_at(anchor.0, tick.0, time.timestep())
                .expect("accepted causal shield cycle");
        }
    }
}

pub(crate) fn declare(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<ShieldProbeProfile>(StateClass::Derived, "shield-probing-policy")
        .declare_state::<ShieldCycleAnchor>(StateClass::Folded, "shield-configuration-cycle");
}
