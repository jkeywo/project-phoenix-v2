// Unified world parser — single-pass deserialization for the merged
// map/scenario world TOML (PRD #337/#341).
//
// This module owns the entire world TOML schema: anchors and `[[entity]]`
// instances. `parse_world` produces a `WorldConfig` in one parse pass. It owned
// the `[[trigger]]` and `[[comms]]` schemas too until issue #985 deleted both —
// scenario logic is authored in the `[script]` block now, lifted and compiled by
// `world::script`, and a world that still carries a declarative block is refused
// by name.
//
// Pure module — no Bevy systems, only the `Resource` derive for the
// `WorldConfig` type. Runtime types (`TriggerState`, `ActiveDialogue`,
// `WorldEvent`, etc.) live in `world::content` / `comms::content` and import the
// pure config types from here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::core::messages::{AiDirective, ObjectiveSource};
use crate::entities::config::GlobalConfig;
use crate::objectives::{ConditionModifier, UtilityConfig, ZeroGateCondition};

// -- World-tree entity instance types ---------------------------------------

/// When to spawn a `WorldEntity` declared in the world TOML.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum WorldEntitySpawnOn {
    /// Spawn immediately when the world is loaded (lobby phase).
    #[default]
    Immediate,
    /// Spawn when the game starts (in-progress phase).
    GameStart,
}

/// Positioning, rotation, and scale for a `WorldEntity`.
///
/// All fields are optional. Resolution precedence in `resolve()`:
/// 1. `relative_to` + `offset` (relative to the already-resolved position of
///    another entity in the same world, named by its `id` or its `name` —
///    declared before or after this one; see
///    [`crate::world::config::build_named_entity_positions`])
/// 2. `anchor` (looked up in the world's `[anchors]` table)
/// 3. `position` (inline `[x, y, z]`)
/// 4. Origin `[0, 0, 0]` when nothing is supplied.
///
/// `rotation` is XYZ Euler in radians and is converted to a `Quat` via
/// `quat()`. `scale` defaults to `[1, 1, 1]` via `scale_vec()`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TransformConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f32; 3]>,
    /// XYZ Euler rotation in radians (pitch, yaw, roll).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[f32; 3]>,
    /// Per-axis scale factors; defaults to `[1, 1, 1]` via `scale_vec()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<[f32; 3]>,
}

impl TransformConfig {
    /// XYZ Euler rotation as a `Quat`; identity when `rotation` is `None`.
    pub fn quat(&self) -> bevy::math::Quat {
        let [x, y, z] = self.rotation.unwrap_or([0.0, 0.0, 0.0]);
        bevy::math::Quat::from_euler(bevy::math::EulerRot::XYZ, x, y, z)
    }

    /// Per-axis scale as a `Vec3`; `Vec3::ONE` when `scale` is `None`.
    pub fn scale_vec(&self) -> bevy::math::Vec3 {
        let [x, y, z] = self.scale.unwrap_or([1.0, 1.0, 1.0]);
        bevy::math::Vec3::new(x, y, z)
    }

    /// Resolve this transform's world-space position.
    ///
    /// `template_path` is used only to make error messages informative when
    /// an `anchor` or `relative_to` reference doesn't resolve.
    pub fn resolve(
        &self,
        template_path: &str,
        anchors: &HashMap<String, [f32; 3]>,
        entities_by_name: &HashMap<String, [f32; 3]>,
    ) -> Result<[f32; 3], String> {
        if let Some(name) = self.relative_to.as_ref() {
            let base = entities_by_name.get(name).ok_or_else(|| {
                format!(
                    "Entity (template '{}') references unknown relative_to entity '{}' \
                     (must be the `id` or `name` of another entity in the same world — \
                     declared before or after this one — that is not itself \
                     positioned with `relative_to`)",
                    template_path, name
                )
            })?;
            let off = self.offset.unwrap_or([0.0, 0.0, 0.0]);
            return Ok([base[0] + off[0], base[1] + off[1], base[2] + off[2]]);
        }
        if let Some(name) = self.anchor.as_ref() {
            let pos = anchors.get(name).ok_or_else(|| {
                format!(
                    "Entity (template '{}') references unknown anchor '{}'",
                    template_path, name
                )
            })?;
            return Ok(*pos);
        }
        Ok(self.position.unwrap_or([0.0, 0.0, 0.0]))
    }
}

/// World-level ambient light override. Applied by the renderer at startup;
/// missing sub-fields fall back to renderer-supplied constants.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AmbientLightConfig {
    /// Linear sRGB colour `[r, g, b]` in 0.0–1.0 range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    /// Ambient brightness in Bevy units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f32>,
}

/// World-level presentation settings for the viewscreen: how the camera resolves
/// light, and how long a visual takes to arrive or leave (PRD #1023, module 5).
///
/// Authored as a `[render]` block, beside `[ambient_light]` and `[dust]`, and —
/// like both of those — read ONLY by render-coupled systems. A headless run
/// registers none of them (`SimPluginOptions::render == false`), so nothing in
/// this block can reach the simulation, and every field defaults, so no shipped
/// world has to author one.
///
/// # Why the numbers below are what they are
///
/// The effects that carry combat — phaser beams, torpedo cores, blaster bolts,
/// explosion flashes — are authored at emissive multipliers between 2.5 and 9.0
/// (`server/pfx.rs`). Every one of those is a value ABOVE screen white, and
/// until this block existed the viewscreen camera rendered to a low-dynamic-range
/// target, which clamps at exactly 1.0: a torpedo core authored nine times
/// brighter than white was drawn the same flat white as a value of one, and the
/// authored range did nothing at all. `hdr` is what stops the clamp; `bloom` is
/// what makes the surviving range visible as a glow rather than as a slightly
/// different white.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RenderConfig {
    /// Render the 3D scene through a high-dynamic-range intermediate target
    /// before tonemapping, so light values above screen white survive to be
    /// tonemapped instead of being clipped at 1.0.
    ///
    /// `[ai] ` On by default. The emissive strengths this makes visible are
    /// already authored, and were authored FOR it; leaving it off is what made
    /// them decorative. It is also the half of the calibration the browser host
    /// can actually run — unlike [`BloomConfig`], whose own documentation says
    /// why. Turning it off is the one-line retreat if the WebGL2 backend turns
    /// out not to like the float target.
    pub hdr: bool,
    /// Which display transform maps the HDR scene onto the screen.
    ///
    /// `[ai] ` `tony_mc_mapface`, which is both Bevy's own default and the
    /// transform its bloom documentation recommends pairing bloom with: it
    /// desaturates brights across the spectrum, so a nine-times-white torpedo
    /// core reads as a hot core with its colour intact at the edges rather than
    /// as a white disc. Named explicitly rather than left implicit because it is
    /// now a calibration decision, not a default nobody chose.
    pub tonemapping: TonemapChoice,
    /// Bloom on the viewscreen camera. See [`BloomConfig`].
    pub bloom: BloomConfig,
    /// Seconds an LOD tier change takes to cross-fade — the incoming tier
    /// fading in while the outgoing one fades out, both at their own correct
    /// scale.
    ///
    /// `[ai] ` A quarter of a second. Long enough that the eye reads a
    /// dissolve rather than a cut at the sort of distance a switch happens at
    /// (the near band of a hull ladder ends in the tens of units, the far one
    /// past 400), short enough that two tiers of the same hull are never both
    /// on screen long enough to be counted. `0` disables the effect: the window
    /// is over before it starts and the swap is the same-frame cut it was.
    pub lod_fade_secs: f32,
    /// Seconds a mid-mission arrival takes to materialise — the fade-in and
    /// scale-in that cover the asynchronous GLB resolve.
    ///
    /// `[ai] ` Six tenths of a second, deliberately longer than the LOD
    /// cross-fade: a cross-fade is meant to be unnoticed, whereas an arrival is
    /// meant to be SEEN — the PRD's complaint is that reinforcements read as a
    /// glitch, and a quarter-second flourish would read as one too. `0`
    /// disables it and restores the pop.
    pub materialise_secs: f32,
    /// The fraction of full size a materialising visual starts at.
    ///
    /// `[ai] ` A quarter. Small enough that the growth is unmistakably an
    /// arrival, large enough that the ship is identifiable for the whole of it
    /// rather than emerging from a dot — a reinforcement the crew cannot name
    /// until it is already there defeats the point of announcing it.
    pub materialise_start_scale: f32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            hdr: true,
            tonemapping: TonemapChoice::default(),
            bloom: BloomConfig::default(),
            lod_fade_secs: 0.25,
            materialise_secs: 0.6,
            materialise_start_scale: 0.25,
        }
    }
}

/// Which display transform the viewscreen camera resolves HDR through.
///
/// Mirrors `bevy::core_pipeline::tonemapping::Tonemapping` by name so a designer
/// can name any of them; the mapping to Bevy's own enum lives in
/// [`crate::render_setup`], keeping this module's Bevy surface to the one
/// `Resource` derive it already had.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TonemapChoice {
    /// No display transform — the raw HDR values, clipped.
    None,
    Reinhard,
    ReinhardLuminance,
    AcesFitted,
    AgX,
    SomewhatBoringDisplayTransform,
    #[default]
    TonyMcMapface,
    BlenderFilmic,
}

/// How the viewscreen camera blooms. Requires [`RenderConfig::hdr`]: with the
/// scene clipped at screen white there is nothing above the threshold to bloom.
///
/// The numbers are a THRESHOLD calibration rather than a whole-image scatter,
/// which is the conservative reading of "the emissives were authored for bloom":
/// what should glow is the handful of things authored brighter than white, not
/// every lit hull and every star in the skybox.
///
/// # Where this runs, and where the platform refuses it
///
/// **Bevy 0.18.1's bloom cannot run on WebGL2, which is what every browser
/// target here is** (Key Constraint 9). Two separate upstream facts, both
/// verified against the vendored sources rather than inferred:
///
/// 1. Bloom's downsample chain binds individual mip LEVELS of one texture for
///    sampling, which WebGL2 does not support. `bevy_post_process` carries a
///    fallback that allocates a separate texture per mip — behind its own
///    `webgl` feature — but `bevy_internal`'s `webgl` feature forwards to eight
///    sub-crates and `bevy_post_process` is not among them, so `bevy/webgl2`
///    never turns it on.
/// 2. Turning it on directly does not help: `prepare_bloom_bind_groups` reads
///    `bloom_texture.texture.texture.id()` for its bind-group cache key with no
///    `cfg`, and under that feature `texture` is a `Vec<CachedTexture>`. The
///    fallback path does not compile in 0.18.1.
///
/// That fact is now ENFORCED rather than merely documented:
/// [`BLOOM_RUNS_ON_THIS_TARGET`](crate::render_setup::BLOOM_RUNS_ON_THIS_TARGET)
/// gates the component insertion, so this block is authored the same way on
/// every platform and the platform decides whether the camera gets it. Before
/// that gate existed, a world writing `enabled = true` would have produced a
/// browser viewscreen whose render graph fails, and nothing would have stopped
/// it.
///
/// HDR and the display transform are NOT affected by any of this and ship on
/// everywhere: they are what stop an emissive of 9.0 being drawn as the same
/// flat white as 1.0, which is the PRD's actual complaint. Bloom is the halo on
/// top.
///
/// # The platform matrix, as it actually stands
///
/// | Target | Backend | Bloom |
/// |---|---|---|
/// | `server.html` (the game host) | WebGL2 | no — upstream |
/// | `viewer.html` (`--features viewer`) | WebGL2 | no — upstream |
/// | `capture-billboard`, `tune-lods` | native wgpu | no — by design |
/// | a future native host, or a WebGPU build | full wgpu | yes |
///
/// Worth being plain about: **no shipped target renders natively today.** The
/// game host and the model viewer are both Trunk/WASM pages on WebGL2, and the
/// only native renderers are the two offscreen bakers, which never call
/// [`apply_render_config`](crate::render_setup::apply_render_config) at all and
/// deliberately bake with `Tonemapping::None` — a halo burnt into a billboard
/// atlas would be a defect, not a feature. So the gate changes nothing visible
/// right now; what it does is make the calibration correct by construction the
/// moment any of the three rows below the line becomes real.
///
/// What would put bloom on a screen, in rough order of likelihood: a Bevy
/// release that fixes the cfg gap above; a move to the WebGPU backend; or a
/// native host. None has been filed upstream by this repo.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BloomConfig {
    /// `[ai] ` ON by default, which is a change: it was `false` while the only
    /// thing standing between an authored `true` and a failed render graph was
    /// this comment. Now that
    /// [`BLOOM_RUNS_ON_THIS_TARGET`](crate::render_setup::BLOOM_RUNS_ON_THIS_TARGET)
    /// enforces the platform fact at the component site, the authored default
    /// can say what the calibration was authored to say — the emissive
    /// strengths this makes visible were authored FOR it — and the platform can
    /// answer the separate question of whether it is drawable. On a WebGL2
    /// target that answer is still no, so the observable default is unchanged
    /// wherever anything currently renders.
    ///
    /// Setting it to `false` remains the way to say "this world should not
    /// bloom even where it could", which is now a distinct statement from "this
    /// platform cannot bloom" rather than the same flag doing both jobs.
    pub enabled: bool,
    /// How much scattered light is added back into the image.
    ///
    /// `[ai] ` `0.15`. Bevy's own additive-mode preset uses `0.05`, but that
    /// preset thresholds at `0.6` — well below screen white — so a large part of
    /// an ordinary lit scene passes the filter and a small intensity is already
    /// plenty. This calibration thresholds at `1.0`, so only genuinely
    /// over-white pixels contribute and a comparable amount of glow needs
    /// roughly three times the intensity.
    pub intensity: f32,
    /// How much the widest (most scattered) blur contributes.
    ///
    /// `[ai] ` Bevy's default `0.7`. A weapon impact should throw a wide, soft
    /// halo across the viewscreen, not a tight rim; this is the term that buys
    /// the halo.
    pub low_frequency_boost: f32,
    /// Curvature of the low-frequency blend.
    ///
    /// `[ai] ` Bevy's default `0.95`, unexamined — it shapes the falloff
    /// between the boosted widest blur and the rest, and there is no authored
    /// content that argues for a different shape.
    pub low_frequency_boost_curvature: f32,
    /// How tightly light scatters (`1.0` is the widest scattering angle).
    ///
    /// `[ai] ` Bevy's default `1.0`. Space is empty and dark; there is nothing
    /// for a tighter scatter to protect from being washed out.
    pub high_pass_frequency: f32,
    /// Pixels dimmer than this do not bloom at all.
    ///
    /// `[ai] ` `1.0` — exactly screen white, which is the same number the LDR
    /// target used to clamp at. That is the whole calibration in one value: what
    /// blooms is precisely what the old pipeline threw away. Everything a
    /// designer authored at or below white looks exactly as it did before this
    /// block existed, so adopting bloom cannot quietly restyle the hulls, the
    /// skybox or the dust.
    pub threshold: f32,
    /// How softly the threshold is approached (`0` is a hard cutoff).
    ///
    /// `[ai] ` `0.4`. A hard cutoff at white would make a beam's own falloff
    /// pop into bloom at a visible ring part-way down its length; softening it
    /// spreads the onset over roughly the top 40% below the threshold.
    pub threshold_softness: f32,
    /// Whether bloom textures are blended between (energy-conserving) or added.
    ///
    /// `[ai] ` `additive`. Bevy's own guidance is that a non-zero prefilter
    /// threshold should be paired with additive compositing — energy-conserving
    /// mode assumes the whole image is participating, and with a threshold it is
    /// not.
    pub composite: BloomComposite,
    /// Largest dimension of the bloom mip chain, in pixels.
    ///
    /// `[ai] ` Bevy's default `512`. This is the effect's cost knob: bloom runs
    /// a down/up-sample chain every frame on the browser host's GPU, so it is
    /// the one number here worth reaching for if the viewscreen's frame time
    /// moves after this lands.
    pub max_mip_dimension: u32,
}

