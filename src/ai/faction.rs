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
    /// Reference name (e.g. "Federation", "Pirate") — the id world triggers and
    /// entity templates name a faction by, and NOT display text: no
    /// player-facing surface renders it.
    pub name: String,
    /// `strings.csv` id for the crew-facing label, when the setting wants this
    /// faction nameable on a player surface (issue #1030's dossier).
    ///
    /// Optional, and beside [`name`](Self::name) rather than replacing it,
    /// because `name` is a reference key shipped world TOML and fragments
    /// already spell out — turning it into a string id would rewrite every
    /// `add_faction_enemy` in the repo to say the same thing. A faction that
    /// authors none has no name the crew can be shown, and the dossier omits
    /// the row rather than putting English on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// UUIDs of factions this faction considers enemies.
    #[serde(default)]
    pub enemies: Vec<Uuid>,
    /// How this faction's civilian traffic answers crew orders (issue #1028).
    ///
    /// The *fallback* half of the two-level ladder an ordered civilian resolves
    /// through: a hull's own `[civilian.compliance]` table wins, this stands in
    /// when it authors none, and a cooperative default stands in when neither
    /// exists. Faction-level because "the Combine's haulers never divert" is a
    /// fact about the operator rather than about one ship, and a scenario that
    /// wants a whole shipping line to be difficult should not have to say so on
    /// every hull.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compliance: Option<crate::civilian::ComplianceDisposition>,
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

    /// Look up a faction's UUID by its human-readable `name` field
    /// (case-sensitive exact match). Returns `None` if no faction matches.
    ///
    /// Used by world trigger actions that reference factions by name
    /// (e.g. `add_faction_enemy { faction = "Harrow", enemy = "Federation" }`)
    /// so scenario authors don't have to write raw UUIDs in TOML.
    ///
    /// Lowest matching uuid wins, rather than "whichever the map yields first"
    /// (issue #965). Names are expected to be unique and nothing here enforces
    /// it; with a `find` over a `HashMap`, two factions sharing a name would
    /// have resolved a world trigger to a DIFFERENT faction in each process,
    /// because the walk order follows `RandomState`'s per-process seed. `min`
    /// is the same single pass and gives duplicate names one answer everywhere.
    pub fn uuid_by_name(&self, name: &str) -> Option<Uuid> {
        self.factions
            .values()
            .filter(|fc| fc.name == name)
            .map(|fc| fc.uuid)
            .min()
    }

    /// Add `enemy_uuid` to `faction_uuid`'s enemies list.
    ///
    /// Returns `true` if the relationship was newly added, `false` if
    /// either faction is unknown or the enemy was already listed.
    /// Idempotent: calling twice with the same arguments is a no-op
    /// (matching `Vec::contains` semantics).
    pub fn add_enemy(&mut self, faction_uuid: Uuid, enemy_uuid: Uuid) -> bool {
        let Some(fc) = self.factions.get_mut(&faction_uuid) else {
            return false;
        };
        if fc.enemies.contains(&enemy_uuid) {
            return false;
        }
        fc.enemies.push(enemy_uuid);
        true
    }

    /// Remove `enemy_uuid` from `faction_uuid`'s enemies list.
    ///
    /// Returns `true` if the relationship was actually removed, `false`
    /// if either faction is unknown or the enemy was not listed.
    /// Idempotent: calling twice with the same arguments is a no-op.
    pub fn remove_enemy(&mut self, faction_uuid: Uuid, enemy_uuid: Uuid) -> bool {
        let Some(fc) = self.factions.get_mut(&faction_uuid) else {
            return false;
        };
        let before = fc.enemies.len();
        fc.enemies.retain(|e| *e != enemy_uuid);
        fc.enemies.len() != before
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
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
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
            display_name: None,
            uuid: fed_uuid(),
            name: "Federation".to_string(),
            enemies: vec![pirate_uuid()],
            compliance: None,
        });
        reg.insert(FactionConfig {
            display_name: None,
            uuid: pirate_uuid(),
            name: "Pirate".to_string(),
            enemies: vec![],
            compliance: None,
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
            display_name: None,
            uuid: alpha,
            name: "Alpha".to_string(),
            enemies: vec![],
            compliance: None,
        });
        reg.insert(FactionConfig {
            display_name: None,
            uuid: beta,
            name: "Beta".to_string(),
            enemies: vec![],
            compliance: None,
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
    fn federation_and_harrow_are_neutral_by_default() {
        // Harrow defaults to neutral so it can be reused as ambient
        // patrols in non-combat worlds (e.g. Starbase Alpha, Before the
        // Fire). Hostile scenarios (combat test) flip the relationship at
        // runtime via the `add_faction_enemy` trigger action.
        let fed_toml = include_str!("../../assets/factions/federation.toml");
        let harrow_toml = include_str!("../../assets/factions/harrow.toml");
        let fed = parse_faction_config(fed_toml).unwrap();
        let harrow = parse_faction_config(harrow_toml).unwrap();

        let mut reg = FactionRegistry::new();
        reg.insert(fed.clone());
        reg.insert(harrow.clone());

        assert!(
            !is_enemy(Some(fed.uuid), Some(harrow.uuid), &reg),
            "Federation must default to neutral toward Harrow"
        );
        assert!(
            !is_enemy(Some(harrow.uuid), Some(fed.uuid), &reg),
            "Harrow must default to neutral toward Federation"
        );
    }

    // ── Mutators ──────────────────────────────────────────────────────────────

    #[test]
    fn uuid_by_name_finds_existing_faction() {
        let reg = make_registry_fed_hostile_to_pirate();
        assert_eq!(reg.uuid_by_name("Federation"), Some(fed_uuid()));
        assert_eq!(reg.uuid_by_name("Pirate"), Some(pirate_uuid()));
    }

    #[test]
    fn uuid_by_name_returns_none_for_unknown() {
        let reg = make_registry_fed_hostile_to_pirate();
        assert!(reg.uuid_by_name("Klingon").is_none());
    }

    #[test]
    fn uuid_by_name_is_case_sensitive() {
        let reg = make_registry_fed_hostile_to_pirate();
        assert!(reg.uuid_by_name("federation").is_none());
        assert!(reg.uuid_by_name("FEDERATION").is_none());
    }

    #[test]
    fn add_enemy_creates_new_relationship() {
        let mut reg = FactionRegistry::new();
        let alpha = Uuid::parse_str("cccccccc-0000-0000-0000-000000000003").unwrap();
        let beta = Uuid::parse_str("dddddddd-0000-0000-0000-000000000004").unwrap();
        reg.insert(FactionConfig {
            display_name: None,
            uuid: alpha,
            name: "Alpha".to_string(),
            enemies: vec![],
            compliance: None,
        });
        reg.insert(FactionConfig {
            display_name: None,
            uuid: beta,
            name: "Beta".to_string(),
            enemies: vec![],
            compliance: None,
        });
        assert!(!is_enemy(Some(alpha), Some(beta), &reg));
        assert!(reg.add_enemy(alpha, beta), "first add returns true");
        assert!(is_enemy(Some(alpha), Some(beta), &reg));
        // Asymmetric — Beta still does not consider Alpha an enemy.
        assert!(!is_enemy(Some(beta), Some(alpha), &reg));
    }

    #[test]
    fn add_enemy_is_idempotent() {
        let mut reg = make_registry_fed_hostile_to_pirate();
        // Federation already lists Pirate as an enemy.
        assert!(!reg.add_enemy(fed_uuid(), pirate_uuid()));
        // And the relationship hasn't been duplicated.
        let fed = reg.get(&fed_uuid()).unwrap();
        assert_eq!(
            fed.enemies.iter().filter(|u| **u == pirate_uuid()).count(),
            1
        );
    }

    #[test]
    fn add_enemy_returns_false_for_unknown_faction() {
        let mut reg = make_registry_fed_hostile_to_pirate();
        let unknown = Uuid::new_v4();
        assert!(!reg.add_enemy(unknown, fed_uuid()));
    }

    #[test]
    fn remove_enemy_clears_relationship() {
        let mut reg = make_registry_fed_hostile_to_pirate();
        assert!(is_enemy(Some(fed_uuid()), Some(pirate_uuid()), &reg));
        assert!(reg.remove_enemy(fed_uuid(), pirate_uuid()));
        assert!(!is_enemy(Some(fed_uuid()), Some(pirate_uuid()), &reg));
    }

    #[test]
    fn remove_enemy_is_idempotent() {
        let mut reg = make_registry_fed_hostile_to_pirate();
        // Pirate has no enemies listed → removing Federation is a no-op.
        assert!(!reg.remove_enemy(pirate_uuid(), fed_uuid()));
    }

    #[test]
    fn remove_enemy_returns_false_for_unknown_faction() {
        let mut reg = make_registry_fed_hostile_to_pirate();
        let unknown = Uuid::new_v4();
        assert!(!reg.remove_enemy(unknown, fed_uuid()));
    }
}
