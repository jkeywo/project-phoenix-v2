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

/// Marker for the one in-game 3-D camera (as opposed to the lobby camera).
///
/// Spawned and driven by `crate::server::renderer` (the presentation stack), but
/// the marker TYPE lives here — the always-compiled shared render-setup module
/// (issue #1194) — because the LOD driver `update_mesh_lod` in
/// `crate::server_app_render` and the `orient_lod_billboards::<GameCamera>`
/// registration in `crate::server_app` both name it from always-compiled code
/// that must still build with the `server` feature off. Keeping it beside
/// [`GAME_CAMERA_FAR`] and [`RenderTuning`], the camera's other always-compiled
/// render properties, also lets the standalone viewer share it (`--features
/// viewer` no longer pulls in `server`).
#[derive(Component)]
pub struct GameCamera;

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

/// Whether this build's render backend can actually run bloom.
///
/// # The platform fact, enforced rather than documented
///
/// Bevy 0.18.1's bloom cannot run on WebGL2. Two upstream facts, both read out
/// of the vendored 0.18.1 sources rather than inferred:
///
/// 1. Bloom's downsample chain binds individual mip LEVELS of one texture for
///    sampling, which WebGL2 does not support. `bevy_post_process` carries a
///    separate-texture-per-mip fallback behind its own `webgl` feature — but
///    `bevy_internal`'s `webgl` feature forwards to eight sub-crates and
///    `bevy_post_process` is not among them, so `bevy/webgl2` never turns it on.
/// 2. Turning that feature on directly does not compile:
///    `prepare_bloom_bind_groups` reads `bloom_texture.texture.texture.id()` for
///    its bind-group cache key with no `cfg`, and under the fallback `texture`
///    is a `Vec<CachedTexture>`.
///
/// Until this constant existed, that fact lived only in `BloomConfig`'s doc
/// comment and in a default of `false` — which a world TOML could override.
/// `[render.bloom] enabled = true` on the browser host would have produced a
/// viewscreen whose render graph fails, and nothing in the code would have
/// stopped it. The gate is here, at the point the component is inserted, so the
/// authored calibration stays exactly as written on every platform and the
/// PLATFORM decides whether the component is attached.
///
/// # Why the target arch, and not a runtime adapter query
///
/// The backend is a build-time choice in this project, not a runtime discovery:
/// `Cargo.toml` pins `bevy = { features = ["webgl2", …] }`, so a `wasm32` build
/// selects the WebGL2 backend and a native build gets full wgpu (Vulkan/DX12/
/// Metal), where bloom runs. Asking wgpu at startup would be answering a
/// question the build already settled, and would have to answer it before the
/// camera is spawned.
///
/// It is `cfg!` rather than `#[cfg]` deliberately: both arms compile on both
/// targets, so this adds no conditional compilation, leaves the wasm build's
/// dependency graph byte-identical (`bevy_post_process` is already linked there
/// — `Bloom` is imported unconditionally at the top of this file today), and
/// leaves the value readable by a test on either platform.
pub const BLOOM_RUNS_ON_THIS_TARGET: bool = cfg!(not(target_arch = "wasm32"));

/// Put an authored `[render]` block onto a 3D camera: the HDR intermediate
/// target, the display transform, and bloom.
///
/// Written as insert-or-remove rather than insert-only so it is idempotent and
/// reversible — the camera is spawned before the world config is loaded, so this
/// runs a second time once the world lands and has to be able to take an effect
/// back off as well as put one on. `Bloom` requires `Hdr`, so an authored
/// `hdr = false` takes bloom with it: there would be nothing above white left to
/// bloom, and leaving it on would only cost frame time.
///
/// Bloom is additionally gated on [`BLOOM_RUNS_ON_THIS_TARGET`]: where the
/// backend cannot draw it, the component is not attached however the world
/// authored it. HDR and the display transform are NOT affected and ship on
/// everywhere — they are what stop an emissive of 9.0 being drawn as the same
/// flat white as 1.0. Bloom is the halo on top.
///
/// This is the 3D camera's half of the `[render]` block. Any OTHER camera
/// sharing that camera's render target needs the same HDR answer, for reasons
/// that have nothing to do with what it draws — see [`apply_target_hdr`].
pub fn apply_render_config(commands: &mut Commands, camera: Entity, cfg: &RenderConfig) {
    let mut entity = commands.entity(camera);
    entity.insert(tonemapping_for(cfg.tonemapping));
    if !cfg.hdr {
        entity.remove::<(Hdr, Bloom)>();
        return;
    }
    entity.insert(Hdr);
    match bloom_for(&cfg.bloom).filter(|_| BLOOM_RUNS_ON_THIS_TARGET) {
        Some(bloom) => {
            entity.insert(bloom);
        }
        None => {
            entity.remove::<Bloom>();
        }
    }
}