impl Default for BloomConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            intensity: 0.15,
            low_frequency_boost: 0.7,
            low_frequency_boost_curvature: 0.95,
            high_pass_frequency: 1.0,
            threshold: 1.0,
            threshold_softness: 0.4,
            composite: BloomComposite::Additive,
            max_mip_dimension: 512,
        }
    }
}

/// How bloom's blurred mips are combined back into the image.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BloomComposite {
    /// Blend between the blurred images — physically motivated, and what Bevy
    /// recommends when nothing is thresholded out.
    EnergyConserving,
    /// Add the scattered light on top. What a thresholded prefilter wants.
    #[default]
    Additive,
}

/// One depth layer of the camera-relative dust field (near / mid / far).
///
/// Authored as `[[dust.layer]]`. Layers are independent emitters sharing the
/// same velocity field; each owns one texture and one material. Ranged fields
/// are `[at_rest, at_full_speed]` pairs interpolated by the speed curve — see
/// [`DustPfxConfig::speed_curve_exponent`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DustLayerConfig {
    /// Human-readable label, for debugging only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Mote texture path, relative to `assets/`. Greyscale-in-alpha; the
    /// renderer tints it at runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    /// Hard cap on live motes in this layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_motes: Option<u32>,
    /// Motes spawned per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_rate: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<[f32; 2]>,
    /// Emissive multiplier. Values above ~1.0 feed bloom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<[f32; 2]>,
    /// Mote width as a fraction of the viewport's **smaller** dimension, **not**
    /// world units — the renderer scales it by the mote's spawn depth so a
    /// layer's apparent size doesn't depend on how far out its `depth_band`
    /// sits, and sizes off `min(width, height)` so it doesn't depend on the
    /// viewport's aspect either. Constant with speed; apparent growth comes
    /// from `length` (spec §7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Streak length as a multiple of `width`. `1.0` renders as a point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<[f32; 2]>,
    /// Upper bound on mote lifetime. The actual lifetime is the time needed to
    /// transit the volume and pass behind the camera; this cap only bites at
    /// low speed, where transit time would otherwise leave motes hanging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lifetime_secs: Option<f32>,
    /// `[min, max]` distance from camera this layer occupies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_band: Option<[f32; 2]>,
    /// `0.0` spawns uniformly across the volume; `1.0` pushes spawns hard
    /// toward the screen edges (spec §13, near layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_bias: Option<f32>,
    /// `true` = additive blending, `false` = alpha blending. Spec §18
    /// recommends alpha for the far layer, additive for mid/near.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additive: Option<bool>,
    /// Optional rare-glint texture for this layer (spec §5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glint_texture: Option<String>,
    /// Fraction of motes (0.0–1.0) drawn with `glint_texture`. Spec §16
    /// suggests 0.01–0.03 for the near layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glint_chance: Option<f32>,
}

/// Transitional warp-speed layer from `[dust.warp]` (spec §14).
///
/// Rather than stretching the ordinary layers indefinitely, impulse swaps in a
/// dedicated high-speed field.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DustWarpConfig {
    /// When false (or the table is absent) impulse leaves the ordinary layers
    /// running and no warp field appears.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motes: Option<u32>,
    /// Fraction of the viewport's smaller dimension, as per
    /// [`DustLayerConfig::width`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Streak length multiplier at full warp, relative to mote width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_multiplier: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f32>,
    /// Seconds to ramp in. Driven by `ImpulseState::charge_progress` while
    /// charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter_secs: Option<f32>,
    /// Seconds to ramp out. Timed render-side: `cancel_charge()` snaps
    /// `Active → Idle` in one frame, so there is no engine-side exit ramp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_secs: Option<f32>,
}

/// Per-world ambient dust particle effect config from `[dust]`.
///
/// A camera-relative velocity field, not world-space particles: speed drives
/// density, luminosity and streak length, while the ship's true velocity
/// vector drives direction.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DustPfxConfig {
    /// Master switch. When false the effect is skipped entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Exponent applied to normalised speed before every ramp lookup. `2.0`
    /// (spec §2's `S²`) keeps the effect restrained at low speed and ramps it
    /// hard under acceleration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_curve_exponent: Option<f32>,
    /// Mote tint at rest — a cool grey-blue (spec §7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_speed_tint: Option<[f32; 3]>,
    /// Mote tint at full speed — near-white.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_speed_tint: Option<[f32; 3]>,
    /// Smoothing time constants (spec §10). Streak length should lead,
    /// brightness follow, density lag — that ordering is what makes
    /// acceleration feel immediate without motes popping in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streak_response_secs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_response_secs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_response_secs: Option<f32>,
    /// Radial screen mask (spec §4), in normalised screen radius from centre.
    /// Fades motes crossing the middle of the viewscreen so they don't
    /// distract from targeting, pushing the streaks into peripheral vision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centre_fade_inner: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centre_fade_outer: Option<f32>,
    /// Fraction of the screen radius over which motes fade before leaving
    /// view, avoiding abrupt clipping (spec §17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_fade: Option<f32>,
    /// Lateral drift added to each mote's velocity, as a fraction of speed.
    /// Keep low — the main direction must stay unambiguous (spec §4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turbulence: Option<f32>,
    /// Scales apparent mote speed relative to true ship speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mote_speed_multiplier: Option<f32>,
    /// Depth layers, authored as `[[dust.layer]]`. When empty the renderer
    /// falls back to its built-in near/mid/far set.
    #[serde(default, rename = "layer", skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<DustLayerConfig>,
    /// Optional `[dust.warp]` table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp: Option<DustWarpConfig>,
}

/// A concrete entity instance declared in the world TOML — a reference to an
/// entity template (under `assets/entities/`) plus instance-level metadata
/// (transform sub-table, spawn timing, optional name for trigger/comms binding,
/// inline overrides).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldEntity {
    /// Path to the entity template TOML (relative to assets/).
    pub template_path: String,
    /// Optional human-readable identifier for this instance.
    #[serde(default)]
    pub id: Option<String>,
    /// Optional named identity for the entity. When present, the entity
    /// becomes trigger- and comms-eligible: `spawn_world_entities` assigns
    /// it a stable UUID and registers `name ? uuid` in `WorldConfig.name_to_uuid`.
    ///
    /// This is the entity's **unique authored reference id**, used only for
    /// world references (triggers, comms, objectives, and qualified
    /// parent/child composition references). It is NOT the player-facing
    /// text — use [`WorldEntity::display_text`] for that. Duplicate `name`s
    /// within one effective namespace are an authoring error, detected by the
    /// composition validator (issue #750).
    #[serde(default)]
    pub name: Option<String>,
    /// Optional player-facing display text, separate from the `name`
    /// reference id (issue #750). When absent, [`WorldEntity::display_text`]
    /// falls back to `name` (which is typically a localization key), so
    /// existing worlds that used `name` for both roles keep working. Authored
    /// data, not a code string — no `strings.csv` entry required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Positioning, rotation and scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<TransformConfig>,
    /// When this entity should be spawned.
    #[serde(default)]
    pub spawn_on: WorldEntitySpawnOn,
    /// Optional inline TOML overrides merged on top of the template.
    #[serde(default)]
    pub overrides: Option<toml::Value>,
    /// Optional raw predicate string parsed at world-load time into `when_predicate`.
    /// When `Some`, the entity is only spawned if the predicate evaluates to
    /// `true` against the world flag/counter store at spawn time.
    ///
    /// Supports the same predicate grammar as trigger `when =` fields:
    /// `flag(name)`, `counter(name) >= N`, `not(...)`, `and(a, b)`, `or(a, b)`.
    #[serde(default)]
    pub when: Option<String>,
    /// Parsed form of `when`. Populated by `parse_world`; `None` when `when`
    /// is absent or the struct was constructed directly (e.g. in tests).
    #[serde(skip)]
    pub when_predicate: Option<crate::world::flags::Predicate>,
}

// -- TOML-facing raw types --------------------------------------------------

/// A single condition-weighted modifier inside an `add_objective` TOML action.
///
/// `pub(crate)` (with `pub(crate)` fields) so it may appear in [`RawActionEntry`]'s
/// (also `pub(crate)`) `modifiers` field without tripping the private-in-public
/// lint AND so the Rhai effect host (`world::script::effects`) can build one from a
/// `#{ … }` script map — the scripted `add_objective` reads its `modifiers` array
/// into these and runs the SHARED [`parse_utility_config`], rather than
/// re-implementing utility parsing (issue #984, Rhai M6). `threshold` / `weight`
/// are `no_float` `f32` leaves the script authors as `flt("…")` or an int.
#[derive(Debug, Deserialize)]
pub(crate) struct RawModifier {
    pub(crate) condition: String,
    #[serde(default)]
    pub(crate) threshold: Option<f32>,
    pub(crate) weight: f32,
}

/// A zero-gate veto condition inside an `add_objective` TOML action.
///
/// `pub(crate)` (with `pub(crate)` fields) for the same reason as [`RawModifier`]:
/// declarative deserialization AND scripted construction from a `#{ … }` map.
#[derive(Debug, Deserialize)]
pub(crate) struct RawZeroGate {
    pub(crate) condition: String,
    #[serde(default)]
    pub(crate) threshold: Option<f32>,
}

/// An objective-contributed Command stance inside an `add_objective` action
/// (issue #1110).
///
/// The authored shape names the target Station and then the ordinary stance
/// fields the target's permanent catalogue authors, e.g.:
///
/// ```toml
/// command_stance = { station = "tactical", id = "objective-escort", \
///                    kind = "standard", high_alert = true, \
///                    persist_behind_human = true, label = "stance.escort" }
/// ```
///
/// `station` is the target Station id; the remaining fields flatten into a
/// [`StationStanceConfig`](crate::ship::config::StationStanceConfig) — the exact
/// type a station's permanent catalogue is authored as — so the contributed
/// stance is validated, exposed and selected through the same seams a permanent
/// one is. `pub(crate)` (with `pub(crate)` fields) for the same reason as
/// [`RawModifier`]: declarative deserialization AND scripted construction from a
/// `#{ … }` Rhai map.
#[derive(Debug, Deserialize)]
pub(crate) struct RawCommandStance {
    pub(crate) station: String,
    #[serde(flatten)]
    pub(crate) stance: crate::ship::config::StationStanceConfig,
}

