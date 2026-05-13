use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HullConfig {
    pub hull_integrity: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum ColliderShape {
    Ball,
    Capsule,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ColliderConfig {
    pub shape: ColliderShape,
    pub radius: f32,
    pub length: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AppearanceConfig {
    pub colour: String,
    pub size_min: f32,
    pub size_max: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HelmConsoleConfig {
    #[serde(default)]
    pub max_speed: f32,
    #[serde(default)]
    pub max_reverse_speed: f32,
    #[serde(default)]
    pub acceleration: f32,
    #[serde(default)]
    pub deceleration: f32,
    #[serde(default)]
    pub max_yaw_rate: f32,
    #[serde(default)]
    pub radar_range: f32,
    #[serde(default)]
    pub radar_shows: bool,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WeaponsConsoleConfig {
    #[serde(default)]
    pub radar_range: f32,
    #[serde(default)]
    pub target_range: f32,
    #[serde(default)]
    pub fire_arc: f32,
    #[serde(default)]
    pub beam_range: f32,
    #[serde(default)]
    pub beam_damage_per_sec: f32,
    #[serde(default)]
    pub beam_duration_secs: f32,
    #[serde(default)]
    pub cooldown_secs: f32,
    /// RGBA beam colour as a 4-element float array `[r, g, b, a]` in 0.0–1.0.
    /// When absent (empty vec), the renderer falls back to `beam_render::DEFAULT_BEAM_COLOR`.
    #[serde(default)]
    pub beam_color: Vec<f32>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EngineeringConsoleConfig {
    #[serde(default)]
    pub repair_rate: f32,
    #[serde(default)]
    pub repair_hp_per_cycle: i32,
    #[serde(default)]
    pub repair_cooldown_secs: f32,
    #[serde(default)]
    pub cooldown_secs: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptainConsoleConfig {}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PowerConfigSection {
    pub capacity: f32,
    pub rates: [f32; 6],
    pub emergency_threshold: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ScienceConsoleConfig {
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityConfig {
    pub tags: Vec<String>,
    pub hull: Option<HullConfig>,
    pub collider: Option<ColliderConfig>,
    pub appearance: Option<AppearanceConfig>,
    pub helm_console: Option<HelmConsoleConfig>,
    pub weapons_console: Option<WeaponsConsoleConfig>,
    pub engineering_console: Option<EngineeringConsoleConfig>,
    pub captain_console: Option<CaptainConsoleConfig>,
    pub power: Option<PowerConfigSection>,
    pub science_console: Option<ScienceConsoleConfig>,
}

#[derive(Deserialize)]
struct TomlConfig {
    #[serde(default)]
    tags: Vec<String>,
    hull: Option<HullConfig>,
    collider: Option<ColliderConfig>,
    appearance: Option<AppearanceConfig>,
    helm_console: Option<HelmConsoleConfig>,
    weapons_console: Option<WeaponsConsoleConfig>,
    engineering_console: Option<EngineeringConsoleConfig>,
    #[serde(default)]
    captain_console: Option<serde::de::IgnoredAny>,
    power: Option<PowerConfigSection>,
    science_console: Option<ScienceConsoleConfig>,
}

impl EntityConfig {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        let raw: TomlConfig = toml::from_str(s)?;
        let has_captain = raw.captain_console.is_some();
        Ok(EntityConfig {
            tags: raw.tags,
            hull: raw.hull,
            collider: raw.collider,
            appearance: raw.appearance,
            helm_console: raw.helm_console,
            weapons_console: raw.weapons_console,
            engineering_console: raw.engineering_console,
            captain_console: if has_captain {
                Some(CaptainConsoleConfig {})
            } else {
                None
            },
            power: raw.power,
            science_console: raw.science_console,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_sections_present_deserializes_to_some() {
        let toml_str = r##"
tags = ["gameplay", "combat", "primary"]

[hull]
hull_integrity = 100

[collider]
shape = "Ball"
radius = 2.0
length = 0.0

[appearance]
colour = "#ff0000"
size_min = 1.0
size_max = 3.0

[helm_console]
max_speed = 50.0
max_reverse_speed = 25.0
acceleration = 16.7
deceleration = 50.0
max_yaw_rate = 0.785
radar_range = 50.0
radar_shows = true

[weapons_console]
radar_range = 60.0
target_range = 60.0
fire_arc = 3.14159
beam_range = 40.0
beam_damage_per_sec = 5.0
beam_duration_secs = 6.0
cooldown_secs = 6.0

[engineering_console]
repair_rate = 0.33
repair_hp_per_cycle = 1
repair_cooldown_secs = 30.0
penalty_cooldown_secs = 10.0

[captain_console]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");

        assert_eq!(
            config.tags,
            vec![
                "gameplay".to_string(),
                "combat".to_string(),
                "primary".to_string()
            ]
        );

        assert!(config.hull.is_some());
        assert_eq!(config.hull.as_ref().unwrap().hull_integrity, 100);

        assert!(config.collider.is_some());
        let c = config.collider.as_ref().unwrap();
        assert_eq!(c.shape, ColliderShape::Ball);
        assert_eq!(c.radius, 2.0);

        assert!(config.appearance.is_some());
        assert_eq!(config.appearance.as_ref().unwrap().colour, "#ff0000");

        assert!(config.helm_console.is_some());
        let h = config.helm_console.as_ref().unwrap();
        assert_eq!(h.max_speed, 50.0);
        assert_eq!(h.radar_range, 50.0);
        assert!(h.radar_shows);

        assert!(config.weapons_console.is_some());
        let w = config.weapons_console.as_ref().unwrap();
        assert_eq!(w.beam_range, 40.0);
        assert_eq!(w.cooldown_secs, 6.0);

        assert!(config.engineering_console.is_some());
        let e = config.engineering_console.as_ref().unwrap();
        assert_eq!(e.repair_cooldown_secs, 30.0);

        assert!(config.captain_console.is_some());
    }

    #[test]
    fn only_hull_and_tags_produces_none_for_console_fields() {
        let toml_str = r##"
tags = ["gameplay", "asteroid"]

[hull]
hull_integrity = 80
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");

        assert_eq!(
            config.tags,
            vec!["gameplay".to_string(), "asteroid".to_string()]
        );
        assert!(config.hull.is_some());
        assert_eq!(config.hull.as_ref().unwrap().hull_integrity, 80);
        assert!(config.collider.is_none());
        assert!(config.appearance.is_none());
        assert!(config.helm_console.is_none());
        assert!(config.weapons_console.is_none());
        assert!(config.engineering_console.is_none());
        assert!(config.captain_console.is_none());
    }

    #[test]
    fn captain_console_with_no_fields_deserializes_to_some() {
        let toml_str = "[captain_console]\n";
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.captain_console.is_some());
        let c = config.captain_console.as_ref().unwrap();
        assert_eq!(c, &CaptainConsoleConfig {});
    }

    #[test]
    fn malformed_field_returns_error() {
        let toml_str = r##"
[hull]
hull_integrity = "not_an_integer"
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn tags_field_deserializes_to_vec_string() {
        let toml_str = r##"
tags = ["foo", "bar", "baz", "quux"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(
            config.tags,
            vec![
                "foo".to_string(),
                "bar".to_string(),
                "baz".to_string(),
                "quux".to_string()
            ]
        );
    }

    #[test]
    fn collider_capsule_shape_round_trips() {
        let toml_str = r##"
[collider]
shape = "Capsule"
radius = 1.5
length = 6.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(
            config.collider.as_ref().unwrap().shape,
            ColliderShape::Capsule
        );
    }

    #[test]
    fn empty_toml_string_produces_all_none() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.tags.is_empty());
        assert!(config.hull.is_none());
        assert!(config.collider.is_none());
        assert!(config.appearance.is_none());
        assert!(config.helm_console.is_none());
        assert!(config.weapons_console.is_none());
        assert!(config.engineering_console.is_none());
        assert!(config.captain_console.is_none());
    }

    #[test]
    fn helm_console_partial_fields_work() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
radar_shows = false
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(h.max_speed, 30.0);
        assert!(!h.radar_shows);
        assert_eq!(h.max_reverse_speed, 0.0);
    }

    #[test]
    fn weapons_console_beam_color_parses_rgba() {
        let toml_str = r##"
[weapons_console]
beam_color = [1.0, 0.5, 0.2, 0.9]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config.weapons_console.expect("weapons_console must be Some");
        assert_eq!(w.beam_color, vec![1.0, 0.5, 0.2, 0.9]);
    }

    #[test]
    fn weapons_console_beam_color_defaults_to_empty_when_omitted() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config.weapons_console.expect("weapons_console must be Some");
        assert!(w.beam_color.is_empty(), "beam_color should default to empty vec when omitted");
    }

    // ── Power section tests ────────────────────────────────────────────────

    #[test]
    fn power_section_parses_capacity_rates_emergency_threshold() {
        let toml_str = r##"
[power]
capacity = 150.0
rates = [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]
emergency_threshold = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let p = config.power.expect("power must be Some");
        assert!((p.capacity - 150.0).abs() < 0.001);
        assert_eq!(p.rates, [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]);
        assert!((p.emergency_threshold - 30.0).abs() < 0.001);
    }

    #[test]
    fn power_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.power.is_none(), "power should be None when not specified");
    }

    #[test]
    fn science_console_parses_with_power_multipliers() {
        let toml_str = r##"
[science_console]
power_multipliers = [-1.0, 0.0, 1.0, 2.0]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let s = config.science_console.expect("science_console must be Some");
        assert_eq!(s.power_multipliers, Some([-1.0, 0.0, 1.0, 2.0]));
    }

    #[test]
    fn science_console_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.science_console.is_none());
    }

    #[test]
    fn helm_console_power_multipliers_parses() {
        let toml_str = r##"
[helm_console]
power_multipliers = [-0.8, 0.0, 0.4, 0.8]
max_speed = 50.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(h.power_multipliers, Some([-0.8, 0.0, 0.4, 0.8]));
    }

    #[test]
    fn weapons_console_power_multipliers_parses() {
        let toml_str = r##"
[weapons_console]
power_multipliers = [-0.3, 0.0, 0.15, 0.3]
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config.weapons_console.expect("weapons_console must be Some");
        assert_eq!(w.power_multipliers, Some([-0.3, 0.0, 0.15, 0.3]));
    }

    #[test]
    fn power_multipliers_defaults_to_none_when_omitted() {
        let toml_str = r##"
[helm_console]
max_speed = 30.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert!(h.power_multipliers.is_none());
    }
}