/// Pure faction module — no Bevy imports.
///
/// A `FactionConfig` describes a named faction with a stable UUID and an
/// optional list of enemy faction UUIDs. The `is_enemy` predicate is
/// *asymmetric by construction*: A listing B as an enemy does not imply B
/// considers A an enemy.
///
/// Factionless entities (those with no faction UUID) are neither enemies nor
/// targets of anyone.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Configuration for a single faction, loaded from a `assets/factions/*.toml`
/// file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactionConfig {
    /// Stable UUID identifying this faction.
    pub uuid: Uuid,
    /// Human-readable name (e.g. "Federation", "Pirate").
    pub name: String,
    /// UUIDs of factions this faction considers enemies.
    #[serde(default)]
    pub enemies: Vec<Uuid>,
}

/// Registry of all loaded factions, keyed by their UUID.
#[derive(Debug, Clone, Default)]
pub struct FactionRegistry {
    factions: HashMap<Uuid, FactionConfig>,
}

impl FactionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a faction into the registry.
    pub fn insert(&mut self, config: FactionConfig) {
        self.factions.insert(config.uuid, config);
    }

    /// Retrieve a faction config by UUID.
    pub fn get(&self, uuid: &Uuid) -> Option<&FactionConfig> {
        self.factions.get(uuid)
    }

    /// Iterate over all registered factions.
    pub fn iter(&self) -> impl Iterator<Item = &FactionConfig> {
        self.factions.values()
    }

    /// Number of registered factions.
    pub fn len(&self) -> usize {
        self.factions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.factions.is_empty()
    }
}

/// Parse a `FactionConfig` from a TOML string.
pub fn parse_faction_config(toml_str: &str) -> Result<FactionConfig, toml::de::Error> {
    toml::from_str(toml_str)
}