/// One flat, all-optional `[[trigger.action]]` row as authored in TOML.
///
/// `pub(crate)` (with `pub(crate)` fields and a `Default` derive) so the Rhai
/// effect host (`world::script::effects`) can populate one from a `#{ … }` script
/// map and run it through the SHARED [`parse_action_entry`] — the scripted
/// `add_objective` / `spawn_entity` verbs reuse the exact directive / utility /
/// anchor-XOR validation the declarative front-end applies, rather than
/// re-implementing it (a divergence magnet). `..Default::default()` fills the
/// fields a given script verb does not read, which is why every field must be
/// crate-visible: struct-literal construction needs all fields in scope.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RawActionEntry {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Runtime values to interpolate into `text`'s `{placeholder}` tokens for an
    /// `add_objective` action. `BTreeMap` so the wire encoding it ends up in is
    /// key-ordered and deterministic; see `messages::TEXT_PARAMS_SUFFIX`.
    #[serde(default)]
    pub(crate) text_params: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) mandatory: Option<bool>,
    /// Optional list of entity names to mark on the nav radar for an
    /// `add_objective` action. Each name may reference a real entity
    /// (station, ship) or an invisible `objective_marker` beacon.
    #[serde(default)]
    pub(crate) targets: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) entity: Option<String>,
    #[serde(default)]
    pub(crate) state: Option<String>,
    #[serde(default)]
    pub(crate) target: Option<String>,
    #[serde(default)]
    pub(crate) tag: Option<String>,
    #[serde(default)]
    pub(crate) slot: Option<String>,
    #[serde(default)]
    pub(crate) bonus: Option<f32>,
    #[serde(default)]
    pub(crate) int_bonus: Option<i32>,
    #[serde(default, rename = "kind")]
    pub(crate) flag_kind: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
    /// Declared run outcome for a `game_over` action (#843):
    /// `"victory"` | `"defeat"`, case-insensitive.
    #[serde(default)]
    pub(crate) outcome: Option<String>,
    #[serde(default)]
    pub(crate) path: Option<String>,
    /// Flag name for `set_flag` / `clear_flag` / `increment_flag` / `set_flag_value`.
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Increment delta for `increment_flag`.
    #[serde(default)]
    pub(crate) by: Option<i64>,
    /// Direct counter assignment for `set_flag_value`.
    #[serde(default)]
    pub(crate) value: Option<i64>,
    /// Entity template path for `spawn_entity` action.
    #[serde(default)]
    pub(crate) template_path: Option<String>,
    /// Anchor reference for `spawn_entity` action (mutually exclusive with `position`).
    #[serde(default)]
    pub(crate) anchor: Option<String>,
    /// Explicit `[x, y, z]` for `spawn_entity` action (mutually exclusive with `anchor`).
    #[serde(default)]
    pub(crate) position: Option<[f32; 3]>,
    /// XYZ Euler rotation in radians for `spawn_entity` (optional).
    #[serde(default)]
    pub(crate) rotation: Option<[f32; 3]>,
    /// Per-axis scale for `spawn_entity` (optional).
    #[serde(default)]
    pub(crate) scale: Option<[f32; 3]>,
    /// Faction `name` for `add_faction_enemy` / `remove_faction_enemy`.
    /// Resolved via `FactionRegistry::uuid_by_name` at dispatch time.
    #[serde(default)]
    pub(crate) faction: Option<String>,
    /// Enemy faction `name` for `add_faction_enemy` / `remove_faction_enemy`.
    #[serde(default)]
    pub(crate) enemy: Option<String>,
    // ── add_objective extended fields (issue #571) ─────────────────────────
    /// Directive kind: `"Patrol"`, `"Destroy"`, `"Reach"`, `"Retreat"`,
    /// `"Hail"`, or omit for `None`.
    #[serde(default)]
    pub(crate) directive_kind: Option<String>,
    /// Anchor names for a `Patrol` directive.
    #[serde(default)]
    pub(crate) directive_anchors: Option<Vec<String>>,
    /// Whether a `Patrol` directive loops back to the first anchor.
    #[serde(default)]
    pub(crate) directive_loop: Option<bool>,
    /// Anchor name for a `Reach` or `Retreat` directive.
    #[serde(default)]
    pub(crate) directive_anchor: Option<String>,
    /// Base utility score for the objective (default 0.0).
    #[serde(default)]
    pub(crate) base_priority: Option<f32>,
    /// Objective source: `"mission"` (default) or `"doctrine"`.
    #[serde(default)]
    pub(crate) source: Option<String>,
    /// Condition-weighted score modifiers.
    #[serde(default)]
    pub(crate) modifiers: Option<Vec<RawModifier>>,
    /// Zero-gate veto conditions.
    #[serde(default)]
    pub(crate) zero_gates: Option<Vec<RawZeroGate>>,
    /// An objective-specific Command stance this `add_objective` contributes to a
    /// named target Station while the objective is active (issue #1110).
    #[serde(default)]
    pub(crate) command_stance: Option<RawCommandStance>,
    /// Named groups for `spawn_entity` action. The entity is tracked as a
    /// member of each group and removed from all groups on destruction.
    #[serde(default)]
    pub(crate) groups: Option<Vec<String>>,
    /// Optional inline TOML overrides for `spawn_entity` action, same shape
    /// as the static `[[entity]] overrides` field.
    #[serde(default)]
    pub(crate) overrides: Option<toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableShipEntry {
    pub template_path: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerSpawnEntry {
    #[serde(default)]
    pub anchor: Option<String>,
    #[serde(default)]
    pub position: Option<[f32; 3]>,
    #[serde(default)]
    pub rotation: Option<[f32; 3]>,
}

/// Raw single-pass deserialization of a world TOML.
#[derive(Debug, Default, Deserialize)]
pub struct RawWorld {
    #[serde(default)]
    pub global: GlobalConfig,
    /// Hull-agnostic scenario detail-floor selectors (console-family/Station id
    /// or System kind). Resolved against the lobby-selected hull at runtime.
    #[serde(default)]
    pub scenario_detail_floor: Vec<String>,
    #[serde(default)]
    pub anchors: HashMap<String, Vec<f32>>,
    #[serde(default, rename = "entity")]
    pub entities: Vec<WorldEntity>,
    /// Retired front-ends, kept as opaque values ONLY so `parse_world` can
    /// refuse them by name (issue #985). Without the fields serde would drop a
    /// `[[trigger]]` / `[[comms]]` block on the floor — no `deny_unknown_fields`
    /// here — and a hand-authored world would load with its scenario logic
    /// silently absent. Both are `Vec<toml::Value>`: nothing reads inside them.
    #[serde(default, rename = "trigger")]
    retired_triggers: Vec<toml::Value>,
    #[serde(default, rename = "comms")]
    retired_comms: Vec<toml::Value>,
    /// Named mission deadlines (issue #1024). The field name already matches the
    /// `[[deadline]]` array key, so no rename is needed.
    #[serde(default)]
    pub deadline: Vec<crate::world::deadlines::Deadline>,
    /// Named civilian traffic routes (issue #1028). The field name already
    /// matches the `[[route]]` array key, so no rename is needed.
    #[serde(default)]
    pub route: Vec<crate::civilian::RouteConfig>,
    /// The sides of a labour dispute (issue #1035). The field name already
    /// matches the `[[workforce]]` array key, so no rename is needed.
    #[serde(default)]
    pub workforce: Vec<crate::world::workforce::Workforce>,
    /// Paths to additional world TOML files to load additively at startup.
    #[serde(default)]
    pub extra_worlds: Vec<String>,
    /// Policy for pending delayed actions when this world layer unloads
    /// (issue #751): `"cancel"` (default) drops them, `"resolve"` dispatches
    /// them immediately. Case-insensitive; unknown values fall back to cancel.
    #[serde(default)]
    pub delayed_unload_policy: Option<String>,
    /// Optional world-level ambient light override.
    #[serde(default)]
    pub ambient_light: Option<AmbientLightConfig>,
    /// Optional world-level viewscreen presentation settings (PRD #1023).
    #[serde(default)]
    pub render: Option<RenderConfig>,
    /// Optional world-level audio (red-alert siren + music). Every other
    /// sound is configured on the ship entity instead.
    #[serde(default)]
    pub audio: Option<crate::audio_config::WorldAudioConfig>,
    /// Optional ambient dust particle effect config.
    #[serde(default)]
    pub dust: Option<DustPfxConfig>,
    /// List of selectable player ship options for this world.
    #[serde(default)]
    pub available_ships: Vec<AvailableShipEntry>,
    /// Optional spawn point for the player ship.
    #[serde(default)]
    pub player_spawn: Option<PlayerSpawnEntry>,
    /// The raw `[script]` block. Deserialized here — rather than left to
    /// `world::script::load::lift_world_scripts`, which still owns activation —
    /// only so `parse_world` can retain the INLINE Rhai bodies for
    /// [`entity_template_paths`] to scan (issue #984).
    #[serde(default)]
    pub script: Option<toml::Value>,
}

// -- Trigger / comms pure config types --------------------------------------

/// Authored policy for what happens to a world layer's pending delayed
/// actions when the layer is unloaded (issue #751).
///
/// Delayed actions queued by a layer's triggers (`action_delays > 0.0`) may
/// still be in flight when the layer unloads. This policy decides their fate.
/// The default is [`DelayedUnloadPolicy::Cancel`] so existing worlds — which
/// never authored the field — drop in-flight work on unload, matching the
/// "owned content is removed" lifecycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DelayedUnloadPolicy {
    /// Drop the layer's pending delayed actions (default).
    #[default]
    Cancel,
    /// Resolve the layer's pending delayed actions immediately on unload
    /// (dispatch them on the next delayed-action tick instead of waiting for
    /// their scheduled fire time).
    Resolve,
}

/// A condition that a trigger can check against incoming world events.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerCondition {
    /// Fires when the named entity (by name, resolved to UUID at runtime) is destroyed.
    OnDestroyed { entity_name: String },
    /// Fires when every entity in a named group has been destroyed.
    ///
    /// Group membership is tracked dynamically via `entity_groups` in
    /// `WorldContentRuntime` — entities spawn into groups via
    /// `SpawnEntity { groups: [...] }`, and are removed when destroyed.
    /// `after_secs` is a minimum time gate; the trigger cannot fire before
    /// that many seconds have elapsed (default 0.0 = no minimum).
    OnAllDestroyed { group: String, after_secs: f32 },
    /// Fires when the named entity is attacked.
    OnAttacked { entity_name: String },
    /// Fires when the named entity's aggregate hull fraction crosses strictly
    /// below the authored threshold.
    OnHullBelow { entity_name: String, threshold: f32 },
    /// Fires once when `elapsed_secs` crosses `after_secs`.
    OnTimer { after_secs: f32 },
    /// Fires when a `Hail` message arrives for the named entity.
    OnHailed { entity_name: String },
    /// Fires on the TRANSITION false→true of a named flag.
    OnFlagSet { name: String },
    /// Fires on the TRANSITION true→false of a named flag.
    OnFlagCleared { name: String },
    /// Fires once when the world containing this trigger finishes loading
    /// (base-world `Startup` or sub-world `LoadWorld`). Single-shot per
    /// trigger lifecycle; on unload + re-load the trigger is re-created
    /// with `fired = false` so it fires again.
    OnWorldLoaded,
    /// Fires when the player ship enters the named region.
    OnEnteredRegion { entity_name: String },
    /// Fires when the player ship exits the named region (or the region
    /// is despawned while the ship is inside).
    OnExitedRegion { entity_name: String },
    /// Fires when the named ship reaches a waypoint on the Patrol/Reach
    /// objective it is currently following.
    ///
    /// `waypoint` names a specific anchor on the route; when `None` the
    /// trigger fires on arrival at *any* waypoint of that ship's route.
    OnWaypointReached {
        entity_name: String,
        waypoint: Option<String>,
    },
}

