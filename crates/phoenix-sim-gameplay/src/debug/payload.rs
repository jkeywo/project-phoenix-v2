//! Gameplay-owned modifier projections.
pub use phoenix_sim_contracts::debug_schema::DEBUG_SCHEMA_VERSION;
use serde::{Deserialize, Serialize};
/// One active boolean modifier flag and the sources that set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModifierFlagEntry {
    /// The `FlagKind` name (its `Debug` spelling).
    pub flag: String,
    /// The sources holding this flag active, sorted.
    pub sources: Vec<String>,
}

/// One source's additive bonus to a float modifier slot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloatContribution {
    /// The rendered `ModifierSource` (e.g. `PowerGroup(helm)`, `Region(1a2b3c4d)`).
    pub source: String,
    /// The additive bonus this source contributes (positive buff, negative debuff).
    pub bonus: f32,
}

/// One float modifier slot: its computed multiplier and per-source breakdown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloatModifierEntry {
    /// The `ModifierSlot` name (its `Debug` spelling).
    pub slot: String,
    /// The cached multiplier the simulation applies for this slot.
    pub multiplier: f32,
    /// Per-source additive contributions, sorted by rendered source.
    pub contributions: Vec<FloatContribution>,
}

/// One source's additive bonus to an integer modifier slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntContribution {
    /// The rendered `ModifierSource`.
    pub source: String,
    /// The additive integer bonus this source contributes.
    pub bonus: i32,
}

/// One integer modifier slot: its summed total and per-source breakdown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntModifierEntry {
    /// The `IntModifierSlot` name (its `Debug` spelling).
    pub slot: String,
    /// The summed total across all active sources.
    pub sum: i32,
    /// Per-source additive contributions, sorted by rendered source.
    pub contributions: Vec<IntContribution>,
}

/// The modifier surface's whole payload for the LocalShip.
///
/// Every section is empty when the LocalShip has no modifiers, or when there is
/// no LocalShip at all (a headless run) â€” the payload is always produced so the
/// dock and the determinism guard have something to read.
///
/// Not `Eq`: `FloatModifierEntry::multiplier` is an `f32`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModifierDebugPayload {
    /// [`DEBUG_SCHEMA_VERSION`] at the time the host produced this payload.
    pub schema_version: u32,
    /// Active boolean flags, sorted by flag name.
    pub flags: Vec<ModifierFlagEntry>,
    /// Active float modifier slots, sorted by slot name.
    pub float_modifiers: Vec<FloatModifierEntry>,
    /// Active integer modifier slots, sorted by slot name.
    pub int_modifiers: Vec<IntModifierEntry>,
}

impl Default for ModifierDebugPayload {
    fn default() -> Self {
        Self {
            schema_version: DEBUG_SCHEMA_VERSION,
            flags: Vec::new(),
            float_modifiers: Vec::new(),
            int_modifiers: Vec::new(),
        }
    }
}
