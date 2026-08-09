//! tune-lods — place each LOD switch boundary at the knee of the
//! difference-vs-distance curve, headless.
//!
//!   cargo run --features capture --bin tune-lods -- \
//!       assets/models/alliance_cruiser.glb \
//!       [--variant large] [--resolution 1920x1080] \
//!       [--distances 12] [--yaws 4] [--pitch 20] [--out <dir>]
//!
//! For each adjacent pair of levels in a model's ladder (fine A, coarse B) this
//! renders BOTH at a swept series of camera distances, from several yaw angles,
//! into a game-resolution viewport, takes the WORST-CASE (max over yaws)
//! alpha-aware image difference at each distance, and finds the KNEE of the
//! resulting difference-vs-distance curve — the diminishing-returns point past
//! which keeping the expensive fine level buys almost nothing on screen. That
//! knee is the proposed `A.max_distance`.
//!
//! It prints the proposed boundaries as JSON on stdout (the node driver
//! `scripts/tune-lods.mjs` reads them and writes the sidecars) and writes review
//! artifacts — an A-vs-B montage and a diff-curve plot per pair — to `--out`.
//!
//! The offscreen-render plumbing and framing maths are shared with
//! `capture-billboard` via [`project_phoenix::render_capture`]; the diff metric
//! and knee rule are the pure, unit-tested [`project_phoenix::lod_tune`].
//!
//! # Deviation from `update_mesh_lod`: co-oriented ladders
//! The game resolves each GLB level's OWN sidecar for its base rig. This tool
//! instead co-orients every GLB level of a ladder on the near (first GLB)
//! level's base rig (`resolve_near_base`). The reason is that the diff only
//! means "same view, different detail" if A and B share a pose, and a decimated
//! `_lodN.glb` generally ships no sidecar — so `update_mesh_lod`'s per-level
//! resolve degrades it to an identity rig, which for a hull whose near level is
//! rotated (e.g. `alliance_cruiser`'s `rotation = [0, 3.14, 0]`) lands the LOD
//! 180° off and the diff never falls. Where the per-level sidecars already
//! agree — the asteroids all carry the same `[base]` — the two approaches are
//! identical. A missing `_lodN` sidecar is a real content gap the mass-apply
//! step should close; see the report.

// Dev-only batch tuning CLI, run offline — never the shipped sim (issue #908; the
// tested pure core is src/lod_tune.rs). Opt out of the transcendental ban, and of
// two style lints that don't earn a refactor in a one-shot tool: Bevy systems
// carry many params, and this tool's state machine uses early `return`s.
#![allow(clippy::disallowed_methods)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_return)]

use std::path::PathBuf;
use std::time::Duration;

use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::{ClearColorConfig, RenderTarget},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::renderer::RenderDevice,
    window::ExitCondition,
    winit::WinitPlugin,
};

use project_phoenix::entities::billboard::{orient_lod_billboards, spawn_billboard_child};
use project_phoenix::entity_config::LodLevel;
use project_phoenix::lod_tune::{find_knee, image_diff_rms};
use project_phoenix::model_rig::{sidecar_path, ModelRig, DEFAULT_VARIANT};
use project_phoenix::render_capture::{
    create_render_target, frame_distance, measure_world_bounds, orbit_transform, unpad_rows,
    ImageCopyPlugin, MainWorldReceiver,
};
use project_phoenix::renderer::GameCamera;

// ── Config from argv ────────────────────────────────────────────────────────

#[derive(Resource, Clone)]
struct TuneConfig {
    model: String,
    variant: String,
    width: u32,
    height: u32,
    distances: u32,
    yaws: u32,
    pitch_deg: f32,
    out_dir: PathBuf,
    /// Framing (world centre, radius) from the near model's `[extents]`, or
    /// `None` to fall back to the live-measured AABB union.
    framing: Option<(Vec3, f32)>,
    /// The ladder from the model's rig sidecar, near→far.
    levels: Vec<LodLevel>,
}