/// An action to execute when a trigger fires.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerAction {
    AddObjective {
        id: String,
        text: String,
        /// Runtime values interpolated into `text`'s `{placeholder}` tokens by
        /// the client. Authored as an optional `text_params` map on the
        /// `add_objective` spec; empty when the objective names a figure-free
        /// string. See `messages::TEXT_PARAMS_SUFFIX`.
        text_params: std::collections::BTreeMap<String, String>,
        mandatory: bool,
        targets: Vec<String>,
        /// AI directive attached to this objective (issue #571).
        directive: AiDirective,
        /// Utility scoring configuration (issue #571).
        utility: UtilityConfig,
        /// Whether this is a mission objective or standing doctrine (issue #571).
        source: ObjectiveSource,
        /// An objective-specific Command stance contributed to a named target
        /// Station while this objective is active (issue #1110). `None` for the
        /// vast majority of objectives, which contribute no stance. The tuple is
        /// (target Station id, authored stance).
        command_stance: Option<(
            crate::core::messages::StationId,
            crate::ship::config::StationStanceConfig,
        )>,
    },
    CompleteObjective {
        id: String,
    },
    FailObjective {
        id: String,
    },
    SetAiState {
        entity: String,
        state: String,
        target: Option<String>,
    },
    ApplyModifier {
        entity: String,
        tag: String,
        slot: crate::core::messages::ModifierSlot,
        bonus: f32,
    },
    RemoveModifier {
        entity: String,
        tag: String,
        slot: crate::core::messages::ModifierSlot,
    },
    ApplyFlag {
        entity: String,
        tag: String,
        kind: crate::core::messages::FlagKind,
    },
    RemoveFlag {
        entity: String,
        tag: String,
        kind: crate::core::messages::FlagKind,
    },
    ApplyIntModifier {
        entity: String,
        tag: String,
        slot: crate::modifiers::IntModifierSlot,
        bonus: i32,
    },
    RemoveIntModifier {
        entity: String,
        tag: String,
        slot: crate::modifiers::IntModifierSlot,
    },
    GameOver {
        message: Option<String>,
        /// Declared run outcome (#843). Parsed from the case-insensitive
        /// `outcome = "victory" | "defeat"` field. `None` when the author left
        /// it off — the headless classifier defaults an undeclared scripted
        /// game-over to victory (the ship ran to a scripted end-state; the
        /// built-in player-death path is separately latched as defeat).
        outcome: Option<crate::core::balance::Outcome>,
    },
    /// Additively load a sub-world from `path` into the running world layer map.
    LoadWorld {
        path: String,
    },
    /// Unload a previously loaded sub-world identified by `path`.
    UnloadWorld {
        path: String,
    },
    /// Set a world flag to true (counter = 1).
    SetWorldFlag {
        name: String,
    },
    /// Clear a world flag to false (counter = 0).
    ClearWorldFlag {
        name: String,
    },
    /// Increment a world flag counter by `by` (can be negative).
    IncrementWorldFlag {
        name: String,
        by: i64,
    },
    /// Assign a world flag counter directly to `value`.
    SetWorldFlagValue {
        name: String,
        value: i64,
    },
    /// Spawn an entity ad-hoc, registering it in `name_to_uuid` under `name`.
    ///
    /// Exactly one of `anchor` or `position` must be `Some` (enforced by the
    /// parser). `rotation` and `scale` mirror the static `[[entity]]` schema.
    /// When fired from a sub-world layer the spawned entity is tracked in
    /// `WorldLayerMap` so `UnloadWorld` despawns it.
    SpawnEntity {
        template_path: String,
        name: String,
        anchor: Option<String>,
        position: Option<[f32; 3]>,
        rotation: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
        /// Named groups this entity belongs to. The entity is automatically
        /// removed from each group when it is destroyed, enabling
        /// `OnAllDestroyed { group }` triggers to track dynamic membership.
        groups: Vec<String>,
        /// Optional inline TOML overrides merged on top of the template,
        /// mirroring the static `[[entity]] overrides` field. Uses the same
        /// by-id merge semantics for `behaviour.doctrine` arrays.
        overrides: Option<toml::Value>,
    },
    /// Destroy an entity by `name` (looked up in `name_to_uuid`).
    ///
    /// Emits `AiEntityDestroyed` so the normal destruction cascade runs and
    /// despawns the underlying entity.
    DestroyEntity {
        entity: String,
    },
    /// Add `enemy` to `faction`'s enemies list in the live
    /// `FactionRegistry`. Both fields are faction `name` strings
    /// (e.g. `"Harrow"`, `"Federation"`) and are resolved to UUIDs via
    /// `FactionRegistry::uuid_by_name` at dispatch time.
    ///
    /// `is_enemy(a, b)` is asymmetric, so flipping a relationship in both
    /// directions requires two actions. Idempotent — listing the same
    /// enemy twice is a no-op. Unknown faction names log a warning and
    /// skip the action.
    ///
    /// Used by scenarios that need to make an otherwise-neutral faction
    /// hostile (e.g. `assets/worlds/combat_test.toml` arms the
    /// Federation<->Harrow rivalry on world load).
    AddFactionEnemy {
        faction: String,
        enemy: String,
    },
    /// Remove `enemy` from `faction`'s enemies list in the live
    /// `FactionRegistry`. Mirror of `AddFactionEnemy` — same name-lookup
    /// semantics, same asymmetric model, idempotent.
    ///
    /// After removal, the dispatcher re-validates all AI controllers'
    /// blackboard targets: any controller whose `target` faction is no
    /// longer hostile to the controller's own faction has its `target`
    /// cleared so an in-progress engagement does not stick on a now-
    /// friendly entity.
    RemoveFactionEnemy {
        faction: String,
        enemy: String,
    },
    /// Re-arm a previously-fired trigger identified by its authored `id`
    /// (issue #751). Clears the target trigger's `fired` flag (and its
    /// `seen_destroyed` accumulation / cooldown clock) so it can fire again.
    /// Unknown ids are a no-op.
    ResetTrigger {
        id: String,
    },
}

/// A single trigger: a condition, its lifecycle policy, and an optional
/// predicate gate.
///
/// It carried the ordered `actions` a fire dispatched, plus the parallel
/// `action_predicates` and `action_delays` that gated and deferred them
/// per-action. All three were written by the `[[trigger.action]]` parser and
/// nothing else — a scripted trigger has always been built with them empty
/// (`scripted_trigger`), because its effects come from running its handler fn.
/// Issue #985 deleted that parser, so the three fields had no writer left and
/// the pipeline's per-action dispatch loop had nothing to dispatch. A scripted
/// handler defers with `ctx.schedule.in_seconds(..)` and gates with ordinary
/// Rhai control flow.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub condition: TriggerCondition,
    /// Optional predicate gate. When `Some`, the predicate is evaluated
    /// against the world flag store before each firing; a `false` result
    /// suppresses actions for that firing but does NOT consume the trigger
    /// lifecycle (the `fired` flag stays unset).
    pub when: Option<crate::world::flags::Predicate>,
    /// Optional authored id (issue #751). Anonymous triggers leave this
    /// `None`; a named trigger can be re-armed by a `ResetTrigger { id }`
    /// action referencing this id.
    pub id: Option<String>,
    /// Trigger lifecycle policy (issue #751). `false` (default) = once-only
    /// single-shot: the trigger fires at most once. `true` = repeatable: the
    /// trigger re-arms after firing and fires again whenever its condition
    /// holds, subject to `cooldown_secs`. Backward compatible — existing
    /// worlds omit the field and stay single-shot.
    pub repeat: bool,
    /// Minimum seconds between successive fires of a `repeat` trigger,
    /// measured on the world-elapsed clock (issue #751). `None` = no cooldown
    /// (may re-fire every tick its condition holds). Ignored for once-only
    /// triggers.
    pub cooldown_secs: Option<f32>,
}

// -- Parser helpers -----------------------------------------------------------

/// Which directive kind reads each `add_objective` directive field, for error
/// messages.
///
/// The mission-side twin of `DIRECTIVE_FIELD_OWNERS` in
/// `src/entities/config.rs`. One shape difference: a mission objective names its
/// target-naming directives (`Destroy`/`Hail`/`Dock` and the issue-#1162 operate
/// verbs `Tow`/`Stabilise`/`Escort`/`Transfer`/`FieldRepair`) with the ONE
/// shared `target` field, where a doctrine entry has a `directive_target`, a
/// `directive_hail_target`, a `directive_dock_target` and a
/// `directive_operate_target` of its own. Both tables carry every directive kind
/// — `Dock` used to be here on the doctrine side only, which this fixes.
const DIRECTIVE_FIELD_OWNERS: &[(&str, &str)] = &[
    ("directive_anchors", "Patrol"),
    ("directive_loop", "Patrol"),
    ("directive_anchor", "Reach / Retreat"),
    (
        "target",
        "Destroy / Hail / Dock / Tow / Stabilise / Escort / Transfer / FieldRepair",
    ),
];

/// Build the [`AiDirective`] for an `add_objective` action, rejecting a
/// directive that authors a field belonging to a *different* kind as well as one
/// that omits a field its own kind requires.
///
/// # Why the misplaced-field half exists
///
/// Each arm below reads exactly one field and ignores the rest, so authoring the
/// neighbouring kind's field does nothing at all and says nothing about it. That
/// is the same silent-nothing failure that lost the Requiem Courier its only
/// goal on the entity side (`validate_doctrine_directives`,
/// `src/entities/config.rs`) — `directive_anchors` (the **Patrol** field,
/// plural) on a `Reach`, which reads the singular `directive_anchor`. A mission
/// `add_objective` can author the identical mistake, so it gets the identical
/// treatment: the world fails to parse rather than activating an objective that
/// can never fire.
///
/// Absent-vs-default is the limit of what this can see, matching the entity
/// side: `directive_loop = false` and `directive_anchors = []` carry no intent
/// worth reporting, so only a field with a real value counts as authored.
///
/// `targets` (plural — the nav-radar marker list) is *not* a directive field and
/// is legitimate alongside any kind; only the singular `target` is checked.
fn parse_directive(raw: &RawActionEntry) -> Result<AiDirective, String> {
    let kind = raw.directive_kind.as_deref();
    let allowed: &[&str] = match kind {
        None | Some("None") => &[],
        Some("Patrol") => &["directive_anchors", "directive_loop"],
        // Every target-naming directive reads the ONE shared `target` field on
        // the mission side, including `Dock` (issue #1028) and the issue-#1162
        // operate verbs.
        Some("Destroy") | Some("Hail") | Some("Dock") | Some("Tow") | Some("Stabilise")
        | Some("Escort") | Some("Transfer") | Some("FieldRepair") => &["target"],
        Some("Reach") | Some("Retreat") => &["directive_anchor"],
        Some(other) => {
            return Err(format!(
                "Unknown directive_kind '{}'; valid: Patrol, Destroy, Reach, Retreat, Hail, \
                 Dock, Tow, Stabilise, Escort, Transfer, FieldRepair",
                other
            ))
        }
    };

    // Fields the author actually filled in with a value.
    let authored: Vec<&str> = [
        raw.directive_anchors
            .as_deref()
            .is_some_and(|a| !a.is_empty())
            .then_some("directive_anchors"),
        raw.directive_loop
            .unwrap_or(false)
            .then_some("directive_loop"),
        raw.directive_anchor
            .as_deref()
            .is_some_and(|a| !a.is_empty())
            .then_some("directive_anchor"),
        raw.target
            .as_deref()
            .is_some_and(|t| !t.is_empty())
            .then_some("target"),
    ]
    .into_iter()
    .flatten()
    .collect();

    // Misplaced fields are reported before missing ones, for the same reason as
    // on the entity side: when both are true at once, "you set
    // `directive_anchors` on a Reach" tells the author far more than "a Reach
    // needs a `directive_anchor`".
    for field in &authored {
        if allowed.contains(field) {
            continue;
        }
        let owner = DIRECTIVE_FIELD_OWNERS
            .iter()
            .find(|(f, _)| f == field)
            .map(|(_, owner)| *owner)
            .unwrap_or("no");
        let reads = if allowed.is_empty() {
            "no directive field".to_string()
        } else {
            allowed
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(" / ")
        };
        return Err(match kind {
            Some(k) => format!(
                "Action 'add_objective': `{field}` is read only for a {owner} directive, but \
                 directive_kind = \"{k}\", which reads {reads}. A field belonging to another \
                 directive kind is silently ignored, so it is rejected here instead."
            ),
            None => format!(
                "Action 'add_objective': `{field}` is read only for a {owner} directive, but no \
                 directive_kind is authored, so nothing reads it."
            ),
        });
    }

    match kind {
        None | Some("None") => Ok(AiDirective::None),
        Some("Patrol") => Ok(AiDirective::Patrol {
            anchors: raw.directive_anchors.clone().unwrap_or_default(),
            loop_path: raw.directive_loop.unwrap_or(false),
        }),
        Some("Destroy") => Ok(AiDirective::Destroy {
            target: raw
                .target
                .clone()
                .ok_or_else(|| "Directive 'Destroy' requires a 'target' field".to_string())?,
        }),
        Some("Reach") => Ok(AiDirective::Reach {
            anchor: raw.directive_anchor.clone().ok_or_else(|| {
                "Directive 'Reach' requires a 'directive_anchor' field".to_string()
            })?,
        }),
        Some("Retreat") => Ok(AiDirective::Retreat {
            anchor: raw.directive_anchor.clone().ok_or_else(|| {
                "Directive 'Retreat' requires a 'directive_anchor' field".to_string()
            })?,
        }),
        Some("Hail") => Ok(AiDirective::Hail {
            target: raw
                .target
                .clone()
                .ok_or_else(|| "Directive 'Hail' requires a 'target' field".to_string())?,
        }),
        // Dock (issue #1028) named the mission side's `target`, but the mission
        // parser never carried it — the "out of step over Dock" the #1162
        // decision fixes.
        Some("Dock") => Ok(AiDirective::Dock {
            target: raw
                .target
                .clone()
                .ok_or_else(|| "Directive 'Dock' requires a 'target' field".to_string())?,
        }),
        // The issue-#1162 operate verbs, each naming its `target` the same way.
        Some("Tow") => Ok(AiDirective::Tow {
            target: raw
                .target
                .clone()
                .ok_or_else(|| "Directive 'Tow' requires a 'target' field".to_string())?,
        }),
        Some("Stabilise") => Ok(AiDirective::Stabilise {
            target: raw
                .target
                .clone()
                .ok_or_else(|| "Directive 'Stabilise' requires a 'target' field".to_string())?,
        }),
        Some("Escort") => Ok(AiDirective::Escort {
            target: raw
                .target
                .clone()
                .ok_or_else(|| "Directive 'Escort' requires a 'target' field".to_string())?,
        }),
        Some("Transfer") => Ok(AiDirective::Transfer {
            target: raw
                .target
                .clone()
                .ok_or_else(|| "Directive 'Transfer' requires a 'target' field".to_string())?,
        }),
        Some("FieldRepair") => Ok(AiDirective::FieldRepair {
            target: raw
                .target
                .clone()
                .ok_or_else(|| "Directive 'FieldRepair' requires a 'target' field".to_string())?,
        }),
        // Unreachable: the `allowed` match above already returned for an
        // unknown kind.
        Some(other) => Err(format!(
            "Unknown directive_kind '{}'; valid: Patrol, Destroy, Reach, Retreat, Hail, \
             Dock, Tow, Stabilise, Escort, Transfer, FieldRepair",
            other
        )),
    }
}

