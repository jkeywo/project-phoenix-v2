//! Candidate-only configuration age checks; historical acquired findings never
//! compare against a target's current shield configuration.
use super::*;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldFrequencyState {
    pub frequency: f32,
    pub cycle_anchor_tick: Option<u64>,
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let live = world
            .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, id)| (id.0 == row.uuid).then_some(entity))
            });
        let config = if live.is_none() {
            row.spawn.as_ref().and_then(|origin| {
                origin.resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
            })
        } else {
            None
        };
        let installed = live
            .map(|entity| {
                world
                    .get::<crate::ship::shields::ShipShields>(entity)
                    .is_some()
            })
            .or_else(|| config.as_ref().map(|config| !config.shield_arcs.is_empty()));
        if installed.is_some_and(|installed| installed != row.shield_frequency.is_some()) {
            return Err((
                row.uuid.clone(),
                "shield frequency owner disagrees with actual equipment".into(),
            ));
        }
        let Some(saved) = &row.shield_frequency else {
            continue;
        };
        if !saved.frequency.is_finite() || !(0.0..=1.0).contains(&saved.frequency) {
            return Err((row.uuid.clone(), "invalid saved shield frequency".into()));
        }
        let profile = live
            .and_then(|entity| {
                world
                    .get::<crate::ship::shield_enquiry::ShieldProbeProfile>(entity)
                    .cloned()
            })
            .or_else(|| {
                let config = config.as_ref()?;
                let console = config.shields_console.as_ref()?;
                Some(crate::ship::shield_enquiry::ShieldProbeProfile {
                    initial_frequency: config
                        .shield_arcs
                        .first()
                        .map_or(console.frequency, |arc| arc.frequency),
                    response: console.frequency_response.unwrap_or_default(),
                    cycle: console.frequency_cycle.clone(),
                })
            });
        if saved.cycle_anchor_tick.is_some()
            && !world.contains_resource::<crate::sim_tick::SimTick>()
        {
            return Err((
                row.uuid.clone(),
                "shield cycle requires its logical tick owner".into(),
            ));
        }
        let result = match (
            profile.as_ref().and_then(|profile| profile.cycle.as_ref()),
            saved.cycle_anchor_tick,
        ) {
            (Some(cycle), Some(anchor)) => world
                .get_resource::<Time<Fixed>>()
                .ok_or_else(|| "shield cycle requires its fixed logical clock".to_owned())
                .and_then(|time| cycle.frequency_at(anchor, snapshot.tick, time.timestep()))
                .and_then(|expected| {
                    if expected.to_bits() == saved.frequency.to_bits() {
                        Ok(())
                    } else {
                        Err("shield frequency disagrees with its causal cycle".into())
                    }
                }),
            (None, None)
                if profile.as_ref().is_none_or(|profile| {
                    profile.initial_frequency.to_bits() == saved.frequency.to_bits()
                }) =>
            {
                Ok(())
            }
            _ => Err("shield cycle metadata disagrees with accepted equipment".into()),
        };
        result.map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

pub(super) fn capture(entity: EntityRef<'_>) -> Option<ShieldFrequencyState> {
    Some(ShieldFrequencyState {
        frequency: entity
            .get::<crate::ship::shields::ShipShields>()?
            .frequency(),
        cycle_anchor_tick: entity
            .get::<crate::ship::shield_enquiry::ShieldCycleAnchor>()
            .map(|anchor| anchor.0),
    })
}
pub(super) fn apply(entity: &mut EntityWorldMut<'_>, saved: &ShieldFrequencyState) {
    if let Some(mut shield) = entity.get_mut::<crate::ship::shields::ShipShields>() {
        shield.1 = saved.frequency;
    }
    if let Some(anchor) = saved.cycle_anchor_tick {
        entity.insert(crate::ship::shield_enquiry::ShieldCycleAnchor(anchor));
    } else {
        entity.remove::<crate::ship::shield_enquiry::ShieldCycleAnchor>();
    }
}

#[cfg(test)]
#[path = "shield_enquiry_tests.rs"]
mod tests;