fn parse_config() -> TuneConfig {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if positional.is_empty() {
        eprintln!(
            "usage: tune-lods <model.glb> [--variant <name>] [--resolution WxH] \
             [--distances N] [--yaws N] [--pitch deg] [--out dir]"
        );
        std::process::exit(2);
    }
    let str_flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let num_flag = |name: &str, dflt: f32| -> f32 {
        str_flag(name).and_then(|v| v.parse().ok()).unwrap_or(dflt)
    };

    let model = positional[0].clone();
    let variant = str_flag("--variant").unwrap_or_else(|| DEFAULT_VARIANT.to_string());
    let (width, height) = str_flag("--resolution")
        .and_then(|s| {
            let mut it = s.split(['x', 'X']);
            let w = it.next()?.parse().ok()?;
            let h = it.next()?.parse().ok()?;
            Some((w, h))
        })
        .unwrap_or((1920, 1080));
    let out_dir = str_flag("--out")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    // The ladder is authored in the rig sidecar (issue #914), at the requested
    // variant. Its `[base]` also orients the yaw ring.
    let sidecar = sidecar_path(&model, Some(&variant));
    let rig = std::fs::read_to_string(&sidecar)
        .ok()
        .and_then(|t| ModelRig::from_toml(&t).ok())
        .unwrap_or_else(|| {
            eprintln!("[tune-lods] no readable rig sidecar at {sidecar}");
            std::process::exit(2);
        });
    if rig.lod.len() < 2 {
        eprintln!("[tune-lods] {sidecar}: ladder has < 2 levels, nothing to tune");
        std::process::exit(2);
    }

    // Framing centre + radius from the near model's cached `[extents]` (this
    // sidecar IS the near level's). Deterministic — and immune to the GPU-upload
    // race that leaves a big hull's live AABB unmeasured at warmup while its tiny
    // billboard quad is not, which would frame the sweep on the billboard. The
    // extents are post-base-rig world units, which is where this tool renders the
    // co-oriented near level. Falls back to the live AABB when absent.
    let framing = rig.extents.as_ref().map(|e| {
        let center = (Vec3::from_array(e.min) + Vec3::from_array(e.max)) * 0.5;
        let radius = Vec3::from_array(e.size).max_element().max(0.05) * 0.5;
        (center, radius)
    });

    TuneConfig {
        model,
        variant,
        width: (width as u32).max(1),
        height: (height as u32).max(1),
        distances: num_flag("--distances", 12.0) as u32,
        yaws: num_flag("--yaws", 4.0) as u32,
        pitch_deg: num_flag("--pitch", 20.0),
        out_dir,
        framing,
        levels: rig.lod,
    }
}

fn main() {
    if std::env::var_os("BEVY_ASSET_ROOT").is_none() {
        if let Ok(cwd) = std::env::current_dir() {
            std::env::set_var("BEVY_ASSET_ROOT", cwd);
        }
    }

    let config = parse_config();

    App::new()
        .insert_resource(config)
        .insert_resource(ClearColor(Color::NONE))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(ImageCopyPlugin)
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .insert_resource(Tune::default())
        .add_systems(Startup, setup)
        // Billboard facing/tile is the game's own system, reused verbatim — the
        // capture camera is tagged `GameCamera` so a billboard level renders
        // exactly as it would in game.
        .add_systems(Update, orient_lod_billboards)
        .add_systems(PostUpdate, drive)
        .run();
}

// ── Scene setup ──────────────────────────────────────────────────────────────

#[derive(Component)]
struct CaptureCamera;

