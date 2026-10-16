//! Single data-owned Command intent and shared accepted composition validation.
use crate::ship::{config::ShipConfig, system_registry::COMMAND_KIND};
use phoenix_model::{
    emission_profile::{EmissionIntent, EmissionProfileConfig},
    identity::SystemId,
};
use serde::{Deserialize, Serialize};

pub fn profile(ship: &ShipConfig) -> Result<Option<(SystemId, EmissionProfileConfig)>, String> {
    let mut selected = None;
    for system in &ship.systems {
        let Some(value) = system
            .config
            .as_ref()
            .and_then(|config| config.get("emission_profile"))
        else {
            continue;
        };
        if system.kind != COMMAND_KIND || ship.console_for_system(&system.id).is_none() {
            return Err("emission_profile requires an actual Console-owned Command System".into());
        }
        if selected.is_some() {
            return Err("only one actual Command System may own emission_profile".into());
        }
        let config: EmissionProfileConfig = value
            .clone()
            .try_into()
            .map_err(|error| format!("invalid emission_profile: {error}"))?;
        config.validate().map_err(str::to_string)?;
        selected = Some((system.id.clone(), config));
    }
    Ok(selected)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmissionIntentState {
    pub owner: SystemId,
    pub intent: EmissionIntent,
}
impl EmissionIntentState {
    pub fn new(owner: SystemId, config: &EmissionProfileConfig) -> Self {
        Self {
            owner,
            intent: config.initial,
        }
    }
    pub fn validate(&self, ship: &ShipConfig) -> Result<(), String> {
        if profile(ship)?.is_none_or(|(owner, _)| owner != self.owner) {
            return Err(
                "emission intent does not belong to the accepted actual Command owner".into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ship() -> ShipConfig {
        toml::from_str(
            r#"
[[console]]
id = "orders"
[[system]]
id = "quiet-command"
kind = "command"
console = "orders"
[system.config.emission_profile]
initial = "low_emission"
advisory_rate = 0.5
"#,
        )
        .unwrap()
    }
    #[test]
    fn actual_authored_owner_has_one_canonical_intent_without_a_seat_rule() {
        let ship = ship();
        let (owner, config) = profile(&ship).unwrap().unwrap();
        assert_eq!(owner, SystemId("quiet-command".into()));
        let mut state = EmissionIntentState::new(owner, &config);
        assert_eq!(state.intent, EmissionIntent::LowEmission);
        state.intent = EmissionIntent::Normal;
        assert!(state.validate(&ship).is_ok());
        state.owner = SystemId("another-command".into());
        assert!(state.validate(&ship).is_err());
        assert!(profile(&ShipConfig {
            systems: vec![],
            ..ship
        })
        .unwrap()
        .is_none());
    }
    #[test]
    fn wrong_kind_missing_console_duplicate_and_open_config_are_refused() {
        let mut wrong = ship();
        wrong.systems[0].kind = "comms".into();
        assert!(profile(&wrong).is_err());
        let mut wrong = ship();
        wrong.systems[0].console = None;
        assert!(profile(&wrong).is_err());
        let mut wrong = ship();
        let mut extra = wrong.systems[0].clone();
        extra.id = SystemId("another-command".into());
        wrong.systems.push(extra);
        assert!(profile(&wrong).is_err());
        let mut wrong = ship();
        wrong.systems[0].config.as_mut().unwrap()["emission_profile"]
            .as_table_mut()
            .unwrap()
            .insert("ceiling".into(), toml::Value::Integer(1));
        assert!(profile(&wrong).is_err());
    }
}
