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
    /// Reference name (e.g. "Alliance", "Pirate") — the id world triggers and
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
    /// (e.g. `add_faction_enemy { faction = "Harrow", enemy = "Alliance" }`)
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
#[path = "faction_tests.rs"]
mod tests;
