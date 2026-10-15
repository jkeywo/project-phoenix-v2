//! Derived fixed-clock context for actual installed component effects. Stowed
//! hardware carries absolute expiries and needs no second inventory tick.
use bevy::prelude::*;

pub struct ComponentEffectsPlugin;
impl Plugin for ComponentEffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            stamp_clock
                .in_set(crate::sim_sets::SimSet::Input)
                .in_set(crate::sim_sets::FixedStep::TickComponentEffects),
        );
    }
}

pub(crate) fn clock(world: &World, tick: u64) -> Result<phoenix_model::emp::EmpClock, String> {
    let fixed = world
        .get_resource::<Time<Fixed>>()
        .ok_or("EMP continuation requires the fixed clock")?;
    let period_nanos = u64::try_from(fixed.timestep().as_nanos())
        .map_err(|_| "EMP fixed period exceeds the supported frontier")?;
    let clock = phoenix_model::emp::EmpClock { tick, period_nanos };
    clock.validate(tick)?;
    Ok(clock)
}

/// A runtime-spawned physical actor may enter after this tick's Input pass.
/// Resolve its existing world clock at materialisation, before any impact can
/// use the new actor. No bootstrap epoch or invented zero tick is substituted.
pub(crate) fn install_clock(commands: &mut EntityCommands) {
    let entity = commands.id();
    commands.commands().queue(move |world: &mut World| {
        let clock = world
            .get_resource::<crate::sim_tick::SimTick>()
            .and_then(|tick| clock(world, tick.0).ok());
        if let Some(mut damage) =
            world.get_mut::<crate::ship::compartment_damage::ShipDamage>(entity)
        {
            damage.emp_clock = clock;
        }
    });
}

pub(crate) fn stamp_clock(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    time: Option<Res<Time<Fixed>>>,
    mut ships: Query<
        &mut crate::ship::compartment_damage::ShipDamage,
        Allow<bevy::ecs::entity_disabling::Disabled>,
    >,
) {
    let clock = tick.zip(time).and_then(|(tick, time)| {
        let period_nanos = u64::try_from(time.timestep().as_nanos()).ok()?;
        (period_nanos > 0).then_some(phoenix_model::emp::EmpClock {
            tick: tick.0,
            period_nanos,
        })
    });
    for mut damage in &mut ships {
        damage.emp_clock = clock;
    }
}

#[cfg(test)]
#[path = "emp_tests.rs"]
mod tests;