fn parse_utility_config(raw: &RawActionEntry) -> UtilityConfig {
    let modifiers = raw
        .modifiers
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|m| ConditionModifier {
            condition: m.condition.clone(),
            threshold: m.threshold,
            weight: m.weight,
        })
        .collect();
    let zero_gates = raw
        .zero_gates
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|g| ZeroGateCondition {
            condition: g.condition.clone(),
            threshold: g.threshold,
        })
        .collect();
    UtilityConfig {
        base_priority: raw.base_priority.unwrap_or(0.0),
        modifiers,
        zero_gates,
    }
}

/// Convert an `add_objective` action's optional `command_stance` (issue #1110)
/// into the (target Station id, authored stance) pair the objective carries.
///
/// The declarative and scripted front-ends share this, so a malformed
/// contribution is refused identically from both — the whole `add_objective`
/// call is discarded (settled decision 10), never a half-registered objective.
/// `kind` and `id` are already required by [`StationStanceConfig`]'s
/// deserialization; here the two fields serde cannot check for emptiness are
/// rejected: a blank target `station` (there would be nothing to lend the stance
/// to) and a blank stance `id` (Command keys selections by id).
fn parse_command_stance(
    raw: &RawActionEntry,
) -> Result<
    Option<(
        crate::core::messages::StationId,
        crate::ship::config::StationStanceConfig,
    )>,
    String,
> {
    let Some(raw_stance) = &raw.command_stance else {
        return Ok(None);
    };
    if raw_stance.station.trim().is_empty() {
        return Err(
            "Action 'add_objective' command_stance requires a non-empty 'station'".to_string(),
        );
    }
    if raw_stance.stance.id.trim().is_empty() {
        return Err(
            "Action 'add_objective' command_stance requires a non-empty stance 'id'".to_string(),
        );
    }
    Ok(Some((
        crate::core::messages::StationId(raw_stance.station.clone()),
        raw_stance.stance.clone(),
    )))
}

fn parse_modifier_slot(s: &str) -> Result<crate::core::messages::ModifierSlot, String> {
    use crate::core::messages::ModifierSlot;
    match s {
        "MaxSpeed" => Ok(ModifierSlot::MaxSpeed),
        "MaxYawRate" => Ok(ModifierSlot::MaxYawRate),
        "RadarRange" => Ok(ModifierSlot::RadarRange),
        "PhaserDamage" => Ok(ModifierSlot::PhaserDamage),
        "HullDamageTaken" => Ok(ModifierSlot::HullDamageTaken),
        "RepairRate" => Ok(ModifierSlot::RepairRate),
        "HelmRadarRange" => Ok(ModifierSlot::HelmRadarRange),
        "SensorRadarRange" => Ok(ModifierSlot::SensorRadarRange),
        other => Err(format!("Unknown slot '{}'; valid values: MaxSpeed, MaxYawRate, RadarRange, PhaserDamage, HullDamageTaken, RepairRate, HelmRadarRange, SensorRadarRange", other)),
    }
}

fn parse_int_modifier_slot(s: &str) -> Result<crate::modifiers::IntModifierSlot, String> {
    use crate::modifiers::IntModifierSlot;
    match s {
        "RepairTeams" => Ok(IntModifierSlot::RepairTeams),
        other => Err(format!(
            "Unknown int slot '{}'; valid values: RepairTeams",
            other
        )),
    }
}

fn parse_flag_kind(s: &str) -> Result<crate::core::messages::FlagKind, String> {
    use crate::core::messages::FlagKind;
    match s {
        "CommsJammed" => Ok(FlagKind::CommsJammed),
        "SensorBlind" => Ok(FlagKind::SensorBlind),
        other => Err(format!(
            "Unknown kind '{}'; valid values: CommsJammed, SensorBlind",
            other
        )),
    }
}

/// Parse ONE `[[trigger.action]]` row into a `TriggerAction`.
///
/// Factored out of [`parse_raw_actions`] so the Rhai effect host
/// (`world::script::effects`) can drive the SAME per-type parse — directive
/// validation, utility config, the `spawn_entity` anchor/position XOR — from a
/// script `#{ … }` map rather than re-implementing it (issue #984, Rhai M6). The
/// declarative loop and the scripted `add_objective` / `spawn_entity` verbs are
/// then two front-ends over one parser. `when` / `delay_secs` are row-level
/// scheduling metadata [`parse_raw_actions`] reads separately, not part of the
/// action itself.
pub(crate) fn parse_action_entry(raw_action: &RawActionEntry) -> Result<TriggerAction, String> {
    let action =
        match raw_action.kind.as_str() {
            "add_objective" => {
                let directive = parse_directive(raw_action)?;
                let utility = parse_utility_config(raw_action);
                let source = match raw_action.source.as_deref() {
                    Some("doctrine") => ObjectiveSource::Doctrine,
                    _ => ObjectiveSource::Mission,
                };
                let command_stance = parse_command_stance(raw_action)?;
                TriggerAction::AddObjective {
                    id: raw_action.id.clone().ok_or_else(|| {
                        "Action 'add_objective' requires an 'id' field".to_string()
                    })?,
                    text: raw_action.text.clone().ok_or_else(|| {
                        "Action 'add_objective' requires a 'text' field".to_string()
                    })?,
                    text_params: raw_action.text_params.clone().unwrap_or_default(),
                    mandatory: raw_action.mandatory.unwrap_or(false),
                    targets: raw_action.targets.clone().unwrap_or_default(),
                    directive,
                    utility,
                    source,
                    command_stance,
                }
            }
            "complete_objective" => TriggerAction::CompleteObjective {
                id: raw_action.id.clone().ok_or_else(|| {
                    "Action 'complete_objective' requires an 'id' field".to_string()
                })?,
            },
            "fail_objective" => TriggerAction::FailObjective {
                id: raw_action
                    .id
                    .clone()
                    .ok_or_else(|| "Action 'fail_objective' requires an 'id' field".to_string())?,
            },
            "set_ai_state" => TriggerAction::SetAiState {
                entity: raw_action.entity.clone().ok_or_else(|| {
                    "Action 'set_ai_state' requires an 'entity' field".to_string()
                })?,
                state: raw_action
                    .state
                    .clone()
                    .ok_or_else(|| "Action 'set_ai_state' requires a 'state' field".to_string())?,
                target: raw_action.target.clone(),
            },
            "apply_modifier" => {
                let slot_str = raw_action
                    .slot
                    .as_deref()
                    .ok_or_else(|| "Action 'apply_modifier' requires a 'slot' field".to_string())?;
                TriggerAction::ApplyModifier {
                    entity: raw_action.entity.clone().ok_or_else(|| {
                        "Action 'apply_modifier' requires an 'entity' field".to_string()
                    })?,
                    tag: raw_action.tag.clone().ok_or_else(|| {
                        "Action 'apply_modifier' requires a 'tag' field".to_string()
                    })?,
                    slot: parse_modifier_slot(slot_str)?,
                    bonus: raw_action.bonus.ok_or_else(|| {
                        "Action 'apply_modifier' requires a 'bonus' field".to_string()
                    })?,
                }
            }
            "remove_modifier" => {
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| {
                    "Action 'remove_modifier' requires a 'slot' field".to_string()
                })?;
                TriggerAction::RemoveModifier {
                    entity: raw_action.entity.clone().ok_or_else(|| {
                        "Action 'remove_modifier' requires an 'entity' field".to_string()
                    })?,
                    tag: raw_action.tag.clone().ok_or_else(|| {
                        "Action 'remove_modifier' requires a 'tag' field".to_string()
                    })?,
                    slot: parse_modifier_slot(slot_str)?,
                }
            }
            "apply_flag" => {
                let kind_str = raw_action
                    .flag_kind
                    .as_deref()
                    .ok_or_else(|| "Action 'apply_flag' requires a 'kind' field".to_string())?;
                TriggerAction::ApplyFlag {
                    entity: raw_action.entity.clone().ok_or_else(|| {
                        "Action 'apply_flag' requires an 'entity' field".to_string()
                    })?,
                    tag: raw_action
                        .tag
                        .clone()
                        .ok_or_else(|| "Action 'apply_flag' requires a 'tag' field".to_string())?,
                    kind: parse_flag_kind(kind_str)?,
                }
            }
            "remove_flag" => {
                let kind_str = raw_action
                    .flag_kind
                    .as_deref()
                    .ok_or_else(|| "Action 'remove_flag' requires a 'kind' field".to_string())?;
                TriggerAction::RemoveFlag {
                    entity: raw_action.entity.clone().ok_or_else(|| {
                        "Action 'remove_flag' requires an 'entity' field".to_string()
                    })?,
                    tag: raw_action
                        .tag
                        .clone()
                        .ok_or_else(|| "Action 'remove_flag' requires a 'tag' field".to_string())?,
                    kind: parse_flag_kind(kind_str)?,
                }
            }
            "apply_int_modifier" => {
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| {
                    "Action 'apply_int_modifier' requires a 'slot' field".to_string()
                })?;
                TriggerAction::ApplyIntModifier {
                    entity: raw_action.entity.clone().ok_or_else(|| {
                        "Action 'apply_int_modifier' requires an 'entity' field".to_string()
                    })?,
                    tag: raw_action.tag.clone().ok_or_else(|| {
                        "Action 'apply_int_modifier' requires a 'tag' field".to_string()
                    })?,
                    slot: parse_int_modifier_slot(slot_str)?,
                    bonus: raw_action.int_bonus.ok_or_else(|| {
                        "Action 'apply_int_modifier' requires an 'int_bonus' field".to_string()
                    })?,
                }
            }
            "remove_int_modifier" => {
                let slot_str = raw_action.slot.as_deref().ok_or_else(|| {
                    "Action 'remove_int_modifier' requires a 'slot' field".to_string()
                })?;
                TriggerAction::RemoveIntModifier {
                    entity: raw_action.entity.clone().ok_or_else(|| {
                        "Action 'remove_int_modifier' requires an 'entity' field".to_string()
                    })?,
                    tag: raw_action.tag.clone().ok_or_else(|| {
                        "Action 'remove_int_modifier' requires a 'tag' field".to_string()
                    })?,
                    slot: parse_int_modifier_slot(slot_str)?,
                }
            }
            "game_over" => TriggerAction::GameOver {
                message: raw_action.message.clone(),
                // Validate at parse time: an unknown value fails the world
                // load loudly rather than silently mis-classifying the run.
                outcome: raw_action
                    .outcome
                    .as_deref()
                    .map(crate::core::balance::Outcome::parse)
                    .transpose()
                    .map_err(|e| format!("Action 'game_over' has an invalid outcome: {e}"))?,
            },
            "load_world" => TriggerAction::LoadWorld {
                path: raw_action
                    .path
                    .clone()
                    .ok_or_else(|| "Action 'load_world' requires a 'path' field".to_string())?,
            },
            "unload_world" => TriggerAction::UnloadWorld {
                path: raw_action
                    .path
                    .clone()
                    .ok_or_else(|| "Action 'unload_world' requires a 'path' field".to_string())?,
            },
            "reset_trigger" => TriggerAction::ResetTrigger {
                id: raw_action
                    .id
                    .clone()
                    .ok_or_else(|| "Action 'reset_trigger' requires an 'id' field".to_string())?,
            },
            "set_flag" => TriggerAction::SetWorldFlag {
                name: raw_action
                    .name
                    .clone()
                    .ok_or_else(|| "Action 'set_flag' requires a 'name' field".to_string())?,
            },
            "clear_flag" => TriggerAction::ClearWorldFlag {
                name: raw_action
                    .name
                    .clone()
                    .ok_or_else(|| "Action 'clear_flag' requires a 'name' field".to_string())?,
            },
            "increment_flag" => TriggerAction::IncrementWorldFlag {
                name: raw_action
                    .name
                    .clone()
                    .ok_or_else(|| "Action 'increment_flag' requires a 'name' field".to_string())?,
                by: raw_action
                    .by
                    .ok_or_else(|| "Action 'increment_flag' requires a 'by' field".to_string())?,
            },
            "set_flag_value" => TriggerAction::SetWorldFlagValue {
                name: raw_action
                    .name
                    .clone()
                    .ok_or_else(|| "Action 'set_flag_value' requires a 'name' field".to_string())?,
                value: raw_action.value.ok_or_else(|| {
                    "Action 'set_flag_value' requires a 'value' field".to_string()
                })?,
            },
            "spawn_entity" => {
                let template_path = raw_action.template_path.clone().ok_or_else(|| {
                    "Action 'spawn_entity' requires a 'template_path' field".to_string()
                })?;
                let name = raw_action
                    .name
                    .clone()
                    .ok_or_else(|| "Action 'spawn_entity' requires a 'name' field".to_string())?;
                let has_anchor = raw_action.anchor.is_some();
                let has_position = raw_action.position.is_some();
                if has_anchor && has_position {
                    return Err(
                        "Action 'spawn_entity' must not set both 'anchor' and 'position'"
                            .to_string(),
                    );
                }
                if !has_anchor && !has_position {
                    return Err(
                        "Action 'spawn_entity' requires exactly one of 'anchor' or 'position'"
                            .to_string(),
                    );
                }
                TriggerAction::SpawnEntity {
                    template_path,
                    name,
                    anchor: raw_action.anchor.clone(),
                    position: raw_action.position,
                    rotation: raw_action.rotation,
                    scale: raw_action.scale,
                    groups: raw_action.groups.clone().unwrap_or_default(),
                    overrides: raw_action.overrides.clone(),
                }
            }
            "destroy_entity" => TriggerAction::DestroyEntity {
                entity: raw_action.entity.clone().ok_or_else(|| {
                    "Action 'destroy_entity' requires an 'entity' field".to_string()
                })?,
            },
            "add_faction_enemy" => TriggerAction::AddFactionEnemy {
                faction: raw_action.faction.clone().ok_or_else(|| {
                    "Action 'add_faction_enemy' requires a 'faction' field".to_string()
                })?,
                enemy: raw_action.enemy.clone().ok_or_else(|| {
                    "Action 'add_faction_enemy' requires an 'enemy' field".to_string()
                })?,
            },
            "remove_faction_enemy" => TriggerAction::RemoveFactionEnemy {
                faction: raw_action.faction.clone().ok_or_else(|| {
                    "Action 'remove_faction_enemy' requires a 'faction' field".to_string()
                })?,
                enemy: raw_action.enemy.clone().ok_or_else(|| {
                    "Action 'remove_faction_enemy' requires an 'enemy' field".to_string()
                })?,
            },
            other => return Err(format!("Unknown trigger action '{}'", other)),
        };
    Ok(action)
}

