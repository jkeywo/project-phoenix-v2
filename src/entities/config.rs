use serde::{Deserialize, Serialize};
use serde::de::Error as SerdeError;
use uuid::Uuid;
use crate::map_config::{StarConfig, PlanetConfig, AsteroidFieldConfig};
use crate::region_shape::RegionShape;
use crate::region_effects::RegionEffectsConfig;

/// Configuration for a single named AI state.
///
/// Each entry in a `[[behaviour.state]]` array defines the parameters
/// for one state. The `name` field is used as a stable identifier for
/// per-spawn `[spawn.overrides]` by-name replacement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StateConfig {
    /// Stable name for this state (used in `initial_state` and overrides).
    pub name: String,
    /// State kind: `"idle"`, `"patrolling"`, `"pursuing"`, or `"attacking"`.
    #[serde(default)]
    pub kind: String,
    /// Ordered waypoint anchor names (used by `patrolling`).
    #[serde(default)]
    pub waypoints: Vec<String>,
    /// Whether to loop back to the first waypoint after the last (patrolling).
    #[serde(default)]
    pub loop_path: bool,
    /// Desired forward speed fraction [0, 1], clamped at load time.
    #[serde(default)]
    pub target_speed: f32,
    /// Distance to maintain from target (world units) for the `attacking` state.
    /// The AI thrusts at `target_speed` when further than this, and holds station
    /// (thrust = 0) when closer.
    #[serde(default)]
    pub maintain_range: f32,
    /// Duration in seconds for the `warping_out` state before the entity self-despawns.
    #[serde(default)]
    pub duration_secs: f32,
}

impl StateConfig {
    /// Clamp mutable fields into valid ranges after deserialisation.
    fn clamp(&mut self) {
        self.target_speed = self.target_speed.clamp(0.0, 1.0);
    }
}

/// Configuration for an AI behaviour controller attached to an entity.
/// Re-exports the AI module's config type so callers only need `entity_config`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehaviourConfig {
    /// Name of the initial AI state (e.g. `"idle"`).
    pub initial_state: String,
    /// Typed state parameter blocks.  An empty vec is valid (only `initial_state`
    /// is required; states with no extra params — like `idle` — need no entry).
    #[serde(default)]
    pub state: Vec<StateConfig>,
    /// Transition rules evaluated in declaration order.
    #[serde(default)]
    pub transition: Vec<crate::ai::TransitionConfig>,
}

/// Visual/render shape for station entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StationShape {
    Sphere,
    Cylinder,
    Torus,
}

/// Configuration for a station entity (space station, outpost, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StationConfig {
    pub name: String,
    pub shape: StationShape,
    pub radius: f32,
    pub hull_integrity: f32,
}

