//! Textured planet rendering (issue: replace GLB planets with shader spheres).
//!
//! A `[planet]` entity renders as a UV sphere with a custom WGSL material
//! sampling equirectangular texture maps, plus an optional slightly-larger
//! alpha-blended cloud/smog/ash shell child. Lighting is computed in the
//! shader from a star-direction uniform rather than Bevy's directional light,
//! because the sun's `DirectionalLight` is `face_player = true` (rotated
//! toward the player each frame) and therefore non-physical — the terminator
//! must track the actual star position so nightside city lights face away
//! from the star.
//!
//! Follows the `star.rs` custom-material precedent (`StarSurfaceMaterial`).

use bevy::{
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, Face, ShaderType},
    shader::ShaderRef,
};

use crate::entity_config::PlanetConfig;
use crate::entity_spawner::StarSection;

const PLANET_SURFACE_SHADER: &str = "shaders/planet_surface.wgsl";
const PLANET_CLOUDS_SHADER: &str = "shaders/planet_clouds.wgsl";

/// Ambient light floor so the nightside silhouette stays faintly readable.
pub const AMBIENT_FLOOR: f32 = 0.03;

/// Presentation-only light controls for a standalone celestial-material view.
/// The game leaves this resource absent and derives planet lighting from its
/// authored star instead.
#[derive(Resource, Clone, Copy, Debug)]
pub struct PlanetLightingOverride {
    pub light_dir: Vec3,
    pub ambient_floor: f32,
    pub directional_strength: f32,
}

impl Default for PlanetLightingOverride {
    fn default() -> Self {
        Self {
            light_dir: Vec3::X,
            ambient_floor: AMBIENT_FLOOR,
            directional_strength: 1.0,
        }
    }
}

// ── Materials ──────────────────────────────────────────────────────────────

/// Uniform block for [`PlanetSurfaceMaterial`]. Field order must match the
/// WGSL struct in `planet_surface.wgsl`; vec3+f32 pairs and vec4s keep
/// 16-byte alignment.
#[derive(Clone, Debug, ShaderType)]
pub struct PlanetSurfaceParams {
    /// World-space direction FROM the planet TO the star (normalised).
    pub light_dir: Vec3,
    pub emissive_strength: f32,
    pub atmosphere_colour: Vec3,
    /// 0.0 = no rim glow.
    pub atmosphere_strength: f32,
    /// x: has_normal, y: has_roughness, z: has_emissive, w: emissive_night_only.
    ///
    /// Optional textures fall back to Bevy's 1x1 white image when `None`; the
    /// shader must gate sampling on these flags instead of trusting fallback
    /// content (a white "normal map" would corrupt the lighting).
    pub flags: Vec4,
    /// x: has_emissive_mask, y: ambient_floor, z/w: reserved.
    pub misc: Vec4,
    /// World-space axes of the planet's local texture frame. These let the
    /// shader derive seam-free spherical UVs without world-locking a rotated
    /// planet.
    pub texture_x: Vec4,
    pub texture_y: Vec4,
    pub texture_z: Vec4,
    /// World-space centre used to derive an exact radial geometric normal.
    pub planet_center: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PlanetSurfaceMaterial {
    #[uniform(0)]
    pub params: PlanetSurfaceParams,
    #[texture(1)]
    #[sampler(2)]
    pub albedo: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub normal: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    pub roughness: Option<Handle<Image>>,
    #[texture(7)]
    #[sampler(8)]
    pub emissive_colour: Option<Handle<Image>>,
    #[texture(9)]
    #[sampler(10)]
    pub emissive_mask: Option<Handle<Image>>,
}

impl Material for PlanetSurfaceMaterial {
    fn fragment_shader() -> ShaderRef {
        PLANET_SURFACE_SHADER.into()
    }
}

/// Uniform block for [`PlanetCloudMaterial`]. Must match `planet_clouds.wgsl`.
#[derive(Clone, Debug, ShaderType)]
pub struct PlanetCloudParams {
    /// World-space direction FROM the planet TO the star (normalised).
    pub light_dir: Vec3,
    pub time: f32,
    /// x: drift_speed (UV wraps/sec), y: has_opacity, z: ambient_floor, w: reserved.
    pub misc: Vec4,
    pub texture_x: Vec4,
    pub texture_y: Vec4,
    pub texture_z: Vec4,
    pub planet_center: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PlanetCloudMaterial {
    #[uniform(0)]
    pub params: PlanetCloudParams,
    #[texture(1)]
    #[sampler(2)]
    pub albedo: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub opacity: Option<Handle<Image>>,
}

impl Material for PlanetCloudMaterial {
    fn fragment_shader() -> ShaderRef {
        PLANET_CLOUDS_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // A transparent sphere must not blend its far hemisphere back through
        // the near one. Apart from darkening the globe, that ordering failure
        // made the longitude wrap look like an opaque black seam.
        descriptor.primitive.cull_mode = Some(Face::Back);
        Ok(())
    }
}

// ── Texture loading ────────────────────────────────────────────────────────

/// Load a planet texture with explicit colour-space and wrap settings.
///
/// MUST be the single load path for planet textures (renderer AND
/// `asset_preload`): Bevy keeps the settings of the FIRST load of a given
/// path, so loading the same texture elsewhere with default settings would
/// silently win and e.g. sample a normal map as sRGB.
///
/// U wraps (longitude tiling + cloud drift); V clamps (poles).
pub fn load_planet_image(asset_server: &AssetServer, path: &str, srgb: bool) -> Handle<Image> {
    let rel = path.strip_prefix("assets/").unwrap_or(path).to_string();
    asset_server.load_with_settings(rel, move |s: &mut ImageLoaderSettings| {
        s.is_srgb = srgb;
        s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::ClampToEdge,
            ..ImageSamplerDescriptor::linear()
        });
    })
}

