//! Shared 3D render setup: the space skybox, the game camera's optical
//! properties, and the default ambient fill.
//!
//! These pieces define what the game *looks* like, independent of what it is
//! simulating. They live outside the `server` feature gate so the standalone
//! model viewer (`--features viewer`) renders through the exact same setup as
//! the real game — if the two ever diverge, the viewer stops being a valid
//! reference for tuning lighting and shaders.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::core_pipeline::Skybox;
use bevy::post_process::bloom::{Bloom, BloomCompositeMode, BloomPrefilter};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};
use bevy::render::view::Hdr;

use crate::world::config::{BloomComposite, BloomConfig, RenderConfig, TonemapChoice};

/// Vertical 6-face cubemap strip, reinterpreted as a cube array at runtime by
/// [`prepare_space_skybox_cubemap`].
pub const SPACE_SKYBOX_PATH: &str = "skybox/phoenix_space_cubemap.png";
/// Brightness the skybox cubemap is sampled at.
///
/// In Bevy's physical light units, divided by the camera's exposure like every
/// other light in the scene — NOT a raw multiplier on the sampled texel. That
/// distinction only started to matter when [`apply_render_config`] gave the
/// camera an HDR intermediate target: an LDR target clipped this (and the
/// ambient fill below) at screen white regardless, so the number could not
/// visibly overshoot. It still resolves to roughly the same picture after
/// tonemapping, which is the calibration claim to check by eye first — a skybox
/// that has quietly gone white-hot would be the loudest possible symptom of a
/// wrong exposure assumption.
pub const SPACE_SKYBOX_BRIGHTNESS: f32 = 450.0;

/// Far plane for the game camera. Anything beyond this is the skybox's job.
pub const GAME_CAMERA_FAR: f32 = 5000.0;

/// Default ambient fill when a world supplies no `[ambient_light]` block —
/// a warm key that stars then layer point lights on top of.
///
/// Colour is sRGB; brightness is in the same physical units as
/// [`SPACE_SKYBOX_BRIGHTNESS`] and carries the same HDR caveat. Both are
/// "authored against what the screen showed", which was a clipped image — see
/// [`RenderConfig`] for why the bloom threshold is set at exactly that clip
/// point rather than below it.
pub const DEFAULT_AMBIENT_COLOUR: [f32; 3] = [0.6, 0.55, 0.5];
pub const DEFAULT_AMBIENT_BRIGHTNESS: f32 = 300.0;

/// The authored presentation timings, lifted out of the world's `[render]`
/// block into a resource the render-coupled systems can read without carrying a
/// `WorldConfig` borrow into every LOD swap.
///
/// Initialised to [`RenderConfig`]'s defaults and overwritten at `PostStartup`
/// from the loaded world (see `apply_world_render_config`), exactly as the
/// ambient light is. Registered only under `SimPluginOptions::render`.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct RenderTuning {
    /// Seconds an LOD tier change cross-fades over. `0` restores the cut.
    pub lod_fade_secs: f32,
    /// Seconds a mid-mission arrival materialises over. `0` restores the pop.
    pub materialise_secs: f32,
    /// Fraction of full size an arrival starts at.
    pub materialise_start_scale: f32,
}

impl Default for RenderTuning {
    fn default() -> Self {
        Self::from_config(&RenderConfig::default())
    }
}

impl RenderTuning {
    pub fn from_config(cfg: &RenderConfig) -> Self {
        Self {
            lod_fade_secs: cfg.lod_fade_secs.max(0.0),
            materialise_secs: cfg.materialise_secs.max(0.0),
            materialise_start_scale: cfg.materialise_start_scale.clamp(0.0, 1.0),
        }
    }

    /// The fade a visual gets as it appears, or `None` for the same-frame
    /// appearance every visual had before PRD #1023.
    ///
    /// `first_visual` distinguishes the two cases the render path has: an
    /// entity's FIRST visual has nothing to replace, so it is an arrival;
    /// anything later is replacing a tier that is fading out beside it, so it is
    /// the incoming half of a cross-fade.
    ///
    /// `mid_mission` gates the arrival, and only the arrival. At mission start
    /// EVERY visual is a first visual, and materialising the whole map at once
    /// would be a light show rather than an announcement — the effect exists to
    /// cover the asynchronous GLB resolve of a REINFORCEMENT, and to read as an
    /// event. A cross-fade is not gated: an LOD switch during the loading phase
    /// should dissolve exactly as one later will.
    ///
    /// One function because two call sites need the answer —
    /// `render_spawned_entities` for a model with no ladder, `update_mesh_lod`
    /// for one with — and a second copy of the rule is how the two would come
    /// to disagree about what an arrival is.
    pub fn arrival(
        &self,
        first_visual: bool,
        mid_mission: bool,
    ) -> Option<crate::entities::visual_fade::VisualFade> {
        use crate::entities::visual_fade::VisualFade;
        if first_visual {
            (mid_mission && self.materialise_secs > 0.0).then(|| {
                VisualFade::materialise(self.materialise_secs, self.materialise_start_scale)
            })
        } else {
            (self.lod_fade_secs > 0.0).then(|| VisualFade::fade_in(self.lod_fade_secs))
        }
    }
}