/// One entry in the `[[hull.console_hull]]` TOML array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsoleHullEntry {
    /// Console name matching the `Console` enum variant (e.g. `"Helm"`).
    pub console: crate::messages::Console,
    /// Maximum (and starting) HP for this console.
    pub max_hp: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HullConfig {
    /// Legacy single-value hull integrity (kept for backward compat with NPC configs).
    #[serde(default)]
    pub hull_integrity: f32,
    /// Per-console hull entries. When present, replaces the single `hull_integrity` value.
    #[serde(default)]
    pub console_hull: Vec<ConsoleHullEntry>,
    /// Number of repair teams available to this ship (default 0 = legacy).
    #[serde(default)]
    pub repair_team_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Ball,
    Capsule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColliderConfig {
    pub shape: ColliderShape,
    pub radius: f32,
    pub length: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppearanceConfig {
    pub colour: String,
    pub size_min: f32,
    pub size_max: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
    /// Total time in seconds to fully charge the impulse drive.
    /// Defaults to `IMPULSE_CHARGE_DURATION` (3.0 s) when absent.
    #[serde(default = "default_impulse_charge_duration")]
    pub impulse_charge_duration: f32,
    /// Speed multiplier applied when impulse drive is active.
    /// Defaults to `IMPULSE_SPEED_MULTIPLIER` (10.0) when absent.
    #[serde(default = "default_impulse_speed_multiplier")]
    pub impulse_speed_multiplier: f32,
}

fn default_impulse_charge_duration() -> f32 {
    crate::impulse::IMPULSE_CHARGE_DURATION
}

fn default_impulse_speed_multiplier() -> f32 {
    crate::impulse::IMPULSE_SPEED_MULTIPLIER
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineeringConsoleConfig {
    #[serde(default)]
    pub repair_rate: f32,
    #[serde(default)]
    pub repair_hp_per_cycle: i32,
    #[serde(default)]
    pub repair_cooldown_secs: f32,
    #[serde(default)]
    pub cooldown_secs: f32,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptainConsoleConfig {
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerConfigSection {
    pub capacity: f32,
    pub rates: [f32; 6],
    pub emergency_threshold: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScienceConsoleConfig {
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    #[serde(default)]
    pub long_range_radar: crate::radar_config::RadarConfig,
    #[serde(default)]
    pub system_map: crate::radar_config::RadarConfig,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

/// Config block for the Shields console focus bonuses/penalties.
///
/// Loaded from `[shields_console]` in `player_ship.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShieldsConsoleConfig {
    /// Extra max HP applied to the focused facing.
    #[serde(default = "default_focus_bonus_max_hp")]
    pub focus_bonus_max_hp: i32,
    /// Extra regen per second applied to the focused facing.
    #[serde(default = "default_focus_bonus_regen")]
    pub focus_bonus_regen: f32,
    /// Max HP subtracted from each non-focused facing.
    #[serde(default = "default_focus_penalty_max_hp")]
    pub focus_penalty_max_hp: i32,
    /// Regen per second subtracted from each non-focused facing.
    #[serde(default = "default_focus_penalty_regen")]
    pub focus_penalty_regen: f32,
    /// HP per second decay applied to non-focused facings when above reduced max.
    #[serde(default = "default_focus_decay_rate")]
    pub focus_decay_rate: f32,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

fn default_focus_bonus_max_hp() -> i32 { 50 }
fn default_focus_bonus_regen() -> f32 { 5.0 }
fn default_focus_penalty_max_hp() -> i32 { 25 }
fn default_focus_penalty_regen() -> f32 { 2.5 }
fn default_focus_decay_rate() -> f32 { 10.0 }

impl Default for ShieldsConsoleConfig {
    fn default() -> Self {
        Self {
            focus_bonus_max_hp: 50,
            focus_bonus_regen: 5.0,
            focus_penalty_max_hp: 25,
            focus_penalty_regen: 2.5,
            focus_decay_rate: 10.0,
            complexity_toml: None,
        }
    }
}

/// Config block for the Sensors console in a ship TOML.
///
/// Loaded from `[sensors_console]` in `player_ship.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorsConsoleConfig {
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Long-range radar config for the Sensors console.
    #[serde(default)]
    pub long_range_radar: crate::radar_config::RadarConfig,
    /// Path to a complexity TOML file for this console.
    #[serde(default)]
    pub complexity_toml: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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
    pub sensors_console: Option<SensorsConsoleConfig>,
    /// Shields console focus config.
    pub shields_console: Option<ShieldsConsoleConfig>,
    /// Star section from entity template (name, radius, colour, etc.)
    pub star: Option<StarConfig>,
    /// Planet section from entity template (name, radius, colour, etc.)
    pub planet: Option<PlanetConfig>,
    /// Asteroid field section from entity template (donut params, grid, etc.)
    pub asteroid_field: Option<AsteroidFieldConfig>,
    /// Region shape section — present for region entities.
    pub shape: Option<RegionShape>,
    /// Region effects section — present for region entities with effects.
    pub effects: Option<RegionEffectsConfig>,
    /// Station section — present for station entities.
    pub station: Option<StationConfig>,
    /// Optional faction UUID this entity belongs to.
    #[serde(default)]
    pub faction: Option<Uuid>,
    /// Optional AI behaviour controller config.
    #[serde(default)]
    pub behaviour: Option<BehaviourConfig>,
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
    captain_console: Option<CaptainConsoleConfig>,
    power: Option<PowerConfigSection>,
    science_console: Option<ScienceConsoleConfig>,
    sensors_console: Option<SensorsConsoleConfig>,
    shields_console: Option<ShieldsConsoleConfig>,
    star: Option<StarConfig>,
    planet: Option<PlanetConfig>,
    asteroid_field: Option<AsteroidFieldConfig>,
    shape: Option<RegionShape>,
    effects: Option<RegionEffectsConfig>,
    station: Option<StationConfig>,
    faction: Option<Uuid>,
    behaviour: Option<BehaviourConfig>,
}

impl EntityConfig {
    /// Collect all `complexity_toml` paths referenced by any console config.
    pub fn complexity_toml_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        if let Some(ref c) = self.helm_console {
            if let Some(ref p) = c.complexity_toml {
                paths.push(p.clone());
            }
        }
        if let Some(ref c) = self.weapons_console {
            if let Some(ref p) = c.complexity_toml {
                paths.push(p.clone());
            }
        }
        if let Some(ref c) = self.engineering_console {
            if let Some(ref p) = c.complexity_toml {
                paths.push(p.clone());
            }
        }
        if let Some(ref c) = self.captain_console {
            if let Some(ref p) = c.complexity_toml {
                paths.push(p.clone());
            }
        }
        if let Some(ref c) = self.science_console {
            if let Some(ref p) = c.complexity_toml {
                paths.push(p.clone());
            }
        }
        if let Some(ref c) = self.sensors_console {
            if let Some(ref p) = c.complexity_toml {
                paths.push(p.clone());
            }
        }
        if let Some(ref c) = self.shields_console {
            if let Some(ref p) = c.complexity_toml {
                paths.push(p.clone());
            }
        }
        paths
    }

    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        let raw: TomlConfig = toml::from_str(s)?;

        // Validation: region entity with effects but no shape is an error.
        if let Some(ref effects) = raw.effects {
            if !effects.is_empty() && raw.shape.is_none() {
                return Err(SerdeError::custom(
                    "region entity has effects but no [shape] section",
                ));
            }
        }

        // Clamp target_speed in every StateConfig entry.
        let behaviour = raw.behaviour.map(|mut b| {
            for s in &mut b.state {
                s.clamp();
            }
            b
        });

        Ok(EntityConfig {
            tags: raw.tags,
            hull: raw.hull,
            collider: raw.collider,
            appearance: raw.appearance,
            helm_console: raw.helm_console,
            weapons_console: raw.weapons_console,
            engineering_console: raw.engineering_console,
            captain_console: raw.captain_console,
            power: raw.power,
            science_console: raw.science_console,
            shields_console: raw.shields_console,
            sensors_console: raw.sensors_console,
            star: raw.star,
            planet: raw.planet,
            asteroid_field: raw.asteroid_field,
            shape: raw.shape,
            effects: raw.effects,
            station: raw.station,
            faction: raw.faction,
            behaviour,
        })
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_tags::EntityTag;

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
        assert!((config.hull.as_ref().unwrap().hull_integrity - 100.0).abs() < 1e-6);

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
        assert!((config.hull.as_ref().unwrap().hull_integrity - 80.0).abs() < 1e-6);
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
        assert_eq!(c, &CaptainConsoleConfig { complexity_toml: None });
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
    fn science_console_with_radar_configs_parses_long_range_and_system_map() {
        let toml_str = r##"
tags = ["player", "ship"]

[science_console]
power_multipliers = [-0.5, 0.0, 0.25, 0.5]

[science_console.long_range_radar]
range = 200.0
shows = ["region", "asteroid_field", "asteroid", "ship"]

[science_console.system_map]
range = 500.0
shows = ["region", "asteroid_field", "star", "planet"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let science = config.science_console.expect("science_console must be Some");
        assert_eq!(science.power_multipliers, Some([-0.5, 0.0, 0.25, 0.5]));
        assert_eq!(science.long_range_radar.range, 200.0);
        assert!(science.long_range_radar.shows.contains(&EntityTag::Region));
        assert!(science.long_range_radar.shows.contains(&EntityTag::AsteroidField));
        assert!(science.long_range_radar.shows.contains(&EntityTag::Asteroid));
        assert_eq!(science.system_map.range, 500.0);
        assert!(science.system_map.shows.contains(&EntityTag::Region));
        assert!(science.system_map.shows.contains(&EntityTag::AsteroidField));
    }

    #[test]
    fn sensors_console_parses_with_long_range_radar() {
        let toml_str = r##"
tags = ["player", "ship"]

[sensors_console]
power_multipliers = [-0.5, 0.0, 0.25, 0.5]

[sensors_console.long_range_radar]
range = 200.0
shows = ["region", "asteroid_field", "asteroid", "ship"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let sensors = config.sensors_console.expect("sensors_console must be Some");
        assert_eq!(sensors.power_multipliers, Some([-0.5, 0.0, 0.25, 0.5]));
        assert_eq!(sensors.long_range_radar.range, 200.0);
        assert!(sensors.long_range_radar.shows.contains(&EntityTag::Region));
        assert!(sensors.long_range_radar.shows.contains(&EntityTag::AsteroidField));
        assert!(sensors.long_range_radar.shows.contains(&EntityTag::Asteroid));
    }

    #[test]
    fn sensors_console_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.sensors_console.is_none());
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

    // ── Complexity TOML reference tests ────────────────────────────────────

    #[test]
    fn weapons_console_complexity_toml_parses() {
        let toml_str = r##"
[weapons_console]
complexity_toml = "assets/complexity/tactical.toml"
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config.weapons_console.expect("weapons_console must be Some");
        assert_eq!(
            w.complexity_toml.as_deref(),
            Some("assets/complexity/tactical.toml")
        );
    }

    #[test]
    fn helm_console_complexity_toml_parses() {
        let toml_str = r##"
[helm_console]
complexity_toml = "assets/complexity/helm.toml"
max_speed = 50.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let h = config.helm_console.expect("helm_console must be Some");
        assert_eq!(
            h.complexity_toml.as_deref(),
            Some("assets/complexity/helm.toml")
        );
    }

    #[test]
    fn engineering_console_complexity_toml_parses() {
        let toml_str = r##"
[engineering_console]
complexity_toml = "assets/complexity/repair.toml"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let e = config.engineering_console.expect("engineering_console must be Some");
        assert_eq!(
            e.complexity_toml.as_deref(),
            Some("assets/complexity/repair.toml")
        );
    }

    #[test]
    fn captain_console_complexity_toml_parses() {
        let toml_str = r##"
[captain_console]
complexity_toml = "assets/complexity/captain.toml"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let c = config.captain_console.expect("captain_console must be Some");
        assert_eq!(
            c.complexity_toml.as_deref(),
            Some("assets/complexity/captain.toml")
        );
    }

    #[test]
    fn science_console_complexity_toml_parses() {
        let toml_str = r##"
[science_console]
complexity_toml = "assets/complexity/science.toml"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let s = config.science_console.expect("science_console must be Some");
        assert_eq!(
            s.complexity_toml.as_deref(),
            Some("assets/complexity/science.toml")
        );
    }

    #[test]
    fn complexity_toml_defaults_to_none_when_omitted() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.weapons_console.is_none());
    }

    #[test]
    fn weapons_console_without_complexity_toml_defaults_to_none() {
        let toml_str = r##"
[weapons_console]
beam_range = 40.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let w = config.weapons_console.expect("weapons_console must be Some");
        assert!(w.complexity_toml.is_none());
    }

    #[test]
    fn complexity_toml_paths_returns_multiple_when_several_consoles_referenced() {
        let toml_str = r##"
[helm_console]
complexity_toml = "assets/complexity/helm.toml"
[weapons_console]
complexity_toml = "assets/complexity/tactical.toml"
[engineering_console]
complexity_toml = "assets/complexity/repair.toml"
[captain_console]
complexity_toml = "assets/complexity/captain.toml"
[science_console]
complexity_toml = "assets/complexity/science.toml"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let paths = config.complexity_toml_paths();
        assert_eq!(paths.len(), 5);
        assert!(paths.contains(&"assets/complexity/helm.toml".to_string()));
        assert!(paths.contains(&"assets/complexity/tactical.toml".to_string()));
        assert!(paths.contains(&"assets/complexity/repair.toml".to_string()));
        assert!(paths.contains(&"assets/complexity/captain.toml".to_string()));
        assert!(paths.contains(&"assets/complexity/science.toml".to_string()));
    }

    #[test]
    fn complexity_toml_paths_returns_empty_when_no_complexity_refs() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.complexity_toml_paths().is_empty());
    }

    // ── Star / Planet / AsteroidField section tests ────────────────────────

    #[test]
    fn star_section_parses_from_template() {
        let toml_str = r##"
tags = ["star", "center"]

[star]
name = "Sun"
radius = 50.0
colour = [1.0, 0.8, 0.0]
position = [0.0, 0.0, 0.0]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.tags, vec!["star", "center"]);
        let star = config.star.expect("star must be Some");
        assert_eq!(star.name, "Sun");
        assert!((star.radius - 50.0).abs() < 1e-6);
        assert_eq!(star.colour, vec![1.0, 0.8, 0.0]);
    }

    #[test]
    fn planet_section_parses_from_template() {
        let toml_str = r##"
tags = ["planet", "habitable"]

[planet]
name = "Earth"
radius = 20.0
colour = [0.0, 0.5, 1.0]
position = [200.0, 0.0, 200.0]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert_eq!(config.tags, vec!["planet", "habitable"]);
        let planet = config.planet.expect("planet must be Some");
        assert_eq!(planet.name, "Earth");
        assert!((planet.radius - 20.0).abs() < 1e-6);
        assert_eq!(planet.colour, vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn asteroid_field_section_parses_from_template() {
        let toml_str = r##"
tags = ["field", "main"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0
asteroid_type_paths = ["assets/entities/asteroid_small.toml", "assets/entities/asteroid_large.toml"]
cosmetic_type_paths = ["assets/entities/asteroid_cosmetic.toml"]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let field = config.asteroid_field.expect("asteroid_field must be Some");
        assert!((field.inner_radius - 100.0).abs() < 1e-6);
        assert_eq!(field.asteroid_type_paths.len(), 2);
        assert_eq!(field.cosmetic_type_paths.len(), 1);
    }

    #[test]
    fn star_section_parses_with_light_config() {
        let toml_str = r##"
[star]
name = "Sun"
radius = 50.0
colour = [1.0, 0.8, 0.0]
light_range = 5000.0
light_intensity = 150000.0
light_colour = [1.0, 0.95, 0.85]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let star = config.star.expect("star must be Some");
        assert_eq!(star.name, "Sun");
        assert!((star.radius - 50.0).abs() < 1e-6);
        assert_eq!(star.light_range, Some(5000.0));
        assert_eq!(star.light_intensity, Some(150000.0));
        assert_eq!(star.light_colour, Some(vec![1.0, 0.95, 0.85]));
    }

    #[test]
    fn star_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.star.is_none());
    }

    #[test]
    fn planet_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.planet.is_none());
    }

    #[test]
    fn asteroid_field_section_omitted_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.asteroid_field.is_none());
    }

    // ── Region shape tests ───────────────────────────────────────────────

    #[test]
    fn region_shape_sphere_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "sphere"
