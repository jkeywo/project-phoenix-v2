//! Closed fitting vocabulary. Equipment identity is separate from fungible Stock.
use crate::{
    damage::{ComponentState, DamageProfile, PartRequirement},
    messages::SystemId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentFamily {
    Beam,
    Reactor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefitClass {
    Field,
    Facility,
}

/// Explicit provider capability; permissions remain in its canonical port policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FittingFacilityConfig {
    pub system: SystemId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefitWorkConfig {
    pub class: RefitClass,
    pub work_secs: f32,
    pub manual_work_fraction: f32,
    #[serde(default)]
    pub requirements: Vec<PartRequirement>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeamHardware {
    #[serde(default)]
    pub operation: crate::weapon_operation::BeamOperation,
    pub beam_range: f32,
    pub auto_arc_deg: f32,
    pub beam_damage_per_sec: f32,
    pub beam_duration_secs: f32,
    pub cooldown_secs: f32,
    pub cycle_jitter: f32,
    pub shield_pierce: f32,
    pub impact: DamageProfile,
    pub continuous_cost_per_sec: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactorHardware {
    pub sustainable_output_per_sec: f32,
    pub max_output_per_sec: f32,
    pub ramp_per_sec: f32,
    pub charge_heat_per_unit: f32,
    pub discharge_heat_per_unit: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EquipmentHardware {
    Beam(BeamHardware),
    Reactor(ReactorHardware),
}
impl EquipmentHardware {
    pub fn family(&self) -> EquipmentFamily {
        match self {
            Self::Beam(_) => EquipmentFamily::Beam,
            Self::Reactor(_) => EquipmentFamily::Reactor,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentDefinition {
    pub id: String,
    pub interface: String,
    pub size: u16,
    pub storage_space: u32,
    pub work: RefitWorkConfig,
    pub hardware: EquipmentHardware,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FittingSlotConfig {
    pub id: String,
    pub system: SystemId,
    pub family: EquipmentFamily,
    pub interface: String,
    pub size: u16,
    pub default_equipment: String,
    /// The replaceable Operation hardware; intake/cooling/buffers stay fixed.
    pub operation_component: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentLockerConfig {
    pub id: String,
    pub compartment: String,
    pub space: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EquipmentLocation {
    Installed { slot: String },
    Stored { locker: String },
    InWork { job: u64, compartment: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentAssembly {
    pub id: String,
    pub definition: String,
    pub location: EquipmentLocation,
    /// None while installed: ShipDamage is then the sole condition owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<ComponentState>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FittingConfig {
    #[serde(default, rename = "slot")]
    pub slots: Vec<FittingSlotConfig>,
    #[serde(default, rename = "equipment")]
    pub equipment: Vec<EquipmentDefinition>,
    #[serde(default, rename = "locker")]
    pub lockers: Vec<EquipmentLockerConfig>,
    #[serde(default, rename = "assembly")]
    pub assemblies: Vec<EquipmentAssembly>,
}
