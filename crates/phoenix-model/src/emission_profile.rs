//! Advisory own-ship intent. This neither limits controls nor promises detection.
use crate::{identity::SystemId, sensing::PassiveSignature};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmissionIntent {
    #[default]
    Normal,
    LowEmission,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmissionProfileConfig {
    #[serde(default)]
    pub initial: EmissionIntent,
    /// Energy per second, presentation only; this is never a permitted ceiling.
    pub advisory_rate: f32,
}
impl EmissionProfileConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.advisory_rate.is_finite() || self.advisory_rate <= 0.0 {
            return Err("emission advisory_rate must be finite and positive");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmissionIntentView {
    pub owner: SystemId,
    pub intent: EmissionIntent,
    pub controllable: bool,
}

/// One actual machinery owner's current measurement, never a foreign listener.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmissionAdvisoryView {
    pub observed_tick: u64,
    /// Finite measured presentation, matching other Eq common status details.
    pub rate: String,
    pub cause: PassiveSignature,
    pub advisory_rate: String,
    pub high: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_is_closed_and_has_no_physical_limit() {
        for rate in [0.0, -1.0, f32::INFINITY, f32::NAN] {
            assert!(EmissionProfileConfig {
                initial: EmissionIntent::Normal,
                advisory_rate: rate
            }
            .validate()
            .is_err());
        }
        let config: EmissionProfileConfig =
            serde_json::from_str(r#"{"initial":"low_emission","advisory_rate":2.5}"#).unwrap();
        assert!(config.validate().is_ok());
        assert!(serde_json::from_str::<EmissionProfileConfig>(
            r#"{"advisory_rate":2.5,"ceiling":3}"#
        )
        .is_err());
    }
}