/// What a ladder level renders as — decides how it is spawned and framed.
#[derive(Clone, Copy, PartialEq)]
enum LevelKind {
    Glb,
    Billboard,
    Shape,
    /// A level with none of model/billboard/shape — invalid, never shown.
    Empty,
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    render_device: Res<RenderDevice>,
    config: Res<TuneConfig>,
    mut tune: ResMut<Tune>,
) {
    let (target, copier) =
        create_render_target(&mut images, &render_device, config.width, config.height);
    commands.spawn(copier);

    // Every GLB level of the ladder is co-oriented on ONE base rig — the near
    // (first GLB) level's resolved sidecar. A ladder is one model at varying
    // detail, so its levels must occupy the same pose for the A-vs-B diff to
    // mean "same view, different detail"; where the per-level sidecars agree
    // (e.g. the asteroids all carry the same `[base]`) this is identical to
    // resolving each level's own sidecar, and where a decimated level ships no
    // sidecar (e.g. every ship's `_lodN.glb`) it corrects what would otherwise
    // be an identity rig 180° off the near level. See the module deviation note.
    let near_base = resolve_near_base(&config);

    // Spawn every ladder level as its own top-level subject, all HIDDEN. GPU
    // mesh/texture upload happens on asset load, not on visibility, so warmup
    // still uploads them; keeping them hidden means no all-levels-visible frame
    // is ever rendered, so an in-flight readback of one cannot contaminate the
    // first sweep capture. Framing comes from `[extents]`, not the live AABB, so
    // nothing needs to be visible at warmup.
    for level in &config.levels {
        let (entity, kind) = spawn_level(
            &mut commands,
            &asset_server,
            &mut meshes,
            &mut materials,
            level,
            near_base,
        );
        tune.level_ents.push(entity);
        tune.level_kinds.push(kind);
    }

    // Same key + fill as capture-billboard, so a GLB level reads as a lit hull.
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            ..default()
        },
        Transform::from_xyz(1.0, 2.0, 1.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        CaptureCamera,
        GameCamera,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        AmbientLight {
            color: Color::WHITE,
            brightness: 400.0,
            ..default()
        },
        RenderTarget::from(target),
        project_phoenix::render_setup::game_camera_projection(),
        Tonemapping::None,
        Transform::default(),
    ));
}

/// The base rig every GLB level of the ladder is co-oriented on: the first GLB
/// level's resolved sidecar (at its variant), or identity when unreadable.
fn resolve_near_base(config: &TuneConfig) -> Transform {
    for level in &config.levels {
        if let Some(model_path) = level.model.as_deref() {
            let variant = level
                .variant
                .clone()
                .unwrap_or_else(|| config.variant.clone());
            return std::fs::read_to_string(sidecar_path(model_path, Some(&variant)))
                .ok()
                .and_then(|t| ModelRig::from_toml(&t).ok())
                .map(|r| r.base_bevy_transform())
                .unwrap_or_default();
        }
    }
    Transform::default()
}

/// Spawn one ladder level as the game would render it: a GLB `SceneRoot` under
/// the ladder's shared near base rig, a billboard quad, or a procedural shape.
/// Returns the top-level subject entity and its kind.
fn spawn_level(
    commands: &mut Commands,
    asset_server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    level: &LodLevel,
    near_base: Transform,
) -> (Entity, LevelKind) {
    // Level scale rides the parent, exactly as `update_mesh_lod` composes it:
    // entity scale = mesh_scale (1 for both tracer models) × level.scale.
    let level_scale = level.scale.map(Vec3::from_array).unwrap_or(Vec3::ONE);

    if let Some(model_path) = level.model.as_deref() {
        // Co-orient on the ladder's shared near base rig (see `resolve_near_base`).
        let level_base = near_base;
        let rel = model_path.strip_prefix("assets/").unwrap_or(model_path);
        let scene = asset_server.load(format!("{rel}#Scene0"));
        let entity = commands
            .spawn((Transform::from_scale(level_scale), Visibility::Hidden))
            .with_children(|p| {
                p.spawn((SceneRoot(scene), level_base));
            })
            .id();
        (entity, LevelKind::Glb)
    } else if let Some(atlas) = level.billboard.as_deref() {
        // Uniform parent scale; the quad's world w/h ride the child, as in the
        // billboard branch of `update_mesh_lod`.
        let [w, h] = level.scale.map(|s| [s[0], s[1]]).unwrap_or([1.0, 1.0]);
        let views = level
            .capture
            .as_ref()
            .and_then(|c| c.yaw_views)
            .unwrap_or(1);
        let entity = commands
            .spawn((Transform::default(), Visibility::Hidden))
            .id();
        let child = spawn_billboard_child(
            commands,
            meshes,
            materials,
            asset_server,
            atlas,
            w,
            h,
            views,
        );
        commands.entity(entity).add_child(child);
        (entity, LevelKind::Billboard)
    } else if let Some(shape) = level.shape {
        // Minimal procedural stand-in — the tracer models never reach here, but a
        // shape level still renders *something* diffable rather than nothing.
        let radius = level.radius.unwrap_or(1.0);
        let colour = level
            .colour
            .as_ref()
            .map(|c| Color::srgb(c[0], c[1], c[2]))
            .unwrap_or(Color::srgb(0.5, 0.5, 0.5));
        let mesh = match shape {
            project_phoenix::entity_config::MeshShape::Sphere => meshes.add(Sphere::new(radius)),
            project_phoenix::entity_config::MeshShape::Cuboid => {
                let s = level.size.unwrap_or([radius * 2.0; 3]);
                meshes.add(Cuboid::new(s[0], s[1], s[2]))
            }
            project_phoenix::entity_config::MeshShape::Torus => meshes.add(Torus::new(
                radius,
                radius + level.minor_radius.unwrap_or(0.3),
            )),
        };
        let mat = materials.add(StandardMaterial {
            base_color: colour,
            ..default()
        });
        let entity = commands
            .spawn((Transform::from_scale(level_scale), Visibility::Hidden))
            .id();
        let child = commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat))).id();
        commands.entity(entity).add_child(child);
        (entity, LevelKind::Shape)
    } else {
        let entity = commands
            .spawn((Transform::default(), Visibility::Hidden))
            .id();
        (entity, LevelKind::Empty)
    }
}

