//! Closed questions and acquired quantities for the existing focused scanner.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusedAim {
    #[default]
    General,
    Environment,
    Resources,
    Anomaly,
}
impl FocusedAim {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Environment => "environment",
            Self::Resources => "resources",
            Self::Anomaly => "anomaly",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusedAimConfig {
    pub aim: FocusedAim,
    pub work_multiplier: f32,
    pub resource_multiplier: f32,
    #[serde(default)]
    pub intrusive: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingPoint {
    #[default]
    Receiver,
    ObservedSubject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectSignature {
    DamageZone,
    SlowZone,
    BlocksImpulse,
    RadarDampening,
    CommsJam,
    SensorBlind,
    NebulaFog,
}
impl EffectSignature {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DamageZone => "damage_zone",
            Self::SlowZone => "slow_zone",
            Self::BlocksImpulse => "blocks_impulse",
            Self::RadarDampening => "radar_dampening",
            Self::CommsJam => "comms_jam",
            Self::SensorBlind => "sensor_blind",
            Self::NebulaFog => "nebula_fog",
        }
    }
}

/// Selects actual supported machinery state, never a result authored as prose.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnquiryProperty {
    FieldExposure,
    Capacity { capacity: String },
    EffectSignature { effect: EffectSignature },
}
impl EnquiryProperty {
    pub fn aim(&self) -> FocusedAim {
        match self {
            Self::FieldExposure => FocusedAim::Environment,
            Self::Capacity { .. } => FocusedAim::Resources,
            Self::EffectSignature { .. } => FocusedAim::Anomaly,
        }
    }
}

/// New properties are unreadable unless the subject explicitly permits them.
/// Normal and intrusive operations never imply permission for each other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyObservation {
    pub property: EnquiryProperty,
    #[serde(default)]
    pub normal: bool,
    #[serde(default)]
    pub intrusive: bool,
    #[serde(default)]
    pub point: SamplingPoint,
    /// Quantisation step for continuous readings. Counts/flags remain discrete.
    pub step: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnquiryObservationConfig {
    #[serde(default)]
    pub property: Vec<PropertyObservation>,
}
impl EnquiryObservationConfig {
    pub fn validate(&self) -> Result<(), String> {
        let mut properties = std::collections::BTreeSet::new();
        for row in &self.property {
            if !properties.insert(&row.property)
                || !row.step.is_finite()
                || row.step <= 0.0
                || matches!(&row.property, EnquiryProperty::Capacity { capacity } if capacity.is_empty() || capacity.len() > 128 || capacity.chars().any(char::is_control))
            {
                return Err("enquiry properties require unique supported selectors and finite positive precision".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanQuantityKind {
    FieldExposure,
    Capacity,
    EffectSignature,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ScanQuantityValue {
    Scalar { value: f64, step: f64 },
    Count(i64),
    Flag(bool),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanQuantity {
    pub kind: ScanQuantityKind,
    /// Known display label, not an internal capacity or component identifier.
    pub label: String,
    pub value: ScanQuantityValue,
}
impl ScanQuantity {
    pub fn validate(&self) -> Result<(), String> {
        let shape = matches!((&self.kind, &self.value),
            (ScanQuantityKind::FieldExposure, ScanQuantityValue::Scalar { value, step })
                if value.is_finite() && *value >= 0.0 && step.is_finite() && *step > 0.0)
            || matches!(
                (&self.kind, &self.value),
                (ScanQuantityKind::Capacity, ScanQuantityValue::Count(value)) if *value >= 0
            )
            || matches!(
                (&self.kind, &self.value),
                (
                    ScanQuantityKind::EffectSignature,
                    ScanQuantityValue::Flag(_)
                )
            );
        if !shape
            || self.label.is_empty()
            || self.label.len() > 256
            || self.label.chars().any(char::is_control)
        {
            return Err("recorded enquiry quantity has unsupported shape or precision".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "focused_enquiry_tests.rs"]
mod tests;
