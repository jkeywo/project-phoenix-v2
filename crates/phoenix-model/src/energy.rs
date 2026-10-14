//! Permitted equipment readings and admitted energy control vocabulary.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnergyAllocationMode {
    #[default]
    Manual,
    Minimal,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoolingUsage {
    None,
    #[default]
    Low,
    High,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnergyControlDepth {
    Min,
    #[default]
    Mid,
    Detail,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PowerEnergyBlackboard {
    pub reactor: ReactorEnergyReading,
    pub mode: EnergyAllocationMode,
    pub cooling_usage: CoolingUsage,
    pub groups: Vec<EnergyGroupReading>,
    pub consumers: Vec<ConsumerEnergyReading>,
    pub coolant: Vec<CoolantReading>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReactorEnergyReading {
    pub energy: f32,
    pub capacity: f32,
    pub heat: f32,
    pub trip_heat: f32,
    pub recover_heat: f32,
    pub thermal_stopped: bool,
    pub output_target_per_sec: f32,
    pub output_actual_per_sec: f32,
    pub sustainable_output_per_sec: f32,
    pub max_output_per_sec: f32,
    pub cooling_per_sec: f32,
    pub max_cooling_per_sec: f32,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EnergyGroupReading {
    pub id: String,
    pub label: String,
    pub parent: Option<String>,
    pub desired_flow_per_sec: f32,
    pub effective_flow_per_sec: f32,
    pub max_flow_per_sec: f32,
    pub desired_cooling_per_sec: f32,
    pub effective_cooling_per_sec: f32,
    pub max_cooling_per_sec: f32,
    pub priority: u16,
    pub cut: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsumerEnergyReading {
    pub system: crate::identity::SystemId,
    pub group: Option<crate::identity::PowerGroupId>,
    pub energy: f32,
    pub capacity: f32,
    pub heat: f32,
    pub trip_heat: f32,
    pub recover_heat: f32,
    pub thermal_stopped: bool,
    pub intake_per_sec: f32,
    pub max_intake_per_sec: f32,
    pub flow_override_per_sec: Option<f32>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoolantReading {
    pub provider: String,
    pub available: i64,
    pub credit: f64,
}
