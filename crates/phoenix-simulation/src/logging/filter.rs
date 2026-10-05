use super::*;
use crate::entities::spawner::EntityName;
use bevy::prelude::*;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
fn empty_per_cat() -> HashMap<LogCat, LevelFilter> {
    HashMap::new()
}

/// Keeps [`EntityFilter::allowed`] in sync with the world.
///
/// Driven by `Added<EntityName>` and `RemovedComponents<EntityName>`, so it is
/// O(changes) rather than O(entities) and costs nothing once the world settles.
/// The whole system is `run_if`-gated on a filter being configured, so in the
/// default case it never runs at all.
pub fn refresh_log_entity_filter(
    mut cfg: ResMut<LogFilterConfig>,
    added: Query<(Entity, &EntityName), Added<EntityName>>,
    mut removed: RemovedComponents<EntityName>,
) {
    // Collect first so we are not holding a borrow of `cfg` across the checks.
    let newly_named: Vec<(Entity, String)> =
        added.iter().map(|(e, name)| (e, name.0.clone())).collect();
    let gone: Vec<Entity> = removed.read().collect();

    if newly_named.is_empty() && gone.is_empty() {
        return;
    }

    let Some(filter) = cfg.entity_filter.as_mut() else {
        return;
    };
    for (entity, name) in newly_named {
        if filter.matches_name(&name) {
            filter.allowed.insert(entity);
        }
    }
    for entity in gone {
        filter.allowed.remove(&entity);
    }
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