/// The Bevy display transform an authored [`TonemapChoice`] names.
pub fn tonemapping_for(choice: TonemapChoice) -> Tonemapping {
    match choice {
        TonemapChoice::None => Tonemapping::None,
        TonemapChoice::Reinhard => Tonemapping::Reinhard,
        TonemapChoice::ReinhardLuminance => Tonemapping::ReinhardLuminance,
        TonemapChoice::AcesFitted => Tonemapping::AcesFitted,
        TonemapChoice::AgX => Tonemapping::AgX,
        TonemapChoice::SomewhatBoringDisplayTransform => {
            Tonemapping::SomewhatBoringDisplayTransform
        }
        TonemapChoice::TonyMcMapface => Tonemapping::TonyMcMapface,
        TonemapChoice::BlenderFilmic => Tonemapping::BlenderFilmic,
    }
}

/// The Bevy [`Bloom`] an authored [`BloomConfig`] describes, or `None` when the
/// block turns it off.
pub fn bloom_for(cfg: &BloomConfig) -> Option<Bloom> {
    cfg.enabled.then(|| Bloom {
        intensity: cfg.intensity,
        low_frequency_boost: cfg.low_frequency_boost,
        low_frequency_boost_curvature: cfg.low_frequency_boost_curvature,
        high_pass_frequency: cfg.high_pass_frequency,
        prefilter: BloomPrefilter {
            threshold: cfg.threshold,
            threshold_softness: cfg.threshold_softness,
        },
        composite_mode: match cfg.composite {
            BloomComposite::EnergyConserving => BloomCompositeMode::EnergyConserving,
            BloomComposite::Additive => BloomCompositeMode::Additive,
        },
        max_mip_dimension: cfg.max_mip_dimension.max(1),
        scale: Vec2::ONE,
    })
}

/// Put an authored `[render]` block onto a 3D camera: the HDR intermediate
/// target, the display transform, and bloom.
///
/// Written as insert-or-remove rather than insert-only so it is idempotent and
/// reversible — the camera is spawned before the world config is loaded, so this
/// runs a second time once the world lands and has to be able to take an effect
/// back off as well as put one on. `Bloom` requires `Hdr`, so an authored
/// `hdr = false` takes bloom with it: there would be nothing above white left to
/// bloom, and leaving it on would only cost frame time.
pub fn apply_render_config(commands: &mut Commands, camera: Entity, cfg: &RenderConfig) {
    let mut entity = commands.entity(camera);
    entity.insert(tonemapping_for(cfg.tonemapping));
    if !cfg.hdr {
        entity.remove::<(Hdr, Bloom)>();
        return;
    }
    entity.insert(Hdr);
    match bloom_for(&cfg.bloom) {
        Some(bloom) => {
            entity.insert(bloom);
        }
        None => {
            entity.remove::<Bloom>();
        }
    }
}

/// The skybox image handle plus a latch so the cubemap reinterpretation runs
/// exactly once, after the PNG finishes loading.
#[derive(Resource)]
pub struct SpaceSkyboxAsset {
    pub image: Handle<Image>,
    pub is_loaded: bool,
}

impl FromWorld for SpaceSkyboxAsset {
    fn from_world(world: &mut World) -> Self {
        let image = world.resource::<AssetServer>().load(SPACE_SKYBOX_PATH);
        Self {
            image,
            is_loaded: false,
        }
    }
}

/// Owns the space skybox asset and its one-shot cubemap conversion. Add this
/// before spawning any camera carrying [`space_skybox`].
pub struct SpaceSkyboxPlugin;

impl Plugin for SpaceSkyboxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpaceSkyboxAsset>()
            .add_systems(Update, prepare_space_skybox_cubemap);
    }
}

/// The `Skybox` component for a 3D camera, at the game's brightness.
pub fn space_skybox(skybox: &SpaceSkyboxAsset) -> Skybox {
    Skybox {
        image: skybox.image.clone(),
        brightness: SPACE_SKYBOX_BRIGHTNESS,
        ..default()
    }
}

/// The game camera's optical properties — perspective with the game's far
/// plane. Callers add their own `Camera`/marker components on top, since those
/// differ between the game (inactive until in-game, `order: -1` so the 3D scene
/// composites under the UI) and the viewer.
pub fn game_camera_projection() -> Projection {
    Projection::Perspective(PerspectiveProjection {
        far: GAME_CAMERA_FAR,
        ..default()
    })
}

