//! Closed physical damage vocabulary. Machinery, stock and team state have
//! their existing owners; this module introduces no controller or inventory.
use crate::messages::SystemId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageType {
    Physical,
    Laser,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageProfile {
    pub damage_type: DamageType,
    pub penetration: f32,
}

// Rust fixture/derive convenience only. Authored equipment fields stay required
// at their owning serde boundary; this is not an old-content inference path.
impl Default for DamageProfile {
    fn default() -> Self {
        Self {
            damage_type: DamageType::Physical,
            penetration: 0.0,
        }
    }
}
impl DamageProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.penetration.is_finite() && self.penetration >= 0.0 {
            Ok(())
        } else {
            Err("damage penetration must be finite and nonnegative".into())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentRef {
    pub system: SystemId,
    pub component: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageBinding {
    Operation,
    Intake,
    Cooling,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageComponentConfig {
    pub id: String,
    pub binding: DamageBinding,
    pub susceptibility: BTreeMap<DamageType, f32>,
    pub fault_dose_threshold: f32,
    pub degraded_factor: f32,
    pub full_remedy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_remedy: Option<String>,
    pub field_replaceable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialConfig {
    pub resistance: BTreeMap<DamageType, f32>,
    pub residual_fraction: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemedyOutcome {
    Structural,
    Full,
    Patch { factor: f32 },
}

/// A named LOCAL resource service. The repair owner resolves this through the
/// existing resource-service contract and commits against the actual stock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartRequirement {
    pub resource: String,
    pub service: String,
    pub amount: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemedyConfig {
    pub work_secs: f32,
    /// Unpowered team work; paid tools accelerate the remaining fraction.
    /// Real crew, access, parts and bounded work are required in either case.
    pub manual_work_fraction: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural_hp: Option<f32>,
    pub outcome: RemedyOutcome,
    #[serde(default)]
    pub requirements: Vec<PartRequirement>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageConfig {
    pub materials: BTreeMap<String, MaterialConfig>,
    pub remedies: BTreeMap<String, RemedyConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepairWorkTarget {
    Compartment { compartment: String },
    Component { system: SystemId, component: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentCondition {
    Healthy,
    Damaged,
    Destroyed,
    Patched { factor: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentState {
    pub condition: ComponentCondition,
    /// Fractional deposited dose below the next fault threshold. Persisted,
    /// never a per-hit probability or a second equipment HP pool.
    pub dose: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompartmentState {
    pub hp: f64,
    pub disabled: bool,
}

/// Permitted status projection: no private dose, source or controller memory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicalComponentStatus {
    pub id: String,
    pub binding: DamageBinding,
    pub condition: ComponentCondition,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicalSystemStatus {
    pub compartment: String,
    pub structural_hp: f32,
    pub structural_max_hp: f32,
    pub structurally_disabled: bool,
    pub components: Vec<PhysicalComponentStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HitSample {
    pub deck: String,
    /// Unit quantile across the UNION of facing intervals. A continuous bank
    /// retains this sample in its own existing attack continuation.
    pub profile_fraction: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageHit {
    /// Actual local planar TRAVEL direction into the hull, not attacker bearing.
    pub incoming: [f32; 2],
    pub profile: DamageProfile,
    pub payload: f64,
    /// A caller may supply only an already acquired compartment. No component
    /// coordinates, deck guarantee or new source of target intelligence.
    pub aim: Option<String>,
    pub source: String,
    pub sample: Option<HitSample>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompartmentDeposit {
    pub compartment: String,
    pub payload: f64,
    pub structural_loss: f64,
    pub penetration_spent: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DamageOutcome {
    pub sample: HitSample,
    pub deposits: Vec<CompartmentDeposit>,
    pub changed_components: Vec<ComponentRef>,
    pub structural_loss: f64,
    pub escaped_payload: f64,
    pub remaining_penetration: f64,
}
