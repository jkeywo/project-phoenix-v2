// Pure Rust module for parsing map TOML files.
// No Bevy dependency. Owns all deserialization for map configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Global configuration for the map.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    /// Global seed for deterministic generation.
    #[serde(default = "default_global_seed")]
    pub seed: u64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self { seed: 42 }
    }
}

fn default_global_seed() -> u64 {
    42
}

/// Configuration for a star entity.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StarConfig {
    /// Display name of the star.
    #[serde(default)]
    pub name: String,
    /// Radius of the star.
    pub radius: f32,
    /// RGB colour of the star (normalized 0.0-1.0).
    pub colour: Vec<f32>,
    /// Position in 3D space [x, y, z].
    #[serde(default)]
    pub position: Vec<f32>,
    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Configuration for a planet entity.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlanetConfig {
    /// Display name of the planet.
    #[serde(default)]
    pub name: String,
    /// Radius of the planet.
    pub radius: f32,
    /// RGB colour of the planet (normalized 0.0-1.0).
    pub colour: Vec<f32>,
    /// Position in 3D space [x, y, z].
    #[serde(default)]
    pub position: Vec<f32>,
    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Configuration for the grid-based asteroid spawner.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridConfig {
    /// Cell size in world units (same for x and z).
    pub resolution: f32,
    /// Probability threshold for gameplay layer.
    #[serde(default = "default_fill_gameplay")]
    pub fill_gameplay: f32,
    /// Probability threshold for each cosmetic layer.
    #[serde(default = "default_fill_cosmetic")]
    pub fill_cosmetic: f32,
    /// Weight between random (1.0) and noise-driven (0.0).
    #[serde(default)]
    pub uniformity: f32,
    /// Spatial noise frequency (cycles/meter) for jitter.
    #[serde(default = "default_noise_freq")]
    pub noise_freq: f32,
    /// Octaves for spatial noise.
    #[serde(default = "default_noise_octaves")]
    pub noise_octaves: u32,
    /// Perlin frequency for density field.
    #[serde(default = "default_density_noise_freq")]
    pub density_noise_freq: f32,
    /// Octaves for density noise.
    #[serde(default = "default_density_noise_octaves")]
    pub density_noise_octaves: u32,
    /// Max offset from cell center in meters.
    #[serde(default)]
    pub jitter: f32,
    /// Base Y offset for cosmetic layers.
    #[serde(default)]
    pub cosmetic_y_offset: f32,
}

fn default_fill_gameplay() -> f32 { 0.4 }
fn default_fill_cosmetic() -> f32 { 0.15 }
fn default_noise_freq() -> f32 { 0.02 }
fn default_noise_octaves() -> u32 { 3 }
fn default_density_noise_freq() -> f32 { 0.01 }
fn default_density_noise_octaves() -> u32 { 2 }

/// Configuration for an asteroid field.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AsteroidFieldConfig {
    /// Inner radius of the donut-shaped field.
    pub inner_radius: f32,
    /// Outer radius of the donut-shaped field.
    pub outer_radius: f32,
    /// Density of asteroids per unit area.
    pub density: f32,
    /// Distance from ship at which asteroids start spawning.
    #[serde(default = "default_spawn_distance")]
    pub spawn_distance: f32,
    /// Distance from ship at which asteroids are despawned.
    #[serde(default = "default_despawn_distance")]
    pub despawn_distance: f32,
    /// Paths to asteroid type configurations for gameplay asteroids.
    #[serde(default)]
    pub asteroid_type_paths: Vec<String>,
    /// Paths to asteroid type configurations for cosmetic asteroids.
    #[serde(default)]
    pub cosmetic_type_paths: Vec<String>,
    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Grid-based spawner configuration. When present, overrides donut-based generation.
    #[serde(default)]
    pub grid: Option<GridConfig>,
}

fn default_spawn_distance() -> f32 {
    150.0
}

fn default_despawn_distance() -> f32 {
    250.0
}

/// Complete map configuration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, bevy::prelude::Resource)]
pub struct MapConfig {
    /// Global configuration.
    #[serde(default)]
    pub global: GlobalConfig,
    /// List of star configurations.
    #[serde(default, rename = "star")]
    pub stars: Vec<StarConfig>,
    /// List of planet configurations.
    #[serde(default, rename = "planet")]
    pub planets: Vec<PlanetConfig>,
    /// List of asteroid field configurations.
    #[serde(default, rename = "asteroid_field")]
    pub asteroid_fields: Vec<AsteroidFieldConfig>,
    /// Named anchors - predefined positions that can be referenced by name.
    #[serde(default)]
    pub anchors: HashMap<String, Vec<f32>>,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            global: GlobalConfig { seed: 42 },
            stars: Vec::new(),
            planets: Vec::new(),
            asteroid_fields: Vec::new(),
            anchors: HashMap::new(),
        }
    }
}

/// Parse a TOML string into a MapConfig.
pub fn parse_map_config(toml_str: &str) -> Result<MapConfig, String> {
    toml::from_str(toml_str).map_err(|e| e.to_string())
}