/// Build the [`Trigger`] a scripted front-end produces (issue #980, M2): the
/// given `condition` with every lifecycle field at its default. The handler fn
/// supplies the effects.
///
/// It was the single canonical constructor the two scripted front-ends shared —
/// the TOML `[[trigger]] script = "fn"` path and the Rhai registration fns — so
/// a scripted trigger was byte-for-byte the same struct whichever authored it.
/// Issue #985 deleted the TOML half; the Rhai registration fns
/// (`crate::world::script::triggers`) are the only caller now, and they leave
/// every lifecycle field at its default.
pub(crate) fn scripted_trigger(condition: TriggerCondition) -> Trigger {
    Trigger {
        condition,
        when: None,
        id: None,
        repeat: false,
        cooldown_secs: None,
    }
}

// -- Public typed config ----------------------------------------------------

/// Parsed unified world configuration.
///
/// Carries the anchor table and the `[[entity]]` instances. Anchors are
/// normalised to fixed-size `[f32; 3]` arrays at parse time so downstream
/// consumers (e.g. AI patrol path lookups, region positioning) don't have to
/// re-validate length on every read.
#[derive(Clone, Debug, Default, bevy::prelude::Resource)]
pub struct WorldConfig {
    pub global: GlobalConfig,
    /// Hull-agnostic scenario detail-floor selectors from world TOML. A value
    /// matches either an authored Station id (console family) or a System kind;
    /// the LocalShip adapter resolves the union to concrete System ids.
    pub scenario_detail_floor: Vec<String>,
    pub anchors: HashMap<String, [f32; 3]>,
    pub entities: Vec<WorldEntity>,
    /// Map of `name ? uuid` for entities spawned via `[[entity]] name = "..."`.
    /// Populated by `spawn_world_entities` (PRD #337/#339 slice 2); read by
    /// trigger and comms lookup paths that resolve a name to a live UUID.
    pub name_to_uuid: HashMap<String, String>,
    /// Paths of additional world TOML files to load additively at startup
    /// (issue #352 — `extra_worlds` field).
    pub extra_worlds: Vec<String>,
    /// Policy for this layer's pending delayed actions on unload (issue #751).
    pub delayed_unload_policy: DelayedUnloadPolicy,
    /// Optional world-level ambient light override; `None` means the
    /// renderer falls back to its built-in constants.
    pub ambient_light: Option<AmbientLightConfig>,
    /// Optional world-level viewscreen presentation settings (PRD #1023, module
    /// 5): HDR, bloom, and how long a visual takes to arrive or leave. `None`
    /// means every field takes [`RenderConfig`]'s own default, which is the
    /// calibration documented there — not "off".
    pub render: Option<RenderConfig>,
    /// Optional world-level audio (red-alert siren + music); `None` means red
    /// alert is silent. Every other sound comes from the local ship's config.
    pub audio: Option<crate::audio_config::WorldAudioConfig>,
    /// Optional ambient dust particle effect config; `None` means the
    /// renderer falls back to built-in dust defaults.
    pub dust: Option<DustPfxConfig>,
    /// List of selectable player ship options for this world.
    pub available_ships: Vec<AvailableShipEntry>,
    /// Optional spawn point for the player ship.
    pub player_spawn: Option<PlayerSpawnEntry>,
    /// Named mission deadlines, in authored order (issue #1024).
    ///
    /// Authored data only. The *live* state — due tick, pending/fired/cancelled,
    /// and the queued call that fires it — is
    /// [`WorldContentRuntime::deadlines`](crate::world::server::WorldContentRuntime::deadlines),
    /// armed from this list on the first simulation tick of the mission. Ids are
    /// unique within a world; [`parse_world`] refuses a duplicate by name.
    pub deadlines: Vec<crate::world::deadlines::Deadline>,
    /// Named civilian traffic routes, in authored order (issue #1028).
    ///
    /// Authored data only — an anchor chain belongs to the map it crosses, so
    /// two haulers running the same lane run the same record. The *live* state
    /// (which leg, which order, whether it is being obeyed) is the per-entity
    /// [`CivilianTraffic`](crate::civilian::CivilianTraffic) component. Ids are
    /// unique within a world; [`parse_world`] refuses a duplicate by name, and
    /// `world::validate` refuses a leg naming an anchor no world in the
    /// composition declares.
    pub routes: Vec<crate::civilian::RouteConfig>,
    /// The sides of a labour dispute, in authored order (issue #1035).
    ///
    /// Authored data only. The *live* status — whether each side is out, and
    /// what it makes of the crew — is
    /// [`WorkforceRegister`](crate::world::workforce::WorkforceRegister) on
    /// [`WorldContentRuntime`](crate::world::server::WorldContentRuntime),
    /// armed from this list on the first simulation tick of the mission exactly
    /// as the deadline table is. Ids are unique within a world; [`parse_world`]
    /// refuses a duplicate by name.
    ///
    /// Deliberately NOT cross-checked against the `workforce` a structure's
    /// `[infrastructure]` block names: an entity template ships in every
    /// scenario and a world that has no dispute about those people is a world
    /// where work there carries on. See
    /// [`WorkforceRegister::on_strike`](crate::world::workforce::WorkforceRegister::on_strike).
    pub workforces: Vec<crate::world::workforce::Workforce>,
    /// Every INLINE `[script.*]` Rhai body this world authors, in key order.
    ///
    /// Retained for exactly one reader: [`entity_template_paths`]'s scripted
    /// `spawn_entity` surface (issue #984). Activation does not read it — the
    /// loader lifts its own [`ScriptSource`]s from the raw TOML, and can also
    /// resolve the sibling-FILE form (`script = "combat.rhai"`) that this field
    /// deliberately cannot: `parse_world` has no resolver, so a world whose
    /// script lives beside it contributes no entry here. That is the boundary of
    /// the preload scan, and it is why shipped worlds author `[script]` inline.
    ///
    /// [`ScriptSource`]: vellum_script::ScriptSource
    pub script_sources: Vec<String>,
}

impl WorldConfig {
    /// Borrow the anchor table.
    ///
    /// Returned values are normalised `[x, y, z]` arrays; 2-element anchors
    /// from the source TOML are widened to 3 elements by inserting `0.0` at
    /// the Y component (mirrors the historical `ai/server.rs` behaviour).
    pub fn anchors(&self) -> &HashMap<String, [f32; 3]> {
        &self.anchors
    }

    /// Borrow the unified `[[entity]]` instance list.
    pub fn entities(&self) -> &[WorldEntity] {
        &self.entities
    }

    /// The authored civilian route with this id (issue #1028).
    ///
    /// Linear over a list a world authors a handful of; a map would buy nothing
    /// and would lose the authored order the vocabulary is read in.
    pub fn route(&self, id: &str) -> Option<&crate::civilian::RouteConfig> {
        self.routes.iter().find(|r| r.id == id)
    }
}

impl WorldEntity {
    /// The player-facing display text for this entity (issue #750).
    ///
    /// Returns `display_name` when the author supplied one; otherwise falls
    /// back to the `name` reference id (typically a localization key), and
    /// finally to the template path when the entity is anonymous. This keeps
    /// the authoring role (`name` = unique reference id) separate from the
    /// runtime role (player-facing text) without breaking worlds that used
    /// `name` for both.
    pub fn display_text(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or(&self.template_path)
    }
}

// -- Parser -----------------------------------------------------------------

/// Reject a `history(...)` atom in a WORLD expression (issue #890).
///
/// The bounded-window operator reads a per-fine-system history bag that an AI
/// policy host folds once per shared AI tick. World triggers and entity `when`
/// guards evaluate through [`crate::world::flags::Predicate::evaluate`], against
/// a flag-store chain and nothing else — there is no bag, nothing folds one, and
/// there is no per-system scope a window could even belong to. Left alone the
/// atom would parse, load, and read `false` for the whole scenario.
/// `pub(crate)` because the Rhai front-end's `.when(…)` modifier
/// ([`crate::world::script::triggers`]) has to refuse the same atom the
/// declarative `when =` field refuses — one rule, both front-ends.
pub(crate) fn reject_world_history(
    pred: &crate::world::flags::Predicate,
    what: &str,
) -> Result<(), String> {
    match pred.history_atom() {
        Some(atom) => Err(format!(
            "{what} reads {}: bounded history windows belong to an AI fine system's \
             policy, which has a host to advance them once per shared AI tick. World \
             expressions evaluate against flags alone, so the window would never fill \
             and this would read false for the whole scenario",
            atom.render()
        )),
        None => Ok(()),
    }
}