radius = 100.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(shape, crate::region_shape::RegionShape::Sphere { radius: 100.0 });
    }

    #[test]
    fn region_shape_box_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "box"
half_extents = [50.0, 30.0, 40.0]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(shape, crate::region_shape::RegionShape::Box { half_extents: [50.0, 30.0, 40.0], yaw: 0.0 });
    }

    #[test]
    fn region_shape_torus_parses_from_toml() {
        let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "torus"
inner_radius = 50.0
outer_radius = 80.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let shape = config.shape.expect("shape must be Some");
        assert_eq!(shape, crate::region_shape::RegionShape::Torus { inner_radius: 50.0, outer_radius: 80.0 });
    }

    #[test]
    fn region_shape_parses_with_effects() {
        let toml_str = r##"
tags = ["region", "nebula"]

[shape]
type = "sphere"
radius = 150.0

[effects]
[effects.comms_jammed]
[effects.sensor_blind]
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.shape.is_some());
        let effects = config.effects.expect("effects must be Some");
        assert!(effects.comms_jammed.is_some());
        assert!(effects.sensor_blind.is_some());
    }

    #[test]
    fn region_effects_without_shape_returns_error() {
        let toml_str = r##"
tags = ["region"]

[effects]
[effects.comms_jammed]
"##;
        let result = EntityConfig::from_toml(toml_str);
        assert!(result.is_err(), "region entity with effects but no shape should error");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("shape"), "error should mention missing shape: {err}");
    }

    #[test]
    fn shape_alone_without_effects_is_valid() {
        let toml_str = r##"
tags = ["region"]

[shape]
type = "sphere"
radius = 100.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.shape.is_some());
        assert!(config.effects.is_none());
    }

    #[test]
    fn empty_toml_produces_no_shape_or_effects() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.shape.is_none());
        assert!(config.effects.is_none());
    }

    // ── Station section tests ─────────────────────────────────────────────

    #[test]
    fn station_section_parses_with_shape_and_hull_integrity() {
        let toml_str = r##"
tags = ["station"]

[station]
name = "Deep Space 9"
shape = "cylinder"
radius = 15.0
hull_integrity = 200.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let station = config.station.expect("station must be Some");
        assert_eq!(station.name, "Deep Space 9");
        assert_eq!(station.shape, StationShape::Cylinder);
        assert!((station.radius - 15.0).abs() < 1e-6);
        assert!((station.hull_integrity - 200.0).abs() < 1e-6);
    }

    #[test]
    fn station_section_parses_sphere_shape() {
        let toml_str = r##"
[station]
name = "Relay Station"
shape = "sphere"
radius = 8.0
hull_integrity = 80.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let station = config.station.expect("station must be Some");
        assert_eq!(station.shape, StationShape::Sphere);
    }

    #[test]
    fn station_section_parses_torus_shape() {
        let toml_str = r##"
[station]
name = "Ring Station"
shape = "torus"
radius = 20.0
hull_integrity = 150.0
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let station = config.station.expect("station must be Some");
        assert_eq!(station.shape, StationShape::Torus);
    }

    #[test]
    fn station_section_absent_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.station.is_none());
    }

    #[test]
    fn station_outpost_template_parses_with_station_section() {
        let toml_str = include_str!("../../assets/entities/station_outpost.toml");
        let config = EntityConfig::from_toml(toml_str).expect("station_outpost.toml must parse");
        let station = config.station.as_ref().expect("must have [station]");
        assert_eq!(station.name, "Outpost Alpha");
        assert_eq!(station.shape, StationShape::Cylinder);
        assert!((station.radius - 12.0).abs() < 1e-6);
        assert!((station.hull_integrity - 200.0).abs() < 1e-6);
    }

    #[test]
    fn all_sections_parsed_in_full_template() {
        let toml_str = r##"
tags = ["full"]

[star]
name = "Sun"
radius = 50.0
colour = [1.0, 0.8, 0.0]
position = [0.0, 0.0, 0.0]

[planet]
name = "Earth"
radius = 20.0
colour = [0.0, 0.5, 1.0]
position = [200.0, 0.0, 200.0]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005

[hull]
hull_integrity = 100
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.star.is_some(), "star should be Some");
        assert!(config.planet.is_some(), "planet should be Some");
        assert!(config.asteroid_field.is_some(), "asteroid_field should be Some");
        assert!(config.hull.is_some(), "hull should be Some");
        assert_eq!(config.tags, vec!["full"]);
    }

    // ── Shipped template TOML files referenced by assets/maps/default.toml ──
    //
    // These tests embed each template at compile time via include_str! so
    // the build fails if a referenced template is missing or malformed.

    #[test]
    fn star_sun_template_parses_with_star_section() {
        let toml_str = include_str!("../../assets/entities/star_sun.toml");
        let config = EntityConfig::from_toml(toml_str).expect("star_sun.toml must parse");
        let star = config.star.as_ref().expect("star_sun.toml must have [star]");
        assert_eq!(star.name, "Sun");
        assert!((star.radius - 50.0).abs() < 1e-6);
        assert_eq!(star.colour, vec![1.0, 0.8, 0.0]);
        assert_eq!(star.light_range, Some(5000.0));
        assert_eq!(star.light_intensity, Some(150000.0));
        assert_eq!(star.light_colour, Some(vec![1.0, 0.95, 0.85]));
    }

    #[test]
    fn planet_earth_template_parses_with_planet_section() {
        let toml_str = include_str!("../../assets/entities/planet_earth.toml");
        let config = EntityConfig::from_toml(toml_str).expect("planet_earth.toml must parse");
        let planet = config.planet.as_ref().expect("planet_earth.toml must have [planet]");
        assert_eq!(planet.name, "Earth");
        assert!((planet.radius - 20.0).abs() < 1e-6);
    }

    #[test]
    fn asteroid_field_main_template_parses_with_field_and_grid() {
        let toml_str = include_str!("../../assets/entities/asteroid_field_main.toml");
        let config = EntityConfig::from_toml(toml_str).expect("asteroid_field_main.toml must parse");
        let field = config.asteroid_field.as_ref().expect("must have [asteroid_field]");
        assert!((field.inner_radius - 100.0).abs() < 1e-6);
        assert!((field.outer_radius - 200.0).abs() < 1e-6);
        let grid = field.grid.as_ref().expect("must have [asteroid_field.grid]");
        assert!((grid.resolution - 15.0).abs() < 1e-6);
        assert_eq!(field.asteroid_type_paths.len(), 2);
        assert_eq!(field.cosmetic_type_paths.len(), 1);
    }

    // ── Faction field tests ────────────────────────────────────────────────

    #[test]
    fn faction_field_parses_from_entity_toml() {
        let toml_str = r#"
tags = ["ship"]
faction = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
"#;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let faction = config.faction.expect("faction must be Some");
        assert_eq!(
            faction.to_string(),
            "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
        );
    }

    #[test]
    fn faction_field_defaults_to_none_when_absent() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.faction.is_none());
    }

    #[test]
    fn player_ship_toml_parses_with_federation_faction() {
        let toml_str = include_str!("../../assets/entities/player_ship.toml");
        let config = EntityConfig::from_toml(toml_str).expect("player_ship.toml must parse");
        let faction = config.faction.expect("player_ship must declare a faction");
        // Must match the Federation UUID in assets/factions/federation.toml
        let fed_toml = include_str!("../../assets/factions/federation.toml");
        let fed = crate::faction::parse_faction_config(fed_toml).unwrap();
        assert_eq!(faction, fed.uuid, "player ship faction must be Federation");
    }

    // ── Behaviour block tests ─────────────────────────────────────────────

    #[test]
    fn behaviour_block_parses_initial_state() {
        let toml_str = r##"
tags = ["npc", "patrol"]

[behaviour]
initial_state = "idle"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.initial_state, "idle");
    }

    #[test]
    fn behaviour_block_absent_when_not_in_toml() {
        let config = EntityConfig::from_toml("").expect("parse must succeed");
        assert!(config.behaviour.is_none());
    }

    #[test]
    fn entity_with_hull_and_behaviour_has_both_sections() {
        let toml_str = r##"
tags = ["npc"]

[hull]
hull_integrity = 50.0

[behaviour]
initial_state = "idle"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        assert!(config.hull.is_some());
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.initial_state, "idle");
    }

    // ── StateConfig tests ──────────────────────────────────────────────────

    #[test]
    fn behaviour_with_patrolling_state_parses() {
        let toml_str = r##"
[behaviour]
initial_state = "patrol_route"

[[behaviour.state]]
name = "patrol_route"
kind = "patrolling"
waypoints = ["alpha", "beta"]
loop_path = true
target_speed = 0.6
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.initial_state, "patrol_route");
        assert_eq!(behaviour.state.len(), 1);
        let state = &behaviour.state[0];
        assert_eq!(state.name, "patrol_route");
        assert_eq!(state.kind, "patrolling");
        assert_eq!(state.waypoints, vec!["alpha", "beta"]);
        assert!(state.loop_path);
        assert!((state.target_speed - 0.6).abs() < 1e-5);
    }

    #[test]
    fn target_speed_clamped_to_zero_when_negative() {
        let toml_str = r##"
[behaviour]
initial_state = "p"

[[behaviour.state]]
name = "p"
kind = "patrolling"
waypoints = []
target_speed = -0.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let state = &config.behaviour.unwrap().state[0];
        assert_eq!(state.target_speed, 0.0, "negative target_speed must clamp to 0");
    }

    #[test]
    fn target_speed_clamped_to_one_when_above_one() {
        let toml_str = r##"
[behaviour]
initial_state = "p"

[[behaviour.state]]
name = "p"
kind = "patrolling"
waypoints = []
target_speed = 1.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let state = &config.behaviour.unwrap().state[0];
        assert_eq!(state.target_speed, 1.0, "target_speed > 1 must clamp to 1");
    }

    #[test]
    fn behaviour_state_empty_by_default() {
        let toml_str = r##"
[behaviour]
initial_state = "idle"
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert!(behaviour.state.is_empty(), "state array must default to empty");
    }

    #[test]
    fn behaviour_multiple_states_parse() {
        let toml_str = r##"
[behaviour]
initial_state = "idle"

[[behaviour.state]]
name = "idle"
kind = "idle"
target_speed = 0.0

[[behaviour.state]]
name = "patrol"
kind = "patrolling"
waypoints = ["wp1", "wp2"]
loop_path = false
target_speed = 0.5
"##;
        let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        assert_eq!(behaviour.state.len(), 2);
        assert_eq!(behaviour.state[0].name, "idle");
        assert_eq!(behaviour.state[1].name, "patrol");
    }

    // ── pirate_raider.toml compile-time template tests ─────────────────────

    #[test]
    fn pirate_raider_template_parses_with_pirate_faction() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        // Must have pirate faction UUID
        let faction = config.faction.expect("pirate_raider must declare a faction");
        assert_eq!(
            faction.to_string(),
            "bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb",
            "pirate_raider faction must be Pirate"
        );
    }

    #[test]
    fn pirate_raider_template_has_hull() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        assert!(config.hull.is_some(), "pirate_raider must have a [hull] section");
        let hull = config.hull.as_ref().unwrap();
        assert!(hull.hull_integrity > 0.0, "hull_integrity must be positive");
    }

    #[test]
    fn pirate_raider_template_has_helm_and_weapons_console() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        assert!(config.helm_console.is_some(), "pirate_raider must have a [helm_console]");
        assert!(config.weapons_console.is_some(), "pirate_raider must have a [weapons_console]");
    }

    #[test]
    fn pirate_raider_template_has_behaviour_with_all_six_states() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        let behaviour = config.behaviour.expect("pirate_raider must have a [behaviour] block");
        let state_kinds: Vec<&str> = behaviour.state.iter().map(|s| s.kind.as_str()).collect();
        assert!(state_kinds.contains(&"patrolling"), "must have patrolling state");
        assert!(state_kinds.contains(&"pursuing"), "must have pursuing state");
        assert!(state_kinds.contains(&"attacking"), "must have attacking state");
        assert!(state_kinds.contains(&"fleeing"), "must have fleeing state");
        assert!(state_kinds.contains(&"warping_out"), "must have warping_out state");
    }

    #[test]
    fn pirate_raider_template_transitions_include_enemy_in_range_and_on_attacked() {
        let toml_str = include_str!("../../assets/entities/pirate_raider.toml");
        let config = EntityConfig::from_toml(toml_str).expect("pirate_raider.toml must parse");
        let behaviour = config.behaviour.expect("behaviour must be Some");
        let conditions: Vec<&str> = behaviour.transition.iter().map(|t| t.condition.as_str()).collect();
        assert!(conditions.contains(&"enemy_in_range"), "must have enemy_in_range transition");
        assert!(conditions.contains(&"on_attacked"), "must have on_attacked transition");
        assert!(conditions.contains(&"in_weapons_range"), "must have in_weapons_range transition");
        assert!(conditions.contains(&"hull_below"), "must have hull_below transition");
        assert!(conditions.contains(&"on_timer"), "must have on_timer transition");
        assert!(conditions.contains(&"on_scenario_unloaded"), "must have on_scenario_unloaded transition");
    }
}