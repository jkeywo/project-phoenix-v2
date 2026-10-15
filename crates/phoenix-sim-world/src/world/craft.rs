//! Closed initial custody references. Physical equipment is checked by Content.
use super::config::{WorldConfig, WorldEntitySpawnOn};

pub fn validate_initial_rows(config: &WorldConfig) -> Result<(), String> {
    for (index, row) in config.entities.iter().enumerate() {
        let Some(custody) = &row.carried_by else {
            continue;
        };
        custody.validate()?;
        let name = row
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .ok_or("initial Carried probe requires its declared base-world name")?;
        if row.when.is_some() || row.when_predicate.is_some() || name == custody.parent {
            return Err("initial Carried probe cannot be conditional or its own parent".into());
        }
        let mut parents = config
            .entities
            .iter()
            .filter(|parent| parent.name.as_deref() == Some(custody.parent.as_str()));
        let parent = parents
            .next()
            .ok_or("initial Carried probe names no declared base-world parent")?;
        if parents.next().is_some()
            || parent.carried_by.is_some()
            || parent.when.is_some()
            || parent.when_predicate.is_some()
            || parent.spawn_on == row.spawn_on
                && config
                    .entities
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, parent))
                    .is_none_or(|parent_index| parent_index >= index)
            || row.spawn_on == WorldEntitySpawnOn::Immediate
                && parent.spawn_on == WorldEntitySpawnOn::GameStart
        {
            return Err("initial Carried probe requires one unconditional already-spawned ordinary parent; nested initial craft is not supported yet".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "craft_tests.rs"]
mod tests;