/// Every texture path declared by a `[planet]` config, with its sRGB flag.
/// Shared by the renderer (via the builders below) and asset preload so both
/// enumerate exactly the same set.
pub fn planet_texture_paths(config: &PlanetConfig) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let s = &config.surface;
    out.push((s.albedo.clone(), true));
    if let Some(p) = &s.normal {
        out.push((p.clone(), false));
    }
    if let Some(p) = &s.roughness {
        out.push((p.clone(), false));
    }
    if let Some(p) = &s.emissive_colour {
        out.push((p.clone(), true));
    }
    if let Some(p) = &s.emissive_mask {
        out.push((p.clone(), false));
    }
    if let Some(clouds) = &config.clouds {
        out.push((clouds.albedo.clone(), true));
        if let Some(p) = &clouds.opacity {
            out.push((p.clone(), false));
        }
    }
    out
}

// ── Material builders ──────────────────────────────────────────────────────

fn flag(present: bool) -> f32 {
    if present {
        1.0
    } else {
        0.0
    }
}

pub fn surface_material_from_config(
    config: &PlanetConfig,
    asset_server: &AssetServer,
) -> PlanetSurfaceMaterial {
    let s = &config.surface;
    let load = |p: &Option<String>, srgb: bool| {
        p.as_ref()
            .map(|path| load_planet_image(asset_server, path, srgb))
    };
    let (atmosphere_colour, atmosphere_strength) = match &config.atmosphere {
        Some(a) => (Vec3::from_array(a.colour), a.strength),
        None => (Vec3::ZERO, 0.0),
    };
    PlanetSurfaceMaterial {
        params: PlanetSurfaceParams {
            // Corrected to the real star direction by `update_planet_materials`
            // on the next frame; X is a harmless placeholder.
            light_dir: Vec3::X,
            emissive_strength: s.emissive_strength,
            atmosphere_colour,
            atmosphere_strength,
            flags: Vec4::new(
                flag(s.normal.is_some()),
                flag(s.roughness.is_some()),
                flag(s.emissive_colour.is_some()),
                flag(s.emissive_night_only),
            ),
            misc: Vec4::new(flag(s.emissive_mask.is_some()), AMBIENT_FLOOR, 0.0, 1.0),
            texture_x: Vec4::X,
            texture_y: Vec4::Y,
            texture_z: Vec4::Z,
            planet_center: Vec4::ZERO,
        },
        albedo: load_planet_image(asset_server, &s.albedo, true),
        normal: load(&s.normal, false),
        roughness: load(&s.roughness, false),
        emissive_colour: load(&s.emissive_colour, true),
        emissive_mask: load(&s.emissive_mask, false),
    }
}

pub fn cloud_material_from_config(
    config: &PlanetConfig,
    asset_server: &AssetServer,
) -> Option<PlanetCloudMaterial> {
    let clouds = config.clouds.as_ref()?;
    Some(PlanetCloudMaterial {
        params: PlanetCloudParams {
            light_dir: Vec3::X,
            time: 0.0,
            misc: Vec4::new(
                clouds.drift_speed,
                flag(clouds.opacity.is_some()),
                AMBIENT_FLOOR,
                1.0,
            ),
            texture_x: Vec4::X,
            texture_y: Vec4::Y,
            texture_z: Vec4::Z,
            planet_center: Vec4::ZERO,
        },
        albedo: load_planet_image(asset_server, &clouds.albedo, true),
        opacity: clouds
            .opacity
            .as_ref()
            .map(|p| load_planet_image(asset_server, p, false)),
    })
}

// ── Plugin + systems ───────────────────────────────────────────────────────

pub struct PlanetRenderPlugin;

impl Plugin for PlanetRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PlanetSurfaceMaterial>::default())
            .add_plugins(MaterialPlugin::<PlanetCloudMaterial>::default())
            .add_systems(Update, update_planet_materials);
    }
}

