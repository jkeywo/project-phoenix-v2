// Pure module: resolve an WorldEntity into a concrete EntityConfig.
// No Bevy dependency — fully unit-testable on native.

use crate::entity_config::EntityConfig;
use crate::world::config::WorldEntity;

/// Resolve an `WorldEntity` to a concrete `EntityConfig` by:
/// 1. Looking up `entity_inst.template_path` in the config cache.
/// 2. Optionally merging `entity_inst.overrides` on top of the template TOML.
///
/// Returns `Err` if the template is not found in the cache.
pub fn resolve_entity(
    entity_inst: &WorldEntity,
    config_cache: &crate::config_cache::ConfigCache,
) -> Result<EntityConfig, String> {
    let template = config_cache
        .get(&entity_inst.template_path)
        .ok_or_else(|| {
            format!(
                "entity template not found in cache: '{}'",
                entity_inst.template_path
            )
        })?;

    let config = match &entity_inst.overrides {
        None => template.clone(),
        Some(overrides) => {
            // Re-serialise the template to a toml::Value, merge, then deserialise back.
            let template_value: toml::Value = toml::from_str(
                &toml::to_string(template).map_err(|e| format!("template serialise error: {e}"))?,
            )
            .map_err(|e| format!("template re-parse error: {e}"))?;

            let merged =
                crate::entity_override::merge_entity_config_toml(&template_value, overrides);
            let merged_str =
                toml::to_string(&merged).map_err(|e| format!("merged serialise error: {e}"))?;
            EntityConfig::from_toml(&merged_str)
                .map_err(|e| format!("merged parse error: {e:?}"))?
        }
    };

    Ok(config)
}

/// Generate a new UUID string for a spawned entity.
pub fn assign_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_cache::ConfigCache;
    use crate::entity_config::EntityConfig;
    use std::collections::HashMap;

    fn make_cache(path: &str, toml: &str) -> ConfigCache {
        let mut m = HashMap::new();
        m.insert(path.to_string(), EntityConfig::from_toml(toml).unwrap());
        ConfigCache::from(m)
    }

    #[test]
    fn resolve_missing_template_returns_err() {
        let cache = ConfigCache::from(HashMap::new());
        let inst = WorldEntity {
            template_path: "assets/entities/missing.toml".to_string(),
            ..Default::default()
        };
        let result = resolve_entity(&inst, &cache);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in cache"));
    }

    #[test]
    fn resolve_template_no_overrides_returns_clone() {
        let cache = make_cache("assets/entities/rock.toml", r#"tags = ["asteroid"]"#);
        let inst = WorldEntity {
            template_path: "assets/entities/rock.toml".to_string(),
            overrides: None,
            ..Default::default()
        };
        let config = resolve_entity(&inst, &cache).unwrap();
        assert_eq!(config.tags, vec!["asteroid"]);
    }

    /// Verifies that a world override on `[behaviour]` round-trips cleanly.
    /// (#572) FSM dissolved — `initial_state`/`transition` fields removed.
    /// Overrides now target `waypoint_arrival_radius` or doctrine entries.
    /// This test confirms that overriding `waypoint_arrival_radius` merges
    /// correctly while preserving the template's doctrine objectives.
    #[test]
    fn pirate_raider_behaviour_override_merges_arrival_radius() {
        let template_toml = std::fs::read_to_string("assets/entities/pirate_raider.toml")
            .expect("pirate_raider.toml must exist");
        let cache = make_cache("assets/entities/pirate_raider.toml", &template_toml);

        let override_value: toml::Value = toml::from_str(
            r#"
[behaviour]
waypoint_arrival_radius = 99.0
"#,
        )
        .unwrap();

        let inst = WorldEntity {
            template_path: "assets/entities/pirate_raider.toml".to_string(),
            overrides: Some(override_value),
            ..Default::default()
        };
        let config = resolve_entity(&inst, &cache).unwrap();
        let beh = config
            .behaviour
            .expect("raider must keep [behaviour] section");
        assert!(
            (beh.waypoint_arrival_radius - 99.0).abs() < 1e-6,
            "overridden waypoint_arrival_radius must be 99.0"
        );
        assert!(
            !beh.doctrine.is_empty(),
            "template doctrine objectives must be preserved by override merge"
        );
    }

    /// End-to-end check that an inline-table `overrides` block parses through
    /// `parse_world` → `resolve_entity` and merges correctly.
    /// (#572) FSM dissolved — override now targets `waypoint_arrival_radius`.
    #[test]
    fn world_inline_override_round_trips_behaviour_merge() {
        let template_toml = std::fs::read_to_string("assets/entities/pirate_raider.toml")
            .expect("pirate_raider.toml must exist");
        let cache = make_cache("assets/entities/pirate_raider.toml", &template_toml);

        let world_toml = r#"
[[entity]]
template_path = "assets/entities/pirate_raider.toml"
name          = "raider_alpha"
transform     = { position = [150.0, 0.0, -20.0] }
overrides     = { behaviour = { waypoint_arrival_radius = 42.0 } }
"#;

        let world = crate::world::config::parse_world(world_toml)
            .expect("world TOML must parse");
        assert_eq!(world.entities.len(), 1, "expected exactly one [[entity]] block");

        let inst = &world.entities[0];
        assert!(inst.overrides.is_some(), "overrides must round-trip through parse_world");

        let config = resolve_entity(inst, &cache).unwrap();
        let beh = config
            .behaviour
            .expect("raider keeps behaviour section after merge");
        assert!(
            (beh.waypoint_arrival_radius - 42.0).abs() < 1e-6,
            "inline override must set waypoint_arrival_radius to 42.0"
        );
        assert!(
            !beh.doctrine.is_empty(),
            "template doctrine must survive an inline override"
        );
    }

    #[test]
    fn assign_uuid_returns_valid_uuid() {
        let id = assign_uuid();
        assert!(
            uuid::Uuid::parse_str(&id).is_ok(),
            "assign_uuid should return a valid UUID v4"
        );
    }

    #[test]
    fn assign_uuid_returns_unique_values() {
        let a = assign_uuid();
        let b = assign_uuid();
        assert_ne!(a, b);
    }
}