/// Parse a unified world TOML string in a single pass.
///
/// Validates that every anchor position has 2 or 3 components and normalises
/// to `[x, y, z]`. Returns an `Err` with a human-readable message on TOML
/// parse errors, invalid anchor shapes, unknown trigger conditions, or
/// invalid trigger actions.
pub fn parse_world(toml_str: &str) -> Result<WorldConfig, String> {
    let raw: RawWorld = toml::from_str(toml_str).map_err(|e| e.to_string())?;

    // The two AI clocks must be commensurate (issue #889). The slower snapshot
    // cadence is DERIVED from the base tick as an integer multiple, so an
    // authored pair that does not divide — `ai_tick_hz = 25` against the
    // default `ai_snapshot_hz = 10`, giving 2.5 base ticks per snapshot tick —
    // is a content error, not something to round silently at runtime.
    if raw.global.checked_snapshot_every_ticks().is_none() {
        return Err(format!(
            "[global] ai_tick_hz = {} and ai_snapshot_hz = {} are not a positive integer \
             relationship: the slower AI cadence is derived as a whole number of base ticks, \
             so ai_tick_hz / ai_snapshot_hz must divide exactly (got {})",
            raw.global.ai_tick_hz,
            raw.global.ai_snapshot_hz,
            raw.global.ai_tick_hz / raw.global.ai_snapshot_hz
        ));
    }

    // The logical tick also has a FLOOR (issue #895). The helm integrator caps
    // its step at `HELM_AI_MAX_DT_SECS`, so a sim tick longer than that cap is
    // silently shortened: the ship under-integrates and two hosts on different
    // authored rates diverge from identical commands. Rejecting the rate at
    // load turns that silent fidelity loss into a content error the author can
    // see, and is what lets `integrate_ship_physics` assert the cap is dead.
    if !raw.global.sim_tick_hz.is_finite()
        || raw.global.sim_tick_hz < crate::entities::config::MIN_SIM_TICK_HZ
    {
        return Err(format!(
            "[global] sim_tick_hz = {} is below the {} Hz floor: the helm integrator caps \
             its step at 1/{} s, so a slower logical tick would be silently shortened and \
             the simulation would under-integrate",
            raw.global.sim_tick_hz,
            crate::entities::config::MIN_SIM_TICK_HZ.round(),
            crate::entities::config::MIN_SIM_TICK_HZ.round(),
        ));
    }

    // The logical tick also has a CEILING (re-review of issue #895 — the
    // floor above had no matching upper bound). `Time<Virtual>::max_delta`
    // (250 ms) bounds how much wall-clock lag a single frame can absorb, but
    // the NUMBER of `FixedUpdate` steps that lag unpacks into is
    // `max_delta / timestep`: an unbounded rate (e.g. `sim_tick_hz =
    // 100000`) demands tens of thousands of fixed steps back-to-back inside
    // one frame and wedges the host. Rejecting the rate at load turns that
    // silent performance cliff into a content error the author can see.
    if raw.global.sim_tick_hz > crate::entities::config::MAX_SIM_TICK_HZ {
        return Err(format!(
            "[global] sim_tick_hz = {} is above the {} Hz ceiling: a lagged frame can \
             unpack into max_delta / timestep FixedUpdate steps, and a rate this fast \
             would run tens of thousands of them back-to-back and wedge the host",
            raw.global.sim_tick_hz,
            crate::entities::config::MAX_SIM_TICK_HZ.round(),
        ));
    }

    // The AI decision tick is in turn derived from the logical simulation tick
    // by counting (issue #895), so the same commensurability contract applies
    // one level up: `sim_tick_hz / ai_tick_hz` must be a positive integer.
    if raw.global.checked_sim_ticks_per_ai_tick().is_none() {
        return Err(format!(
            "[global] sim_tick_hz = {} and ai_tick_hz = {} are not a positive integer \
             relationship: the AI decision cadence is derived as a whole number of logical \
             sim ticks, so sim_tick_hz / ai_tick_hz must divide exactly (got {})",
            raw.global.sim_tick_hz,
            raw.global.ai_tick_hz,
            raw.global.sim_tick_hz / raw.global.ai_tick_hz
        ));
    }

    // The attacked-memory window (issue #1010) feeds straight into
    // `now - last < attacked_memory_secs` in `objectives::attacked_recently`
    // with no clamp. TOML floats admit `nan`, and IEEE 754 makes every
    // comparison against a NaN false — so a NaN window would make
    // `attacked_recently` silently and permanently read `false`, the
    // doctrine `not_attacked` gate would never close, and a raid under
    // active fire would never break off for self-defence. Rejecting a
    // non-finite value at load turns that silent lock-up into a content
    // error the author can see. A non-positive value is deliberately left
    // alone: per `GlobalConfig::attacked_memory_secs`'s docs, zero or
    // negative is the honest reading of a designer authoring "never counts
    // as attacked", not a mistake to reject.
    if !raw.global.attacked_memory_secs.is_finite() {
        return Err(format!(
            "[global] attacked_memory_secs = {} is not a finite number: \
             `objectives::attacked_recently` compares `now - last_hit < \
             attacked_memory_secs`, and every comparison against a non-finite value \
             is false, so the doctrine `not_attacked` gate would never close and a \
             raid under active fire would silently never break off",
            raw.global.attacked_memory_secs,
        ));
    }

    let mut anchors: HashMap<String, [f32; 3]> = HashMap::with_capacity(raw.anchors.len());
    for (name, pos) in raw.anchors {
        let normalised = match pos.len() {
            3 => [pos[0], pos[1], pos[2]],
            2 => [pos[0], 0.0, pos[1]],
            other => {
                return Err(format!(
                    "Anchor '{name}' has invalid position array length: {other} (expected 2 or 3)"
                ));
            }
        };
        anchors.insert(name, normalised);
    }

    // The retired declarative front-ends (issue #985). Refused BY NAME rather
    // than ignored: `RawWorld` sets no `deny_unknown_fields`, so without these
    // two checks a world that still authors `[[trigger]]` or `[[comms]]` would
    // parse clean and load with its scenario logic simply absent — the silent
    // failure this teardown exists to avoid, and the one a hand-authored or
    // mod-pack world is most likely to hit.
    if !raw.retired_triggers.is_empty() {
        return Err(format!(
            "this world authors {} [[trigger]] block(s), which are no longer parsed \
             (issue #985). Scenario logic is authored in the [script] block: register \
             the condition (`on_destroyed(\"name\", \"handler\")`, `on_timer(30, \"handler\")`, \
             …) and write the handler fn. See docs/toml-authoring-guide.md",
            raw.retired_triggers.len()
        ));
    }
    if !raw.retired_comms.is_empty() {
        return Err(format!(
            "this world authors {} [[comms]] block(s), which are no longer parsed \
             (issue #985). A comms thread is opened from a script handler with \
             `ctx.effects.open_comms(#{{ from: \"sender\", node_fn: \"root\" }})`, and its \
             dialogue tree is one fn per node. A hailable contact now opts in on the \
             ENTITY with `[comms] hailable = true`. See docs/toml-authoring-guide.md",
            raw.retired_comms.len()
        ));
    }

    // Named mission deadlines (issue #1024): ids are the only handle script has
    // on a deadline — `on_deadline("id", …)`, `ctx.deadlines.slip("id", …)` — so
    // a duplicate is not a cosmetic clash, it is two records competing for every
    // mutation. Refused at parse time, naming BOTH entries by index and id, so a
    // designer sees which two lines to reconcile rather than which one silently
    // won. An empty id is refused for the same reason: nothing can address it.
    for (i, deadline) in raw.deadline.iter().enumerate() {
        if deadline.id.trim().is_empty() {
            return Err(format!(
                "[[deadline]] #{i} has an empty id; every deadline needs a stable \
                 id for script to name it"
            ));
        }
        if let Some((j, earlier)) = raw
            .deadline
            .iter()
            .enumerate()
            .take(i)
            .find(|(_, other)| other.id == deadline.id)
        {
            return Err(format!(
                "duplicate deadline id '{}': [[deadline]] #{j} (due_secs = {}) and \
                 [[deadline]] #{i} (due_secs = {}) both declare it; deadline ids must \
                 be unique within a world",
                deadline.id, earlier.due_secs, deadline.due_secs
            ));
        }
    }

    // Civilian traffic routes (issue #1028). Same argument as deadlines: the id
    // is the only handle an entity's `[civilian] route` or a `divert` order has
    // on a lane, so a duplicate is two records competing for every hauler that
    // names it. The per-route structural checks (a lane with no legs, a leg with
    // no anchor, a cruise fraction outside (0, 1]) live on the vocabulary itself
    // so the same rules apply wherever a route is built. What is NOT checked
    // here is whether each leg's anchor exists: routes may legitimately cross a
    // sibling layer's anchors, so resolution is a composition-wide pass in
    // `world::validate` — the same place doctrine anchors are resolved.
    for (i, route) in raw.route.iter().enumerate() {
        route
            .validate()
            .map_err(|e| format!("[[route]] #{i}: {e}"))?;
        if let Some(j) = raw
            .route
            .iter()
            .enumerate()
            .take(i)
            .position(|(_, other)| other.id == route.id)
        {
            return Err(format!(
                "duplicate route id '{}': [[route]] #{j} and [[route]] #{i} both \
                 declare it; route ids must be unique within a world",
                route.id
            ));
        }
    }

    // The sides of a labour dispute (issue #1035). Same argument as deadlines
    // and routes, one vocabulary later: the id is the only handle a structure's
    // `[infrastructure] workforce` and a script's `ctx.effects.settle_strike`
    // have on a side, so a duplicate is two records competing for every
    // settlement. The per-side checks (a non-empty id, a disposition on its
    // authored scale) live on the vocabulary itself so the same rules apply
    // wherever a `[[workforce]]` is built.
    for (i, workforce) in raw.workforce.iter().enumerate() {
        workforce
            .validate()
            .map_err(|e| format!("[[workforce]] #{i}: {e}"))?;
        if let Some(j) = raw
            .workforce
            .iter()
            .enumerate()
            .take(i)
            .position(|(_, other)| other.id == workforce.id)
        {
            return Err(format!(
                "duplicate workforce id '{}': [[workforce]] #{j} and [[workforce]] #{i} \
                 both declare it; workforce ids must be unique within a world",
                workforce.id
            ));
        }
    }

    // Validate extra_worlds: every entry must be a non-empty string.
    for (i, path) in raw.extra_worlds.iter().enumerate() {
        if path.trim().is_empty() {
            return Err(format!(
                "extra_worlds[{i}] is an empty string; all paths must be non-empty"
            ));
        }
    }

    let available_ships = raw.available_ships;

    // Parse `when` predicates on entity entries.
    let mut entities = raw.entities;
    for entity in &mut entities {
        if let Some(ref src) = entity.when.clone() {
            let pred = crate::world::flags::parse_predicate(src).map_err(|e| {
                format!(
                    "Entity '{}' when predicate parse error: {e}",
                    entity.template_path
                )
            })?;
            reject_world_history(
                &pred,
                &format!("Entity '{}' when predicate", entity.template_path),
            )?;
            entity.when_predicate = Some(pred);
        }
    }

    let delayed_unload_policy = match raw
        .delayed_unload_policy
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("resolve") => DelayedUnloadPolicy::Resolve,
        _ => DelayedUnloadPolicy::Cancel,
    };

    Ok(WorldConfig {
        global: raw.global,
        scenario_detail_floor: raw.scenario_detail_floor,
        anchors,
        entities,
        name_to_uuid: HashMap::new(),
        extra_worlds: raw.extra_worlds,
        delayed_unload_policy,
        ambient_light: raw.ambient_light,
        render: raw.render,
        audio: raw.audio,
        dust: raw.dust,
        available_ships,
        player_spawn: raw.player_spawn,
        deadlines: raw.deadline,
        routes: raw.route,
        workforces: raw.workforce,
        script_sources: inline_script_sources(raw.script.as_ref()),
    })
}