/// Per-frame: point every planet material's `light_dir` at the star and tick
/// cloud drift time.
///
/// Materials are not shared between planets (each has its own texture set and
/// position), so writing per-entity values through the material asset is safe.
fn update_planet_materials(
    time: Res<Time>,
    star: Query<&GlobalTransform, With<StarSection>>,
    lighting_override: Option<Res<PlanetLightingOverride>>,
    surfaces: Query<(&GlobalTransform, &MeshMaterial3d<PlanetSurfaceMaterial>)>,
    clouds: Query<(&GlobalTransform, &MeshMaterial3d<PlanetCloudMaterial>)>,
    mut surface_assets: ResMut<Assets<PlanetSurfaceMaterial>>,
    mut cloud_assets: ResMut<Assets<PlanetCloudMaterial>>,
) {
    let star_pos = star
        .iter()
        .next()
        .map(|t| t.translation())
        .unwrap_or(Vec3::ZERO);
    let elapsed = time.elapsed_secs();
    let light_dir = lighting_override
        .as_ref()
        .map(|lighting| lighting.light_dir)
        .unwrap_or_else(|| (star_pos - Vec3::ZERO).normalize_or(Vec3::X));
    let ambient_floor = lighting_override
        .as_ref()
        .map(|lighting| lighting.ambient_floor)
        .unwrap_or(AMBIENT_FLOOR);
    let directional_strength = lighting_override
        .as_ref()
        .map(|lighting| lighting.directional_strength)
        .unwrap_or(1.0);

    let texture_axes = |transform: &GlobalTransform| {
        let (_, rotation, _) = transform.to_scale_rotation_translation();
        (
            (rotation * Vec3::X).extend(0.0),
            (rotation * Vec3::Y).extend(0.0),
            (rotation * Vec3::Z).extend(0.0),
        )
    };

    for (transform, mat_handle) in &surfaces {
        if let Some(material) = surface_assets.get_mut(&mat_handle.0) {
            let (texture_x, texture_y, texture_z) = texture_axes(transform);
            material.params.light_dir = if lighting_override.is_some() {
                light_dir
            } else {
                (star_pos - transform.translation()).normalize_or(Vec3::X)
            };
            material.params.misc.y = ambient_floor;
            material.params.misc.w = directional_strength;
            material.params.texture_x = texture_x;
            material.params.texture_y = texture_y;
            material.params.texture_z = texture_z;
            material.params.planet_center = transform.translation().extend(0.0);
        }
    }
    for (transform, mat_handle) in &clouds {
        if let Some(material) = cloud_assets.get_mut(&mat_handle.0) {
            let (texture_x, texture_y, texture_z) = texture_axes(transform);
            material.params.light_dir = if lighting_override.is_some() {
                light_dir
            } else {
                (star_pos - transform.translation()).normalize_or(Vec3::X)
            };
            material.params.misc.z = ambient_floor;
            material.params.misc.w = directional_strength;
            material.params.texture_x = texture_x;
            material.params.texture_y = texture_y;
            material.params.texture_z = texture_z;
            material.params.planet_center = transform.translation().extend(0.0);
            material.params.time = elapsed;
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::{PlanetAtmosphereConfig, PlanetCloudsConfig, PlanetSurfaceConfig};

    fn full_config() -> PlanetConfig {
        PlanetConfig {
            radius: 20.0,
            longitude_segments: 64,
            latitude_segments: 32,
            surface: PlanetSurfaceConfig {
                albedo: "assets/planets/earth/albedo.webp".into(),
                normal: Some("assets/planets/earth/normal.webp".into()),
                roughness: Some("assets/planets/earth/roughness.webp".into()),
                emissive_colour: Some("assets/planets/earth/emissive_colour.webp".into()),
                emissive_mask: None,
                emissive_night_only: true,
                emissive_strength: 1.5,
            },
            clouds: Some(PlanetCloudsConfig {
                albedo: "assets/planets/earth/cloud_albedo.webp".into(),
                opacity: Some("assets/planets/earth/cloud_opacity.webp".into()),
                normal: None,
                scale: 1.03,
                drift_speed: 0.0,
            }),
            atmosphere: Some(PlanetAtmosphereConfig {
                colour: [0.35, 0.55, 1.0],
                strength: 1.0,
            }),
        }
    }

    #[test]
    fn planet_texture_paths_enumerates_all_declared_maps_with_srgb_flags() {
        let paths = planet_texture_paths(&full_config());
        assert_eq!(
            paths,
            vec![
                ("assets/planets/earth/albedo.webp".to_string(), true),
                ("assets/planets/earth/normal.webp".to_string(), false),
                ("assets/planets/earth/roughness.webp".to_string(), false),
                (
                    "assets/planets/earth/emissive_colour.webp".to_string(),
                    true
                ),
                ("assets/planets/earth/cloud_albedo.webp".to_string(), true),
                ("assets/planets/earth/cloud_opacity.webp".to_string(), false),
            ]
        );
    }

    #[test]
    fn planet_texture_paths_minimal_config_is_albedo_only() {
        let mut config = full_config();
        config.surface.normal = None;
        config.surface.roughness = None;
        config.surface.emissive_colour = None;
        config.clouds = None;
        let paths = planet_texture_paths(&config);
        assert_eq!(
            paths,
            vec![("assets/planets/earth/albedo.webp".to_string(), true)]
        );
    }
}