// ── State machine ─────────────────────────────────────────────────────────────

#[derive(Default, PartialEq, Clone, Copy)]
enum Phase {
    #[default]
    Warmup,
    Sweep,
    Montage,
    Done,
}

#[derive(Resource, Default)]
struct Tune {
    phase: Phase,
    warmup: u32,
    level_ents: Vec<Entity>,
    level_kinds: Vec<LevelKind>,
    /// Which level is currently the sole visible subject (`None` forces a reshow).
    shown: Option<usize>,
    settle: u32,
    /// The previous post-settle candidate frame, held so a capture is only
    /// accepted once two consecutive candidates agree (see `settle_and_capture`).
    pending_tile: Vec<u8>,
    confirm_frames: u32,
    center: Vec3,
    radius: f32,
    /// The swept camera distances, log-spaced near→far.
    distances: Vec<f32>,
    // Sweep cursor: pair p diffs level p (fine) vs p+1 (coarse).
    pair: usize,
    di: usize,
    yi: usize,
    /// 0 = capturing the fine level A, 1 = capturing the coarse level B.
    cap_which: u8,
    tile_a: Vec<u8>,
    /// `diffs[pair][di]` = worst-case (max over yaw) diff at that distance.
    diffs: Vec<Vec<f64>>,
    /// The chosen boundary distance for each pair (the knee, or the current
    /// bound when no knee is found).
    knee_dist: Vec<f32>,
    knee_found: Vec<bool>,
    // Montage cursor.
    m_pair: usize,
    m_which: u8,
    m_tile_a: Vec<u8>,
}

const WARMUP_FRAMES: u32 = 90;
/// Frames after a visibility/camera change before the readback reflects it —
/// generous, so an async readback still in the GPU pipeline from the previous
/// shot can never be mistaken for this shot's.
const SETTLE_FRAMES: u32 = 8;
/// Two post-settle candidate frames closer than this (alpha-RMS) count as a
/// stable render; a bigger gap means a transient frame slipped in, so wait.
const STABLE_EPS: f64 = 0.001;
/// Give up confirming stability after this many extra frames and accept anyway,
/// so a legitimately noisy render can never hang the run.
const MAX_CONFIRM_FRAMES: u32 = 30;
/// A capture that must not be blank needs at least this many opaque pixels — a
/// level whose GLB has not finished uploading past `WARMUP_FRAMES` renders empty,
/// and an empty A vs empty B diffs as a false zero (see `require_nonblank`).
const MIN_OPAQUE_PX: usize = 40;
/// How long to keep waiting for a required-non-blank frame before giving up and
/// accepting whatever rendered, so a genuinely empty view cannot hang the run.
const MAX_BLANK_WAIT_FRAMES: u32 = 180;

