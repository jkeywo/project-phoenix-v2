//! Actual transponder claim data. No actor truth, observer selection or controller.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityEmitterConfig {
    /// Literal emitted callsign, not a scenario/entity lookup.
    pub callsign: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faction: Option<String>,
}
impl IdentityEmitterConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.callsign.trim().is_empty()
            || self.callsign.len() > 128
            || self.callsign.chars().any(char::is_control)
            || self
                .faction
                .as_ref()
                .is_some_and(|value| uuid::Uuid::parse_str(value).is_err())
        {
            return Err("identity emitter requires a bounded literal callsign and optional valid faction identity".into());
        }
        Ok(())
    }
    /// Canonical spelling only; declaring a claim changes no actual faction.
    pub fn faction(&self) -> Option<String> {
        self.faction
            .as_ref()
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .map(|id| id.to_string())
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
        return Err("identity emitter requires an actual positive continuous work consumer".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claims_are_closed_literal_data_without_actor_or_observer_lookup() {
        let claim = IdentityEmitterConfig {
            callsign: "Transit courier".into(),
            faction: None,
        };
        claim.validate().unwrap();
        assert!(claim.faction().is_none());
        for callsign in ["", "\nfalse", &"x".repeat(129)] {
            assert!(IdentityEmitterConfig {
                callsign: callsign.into(),
                faction: None
            }
            .validate()
            .is_err());
        }
        assert!(IdentityEmitterConfig {
            faction: Some("hidden-actor".into()),
            ..claim.clone()
        }
        .validate()
        .is_err());
        assert!(serde_json::from_value::<IdentityEmitterConfig>(
            serde_json::json!({"callsign":"x","source_uuid":"secret"})
        )
        .is_err());
        assert!(bindings("enabled").facts.contains("enabled"));
        assert_eq!(bindings("enabled").facts.len(), 1);
    }
}
