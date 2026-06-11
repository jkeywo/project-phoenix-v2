use serde::{Deserialize, Serialize};

/// Targetability tags, threat level, and description for an entity.
///
/// Loaded from the entity TOML's `[target]` section. When absent, the entity
/// is **not targetable** (empty tags = matches no `selects` filter).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetSection {
    /// Targetability tags e.g. `["hostile", "neutral", "civilian"]`.
    /// A console's radars can filter on these via their `selects` list.
    /// Empty = not targetable by any console.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Cosmetic threat level shown in the target info panel.
    #[serde(default)]
    pub threat_level: ThreatLevel,
    /// Short description shown in the target info panel (e.g. "Klingon Bird-of-Prey").
    /// Falls back to the entity's `name` when absent.
    #[serde(default)]
    pub description: Option<String>,
}

/// Purely cosmetic threat level for the target info panel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreatLevel {
    None,
    Low,
    Medium,
    High,
}

impl Default for ThreatLevel {
    fn default() -> Self {
        Self::None
    }
}

impl ThreatLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            ThreatLevel::None => "none",
            ThreatLevel::Low => "low",
            ThreatLevel::Medium => "medium",
            ThreatLevel::High => "high",
        }
    }
}
