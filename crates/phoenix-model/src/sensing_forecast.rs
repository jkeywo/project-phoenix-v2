//! Conditional projections over held observations; never a future-state query.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanForecastPolicy {
    pub horizon_secs: f32,
}
impl ScanForecastPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !self.horizon_secs.is_finite() || self.horizon_secs <= 0.0 {
            return Err("forecast horizon must be finite and positive".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ForecastQuantity {
    Condition,
    Capacity {
        label: String,
    },
    Observed {
        measurement: crate::focused_enquiry::ScanQuantityKind,
        label: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForecastRange {
    pub quantity: ForecastQuantity,
    pub lower: f64,
    pub upper: f64,
}

/// Times identify the original samples, not receipt/presentation times. Values
/// are conditional extrapolations at the horizon after the second sample.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanForecast {
    pub sample_ticks: [u64; 2],
    pub sample_interval_secs: f64,
    pub horizon_secs: f32,
    pub assumptions: Vec<String>,
    pub ranges: Vec<ForecastRange>,
}
impl ScanForecast {
    pub fn validate(
        &self,
        sample_tick: u64,
        condition: Option<f32>,
        capacities: &[(String, i64)],
        observed: &[crate::focused_enquiry::ScanQuantity],
    ) -> Result<(), String> {
        if self.sample_ticks[0] >= self.sample_ticks[1]
            || self.sample_ticks[1] != sample_tick
            || !self.sample_interval_secs.is_finite()
            || self.sample_interval_secs <= 0.0
            || (ScanForecastPolicy {
                horizon_secs: self.horizon_secs,
            })
            .validate()
            .is_err()
            || self.assumptions
                != [
                    "component.scan.forecast.linear",
                    "component.scan.forecast.quantisation",
                ]
            || self.ranges.is_empty()
            || self.ranges.len()
                > capacities.len() + usize::from(condition.is_some()) + observed.len()
        {
            return Err("invalid recorded forecast frontier or assumptions".into());
        }
        let mut quantities = std::collections::BTreeSet::new();
        for range in &self.ranges {
            if !range.lower.is_finite() || !range.upper.is_finite() || range.lower > range.upper {
                return Err("invalid recorded forecast interval".into());
            }
            let key = match &range.quantity {
                ForecastQuantity::Condition => {
                    if condition.is_none() || range.lower < 0.0 || range.upper > 1.0 {
                        return Err("invalid recorded condition forecast".into());
                    }
                    (0, String::new())
                }
                ForecastQuantity::Capacity { label } => {
                    if label.is_empty() || !capacities.iter().any(|(held, _)| held == label) {
                        return Err("forecast references an unobserved quantity".into());
                    }
                    (1, label.clone())
                }
                ForecastQuantity::Observed {
                    measurement: kind,
                    label,
                } => {
                    let matching: Vec<_> = observed
                        .iter()
                        .filter(|row| row.kind == *kind && row.label == *label)
                        .collect();
                    if matching.len() != 1
                        || matching[0].validate().is_err()
                        || matches!(
                            matching[0].value,
                            crate::focused_enquiry::ScanQuantityValue::Flag(_)
                        )
                        || (*kind == crate::focused_enquiry::ScanQuantityKind::FieldExposure
                            && range.lower < 0.0)
                    {
                        return Err(
                            "forecast references an unobserved or nonnumeric enquiry quantity"
                                .into(),
                        );
                    }
                    (2 + *kind as u8, label.clone())
                }
            };
            if !quantities.insert(key) {
                return Err("duplicate recorded forecast quantity".into());
            }
        }
        if condition.is_some() && !quantities.contains(&(0, String::new())) {
            return Err("missing condition forecast".into());
        }
        Ok(())
    }
}
