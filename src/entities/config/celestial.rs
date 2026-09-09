//! Entity schema: celestial. Public paths remain in the parent module.
use super::*;

fn default_star_radius() -> f32 {
    40.0
}

fn default_star_longitude_segments() -> u32 {
    64
}

fn default_star_latitude_segments() -> u32 {
    32
}

fn default_star_surface_colour() -> [f32; 3] {
    [1.0, 0.72, 0.12]
}

fn default_star_hot_colour() -> [f32; 3] {
    [1.0, 0.96, 0.65]
}

fn default_star_cell_colour() -> [f32; 3] {
    [0.95, 0.32, 0.04]
}

fn default_star_halo_colour() -> [f32; 3] {
    [1.0, 0.78, 0.18]
}

fn default_star_halo_radius_multiplier() -> f32 {
    2.4
}

fn default_star_animation_speed() -> f32 {
    1.0
}

/// Animated procedural star/sun visual definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StarConfig {
    pub radius: f32,
    pub longitude_segments: u32,
    pub latitude_segments: u32,
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub surface_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub hot_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub cell_colour: [f32; 3],
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub halo_colour: [f32; 3],
    pub halo_radius_multiplier: f32,
    pub animation_speed: f32,
}

impl Default for StarConfig {
    fn default() -> Self {
        Self {
            radius: default_star_radius(),
            longitude_segments: default_star_longitude_segments(),
            latitude_segments: default_star_latitude_segments(),
            surface_colour: default_star_surface_colour(),
            hot_colour: default_star_hot_colour(),
            cell_colour: default_star_cell_colour(),
            halo_colour: default_star_halo_colour(),
            halo_radius_multiplier: default_star_halo_radius_multiplier(),
            animation_speed: default_star_animation_speed(),
        }
    }
}

fn default_planet_radius() -> f32 {
    20.0
}

fn default_planet_emissive_strength() -> f32 {
    1.0
}

fn default_planet_emissive_night_only() -> bool {
    true
}

fn default_planet_cloud_scale() -> f32 {
    1.03
}

fn default_planet_atmosphere_strength() -> f32 {
    1.0
}

fn default_planet_longitude_segments() -> u32 {
    128
}

fn default_planet_latitude_segments() -> u32 {
    64
}

/// Textured planet visual definition (`[planet]` section).
///
/// Renders as a UV sphere with a custom shader sampling equirectangular
/// texture maps: day/night lighting relative to the star, optional
/// nightside-gated emissive (city lights / nightglow), an optional
/// alpha-blended cloud/smog/ash shell on a slightly larger sphere, and an
/// optional fresnel atmosphere rim glow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetConfig {
    #[serde(default = "default_planet_radius")]
    pub radius: f32,
    #[serde(default = "default_planet_longitude_segments")]
    pub longitude_segments: u32,
    #[serde(default = "default_planet_latitude_segments")]
    pub latitude_segments: u32,
    /// Core surface texture set (`[planet.surface]`). Required.
    pub surface: PlanetSurfaceConfig,
    /// Optional cloud/smog/ash shell (`[planet.clouds]`).
    #[serde(default)]
    pub clouds: Option<PlanetCloudsConfig>,
    /// Optional atmosphere rim glow (`[planet.atmosphere]`).
    #[serde(default)]
    pub atmosphere: Option<PlanetAtmosphereConfig>,
}

/// Core surface texture maps for a `[planet]`. Paths are TOML-style
/// (`assets/...`-prefixed) like `MeshConfig.model`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetSurfaceConfig {
    /// Packed city material: roughness RGBA = roughness/AO/metal/daytime activity;
    /// emissive_mask RGBA = windows/neon/thermal/traffic. Absent = legacy maps.
    #[serde(default)]
    pub city: Option<PlanetCityConfig>,
    /// Base colour map (sRGB). Required.
    pub albedo: String,
    /// Tangent-space normal map (linear).
    #[serde(default)]
    pub normal: Option<String>,
    /// Grayscale roughness map (linear).
    #[serde(default)]
    pub roughness: Option<String>,
    /// Emissive colour map (sRGB): city lights, nightglow, lava glow.
    #[serde(default)]
    pub emissive_colour: Option<String>,
    /// Grayscale emissive mask (linear). When absent the emissive colour map
    /// is used unmasked (maps that are black where unlit need no mask).
    #[serde(default)]
    pub emissive_mask: Option<String>,
    /// Gate emission to the night side (city lights). `false` for emission
    /// that is visible on the day side too (lava).
    #[serde(default = "default_planet_emissive_night_only")]
    pub emissive_night_only: bool,
    #[serde(default = "default_planet_emissive_strength")]
    pub emissive_strength: f32,
}

