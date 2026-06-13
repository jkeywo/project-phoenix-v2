use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a single complexity preset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComplexityPreset {
    pub name: String,
    #[serde(default)]
    pub hidden_elements: Vec<String>,
    #[serde(default)]
    pub delegated: HashMap<String, DelegatedConfig>,
    #[serde(default)]
    pub ai: HashMap<String, AiBehaviorConfig>,
}

/// Controls delegated to a receiver console under a complexity preset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegatedConfig {
    /// List of control IDs delegated to this receiver console.
    #[serde(default)]
    pub controls: Vec<String>,
}

/// Tuning parameters for an AI behavior under a complexity preset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiBehaviorConfig {
    /// Additional key-value tuning parameters.
    #[serde(flatten, default)]
    pub params: HashMap<String, toml::Value>,
}

/// Complete complexity configuration loaded from a TOML file.
/// When `presets` has exactly one entry, the console uses a single implicit preset
/// (backward compatible with consoles that have no complexity reference).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComplexityConfig {
    #[serde(rename = "preset")]
    pub presets: Vec<ComplexityPreset>,
}

impl ComplexityConfig {
    /// Look up a preset by name.
    pub fn get_preset(&self, name: &str) -> Option<&ComplexityPreset> {
        self.presets.iter().find(|p| p.name == name)
    }

    /// Returns `true` when the config offers more than one preset choice.
    pub fn has_multiple_presets(&self) -> bool {
        self.presets.len() > 1
    }
}

/// Default is a single implicit "Std" preset (backward compatible).
impl Default for ComplexityConfig {
    fn default() -> Self {
        Self {
            presets: vec![ComplexityPreset {
                name: "Std".into(),
                hidden_elements: vec![],
                delegated: HashMap::new(),
                ai: HashMap::new(),
            }],
        }
    }
}