/// Parse a TOML string into a MapConfig with additional validation.
pub fn parse_and_validate_map_config(toml_str: &str) -> Result<MapConfig, String> {
    let config: MapConfig = toml::from_str(toml_str).map_err(|e| e.to_string())?;
    
    // Validate that colour arrays have 3 elements
    for star in &config.stars {
        if star.colour.len() != 3 {
            return Err(format!(
                "Star '{}' has invalid colour array length: {}",
                star.name,
                star.colour.len()
            ));
        }
        if star.position.len() != 3 {
            return Err(format!(
                "Star '{}' has invalid position array length: {}",
                star.name,
                star.position.len()
            ));
        }
    }
    
    for planet in &config.planets {
        if planet.colour.len() != 3 {
            return Err(format!(
                "Planet '{}' has invalid colour array length: {}",
                planet.name,
                planet.colour.len()
            ));
        }
        if planet.position.len() != 3 {
            return Err(format!(
                "Planet '{}' has invalid position array length: {}",
                planet.name,
                planet.position.len()
            ));
        }
    }
    
    // Validate anchor positions have 3 elements
    for (name, pos) in &config.anchors {
        if pos.len() != 3 {
            return Err(format!(
                "Anchor '{}' has invalid position array length: {}",
                name,
                pos.len()
            ));
        }
    }
    
    // Validate that spawn_distance <= despawn_distance for each field
    for field in &config.asteroid_fields {
        if field.spawn_distance > field.despawn_distance {
            return Err(format!(
                "Asteroid field has spawn_distance ({}) > despawn_distance ({})",
                field.spawn_distance,
                field.despawn_distance
            ));
        }
    }
    
    // Validate that inner_radius < outer_radius for each field
    for field in &config.asteroid_fields {
        if field.inner_radius >= field.outer_radius {
            return Err(format!(
                "Asteroid field has inner_radius ({}) >= outer_radius ({})",
                field.inner_radius,
                field.outer_radius
            ));
        }
    }
    
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_map() {
        let toml = "";
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.stars.len(), 0);
        assert_eq!(config.planets.len(), 0);
        assert_eq!(config.asteroid_fields.len(), 0);
        assert_eq!(config.anchors.len(), 0);
    }

    #[test]
    fn parse_global_config() {
        let toml = r#"
[global]
seed = 12345
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.global.seed, 12345);
    }

    #[test]
    fn parse_global_config_default_seed() {
        let toml = "";
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.global.seed, 42);
    }

    #[test]
    fn parse_single_star() {
        let toml = r#"
[[star]]
name = "Sun"
radius = 50.0
colour = [1.0, 0.8, 0.0]
position = [0.0, 0.0, 0.0]
tags = ["star", "center"]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.stars.len(), 1);
        assert_eq!(config.stars[0].name, "Sun");
        assert_eq!(config.stars[0].radius, 50.0);
        assert_eq!(config.stars[0].colour, vec![1.0, 0.8, 0.0]);
        assert_eq!(config.stars[0].position, vec![0.0, 0.0, 0.0]);
        assert_eq!(config.stars[0].tags, vec!["star", "center"]);
    }

    #[test]
    fn parse_multiple_stars() {
        let toml = r#"
[[star]]
name = "Sun"
radius = 50.0
colour = [1.0, 0.8, 0.0]

[[star]]
name = "Alpha Centauri"
radius = 45.0
colour = [1.0, 0.9, 0.7]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.stars.len(), 2);
        assert_eq!(config.stars[0].name, "Sun");
        assert_eq!(config.stars[1].name, "Alpha Centauri");
    }

    #[test]
    fn parse_single_planet() {
        let toml = r#"
[[planet]]
name = "Earth"
radius = 20.0
colour = [0.0, 0.5, 1.0]
position = [200.0, 0.0, 200.0]
tags = ["planet", "habitable"]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.planets.len(), 1);
        assert_eq!(config.planets[0].name, "Earth");
        assert_eq!(config.planets[0].radius, 20.0);
        assert_eq!(config.planets[0].colour, vec![0.0, 0.5, 1.0]);
        assert_eq!(config.planets[0].position, vec![200.0, 0.0, 200.0]);
        assert_eq!(config.planets[0].tags, vec!["planet", "habitable"]);
    }

    #[test]
    fn parse_multiple_planets() {
        let toml = r#"
[[planet]]
name = "Earth"
radius = 20.0
colour = [0.0, 0.5, 1.0]

[[planet]]
name = "Mars"
radius = 10.0
colour = [1.0, 0.3, 0.0]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.planets.len(), 2);
        assert_eq!(config.planets[0].name, "Earth");
        assert_eq!(config.planets[1].name, "Mars");
    }

    #[test]
    fn parse_asteroid_field() {
        let toml = r#"
[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0
asteroid_type_paths = ["assets/entities/asteroid_small.toml", "assets/entities/asteroid_large.toml"]
cosmetic_type_paths = ["assets/entities/asteroid_cosmetic.toml"]
tags = ["field", "main"]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.asteroid_fields.len(), 1);
        let field = &config.asteroid_fields[0];
        assert_eq!(field.inner_radius, 100.0);
        assert_eq!(field.outer_radius, 200.0);
        assert_eq!(field.density, 0.005);
        assert_eq!(field.spawn_distance, 150.0);
        assert_eq!(field.despawn_distance, 250.0);
        assert_eq!(field.asteroid_type_paths.len(), 2);
        assert_eq!(field.cosmetic_type_paths.len(), 1);
        assert_eq!(field.tags, vec!["field", "main"]);
    }

    #[test]
    fn parse_asteroid_field_default_distances() {
        let toml = r#"
[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
"#;
        let config = parse_map_config(toml).unwrap();
        let field = &config.asteroid_fields[0];
        assert_eq!(field.spawn_distance, 150.0);
        assert_eq!(field.despawn_distance, 250.0);
    }

    #[test]
    fn parse_multiple_asteroid_fields() {
        let toml = r#"
[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005

[[asteroid_field]]
inner_radius = 300.0
outer_radius = 400.0
density = 0.003
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.asteroid_fields.len(), 2);
        assert_eq!(config.asteroid_fields[0].inner_radius, 100.0);
        assert_eq!(config.asteroid_fields[1].inner_radius, 300.0);
    }

    #[test]
    fn parse_anchors() {
        let toml = r#"
[anchors]
spawn_point = [0.0, 0.0, 0.0]
waypoint_alpha = [100.0, 50.0, 200.0]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.anchors.len(), 2);
        assert_eq!(config.anchors.get("spawn_point"), Some(&vec![0.0, 0.0, 0.0]));
        assert_eq!(config.anchors.get("waypoint_alpha"), Some(&vec![100.0, 50.0, 200.0]));
    }

    #[test]
    fn parse_complete_map() {
        let toml = r#"
[global]
seed = 42

[[star]]
name = "Sun"
radius = 50.0
colour = [1.0, 0.8, 0.0]
position = [0.0, 0.0, 0.0]
tags = ["star", "center"]

[[planet]]
name = "Earth"
radius = 20.0
colour = [0.0, 0.5, 1.0]
position = [200.0, 0.0, 200.0]
tags = ["planet", "habitable"]

[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0
asteroid_type_paths = ["assets/entities/asteroid_small.toml", "assets/entities/asteroid_large.toml"]
cosmetic_type_paths = ["assets/entities/asteroid_cosmetic.toml"]
tags = ["field", "main"]

[anchors]
spawn_point = [0.0, 0.0, 0.0]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.global.seed, 42);
        assert_eq!(config.stars.len(), 1);
        assert_eq!(config.planets.len(), 1);
        assert_eq!(config.asteroid_fields.len(), 1);
        assert_eq!(config.anchors.len(), 1);
    }

    #[test]
    fn parse_and_validate_valid_map() {
        let toml = r#"
[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0

[[star]]
name = "Sun"
radius = 50.0
colour = [1.0, 0.8, 0.0]
position = [0.0, 0.0, 0.0]
"#;
        let config = parse_and_validate_map_config(toml).unwrap();
        assert_eq!(config.asteroid_fields.len(), 1);
        assert_eq!(config.stars.len(), 1);
    }

    #[test]
    fn validate_rejects_spawn_greater_than_despawn() {
        let toml = r#"
[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 300.0
despawn_distance = 250.0
"#;
        let result = parse_and_validate_map_config(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("spawn_distance"));
    }

    #[test]
    fn validate_rejects_inner_geq_outer() {
        let toml = r#"
[[asteroid_field]]
inner_radius = 200.0
outer_radius = 100.0
density = 0.005
"#;
        let result = parse_and_validate_map_config(toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("inner_radius"));
    }

    #[test]
    fn default_map_config() {
        let config = MapConfig::default();
        assert_eq!(config.global.seed, 42);
        assert!(config.stars.is_empty());
        assert!(config.planets.is_empty());
        assert!(config.asteroid_fields.is_empty());
        assert!(config.anchors.is_empty());
    }

    #[test]
    fn parse_invalid_toml() {
        let toml = r#"
[[asteroid_field
inner_radius = 100.0
"#;
        let result = parse_map_config(toml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_star_without_name() {
        let toml = r#"
[[star]]
radius = 50.0
colour = [1.0, 0.8, 0.0]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.stars.len(), 1);
        assert_eq!(config.stars[0].name, "");
    }

    #[test]
    fn parse_planet_without_name() {
        let toml = r#"
[[planet]]
radius = 20.0
colour = [0.0, 0.5, 1.0]
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.planets.len(), 1);
        assert_eq!(config.planets[0].name, "");
    }

    #[test]
    fn parse_asteroid_field_without_type_paths() {
        let toml = r#"
[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
"#;
        let config = parse_map_config(toml).unwrap();
        assert_eq!(config.asteroid_fields.len(), 1);
        assert!(config.asteroid_fields[0].asteroid_type_paths.is_empty());
        assert!(config.asteroid_fields[0].cosmetic_type_paths.is_empty());
    }

    #[test]
    fn parse_asteroid_field_with_empty_tags() {
        let toml = r#"
[[asteroid_field]]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
"#;
        let config = parse_map_config(toml).unwrap();
        assert!(config.asteroid_fields[0].tags.is_empty());
    }
}
