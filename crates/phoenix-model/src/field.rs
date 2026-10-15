//! Closed authored spatial fields and substantial signal materials.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldFalloff {
    Step,
    Linear,
    Smooth,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldEvolution {
    pub duration_secs: f32,
    pub translation: [f32; 3],
    pub final_scale: f32,
    pub final_strength: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldConfig {
    pub falloff: FieldFalloff,
    pub strength: f32,
    pub sensor_absorption_per_unit: f32,
    pub comms_absorption_per_unit: f32,
    pub evolution: Option<FieldEvolution>,
}

impl FieldConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("strength", self.strength),
            (
                "sensor_absorption_per_unit",
                self.sensor_absorption_per_unit,
            ),
            ("comms_absorption_per_unit", self.comms_absorption_per_unit),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("field.{name} must be finite and nonnegative"));
            }
        }
        if let Some(evolution) = &self.evolution {
            if !evolution.duration_secs.is_finite()
                || evolution.duration_secs <= 0.0
                || !evolution.final_scale.is_finite()
                || evolution.final_scale <= 0.0
                || !evolution.final_strength.is_finite()
                || evolution.final_strength < 0.0
                || evolution.translation.iter().any(|value| !value.is_finite())
            {
                return Err("field.evolution requires positive finite duration/scale, nonnegative strength and finite translation".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ecs", derive(bevy_ecs::prelude::Component))]
#[serde(deny_unknown_fields)]
pub struct SignalMaterialConfig {
    pub sensor_absorption_per_unit: f32,
    pub comms_absorption_per_unit: f32,
    pub sensor_opaque: bool,
    pub comms_opaque: bool,
}
impl SignalMaterialConfig {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.sensor_absorption_per_unit,
            self.comms_absorption_per_unit,
        ]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        {
            Err("signal_material absorption must be finite and nonnegative".into())
        } else {
            Ok(())
        }
    }
}