/// Collect the INLINE Rhai bodies out of a raw `[script]` block.
///
/// A table (`[script] setup = """…"""`) yields one entry per string-valued key,
/// in `toml::Map` key order; a bare string (`script = "combat.rhai"`) names a
/// sibling file this pass cannot read and yields nothing. Non-string table
/// values are skipped here and rejected as findings by
/// `world::script::load::lift_world_scripts`, which is the validator.
fn inline_script_sources(script: Option<&toml::Value>) -> Vec<String> {
    match script {
        Some(toml::Value::Table(table)) => table
            .values()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Build a `name ? uuid` map for the named entries in an `[[entity]]` slice.
///
/// PRD #337/#339 slice 2: anonymous `[[entity]]` instances stay unaddressable;
/// `[[entity]]` instances carrying a `name` field become trigger- and
/// comms-eligible. The UUID generator is supplied by the caller so this
/// helper stays a pure function (tests pass a counter; production passes
/// `|| Uuid::new_v4().to_string()`).
pub fn assign_named_entity_uuids<F>(
    entities: &[WorldEntity],
    mut gen_uuid: F,
) -> HashMap<String, String>
where
    F: FnMut() -> String,
{
    let mut out = HashMap::new();
    for entity in entities {
        if let Some(name) = entity.name.as_ref() {
            out.insert(name.clone(), gen_uuid());
        }
    }
    out
}

/// Predicate: is this `[[entity]]` instance owned by the unified pipeline
/// (`spawn_world_entities`), rather than the complementary `setup_world`
/// path in `server_app.rs`?
///
/// PRD #337 routes two kinds of entries through the unified pipeline:
/// * **Slice 1**: any entry whose resolved template is an asteroid field.
/// * **Slice 2**: any entry carrying a `name` field — the unified pipeline
///   assigns the UUID so `name ? uuid` is single-sourced.
///
/// Both call sites (legacy + unified) call this helper with the same
/// `is_asteroid_field` lookup to guarantee no entry is spawned twice.
pub fn is_owned_by_unified_pipeline<F>(entity_inst: &WorldEntity, is_asteroid_field: F) -> bool
where
    F: Fn(&str) -> bool,
{
    if entity_inst.name.is_some() {
        return true;
    }
    is_asteroid_field(&entity_inst.template_path)
}

/// Collect the deduplicated entity template paths referenced by a `WorldConfig`.
///
/// Used by `wasm_load_world` to queue entity TOML fetches via the JS preload
/// callback (PRD #338). Returned in stable iteration order so the queue
/// sequence is deterministic across runs.
///
/// `curated_ships` is the locked scenario's playable-hull allowlist (issue
/// #917's `world::manifest::ScenarioEntry::ships`), threaded through from the
/// host's preload seam so a curated demo/mod-pack build never fetches hulls
/// the player can't choose. Empty means unrestricted — every ship the world
/// offers is queued, exactly as before #917. Only reference surface 2 below is
/// filtered; NPC/scenery templates (surfaces 1, 3, 4) always preload in full.
///
/// Walks four reference surfaces:
/// 1. Static `[[entity]]` declarations.
/// 2. `available_ships[*].template_path` entries (issue #623), filtered to
///    `curated_ships` when non-empty.
/// 3. `[[trigger.action]] type = "spawn_entity"` references (needed for
///    timer-driven wave spawns and similar — discovered too late by the
///    asset-preload pipeline otherwise, since trigger actions don't run
///    until after preload completes). (#475)
/// 4. `[[comms.response.action]] type = "spawn_entity"` references nested
///    arbitrarily deep in dialogue follow-ups. (#475)
/// 5. `ctx.effects.spawn_entity(#{ template_path: "…" })` references inside an
///    inline `[script]` body (#984) — the Rhai equivalent of surfaces 3 and 4,
///    and load-bearing for the same reason: `combat_test`, the one selectable
///    scenario, spawns its whole eight-wave raid from script handlers that do
///    not run until long after preload has finished.
pub fn entity_template_paths(world: &WorldConfig, curated_ships: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();

    // 1. Static `[[entity]]` declarations.
    for ent in &world.entities {
        if seen.insert(ent.template_path.clone()) {
            out.push(ent.template_path.clone());
        }
    }

    // 2. `available_ships[*].template_path` entries (issue #623), restricted
    //    to the curated allowlist when the host resolved one (issue #917).
    for ship in &world.available_ships {
        if !curated_ships.is_empty() && !curated_ships.iter().any(|p| p == &ship.template_path) {
            continue;
        }
        if seen.insert(ship.template_path.clone()) {
            out.push(ship.template_path.clone());
        }
    }

    // 3. Scripted `spawn_entity` references (issue #984). Since issue #985 this
    //    is the only source of a *dynamic* spawn's template path: the
    //    `[[trigger.action]]` and `[[comms.response.action]]` arrays steps 3
    //    and 4 used to walk no longer parse.
    for source in &world.script_sources {
        for path in script_spawn_template_paths(source) {
            if seen.insert(path.clone()) {
                out.push(path);
            }
        }
    }

    out
}

/// Scan an inline Rhai body for the `template_path` string literals its
/// `spawn_entity` maps name.
///
/// A STATIC scan, because there is nothing else available: a handler's body
/// never runs at load (`Engine::run_ast` executes only a unit's top level), so
/// the only thing that can be known about a scripted spawn before preload is
/// what its source says literally. That is enough for the shape every converted
/// world authors — a literal `template_path: "assets/entities/….toml"` inside
/// the map — and the failure mode of a miss is asymmetric: an extra path is a
/// wasted fetch, a missed one is a wave that never spawns in the browser.
///
/// `//` line comments are stripped first so prose that mentions the key does not
/// queue a fetch. A computed path (`template_path: hull_for(wave)`) is invisible
/// to this and always will be; shipped content authors literals.
fn script_spawn_template_paths(source: &str) -> Vec<String> {
    const KEY: &str = "template_path";
    let mut out = Vec::new();
    for line in source.lines() {
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        let bytes = code.as_bytes();
        let mut from = 0;
        while let Some(rel) = code[from..].find(KEY) {
            let at = from + rel;
            from = at + KEY.len();
            // Word boundary, so `xtemplate_path` / `template_pathy` do not match.
            let before_ok = at == 0 || !is_ident_byte(bytes[at - 1]);
            let after_ok = from >= bytes.len() || !is_ident_byte(bytes[from]);
            if !before_ok || !after_ok {
                continue;
            }
            // Skip whitespace and the one separator (`:` in Rhai, `=` in TOML).
            let mut i = from;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b':' || bytes[i] == b'=') {
                i += 1;
            }
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'"' {
                continue;
            }
            let start = i + 1;
            if let Some(len) = code[start..].find('"') {
                out.push(code[start..start + len].to_string());
                from = start + len + 1;
            }
        }
    }
    out
}

/// Is `b` part of a Rust/Rhai identifier?
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Partition immediate-spawn entity instances into (asteroid_field, other).
///
/// The classifier closure inspects the resolved template (typically by looking
/// it up in the config cache and checking `EntityConfig.asteroid_field`) and
/// returns `true` for asteroid-field templates.
///
/// Asteroid-field instances and any `[[entity]]` carrying a `name` field flow
/// through the `spawn_world_entities` Bevy system; every other immediate-spawn
/// instance flows through the complementary `setup_world` system in
/// `server_app.rs`. Keeping the partitioning logic pure means both call sites
/// consult the same source of truth and double-spawn is impossible.
///
/// Only `WorldEntitySpawnOn::Immediate` entries are considered; `GameStart`
/// entries are returned in neither bucket (they're handled by
/// `spawn_game_start_entities`).
pub fn partition_immediate_entities<F>(
    world: &WorldConfig,
    is_asteroid_field: F,
) -> (
    Vec<&crate::world::config::WorldEntity>,
    Vec<&crate::world::config::WorldEntity>,
)
where
    F: Fn(&str) -> bool,
{
    use crate::world::config::WorldEntitySpawnOn;
    let mut fields = Vec::new();
    let mut others = Vec::new();
    for ent in &world.entities {
        if ent.spawn_on != WorldEntitySpawnOn::Immediate {
            continue;
        }
        if is_asteroid_field(&ent.template_path) {
            fields.push(ent);
        } else {
            others.push(ent);
        }
    }
    (fields, others)
}

/// Resolve the spawn position of an `[[entity]]` instance against the
/// world's anchor table.
///
/// Precedence:
/// 1. `anchor = "name"` — look up the anchor; error if missing.
/// 2. `position = [x, y, z]` — return as-is when length = 3.
/// 3. Neither — `[0, 0, 0]`.
///
/// `anchor` and `position` are not strictly mutually exclusive at parse
/// time; when both are supplied the anchor wins (matches the legacy
/// `[[spawn]]` semantics where anchor lookups happened first).
///
/// PRD #337 slice 3: lifts anchor positioning from the scenario half into
/// the unified `[[entity]]` pipeline so NPCs can be migrated off
/// `[[spawn]]`. Pure function — tested without Bevy.
/// Build the `reference ? resolved_position` lookup table `relative_to`
/// references resolve against during spawning.
///
/// # Which authored identifier a `relative_to` may name (issue #969)
///
/// Both of an `[[entity]]`'s authored identifiers key the table:
///
/// * `name` — the world-reference id triggers, comms and objectives resolve
///   against, and
/// * `id` — the authored instance id carried onto the spawned entity.
///
/// Accepting only `name` is what silently dropped `combat_test.toml`'s ice
/// moon: the localisation pass (commit `65becb5e`) rewrote landmark `name`s
/// from bare ids (`earth`, `luna`) to strings.csv keys
/// (`world.entity.earth.name`) while leaving `id = "earth"` and every
/// `relative_to = "earth"` alone, so the reference matched nothing. A
/// positioning reference is authored beside the entity it points at, and an
/// author reading `id = "gas-giant"` two lines up reasonably writes
/// `relative_to = "gas-giant"`.
///
/// The table is filled in **two passes** — every `id`, then every `name` over
/// the top — so that when some world authors one entity's `id` as another's
/// `name`, `name` wins whichever of the two is declared first: `name` is the
/// reference id proper, `id` only an accepted alias. One interleaved pass would
/// instead hand the spelling to whichever entity is last in file order, and
/// that is the single shape in which admitting `id` as a key could silently
/// re-point a `relative_to` that already resolved.
///
/// Two entities claiming one spelling through the *same* identifier still
/// resolve by file order, because there is no principled winner between them:
/// [`crate::world::validate::validate_entity_identity`] errors on a duplicate
/// `name` and warns on every other ambiguous spelling.
///
/// # Ordering
///
/// The whole table is built before any entity is positioned, so a reference
/// resolves whether its target is declared **earlier or later** in the file.
///
/// Base positions come from anchor/inline `position` only (NOT `relative_to`),
/// which means relative-to-relative chains are not supported. This is
/// intentional: it keeps resolution single-pass and avoids cycle detection
/// complexity for a feature whose primary use is "spawn an enemy 10 units off
/// this landmark". [`crate::world::validate::validate_relative_to`] rejects a
/// chain by name rather than letting it read as a missing entity.
///
/// Resolution failures (missing anchor) are silently skipped — the affected
/// entity will produce its own error when its position is resolved at spawn
/// time, so this helper doesn't need to duplicate error reporting.
pub fn build_named_entity_positions(world: &WorldConfig) -> HashMap<String, [f32; 3]> {
    // Every entity usable as a positioning base, with the position it resolved
    // to. Collected once so the two keying passes below cannot disagree about
    // which entities qualify.
    let bases: Vec<(&WorldEntity, [f32; 3])> = world
        .entities
        .iter()
        .filter(|ent| ent.id.is_some() || ent.name.is_some())
        // Skip entities whose own position is relative_to-based — their
        // position isn't valid as a base for further relative_to lookups.
        .filter(|ent| {
            ent.transform
                .as_ref()
                .and_then(|t| t.relative_to.as_ref())
                .is_none()
        })
        .filter_map(|ent| {
            resolve_entity_position(ent, &world.anchors)
                .ok()
                .map(|pos| (ent, pos))
        })
        .collect();

    let mut out = HashMap::new();
    // Pass 1: the alias.
    for (ent, pos) in &bases {
        if let Some(id) = ent.id.as_ref() {
            out.insert(id.clone(), *pos);
        }
    }
    // Pass 2: the reference id proper, over the top — a `name` beats another
    // entity's `id` no matter which was declared first.
    for (ent, pos) in &bases {
        if let Some(name) = ent.name.as_ref() {
            out.insert(name.clone(), *pos);
        }
    }
    out
}

pub fn resolve_entity_position(
    entity_inst: &crate::world::config::WorldEntity,
    anchors: &HashMap<String, [f32; 3]>,
) -> Result<[f32; 3], String> {
    resolve_entity_position_with(entity_inst, anchors, &HashMap::new())
}

/// Extended position resolver supporting `relative_to`+`offset`.
///
/// Thin wrapper over `TransformConfig::resolve` that supplies a default
/// transform when none is present on the entity.
pub fn resolve_entity_position_with(
    entity_inst: &crate::world::config::WorldEntity,
    anchors: &HashMap<String, [f32; 3]>,
    entities_by_name: &HashMap<String, [f32; 3]>,
) -> Result<[f32; 3], String> {
    let default_xf;
    let xf = match entity_inst.transform.as_ref() {
        Some(t) => t,
        None => {
            default_xf = TransformConfig::default();
            &default_xf
        }
    };
    xf.resolve(&entity_inst.template_path, anchors, entities_by_name)
}

/// Three-way partition of immediate `[[entity]]` instances.
///
/// PRD #339 slice 2: the unified pipeline owns BOTH asteroid-field templates
/// AND any entry carrying a `name` field (so the entity that triggers / comms
/// resolve through `name ? uuid` is actually spawned with that UUID). The
/// complementary `setup_world` path in `server_app.rs` only spawns the third
/// bucket (anonymous non-asteroid entries).
///
/// Returns `(asteroid_fields, named_non_asteroid, anonymous_non_asteroid)`.
/// `GameStart` entries are returned in none of the three buckets.
pub fn partition_immediate_entities_three_way<F>(
    world: &WorldConfig,
    is_asteroid_field: F,
) -> (
    Vec<&crate::world::config::WorldEntity>,
    Vec<&crate::world::config::WorldEntity>,
    Vec<&crate::world::config::WorldEntity>,
)
where
    F: Fn(&str) -> bool,
{
    use crate::world::config::WorldEntitySpawnOn;
    let mut fields = Vec::new();
    let mut named = Vec::new();
    let mut anon = Vec::new();
    for ent in &world.entities {
        if ent.spawn_on != WorldEntitySpawnOn::Immediate {
            continue;
        }
        if is_asteroid_field(&ent.template_path) {
            fields.push(ent);
        } else if ent.name.is_some() {
            named.push(ent);
        } else {
            anon.push(ent);
        }
    }
    (fields, named, anon)
}

// -- Unit Tests -------------------------------------------------------------

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