/// Cloud/smog/ash shell rendered on a second, slightly larger sphere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetCloudsConfig {
    #[serde(default)]
    pub smog: Option<PlanetSmogConfig>,
    /// Cloud colour map (sRGB). Required.
    pub albedo: String,
    /// Grayscale opacity map (linear). When absent the albedo luminance is
    /// used as opacity.
    #[serde(default)]
    pub opacity: Option<String>,
    /// Tangent-space cloud normal map, sampled as linear data.
    #[serde(default)]
    pub normal: Option<String>,
    /// Shell radius as a multiple of the planet radius.
    #[serde(default = "default_planet_cloud_scale")]
    pub scale: f32,
    /// Longitudinal drift in UV wraps per second. 0 = static.
    #[serde(default)]
    pub drift_speed: f32,
}

/// Fresnel rim atmosphere glow parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetAtmosphereConfig {
    /// A separate scattering shell. Absent preserves the inexpensive rim.
    #[serde(default)]
    pub scattering: Option<PlanetScatteringConfig>,
    /// RGB colour `[r, g, b]` in linear 0-1 range.
    pub colour: [f32; 3],
    #[serde(default = "default_planet_atmosphere_strength")]
    pub strength: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanetCityConfig {
    pub normal_strength: f32,
    pub windows: f32,
    pub neon: f32,
    pub thermal: f32,
    pub traffic: f32,
    pub traffic_speed: f32,
    pub shadow_strength: f32,
    pub neon_colour: [f32; 3],
    pub thermal_colour: [f32; 3],
    pub traffic_colour: [f32; 3],
}

impl Default for PlanetCityConfig {
    fn default() -> Self {
        Self {
            normal_strength: 0.65,
            windows: 1.4,
            neon: 0.7,
            thermal: 0.2,
            traffic: 0.5,
            traffic_speed: 0.035,
            shadow_strength: 0.55,
            neon_colour: [0.15, 0.65, 1.0],
            thermal_colour: [1.0, 0.16, 0.025],
            traffic_colour: [1.0, 0.43, 0.13],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetSmogConfig {
    /// Surface-aligned city illumination, sampled beneath the drifting smog.
    pub city_glow: String,
    pub opacity: f32,
    pub glow_strength: f32,
    pub normal_strength: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetScatteringConfig {
    /// Baked for this shell scale, with density falloffs 6 and 12 per shell.
    pub optical_depth: String,
    pub haze: String,
    /// Optional atmospheric emission; omitted when the authored map is negligible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skyglow: Option<String>,
    pub scale: f32,
    pub rayleigh: [f32; 3],
    pub mie: [f32; 3],
    pub mie_anisotropy: f32,
    pub skyglow_strength: f32,
}

/// Kind of a `[[light]]` entry: a point light or a directional light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Point,
    Directional,
}

/// One `[[light]]` entry from an entity template. Renderer-only data.
///
/// Replaces the per-section light fields that used to live on `[star]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightConfig {
    pub kind: LightKind,
    /// RGB colour `[r, g, b]` in linear 0–1 range.
    pub colour: [f32; 3],
    /// Light intensity (candela for point lights, illuminance for directional).
    pub intensity: f32,
    /// Range in world units. Required for point lights; ignored for directional.
    #[serde(default)]
    pub range: Option<f32>,
    /// If true, the light is spawned as a child entity and continuously
    /// rotated to face the player's ship, regardless of how the parent
    /// entity itself is oriented.
    #[serde(default)]
    pub face_player: bool,
}