/// Put the target's HDR decision onto a camera that SHARES the game camera's
/// render target but draws none of the 3D scene — the 2D UI camera.
///
/// # Why a camera that renders no 3D has an opinion about HDR
///
/// [`Hdr`] does not only describe what a camera draws; it selects which
/// intermediate texture the camera draws INTO. Bevy hands those textures out of
/// a cache keyed by `(target, texture_usages, hdr, msaa)` — `hdr` is part of
/// the key (`bevy_render`'s `prepare_view_targets`) — so two cameras aimed at
/// the same window with different [`Hdr`] do not share a main texture. They get
/// one each.
///
/// Here that is fatal rather than merely wasteful, because the two cameras on
/// the browser host's canvas are a COMPOSITE PAIR, not two independent views:
/// the game camera draws the scene at `order: -1`, and the UI camera draws the
/// viewscreen border and HUD over it at `order: 0` with
/// `ClearColorConfig::None` precisely so that it does not wipe what the first
/// one drew. Both graphs end in an `Upscaling` node that blits their OWN main
/// texture to the surface. Give the two cameras separate textures and the UI
/// camera's blit — which runs second, being the higher order — replaces the
/// finished 3D image with its own, which has never had a scene drawn into it.
///
/// The result is a viewscreen that is exactly, silently black: every draw is
/// valid, so wgpu logs nothing, and the HTML around the canvas is untouched
/// because it is not in the canvas. That is the shape the PRD #1023 HDR
/// regression took, and it is why `tests/smoke/viewscreen.render.spec.js`
/// asserts on canvas pixels rather than on the console — a clean console was
/// the one thing that defect never lacked.
///
/// # Why only the marker
///
/// Only [`Hdr`] is synced, deliberately: this is not [`apply_render_config`]
/// called twice. The UI camera keeps the `Tonemapping::None` that Bevy's own
/// `Core2dPlugin` requires onto every `Camera2d`, because by the time the HUD
/// is drawn the game camera's tonemapping node has already resolved the shared
/// texture to display-referred values. A second display transform would tonemap
/// the HUD and re-tonemap the scene underneath it. Bloom is likewise not the UI
/// camera's business — it has nothing above white to bloom.
pub fn apply_target_hdr(commands: &mut Commands, camera: Entity, hdr: bool) {
    let mut entity = commands.entity(camera);
    if hdr {
        entity.insert(Hdr);
    } else {
        entity.remove::<Hdr>();
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
    use bevy::ecs::world::CommandQueue;

    fn tuning() -> RenderTuning {
        RenderTuning::default()
    }

    /// Apply a `[render]` block to a game camera and a companion UI camera the
    /// way `RendererPlugin` does, and hand back both entities' components.
    ///
    /// `seed_hdr` pre-sets the OPPOSITE state so each call has to change it,
    /// which is what makes this a test of the reversibility both functions
    /// claim rather than of a lucky initial state.
    fn apply_to_pair(cfg: &RenderConfig, seed_hdr: bool) -> (bool, bool, bool, bool) {
        let mut world = World::new();
        let game = world.spawn_empty().id();
        let ui = world.spawn_empty().id();
        if seed_hdr {
            world.entity_mut(game).insert(Hdr);
            world.entity_mut(ui).insert(Hdr);
        }

        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, &world);
            apply_render_config(&mut commands, game, cfg);
            apply_target_hdr(&mut commands, ui, cfg.hdr);
        }
        queue.apply(&mut world);

        (
            world.entity(game).contains::<Hdr>(),
            world.entity(ui).contains::<Hdr>(),
            world.entity(ui).contains::<Tonemapping>(),
            world.entity(ui).contains::<Bloom>(),
        )
    }

    /// The invariant the black-viewscreen regression broke.
    ///
    /// Bevy keys its main-texture cache on `hdr`, so the game camera and the UI
    /// camera that composites over it must give the same answer or they stop
    /// sharing a texture and the UI camera's blit wipes the scene. Both
    /// directions of the switch, because an authored `hdr = false` that reached
    /// only one of them would split the pair exactly as badly as the default
    /// `hdr = true` did.
    #[test]
    fn both_cameras_on_the_shared_canvas_agree_about_hdr() {
        for hdr in [true, false] {
            let cfg = RenderConfig {
                hdr,
                ..Default::default()
            };
            let (game_hdr, ui_hdr, _, _) = apply_to_pair(&cfg, !hdr);
            assert_eq!(game_hdr, hdr, "the game camera takes the authored hdr");
            assert_eq!(
                ui_hdr, hdr,
                "the UI camera sharing the canvas takes the same one"
            );
        }
    }

    /// The UI camera takes the marker and NOTHING else: it keeps the
    /// `Tonemapping::None` Bevy requires onto every `Camera2d`, because the
    /// game camera has already tonemapped the texture it draws into.
    #[test]
    fn the_ui_camera_takes_the_hdr_marker_but_not_the_display_transform() {
        let (_, ui_hdr, ui_tonemapping, ui_bloom) = apply_to_pair(&RenderConfig::default(), false);
        assert!(ui_hdr, "the marker is the point of the call");
        assert!(
            !ui_tonemapping,
            "a second display transform would tonemap the HUD and re-tonemap the scene"
        );
        assert!(!ui_bloom, "the UI layer has nothing above white to bloom");
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