/// Count of pixels with meaningful alpha, over an RGBA8 buffer.
fn opaque_px(tile: &[u8]) -> usize {
    tile.chunks_exact(4).filter(|p| p[3] > 8).count()
}

fn drive(
    mut tune: ResMut<Tune>,
    config: Res<TuneConfig>,
    receiver: Res<MainWorldReceiver>,
    mut cameras: Query<&mut Transform, With<CaptureCamera>>,
    mut visibilities: Query<&mut Visibility>,
    bounds_q: Query<(&GlobalTransform, &bevy::camera::primitives::Aabb)>,
    mut exit: MessageWriter<AppExit>,
) {
    match tune.phase {
        Phase::Warmup => {
            tune.warmup += 1;
            while receiver.try_recv().is_ok() {}
            if tune.warmup < WARMUP_FRAMES {
                return;
            }
            let (center, radius) = match config.framing {
                Some(f) => f,
                None => {
                    let Some((center, radius, _size)) = measure_world_bounds(bounds_q.iter())
                    else {
                        return; // geometry not measurable yet — wait
                    };
                    (center, radius)
                }
            };
            tune.center = center;
            tune.radius = radius;
            tune.distances = sweep_distances(&config, radius);

            let pairs = config.levels.len() - 1;
            tune.diffs = vec![vec![0.0; tune.distances.len()]; pairs];
            eprintln!(
                "[tune-lods] {} ({}): {} levels, {} pairs, {} distances × {} yaws @ {}×{}",
                config.model,
                config.variant,
                config.levels.len(),
                pairs,
                tune.distances.len(),
                config.yaws,
                config.width,
                config.height,
            );
            tune.phase = Phase::Sweep;
            tune.shown = None;
            tune.settle = 0;
            return;
        }

        Phase::Sweep => {
            let dist = tune.distances[tune.di];
            let yaw = tune.yi as f32 * std::f32::consts::TAU / config.yaws.max(1) as f32;
            aim_camera(&mut cameras, tune.center, dist, yaw, config.pitch_deg);

            let want = tune.pair + tune.cap_which as usize;
            if tune.shown != Some(want) {
                show_only(&mut visibilities, &tune.level_ents, want);
                tune.shown = Some(want);
                tune.settle = 0;
                tune.pending_tile.clear();
                tune.confirm_frames = 0;
                while receiver.try_recv().is_ok() {}
                return;
            }

            // At the nearest distance require a non-blank frame, so a not-yet-
            // uploaded level cannot poison the near end of the diff curve.
            let require_nonblank = tune.di == 0;
            let Some(tile) = settle_and_capture(&mut tune, &receiver, &config, require_nonblank)
            else {
                return;
            };

            if tune.cap_which == 0 {
                tune.tile_a = tile;
                tune.cap_which = 1;
                tune.shown = None; // force the coarse level to be shown next
                return;
            }

            let diff = image_diff_rms(&tune.tile_a, &tile);
            let (p, di) = (tune.pair, tune.di);
            if diff > tune.diffs[p][di] {
                tune.diffs[p][di] = diff;
            }

            // Advance the cursor: yaw → distance → pair.
            tune.cap_which = 0;
            tune.shown = None;
            tune.yi += 1;
            if tune.yi >= config.yaws.max(1) as usize {
                tune.yi = 0;
                tune.di += 1;
                if tune.di >= tune.distances.len() {
                    tune.di = 0;
                    tune.pair += 1;
                    if tune.pair >= config.levels.len() - 1 {
                        finish_sweep(&mut tune);
                        tune.phase = Phase::Montage;
                        tune.pair = 0;
                        tune.m_pair = 0;
                        tune.m_which = 0;
                        tune.shown = None;
                    }
                }
            }
            return;
        }

        Phase::Montage => {
            let p = tune.m_pair;
            let dist = tune.knee_dist[p];
            aim_camera(&mut cameras, tune.center, dist, 0.0, config.pitch_deg);

            let want = p + tune.m_which as usize;
            if tune.shown != Some(want) {
                show_only(&mut visibilities, &tune.level_ents, want);
                tune.shown = Some(want);
                tune.settle = 0;
                tune.pending_tile.clear();
                tune.confirm_frames = 0;
                while receiver.try_recv().is_ok() {}
                return;
            }
            // The montage renders at the chosen boundary, which may be far and
            // legitimately sparse, so no non-blank requirement here.
            let Some(tile) = settle_and_capture(&mut tune, &receiver, &config, false) else {
                return;
            };

            if tune.m_which == 0 {
                tune.m_tile_a = tile;
                tune.m_which = 1;
                tune.shown = None;
                return;
            }

            write_artifacts(&config, &tune, p, &tune.m_tile_a, &tile);
            tune.m_which = 0;
            tune.shown = None;
            tune.m_pair += 1;
            if tune.m_pair >= config.levels.len() - 1 {
                tune.phase = Phase::Done;
            }
            return;
        }

        Phase::Done => {
            print_json(&config, &tune);
            exit.write(AppExit::Success);
        }
    }
}