/// Parse a TOML string into a `ComplexityConfig`.
pub fn parse_complexity_config(toml_str: &str) -> Result<ComplexityConfig, String> {
    toml::from_str::<ComplexityConfig>(toml_str).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_two_presets_and_lookup_by_name() {
        let toml = r##"
[[preset]]
name = "Low"
hidden_elements = ["phaser_mode"]

[preset.delegated]
Tactical = { controls = ["auto_fire"] }

[preset.ai]
torpedo_aim = {}

[[preset]]
name = "Std"
"##;
        let config = parse_complexity_config(toml).expect("parse should succeed");
        assert!(config.has_multiple_presets());

        let low = config.get_preset("Low").expect("Low preset should exist");
        assert_eq!(low.name, "Low");

        let full = config.get_preset("Std").expect("Full preset should exist");
        assert_eq!(full.name, "Std");

        assert!(config.get_preset("NonExistent").is_none());
    }

    #[test]
    fn single_preset_returns_false_for_has_multiple() {
        let config = ComplexityConfig::default();
        assert!(!config.has_multiple_presets());
    }

    #[test]
    fn default_is_single_full_preset() {
        let config = ComplexityConfig::default();
        assert_eq!(config.presets.len(), 1);
        let preset = &config.presets[0];
        assert_eq!(preset.name, "Std");
        assert!(preset.hidden_elements.is_empty());
        assert!(preset.delegated.is_empty());
        assert!(preset.ai.is_empty());
    }

    #[test]
    fn missing_optional_fields_default_sanely() {
        let toml = r##"
[[preset]]
name = "Minimal"
"##;
        let config = parse_complexity_config(toml).expect("parse should succeed");
        assert_eq!(config.presets.len(), 1);
        let preset = &config.presets[0];
        assert_eq!(preset.name, "Minimal");
        assert!(preset.hidden_elements.is_empty());
        assert!(preset.delegated.is_empty());
        assert!(preset.ai.is_empty());
    }

    #[test]
    fn full_preset_parses_hidden_elements() {
        let toml = r##"
[[preset]]
name = "Low"
hidden_elements = ["phaser_mode_selector", "torpedo_tube_selector", "fire_confirm"]
"##;
        let config = parse_complexity_config(toml).expect("parse should succeed");
        let low = config.get_preset("Low").expect("Low preset");
        assert_eq!(
            low.hidden_elements,
            vec![
                "phaser_mode_selector",
                "torpedo_tube_selector",
                "fire_confirm"
            ]
        );
    }

    #[test]
    fn empty_hidden_elements_defaults_to_empty_vec() {
        let toml = r##"
[[preset]]
name = "Std"
"##;
        let config = parse_complexity_config(toml).expect("parse should succeed");
        let preset = config.get_preset("Std").unwrap();
        assert!(preset.hidden_elements.is_empty());
    }

    #[test]
    fn delegated_parses_receiver_console_and_controls() {
        let toml = r##"
[[preset]]
name = "Low"

[preset.delegated]
Tactical = { controls = ["auto_fire_torpedoes", "auto_frequency_match"] }
Helm = { controls = ["auto_steering"] }
"##;
        let config = parse_complexity_config(toml).expect("parse should succeed");
        let low = config.get_preset("Low").expect("Low preset");
        assert_eq!(low.delegated.len(), 2);

        let tactical = low.delegated.get("Tactical").expect("Tactical delegation");
        assert_eq!(
            tactical.controls,
            vec!["auto_fire_torpedoes", "auto_frequency_match"]
        );

        let helm = low.delegated.get("Helm").expect("Helm delegation");
        assert_eq!(helm.controls, vec!["auto_steering"]);
    }

    #[test]
    fn ai_parses_behavior_names_and_tuning_params() {
        let toml = r##"
[[preset]]
name = "Low"

[preset.ai]
torpedo_auto_fire = { lead_prediction = true, min_accuracy = 0.7 }
frequency_match = { sweep_interval_secs = 2.0 }
"##;
        let config = parse_complexity_config(toml).expect("parse should succeed");
        let low = config.get_preset("Low").expect("Low preset");
        assert_eq!(low.ai.len(), 2);

        let torpedo = low.ai.get("torpedo_auto_fire").expect("torpedo_auto_fire");
        assert_eq!(
            torpedo
                .params
                .get("lead_prediction")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            torpedo
                .params
                .get("min_accuracy")
                .and_then(|v| v.as_float()),
            Some(0.7)
        );

        let freq = low.ai.get("frequency_match").expect("frequency_match");
        assert_eq!(
            freq.params
                .get("sweep_interval_secs")
                .and_then(|v| v.as_float()),
            Some(2.0)
        );
    }

    #[test]
    fn ai_behavior_without_params_defaults_to_empty_map() {
        let toml = r##"
[[preset]]
name = "Low"

[preset.ai]
bare_behavior = {}
"##;
        let config = parse_complexity_config(toml).expect("parse should succeed");
        let low = config.get_preset("Low").expect("Low preset");
        let bare = low.ai.get("bare_behavior").expect("bare_behavior");
        assert!(bare.params.is_empty());
    }

    #[test]
    fn invalid_toml_returns_error() {
        let toml = r##"
[[preset
name = "Bad"
"##;
        let result = parse_complexity_config(toml);
        assert!(result.is_err());
    }

    #[test]
    fn shields_toml_has_only_std_preset() {
        let toml = include_str!("../../assets/complexity/shields.toml");
        let config = parse_complexity_config(toml).expect("shields.toml should parse");
        assert_eq!(
            config.presets.len(),
            1,
            "Shields should have exactly one preset"
        );
        assert_eq!(
            config.presets[0].name, "Std",
            "Shields preset should be 'Std'"
        );
        assert!(
            !config.has_multiple_presets(),
            "Shields should not have multiple presets"
        );
    }

    #[test]
    fn navigation_toml_has_only_std_preset() {
        let toml = include_str!("../../assets/complexity/navigation.toml");
        let config = parse_complexity_config(toml).expect("navigation.toml should parse");
        assert_eq!(
            config.presets.len(),
            1,
            "Navigation should have exactly one preset"
        );
        assert_eq!(
            config.presets[0].name, "Std",
            "Navigation preset should be 'Std'"
        );
        assert!(
            !config.has_multiple_presets(),
            "Navigation should not have multiple presets"
        );
    }

    /// Sensors carries the auto-hint AI rule on its Low preset (merged from
    /// the retired science.toml). NOTE: the JS client does not yet offer a
    /// Low preset for Sensors (`gui/complexity-store.js defaultPresetsFor`),
    /// so the Low preset is currently selectable only programmatically.
    #[test]
    fn sensors_toml_low_preset_has_auto_hint() {
        let toml = include_str!("../../assets/complexity/sensors.toml");
        let config = parse_complexity_config(toml).expect("sensors.toml should parse");
        let low = config
            .get_preset("Low")
            .expect("Sensors should have a Low preset");
        assert!(
            low.ai.contains_key("auto_hint"),
            "Sensors Low preset should enable the auto_hint AI rule"
        );
        assert!(
            config.get_preset("Std").is_some(),
            "Sensors should have a Std preset"
        );
    }
}