/// The ambient light used when a world config supplies no override.
pub fn default_ambient_light() -> AmbientLight {
    AmbientLight {
        color: Color::srgb(
            DEFAULT_AMBIENT_COLOUR[0],
            DEFAULT_AMBIENT_COLOUR[1],
            DEFAULT_AMBIENT_COLOUR[2],
        ),
        brightness: DEFAULT_AMBIENT_BRIGHTNESS,
        ..default()
    }
}

/// One-shot: the skybox PNG ships as a vertical 6-face strip, which wgpu will
/// not sample as a cubemap. Once loaded, reinterpret it as a 6-layer array with
/// a `Cube` texture view, then push the handle onto every skybox camera.
pub fn prepare_space_skybox_cubemap(
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    skybox_asset: Option<ResMut<SpaceSkyboxAsset>>,
    mut skyboxes: Query<&mut Skybox>,
) {
    let Some(mut skybox_asset) = skybox_asset else {
        return;
    };
    if skybox_asset.is_loaded || !asset_server.load_state(&skybox_asset.image).is_loaded() {
        return;
    }

    let Some(image) = images.get_mut(&skybox_asset.image) else {
        return;
    };
    if image.texture_descriptor.array_layer_count() == 1 {
        let layers = image.height() / image.width();
        if layers != 6 {
            bevy::log::error!(
                "space skybox expected a vertical 6-face cubemap, got {}x{}",
                image.width(),
                image.height()
            );
            skybox_asset.is_loaded = true;
            return;
        }
        if let Err(err) = image.reinterpret_stacked_2d_as_array(layers) {
            bevy::log::error!("space skybox cubemap conversion failed: {err}");
            skybox_asset.is_loaded = true;
            return;
        }

        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        });
    }

    for mut skybox in &mut skyboxes {
        skybox.image = skybox_asset.image.clone();
    }
    skybox_asset.is_loaded = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::visual_fade::FadeDirection;

    fn tuning() -> RenderTuning {
        RenderTuning::default()
    }

    /// Loading a mission is not an arrival. Every visual on the map is a first
    /// visual at that point, and materialising all of them would be a light
    /// show rather than the announcement of a reinforcement.
    #[test]
    fn a_visual_that_appears_before_the_mission_starts_just_appears() {
        assert!(tuning().arrival(true, false).is_none());
    }

    /// The PRD's case: a mid-mission spawn materialises rather than popping.
    #[test]
    fn a_visual_that_appears_mid_mission_materialises() {
        let fade = tuning().arrival(true, true).expect("a mid-mission arrival");
        assert_eq!(fade.direction, FadeDirection::In);
        assert!(
            fade.scale_in_from.is_some(),
            "an arrival scales in as well as fading in"
        );
    }

    /// A tier replacing another cross-fades in, whatever phase the game is in:
    /// an LOD switch during loading should dissolve exactly as one later will.
    #[test]
    fn a_replacement_tier_cross_fades_in_whatever_the_phase() {
        for mid_mission in [false, true] {
            let fade = tuning()
                .arrival(false, mid_mission)
                .expect("a replacement tier cross-fades");
            assert_eq!(fade.direction, FadeDirection::In);
            assert!(
                fade.scale_in_from.is_none(),
                "a cross-fade must not touch the scale the tier just landed on"
            );
            assert_eq!(fade.duration, tuning().lod_fade_secs);
        }
    }

    /// The two windows are disabled independently — a world can keep its
    /// cross-fades and drop the arrival flourish, or the other way round.
    #[test]
    fn a_zero_window_disables_only_its_own_effect() {
        let no_arrival = RenderTuning {
            materialise_secs: 0.0,
            ..tuning()
        };
        assert!(no_arrival.arrival(true, true).is_none());
        assert!(no_arrival.arrival(false, true).is_some());

        let no_cross_fade = RenderTuning {
            lod_fade_secs: 0.0,
            ..tuning()
        };
        assert!(no_cross_fade.arrival(false, true).is_none());
        assert!(no_cross_fade.arrival(true, true).is_some());
    }

    /// The whole `[render]` block, clamped into the resource the render path
    /// reads: a designer cannot author a window that runs backwards or an
    /// arrival that starts bigger than it ends.
    #[test]
    fn nonsense_authored_values_are_clamped_rather_than_trusted() {
        let got = RenderTuning::from_config(&RenderConfig {
            lod_fade_secs: -1.0,
            materialise_secs: -1.0,
            materialise_start_scale: 9.0,
            ..Default::default()
        });
        assert_eq!(got.lod_fade_secs, 0.0);
        assert_eq!(got.materialise_secs, 0.0);
        assert_eq!(got.materialise_start_scale, 1.0);
    }
}