/// Advance the settle counter and return a tile only once the render has proven
/// STABLE: past [`SETTLE_FRAMES`], two consecutive post-settle candidate frames
/// must agree to within [`STABLE_EPS`] before one is accepted. This filters the
/// occasional transient frame the async readback pipeline delivers on the first
/// capture of a run (or right after a visibility swap), which would otherwise
/// spike a single distance's worst-case diff and shift the knee. `None` means
/// "still settling or not yet stable".
fn settle_and_capture(
    tune: &mut Tune,
    receiver: &MainWorldReceiver,
    config: &TuneConfig,
    require_nonblank: bool,
) -> Option<Vec<u8>> {
    tune.settle += 1;
    let mut bytes = Vec::new();
    while let Ok(data) = receiver.try_recv() {
        bytes = data;
    }
    if tune.settle < SETTLE_FRAMES || bytes.is_empty() {
        return None;
    }
    let tile = unpad_rows(&bytes, config.width, config.height);
    // At the nearest distance a properly-uploaded level fills a large footprint;
    // a blank frame there means the GLB is still streaming, and accepting it
    // would diff empty-vs-empty as a false zero. Keep waiting (up to a cap) for
    // real geometry before trusting the shot.
    if require_nonblank && opaque_px(&tile) < MIN_OPAQUE_PX && tune.settle < MAX_BLANK_WAIT_FRAMES {
        return None;
    }
    if tune.pending_tile.is_empty() {
        tune.pending_tile = tile;
        return None;
    }
    tune.confirm_frames += 1;
    let stable = image_diff_rms(&tune.pending_tile, &tile) < STABLE_EPS;
    if stable || tune.confirm_frames >= MAX_CONFIRM_FRAMES {
        tune.pending_tile.clear();
        tune.confirm_frames = 0;
        return Some(tile);
    }
    tune.pending_tile = tile;
    None
}

fn aim_camera(
    cameras: &mut Query<&mut Transform, With<CaptureCamera>>,
    center: Vec3,
    distance: f32,
    yaw: f32,
    pitch_deg: f32,
) {
    if let Ok(mut tf) = cameras.single_mut() {
        *tf = orbit_transform(center, distance, yaw, pitch_deg.to_radians());
    }
}