/// Returns `true` if faction `a` considers faction `b` an enemy.
///
/// Returns `false` when either argument is `None` (factionless entities are
/// neutral to everyone).
pub fn is_enemy(a: Option<Uuid>, b: Option<Uuid>, registry: &FactionRegistry) -> bool {
    let (Some(a_id), Some(b_id)) = (a, b) else {
        return false;
    };
    registry
        .get(&a_id)
        .map(|fc| fc.enemies.contains(&b_id))
        .unwrap_or(false)
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fed_uuid() -> Uuid {
        Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap()
    }

    fn pirate_uuid() -> Uuid {
        Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap()
    }

    fn make_registry_fed_hostile_to_pirate() -> FactionRegistry {
        let mut reg = FactionRegistry::new();
        reg.insert(FactionConfig {
            uuid: fed_uuid(),
            name: "Federation".to_string(),
            enemies: vec![pirate_uuid()],
        });
        reg.insert(FactionConfig {
            uuid: pirate_uuid(),
            name: "Pirate".to_string(),
            enemies: vec![],
        });
        reg
    }

    // Tracer bullet: both factionless → not enemies
    #[test]
    fn both_factionless_are_not_enemies() {
        let reg = FactionRegistry::new();
        assert!(!is_enemy(None, None, &reg));
    }

    // One factionless → not enemies
    #[test]
    fn one_factionless_is_not_enemy() {
        let reg = make_registry_fed_hostile_to_pirate();
        assert!(!is_enemy(Some(fed_uuid()), None, &reg));
        assert!(!is_enemy(None, Some(pirate_uuid()), &reg));
    }

    // A lists B → A considers B an enemy (asymmetric)
    #[test]
    fn a_lists_b_as_enemy_is_true() {
        let reg = make_registry_fed_hostile_to_pirate();
        assert!(is_enemy(Some(fed_uuid()), Some(pirate_uuid()), &reg));
    }

    // B does NOT list A → not an enemy (asymmetry)
    #[test]
    fn b_does_not_list_a_is_not_enemy() {
        let reg = make_registry_fed_hostile_to_pirate();
        // Pirate has no enemies listed
        assert!(!is_enemy(Some(pirate_uuid()), Some(fed_uuid()), &reg));
    }

    // Neither lists the other → not enemies
    #[test]
    fn neither_lists_other_is_not_enemy() {
        let mut reg = FactionRegistry::new();
        let alpha = Uuid::parse_str("cccccccc-0000-0000-0000-000000000003").unwrap();
        let beta = Uuid::parse_str("dddddddd-0000-0000-0000-000000000004").unwrap();
        reg.insert(FactionConfig {
            uuid: alpha,
            name: "Alpha".to_string(),
            enemies: vec![],
        });
        reg.insert(FactionConfig {
            uuid: beta,
            name: "Beta".to_string(),
            enemies: vec![],
        });
        assert!(!is_enemy(Some(alpha), Some(beta), &reg));
        assert!(!is_enemy(Some(beta), Some(alpha), &reg));
    }

    // TOML round-trip for FactionConfig
    #[test]
    fn faction_config_toml_round_trip() {
        let toml_str = r#"
uuid = "aaaaaaaa-0000-0000-0000-000000000001"
name = "Federation"
enemies = ["bbbbbbbb-0000-0000-0000-000000000002"]
"#;
        let config = parse_faction_config(toml_str).expect("parse must succeed");
        assert_eq!(config.uuid, fed_uuid());
        assert_eq!(config.name, "Federation");
        assert_eq!(config.enemies, vec![pirate_uuid()]);
    }

    // TOML round-trip: enemies defaults to empty when omitted
    #[test]
    fn faction_config_no_enemies_defaults_to_empty() {
        let toml_str = r#"
uuid = "bbbbbbbb-0000-0000-0000-000000000002"
name = "Pirate"
"#;
        let config = parse_faction_config(toml_str).expect("parse must succeed");
        assert!(config.enemies.is_empty());
    }

    // FactionRegistry insert and lookup
    #[test]
    fn registry_insert_and_get() {
        let reg = make_registry_fed_hostile_to_pirate();
        assert_eq!(reg.len(), 2);
        let fed = reg.get(&fed_uuid()).expect("federation must be present");
        assert_eq!(fed.name, "Federation");
    }

    // Unknown faction UUID → not an enemy (registry miss)
    #[test]
    fn unknown_faction_uuid_is_not_enemy() {
        let reg = FactionRegistry::new();
        let unknown = Uuid::new_v4();
        let other = Uuid::new_v4();
        assert!(!is_enemy(Some(unknown), Some(other), &reg));
    }

    // Load actual TOML asset files
    #[test]
    fn federation_toml_parses_correctly() {
        let toml_str = include_str!("../../assets/factions/federation.toml");
        let config = parse_faction_config(toml_str).expect("federation.toml must parse");
        assert_eq!(config.name, "Federation");
        assert!(!config.uuid.is_nil());
        // Must list pirates as enemies
        assert!(!config.enemies.is_empty(), "Federation must have enemies");
    }

    #[test]
    fn pirate_toml_parses_correctly() {
        let toml_str = include_str!("../../assets/factions/pirate.toml");
        let config = parse_faction_config(toml_str).expect("pirate.toml must parse");
        assert_eq!(config.name, "Pirate");
        assert!(!config.uuid.is_nil());
    }

    #[test]
    fn federation_and_pirate_are_mutually_hostile() {
        let fed_toml = include_str!("../../assets/factions/federation.toml");
        let pirate_toml = include_str!("../../assets/factions/pirate.toml");
        let fed = parse_faction_config(fed_toml).unwrap();
        let pirate = parse_faction_config(pirate_toml).unwrap();

        let mut reg = FactionRegistry::new();
        reg.insert(fed.clone());
        reg.insert(pirate.clone());

        assert!(
            is_enemy(Some(fed.uuid), Some(pirate.uuid), &reg),
            "Federation must consider Pirates as enemies"
        );
        assert!(
            is_enemy(Some(pirate.uuid), Some(fed.uuid), &reg),
            "Pirates must consider Federation as enemies"
        );
    }

    #[test]
    fn federation_and_harrow_are_mutually_hostile() {
        // (#472) Federation now lists Harrow as an enemy so the player
        // ship's auto-fire engages Harrow patrols, cruisers, and
        // battleships in the combat-test scenario (#475).
        let fed_toml = include_str!("../../assets/factions/federation.toml");
        let harrow_toml = include_str!("../../assets/factions/harrow.toml");
        let fed = parse_faction_config(fed_toml).unwrap();
        let harrow = parse_faction_config(harrow_toml).unwrap();

        let mut reg = FactionRegistry::new();
        reg.insert(fed.clone());
        reg.insert(harrow.clone());

        assert!(
            is_enemy(Some(fed.uuid), Some(harrow.uuid), &reg),
            "Federation must consider Harrow as enemies (#472)"
        );
        assert!(
            is_enemy(Some(harrow.uuid), Some(fed.uuid), &reg),
            "Harrow must consider Federation as enemies"
        );
    }
}
