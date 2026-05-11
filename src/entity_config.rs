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
}