fn show_only(visibilities: &mut Query<&mut Visibility>, ents: &[Entity], want: usize) {
    for (i, &e) in ents.iter().enumerate() {
        if let Ok(mut v) = visibilities.get_mut(e) {
            *v = if i == want {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
}

/// Log-spaced sweep distances: from where the model frames comfortably out to
/// past the current far bound, so both the near detail-rich band and the far
/// tail where every level looks alike are sampled.
///
/// The near end uses a 1.4× framing margin, not a razor-thin one: at exactly
/// edge-to-edge framing the model is at its largest on screen and its silhouette
/// pixels dominate, so subpixel edge disagreements between two levels spike that
/// one nearest sample well above the smooth trend of the rest of the curve —
/// noise that would drag the knee inward. A little breathing room removes it and
/// starts the sweep around where the first band lives anyway.
fn sweep_distances(config: &TuneConfig, radius: f32) -> Vec<f32> {
    let fov = std::f32::consts::FRAC_PI_4;
    let near = frame_distance(radius, fov, 1.4).max(0.1);
    let last_bound = config
        .levels
        .iter()
        .filter_map(|l| l.max_distance)
        .fold(0.0f32, f32::max);
    let far = if last_bound > near {
        last_bound * 1.6
    } else {
        (near * 40.0).max(radius * 60.0)
    };
    let n = config.distances.max(3) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            near * (far / near).powf(t)
        })
        .collect()
}

/// Compute each pair's boundary from its diff curve: the knee, judged in
/// log-distance space, or the current authored bound as a fallback when the
/// curve has no knee (e.g. two near-identical levels).
fn finish_sweep(tune: &mut Tune) {
    let xs: Vec<f64> = tune.distances.iter().map(|&d| (d as f64).ln()).collect();
    let pairs = tune.diffs.len();
    tune.knee_dist = vec![0.0; pairs];
    tune.knee_found = vec![false; pairs];
    for p in 0..pairs {
        let ys = &tune.diffs[p];
        // The raw curve, for the reviewer to eyeball alongside the plot PNG.
        let samples: Vec<String> = tune
            .distances
            .iter()
            .zip(ys.iter())
            .map(|(d, v)| format!("{:.1}:{:.4}", d, v))
            .collect();
        eprintln!(
            "[tune-lods] pair {p} diff-vs-distance  {}",
            samples.join("  ")
        );
        match find_knee(&xs, ys) {
            Some(idx) => {
                tune.knee_dist[p] = tune.distances[idx];
                tune.knee_found[p] = true;
            }
            None => {
                // No knee — leave `knee_found[p]` false. `print_json` then emits
                // the authored bound as the proposal (a no-op) and the driver
                // skips the pair, so a knee-less curve never overwrites a
                // hand-authored switch distance with a guess.
            }
        }
    }
}

// ── Artifacts ──────────────────────────────────────────────────────────────

/// Round to one decimal — as precise as a switch distance ever needs to be.
fn round1(x: f32) -> f32 {
    (x * 10.0).round() / 10.0
}

fn print_json(config: &TuneConfig, tune: &Tune) {
    let mut pairs_json = Vec::new();
    for p in 0..config.levels.len() - 1 {
        let current = config.levels[p]
            .max_distance
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        // Without a knee the proposal is the authored bound — a no-op the driver
        // skips anyway, but emitted honestly rather than as a guessed distance.
        let proposed = if tune.knee_found.get(p).copied().unwrap_or(false) {
            round1(tune.knee_dist[p])
        } else {
            config.levels[p].max_distance.map(round1).unwrap_or(0.0)
        };
        let peak = tune.diffs[p].iter().cloned().fold(0.0f64, f64::max);
        pairs_json.push(format!(
            "{{\"fine\":{p},\"coarse\":{},\"current_max_distance\":{current},\
             \"proposed_max_distance\":{proposed},\"knee_found\":{},\"peak_diff\":{:.5}}}",
            p + 1,
            tune.knee_found.get(p).copied().unwrap_or(false),
            peak
        ));
    }
    println!(
        "{{\"model\":{:?},\"variant\":{:?},\"resolution\":\"{}x{}\",\"pairs\":[{}]}}",
        config.model,
        config.variant,
        config.width,
        config.height,
        pairs_json.join(",")
    );
}

/// Write the A-vs-B montage and the diff-curve plot for one pair.
fn write_artifacts(config: &TuneConfig, tune: &Tune, pair: usize, tile_a: &[u8], tile_b: &[u8]) {
    let _ = std::fs::create_dir_all(&config.out_dir);
    let stem = model_stem(&config.model);
    let base = format!("{stem}_{}_pair{pair}_{}v{}", config.variant, pair, pair + 1);

    // Montage: A | B side by side at the chosen boundary distance.
    let (w, h) = (config.width, config.height);
    if tile_a.len() == (w * h * 4) as usize && tile_b.len() == tile_a.len() {
        let mut montage = vec![0u8; (w * 2 * h * 4) as usize];
        let row = (w * 4) as usize;
        let mrow = (w * 2 * 4) as usize;
        for y in 0..h as usize {
            montage[y * mrow..y * mrow + row].copy_from_slice(&tile_a[y * row..y * row + row]);
            montage[y * mrow + row..y * mrow + 2 * row]
                .copy_from_slice(&tile_b[y * row..y * row + row]);
        }
        if let Some(img) = image::RgbaImage::from_raw(w * 2, h, montage) {
            let path = config.out_dir.join(format!("{base}_montage.png"));
            let _ = img.save(&path);
            eprintln!("[tune-lods] wrote {}", path.display());
        }
    }

    // Diff curve plot.
    let curve = plot_curve(&tune.distances, &tune.diffs[pair], tune.knee_dist[pair]);
    let path = config.out_dir.join(format!("{base}_curve.png"));
    let _ = curve.save(&path);
    eprintln!("[tune-lods] wrote {}", path.display());
}

fn model_stem(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.strip_suffix(".glb").unwrap_or(file).to_string()
}

/// A small dark line plot of diff vs. log(distance), with the chosen knee marked
/// as a vertical line. Enough to eyeball the curve shape in review.
fn plot_curve(distances: &[f32], diffs: &[f64], knee: f32) -> image::RgbaImage {
    const W: u32 = 640;
    const H: u32 = 400;
    const PAD: u32 = 40;
    let mut img = image::RgbaImage::from_pixel(W, H, image::Rgba([24, 26, 30, 255]));

    let x_of = |d: f32| -> i64 {
        let (lo, hi) = (
            (distances[0] as f64).ln(),
            (*distances.last().unwrap() as f64).ln(),
        );
        let t = if hi > lo {
            ((d as f64).ln() - lo) / (hi - lo)
        } else {
            0.0
        };
        (PAD as f64 + t * (W - 2 * PAD) as f64) as i64
    };
    let y_max = diffs.iter().cloned().fold(1e-9, f64::max);
    let y_of = |v: f64| -> i64 {
        let t = v / y_max;
        ((H - PAD) as f64 - t * (H - 2 * PAD) as f64) as i64
    };

    // Axes.
    let axis = image::Rgba([90, 96, 104, 255]);
    draw_line(
        &mut img,
        PAD as i64,
        (H - PAD) as i64,
        (W - PAD) as i64,
        (H - PAD) as i64,
        axis,
    );
    draw_line(
        &mut img,
        PAD as i64,
        PAD as i64,
        PAD as i64,
        (H - PAD) as i64,
        axis,
    );

    // Knee marker.
    if knee > 0.0 {
        let kx = x_of(knee);
        draw_line(
            &mut img,
            kx,
            PAD as i64,
            kx,
            (H - PAD) as i64,
            image::Rgba([230, 170, 60, 255]),
        );
    }

    // The curve.
    let line = image::Rgba([90, 200, 250, 255]);
    for i in 1..distances.len() {
        draw_line(
            &mut img,
            x_of(distances[i - 1]),
            y_of(diffs[i - 1]),
            x_of(distances[i]),
            y_of(diffs[i]),
            line,
        );
    }
    // Sample dots.
    for i in 0..distances.len() {
        let (px, py) = (x_of(distances[i]), y_of(diffs[i]));
        for dy in -2..=2 {
            for dx in -2..=2 {
                put(
                    &mut img,
                    px + dx,
                    py + dy,
                    image::Rgba([240, 240, 240, 255]),
                );
            }
        }
    }
    img
}

fn put(img: &mut image::RgbaImage, x: i64, y: i64, c: image::Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, c);
    }
}

/// Bresenham line, clipped to the image.
fn draw_line(img: &mut image::RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64, c: image::Rgba<u8>) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y) = (x0, y0);
    let mut err = dx + dy;
    loop {
        put(img, x, y, c);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}
