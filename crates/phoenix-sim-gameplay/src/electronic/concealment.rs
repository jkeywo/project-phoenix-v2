//! A paid physical field reduces perceptibility; it never supplies invisibility.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcealmentConfig {
    pub max_suppression: f32,
    pub emission_scale: f32,
}
impl ConcealmentConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.max_suppression.is_finite()
            || self.max_suppression <= 0.0
            || self.max_suppression >= 1.0
            || !self.emission_scale.is_finite()
            || self.emission_scale <= 0.0
        {
            return Err("concealment requires suppression strictly between zero and one and a finite positive emission scale".into());
        }
        Ok(())
    }
    /// Actual work fraction and operating emission are measured by the caller.
    /// Invalid or unpaid work is neutral. Widen arithmetic before division so
    /// extreme finite operating work weakens suppression without overflow.
    pub fn perceptibility(&self, paid_fraction: f32, actual_emission: f32) -> f32 {
        if self.validate().is_err()
            || !paid_fraction.is_finite()
            || paid_fraction <= 0.0
            || paid_fraction > 1.0
            || !actual_emission.is_finite()
            || actual_emission < 0.0
        {
            return 1.0;
        }
        let suppression = f64::from(self.max_suppression) * f64::from(paid_fraction)
            / (1.0 + f64::from(actual_emission) / f64::from(self.emission_scale));
        (1.0 - suppression) as f32
    }
}
pub fn bindings(enabled: &str) -> crate::ai::state_tree::SupportedBindings {
    crate::ai::state_tree::SupportedBindings {
        facts: ["enabled".into()].into(),
        channels: [enabled.into()].into(),
    }
}
pub fn validate_equipment(
    entity: &crate::entities::config::EntityConfig,
    system: &crate::ship::config::SystemInstanceConfig,
) -> Result<(), String> {
    crate::ship::composition::validate_composition(system)?;
    if entity.energy.as_ref().is_none_or(|energy| {
        !energy
            .consumers
            .iter()
            .any(|consumer| consumer.system == system.id && consumer.continuous_cost_per_sec > 0.0)
    }) {
        return Err("concealment requires an actual positive continuous work consumer".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paid_partial_work_and_actual_emissions_have_bounded_independent_effects() {
        let field = ConcealmentConfig {
            max_suppression: 0.8,
            emission_scale: 10.0,
        };
        assert_eq!(field.perceptibility(0.0, 0.0), 1.0);
        let quiet = field.perceptibility(1.0, 0.0);
        let partial = field.perceptibility(0.5, 0.0);
        let emitting = field.perceptibility(1.0, 100.0);
        assert!(0.0 < quiet && quiet < partial && partial < emitting && emitting < 1.0);
        assert!(field.perceptibility(1.0, f32::MAX).is_finite());
        for invalid in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(field.perceptibility(invalid, 0.0), 1.0);
            assert_eq!(field.perceptibility(1.0, invalid), 1.0);
        }
    }
    #[test]
    fn accepted_profiles_cannot_make_a_perfectly_invisible_return() {
        for suppression in [0.0, 1.0, -0.1, f32::NAN, f32::INFINITY] {
            assert!(ConcealmentConfig {
                max_suppression: suppression,
                emission_scale: 1.0
            }
            .validate()
            .is_err());
        }
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(ConcealmentConfig {
                max_suppression: 0.8,
                emission_scale: scale
            }
            .validate()
            .is_err());
        }
        let strongest = ConcealmentConfig {
            max_suppression: f32::from_bits(1.0_f32.to_bits() - 1),
            emission_scale: f32::MIN_POSITIVE,
        };
        assert!(strongest.perceptibility(1.0, 0.0) > 0.0);
        assert!(serde_json::from_value::<ConcealmentConfig>(
            serde_json::json!({"max_suppression":0.8,"emission_scale":1,"invisible":true})
        )
        .is_err());
    }
}
