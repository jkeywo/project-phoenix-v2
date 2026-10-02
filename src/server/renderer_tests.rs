use super::*;
use crate::server_app::LocalShip;
use crate::ship::state::ShipViewMode;

#[test]
fn crew_spectator_camera_follows_selected_ship_without_moving_local_identity() {
    let mut app = App::new();
    app.init_resource::<Time>().init_resource::<Time<Fixed>>();
    app.init_resource::<CinematicCameraState>();
    app.init_resource::<crate::crew_spectator::CrewSpectator>();
    app.add_systems(Update, cinematic_camera);
    let mut mode = ShipViewMode::default();
    mode.force_view_mode(Some(ViewMode::Cinematic));
    let config = toml::from_str::<crate::entities::config::CinematicCameraConfig>(
        "position = [0.0, 8.0, 15.0]",
    )
    .unwrap();
    let own = app
        .world_mut()
        .spawn((
            LocalShip,
            EntityUuid("own".into()),
            mode,
            ShipPhysics::default(),
            CinematicCameraSection(config),
            Transform::default(),
        ))
        .id();
    let target = app
        .world_mut()
        .spawn((
            EntityUuid("other".into()),
            ShipPhysics {
                x: 100.0,
                ..default()
            },
        ))
        .id();
    let camera = app
        .world_mut()
        .spawn((GameCamera, Transform::default()))
        .id();
    {
        let mut spectator = app
            .world_mut()
            .resource_mut::<crate::crew_spectator::CrewSpectator>();
        spectator.active = true;
        spectator.target = Some("other".into());
    }
    app.update();
    assert_eq!(
        app.world().get::<Transform>(camera).unwrap().translation,
        Vec3::new(100.0, 8.0, 15.0)
    );
    assert!(app.world().get::<LocalShip>(own).is_some());
    assert!(app.world().get::<LocalShip>(target).is_none());
    app.world_mut()
        .resource_mut::<crate::crew_spectator::CrewSpectator>()
        .target = None;
    app.update();
    assert_eq!(
        app.world().get::<Transform>(camera).unwrap().translation,
        Vec3::new(0.0, 8.0, 15.0)
    );
}

#[test]
fn render_interp_blends_between_committed_ship_poses() {
    let previous = ShipPhysics {
        x: 10.0,
        y: 2.0,
        z: -4.0,
        yaw: 0.2,
        roll: -0.1,
        ..default()
    };
    let current = ShipPhysics {
        x: 14.0,
        y: 4.0,
        z: -8.0,
        yaw: 0.6,
        roll: 0.3,
        ..default()
    };
    let mut interp = RenderInterp::new(previous);
    interp.capture(current);

    let pose = interp.pose(0.25);

    assert!((pose.x - 11.0).abs() < 1e-6);
    assert!((pose.y - 2.5).abs() < 1e-6);
    assert!((pose.z + 5.0).abs() < 1e-6);
    assert!((pose.yaw - 0.3).abs() < 1e-6);
    assert!(pose.roll.abs() < 1e-6);
}

#[test]
fn render_interp_takes_short_path_across_yaw_wrap() {
    let previous = ShipPhysics {
        yaw: 179.0_f32.to_radians(),
        ..default()
    };
    let current = ShipPhysics {
        yaw: (-179.0_f32).to_radians(),
        ..default()
    };
    let mut interp = RenderInterp::new(previous);
    interp.capture(current);

    let halfway = interp.pose(0.5).yaw.to_degrees();

    assert!((halfway.abs() - 180.0).abs() < 1e-3);
}

#[test]
fn restoring_authoritative_pose_discards_frame_interpolation() {
    let committed = ShipPhysics {
        x: 20.0,
        y: 3.0,
        z: -12.0,
        yaw: 0.75,
        roll: 0.1,
        ..default()
    };
    let mut transform = Transform::from_xyz(19.5, 2.5, -11.5);

    write_ship_pose(&mut transform, committed);

    assert_eq!(transform.translation, Vec3::new(20.0, 3.0, -12.0));
    assert_eq!(
        transform.rotation,
        Quat::from_euler(EulerRot::YXZ, -0.75, 0.0, 0.1)
    );
}

#[test]
fn hull_camera_queries_are_disjoint() {
    let mut app = App::new();
    app.add_systems(Update, hull_camera);
    app.world_mut()
        .spawn((LocalShip, ShipViewMode::default(), Transform::default()));
    app.world_mut().spawn((GameCamera, Transform::default()));

    // Initialising the schedule is the regression check: overlapping
    // `Transform` queries panic with Bevy error B0001 before the system
    // body runs.
    app.update();
}

// ── Frame-rate-independent presentation (thrust-burst, track/thrust-burst-v2) ──
//
// The reported "AI/player thrust-burst" is a PRESENTATION artefact, not a
// sim one: the headless digest folds `ShipPhysics` and has held byte-
// identical, so the authoritative per-tick motion is provably smooth. The
// burst is browser-only and only appears when the render frame rate exceeds
// the `sim_tick_hz` (John's 144 Hz monitor against the shipped 60 Hz sim) —
// a regime a fixed-timestep headless run cannot exhibit and, because the
// sim floor is 30 Hz while a SwiftShader CI browser renders well under that,
// one no in-browser CI harness here can reach either.
//
// This test reaches it deterministically instead: it drives the REAL
// presentation bracket (`restore`/`capture`/`apply_local_ship_render_
// interpolation`) with a sub-tick `ManualDuration`, so several `Update`
// frames fall between each fixed step exactly as they do at 180 fps against
// a 30 Hz tick. It samples the drawn `Transform` of two constant-velocity
// ships — one `LocalShip`, one not — every frame. Before the fix the non-
// local hull is a step function (frozen between ticks, jumping on each one:
// the visible burst); the local hull, interpolated, moves a little each
// frame. The guard asserts BOTH move smoothly.
const REPRO_HZ: f32 = 30.0;
const REPRO_SUBFRAMES: u32 = 6; // 180 "fps" against a 30 Hz tick
const REPRO_STEP: f32 = 3.0; // world units advanced per fixed tick

#[derive(Component)]
struct ReproOther;

/// Constant-velocity authoritative motion: every ship advances the same
/// `REPRO_STEP` along +X each fixed tick. Runs in `FixedUpdate` before
/// `sync_ship_position` writes the authoritative transform.
fn repro_advance(mut q: Query<&mut ShipPhysics>) {
    for mut p in &mut q {
        p.x += REPRO_STEP;
    }
}

/// Build an app wired exactly like `RendererPlugin`'s presentation bracket,
/// plus authoritative motion, and return the per-frame drawn X of the local
/// and the non-local hull. `interpolate_all` mirrors the fix: when true the
/// non-local hull is given the same bracket the local hull has.
fn presented_x_series(interpolate_all: bool) -> (Vec<f32>, Vec<f32>) {
    use crate::ship::physics_systems::sync_ship_position;

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    let period = std::time::Duration::from_secs_f32(1.0 / REPRO_HZ);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    // Sub-tick pacing: REPRO_SUBFRAMES frames per fixed step.
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        period / REPRO_SUBFRAMES,
    ));

    app.add_systems(FixedFirst, restore_authoritative_local_ship_transform);
    app.add_systems(FixedLast, capture_local_ship_render_pose);
    app.add_systems(FixedUpdate, repro_advance.before(sync_ship_position));
    app.add_systems(FixedUpdate, sync_ship_position);
    app.add_systems(Update, apply_local_ship_render_interpolation);
    if interpolate_all {
        app.add_systems(FixedFirst, restore_authoritative_ship_transforms);
        app.add_systems(FixedLast, capture_ship_render_pose);
        app.add_systems(
            Update,
            apply_ship_render_interpolation.after(apply_local_ship_render_interpolation),
        );
    }

    let mut view_mode = ShipViewMode::default();
    view_mode.view_mode = ViewMode::Cinematic;
    let local = app
        .world_mut()
        .spawn((
            LocalShip,
            view_mode,
            ShipPhysics::default(),
            Transform::default(),
        ))
        .id();
    let other = app
        .world_mut()
        .spawn((ReproOther, ShipPhysics::default(), Transform::default()))
        .id();

    // Baseline frame (zero delta, no fixed step) then run several ticks.
    app.update();
    let mut local_xs = Vec::new();
    let mut other_xs = Vec::new();
    for _ in 0..(REPRO_SUBFRAMES * 6) {
        app.update();
        local_xs.push(app.world().get::<Transform>(local).unwrap().translation.x);
        other_xs.push(app.world().get::<Transform>(other).unwrap().translation.x);
    }
    (local_xs, other_xs)
}

/// The largest single-frame jump in a presented position series, ignoring a
/// warm-up prefix (the first fixed step has `prev == curr`, so nothing moves
/// until the second tick commits a real delta).
fn max_frame_jump(series: &[f32]) -> f32 {
    let warmup = (REPRO_SUBFRAMES * 2) as usize;
    series
        .windows(2)
        .skip(warmup)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0_f32, f32::max)
}

#[test]
fn unfixed_non_local_hull_bursts_while_local_hull_is_smooth() {
    // Documents the defect: with only the local-ship bracket, the non-local
    // hull's drawn position is a step function. A smooth frame advances
    // about REPRO_STEP / REPRO_SUBFRAMES = 0.5; a burst frame advances a
    // whole REPRO_STEP = 3.0.
    let (local, other) = presented_x_series(false);
    let smooth_frame = REPRO_STEP / REPRO_SUBFRAMES as f32;
    eprintln!(
        "UNFIXED local presented X per frame: {:?}",
        local
            .iter()
            .map(|x| (x * 100.0).round() / 100.0)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "UNFIXED non-local presented X per frame: {:?}",
        other
            .iter()
            .map(|x| (x * 100.0).round() / 100.0)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "UNFIXED max frame jump  local={:.3}  non-local={:.3}  (smooth≈{:.3}, tick step={:.1})",
        max_frame_jump(&local),
        max_frame_jump(&other),
        smooth_frame,
        REPRO_STEP
    );

    // The local hull never jumps a whole tick's worth — it interpolates.
    assert!(
        max_frame_jump(&local) < smooth_frame * 1.5,
        "local hull should be smooth, max frame jump was {}",
        max_frame_jump(&local)
    );
    // The non-local hull jumps a full tick's displacement — the burst.
    assert!(
        max_frame_jump(&other) > REPRO_STEP * 0.9,
        "non-local hull should burst by a full tick step, max frame jump was {}",
        max_frame_jump(&other)
    );
}

#[test]
fn every_hull_presents_smoothly_when_all_ships_interpolate() {
    // The guard: with the generalised bracket, BOTH hulls move a little each
    // frame and neither jumps a whole tick's displacement. Red against the
    // unfixed tree, green with the fix.
    let (local, other) = presented_x_series(true);
    let smooth_frame = REPRO_STEP / REPRO_SUBFRAMES as f32;
    eprintln!(
        "FIXED non-local presented X per frame: {:?}",
        other
            .iter()
            .map(|x| (x * 100.0).round() / 100.0)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "FIXED max frame jump  local={:.3}  non-local={:.3}  (smooth≈{:.3}, tick step={:.1})",
        max_frame_jump(&local),
        max_frame_jump(&other),
        smooth_frame,
        REPRO_STEP
    );

    assert!(
        max_frame_jump(&local) < smooth_frame * 1.5,
        "local hull should be smooth, max frame jump was {}",
        max_frame_jump(&local)
    );
    assert!(
        max_frame_jump(&other) < smooth_frame * 1.5,
        "non-local hull should be smooth once all ships interpolate, \
             max frame jump was {}",
        max_frame_jump(&other)
    );
}

// ── First-person half of the thrust-burst fix ──────────────────────────
//
// In first-person `Camera` view the local hull is HIDDEN, but `hull_camera`
// positions the whole view from the local ship's root `Transform` (a marker
// point on its rig). Before this fix `apply_local_ship_render_interpolation`
// wrote the exact-tick pose there in every non-Cinematic mode, so on a
// >tick-rate monitor the first-person view stepped once per sim tick — the
// same burst the Cinematic path already fixed. The fix interpolates the root
// transform in `Camera` view too; overlay views (Radar/Comms/charts) keep
// the exact tick pose, since the hull is hidden AND `hull_camera` freezes
// the last view, so the pose is never read there.

/// Drive the local-ship presentation bracket with a given view mode and
/// return the local hull's per-frame drawn X. Mirrors `presented_x_series`
/// but focuses on the local hull under different `ViewMode`s.
fn presented_local_x_in_view(view_mode: ViewMode) -> Vec<f32> {
    use crate::ship::physics_systems::sync_ship_position;

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    let period = std::time::Duration::from_secs_f32(1.0 / REPRO_HZ);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        period / REPRO_SUBFRAMES,
    ));

    app.add_systems(FixedFirst, restore_authoritative_local_ship_transform);
    app.add_systems(FixedLast, capture_local_ship_render_pose);
    app.add_systems(FixedUpdate, repro_advance.before(sync_ship_position));
    app.add_systems(FixedUpdate, sync_ship_position);
    app.add_systems(Update, apply_local_ship_render_interpolation);

    let mut vm = ShipViewMode::default();
    vm.view_mode = view_mode;
    let local = app
        .world_mut()
        .spawn((LocalShip, vm, ShipPhysics::default(), Transform::default()))
        .id();

    app.update();
    let mut xs = Vec::new();
    for _ in 0..(REPRO_SUBFRAMES * 6) {
        app.update();
        xs.push(app.world().get::<Transform>(local).unwrap().translation.x);
    }
    xs
}

#[test]
fn local_hull_presents_smoothly_in_first_person_view() {
    // The guard for the first-person half of the fix: in `Camera` view the
    // local ship's root transform (which `hull_camera` reads to place the
    // view) advances a fraction of a tick each frame instead of a whole
    // tick's step. Red against the un-interpolated first-person path (which
    // wrote `interp.current`), green with the fix.
    let xs = presented_local_x_in_view(ViewMode::Camera(CameraView::default()));
    let smooth_frame = REPRO_STEP / REPRO_SUBFRAMES as f32;
    eprintln!(
        "FIRST-PERSON local presented X per frame: {:?}",
        xs.iter()
            .map(|x| (x * 100.0).round() / 100.0)
            .collect::<Vec<_>>()
    );
    eprintln!(
        "FIRST-PERSON max frame jump = {:.3}  (smooth≈{:.3}, tick step={:.1})",
        max_frame_jump(&xs),
        smooth_frame,
        REPRO_STEP
    );
    assert!(
        max_frame_jump(&xs) < smooth_frame * 1.5,
        "first-person local hull should present smoothly, max frame jump was {}",
        max_frame_jump(&xs)
    );
}

#[test]
fn overlay_view_keeps_exact_tick_pose() {
    // The paired sibling that proves the harness above genuinely detects a
    // step function: overlay views deliberately keep the exact tick pose (the
    // hull is hidden and `hull_camera` freezes the view, so nothing reads it)
    // and therefore still burst by a whole tick. This is exactly the
    // un-interpolated path the first-person fix moved AWAY from.
    let xs = presented_local_x_in_view(ViewMode::Radar);
    assert!(
        max_frame_jump(&xs) > REPRO_STEP * 0.9,
        "overlay view keeps the exact tick pose and steps a full tick, jump was {}",
        max_frame_jump(&xs)
    );
}

// ── Viewscreen HDR / bloom calibration (PRD #1023, module 5) ──────

use crate::render_setup::RenderTuning;
use crate::world::config::{RenderConfig, WorldConfig};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::{Bloom, BloomCompositeMode};
use bevy::render::view::Hdr;

/// Run `apply_world_render_config` over a world config and report what the
/// game camera ended up carrying.
fn adopt(world: Option<WorldConfig>) -> App {
    let mut app = App::new();
    app.init_resource::<RenderTuning>()
        .add_systems(Update, apply_world_render_config);
    if let Some(world) = world {
        app.insert_resource(world);
    }
    app.world_mut().spawn(GameCamera);
    app.update();
    app
}

fn game_camera(app: &mut App) -> Entity {
    let mut q = app.world_mut().query_filtered::<Entity, With<GameCamera>>();
    q.single(app.world()).unwrap()
}

/// The whole point of the module: the viewscreen renders through an HDR
/// intermediate, so an emissive authored at nine times white is no longer
/// clipped to the same white as a value of one, and blooms.
#[test]
fn a_world_with_no_render_block_still_gets_hdr() {
    let mut app = adopt(None);
    let camera = game_camera(&mut app);
    assert!(
        app.world().get::<Hdr>(camera).is_some(),
        "the default calibration is HDR ON — that is what stops the clamp"
    );
    assert_eq!(
        app.world().get::<Tonemapping>(camera),
        Some(&Tonemapping::TonyMcMapface),
        "the display transform is named rather than inherited"
    );
    assert_eq!(
        app.world().get::<Bloom>(camera).is_some(),
        crate::render_setup::BLOOM_RUNS_ON_THIS_TARGET,
        "the default calibration asks for bloom and the PLATFORM answers: \
             attached where the backend can draw it, absent on WebGL2, which is \
             what every browser target here is. See `BloomConfig`."
    );
}

/// The gate is the whole platform story, so it gets its own assertion rather
/// than being left implied by the default: a world that explicitly asks for
/// bloom still does not get it where the backend cannot draw it.
///
/// This is the case that used to be only a comment. Before the gate, an
/// authored `enabled = true` reached the browser host's camera and failed
/// its render graph, and the only thing between a designer and that was
/// documentation.
#[test]
fn an_authored_bloom_is_refused_where_the_backend_cannot_draw_it() {
    let mut cfg = RenderConfig::default();
    cfg.bloom.enabled = true;
    let mut app = adopt(Some(WorldConfig {
        render: Some(cfg),
        ..Default::default()
    }));
    let camera = game_camera(&mut app);
    assert_eq!(
        app.world().get::<Bloom>(camera).is_some(),
        crate::render_setup::BLOOM_RUNS_ON_THIS_TARGET,
        "authoring cannot override a platform that has no bloom pass"
    );
    assert!(
        app.world().get::<Hdr>(camera).is_some(),
        "HDR and the display transform are unaffected by the bloom gate — \
             they are what stop the clamp, and they run on every target"
    );
}

/// The authored calibration is what reaches the camera where it can run:
/// thresholded at exactly screen white, composited additively. The numbers
/// are `[ai]`-flagged in `BloomConfig` and this pins them, so a platform
/// gaining bloom inherits the calibration rather than Bevy's own preset.
#[cfg_attr(
    target_arch = "wasm32",
    ignore = "no bloom component on a WebGL2 target — see BLOOM_RUNS_ON_THIS_TARGET"
)]
#[test]
fn the_bloom_calibration_is_authored_and_one_line_away() {
    let mut cfg = RenderConfig::default();
    cfg.bloom.enabled = true;
    let mut app = adopt(Some(WorldConfig {
        render: Some(cfg),
        ..Default::default()
    }));
    let camera = game_camera(&mut app);
    let bloom = app
        .world()
        .get::<Bloom>(camera)
        .expect("an enabled bloom block reaches the camera");
    assert_eq!(
        bloom.prefilter.threshold, 1.0,
        "bloom is thresholded at exactly screen white, so what glows is \
             precisely what the low-dynamic-range target used to throw away"
    );
    assert_eq!(
        bloom.composite_mode,
        BloomCompositeMode::Additive,
        "a thresholded prefilter wants additive compositing"
    );
}

/// The retreat has to work: a world that turns HDR off must leave the
/// camera with neither the float target nor the bloom pass, or the off
/// switch costs frame time for an effect that cannot show.
#[test]
fn a_world_can_turn_the_whole_thing_off() {
    let mut app = adopt(Some(WorldConfig {
        render: Some(RenderConfig {
            hdr: false,
            ..Default::default()
        }),
        ..Default::default()
    }));
    let camera = game_camera(&mut app);
    assert!(app.world().get::<Hdr>(camera).is_none());
    assert!(
        app.world().get::<Bloom>(camera).is_none(),
        "bloom with nothing above white to bloom is pure cost"
    );
}

/// Bloom can be taken back OFF after being turned on.
/// `apply_render_config` runs twice — once when the camera is spawned, once
/// when the world lands — so it has to be able to reverse itself, or a
/// world's `[render]` block could only ever add effects.
///
/// The reversal is now driven by an explicit `enabled = false` rather than
/// by the absent-config default, because that default asks for bloom since
/// the platform gate took over the "can it be drawn" question. This is the
/// stronger test of the two: it proves a world can decline an effect the
/// platform is perfectly able to run.
#[cfg_attr(
    target_arch = "wasm32",
    ignore = "nothing to take back off on a WebGL2 target"
)]
#[test]
fn bloom_can_be_taken_back_off_while_hdr_stays() {
    let mut app = App::new();
    app.init_resource::<RenderTuning>()
        .add_systems(Update, apply_world_render_config);
    let camera = app.world_mut().spawn(GameCamera).id();
    let mut on = RenderConfig::default();
    on.bloom.enabled = true;
    {
        let mut commands = app.world_mut().commands();
        crate::render_setup::apply_render_config(&mut commands, camera, &on);
    }
    app.world_mut().flush();
    assert!(app.world().get::<Bloom>(camera).is_some());

    // A world that declines it: HDR stays, the bloom pass goes.
    let mut off = RenderConfig::default();
    off.bloom.enabled = false;
    app.world_mut().insert_resource(WorldConfig {
        render: Some(off),
        ..Default::default()
    });
    app.update();
    assert!(app.world().get::<Hdr>(camera).is_some());
    assert!(app.world().get::<Bloom>(camera).is_none());
}

/// The authored timings reach the resource the LOD swap reads, and the
/// nonsense values a TOML can carry are clamped rather than trusted.
#[test]
fn the_authored_timings_reach_the_render_tuning_resource() {
    let app = adopt(Some(WorldConfig {
        render: Some(RenderConfig {
            lod_fade_secs: 0.5,
            materialise_secs: -3.0,
            materialise_start_scale: 4.0,
            ..Default::default()
        }),
        ..Default::default()
    }));
    let tuning = *app.world().resource::<RenderTuning>();
    assert_eq!(tuning.lod_fade_secs, 0.5);
    assert_eq!(
        tuning.materialise_secs, 0.0,
        "a negative window is no window, not a window that runs backwards"
    );
    assert_eq!(tuning.materialise_start_scale, 1.0);
}

fn camera_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_systems(Update, toggle_cameras);
    app.world_mut().spawn((LocalShip, ShipViewMode::default()));
    app.world_mut().spawn((
        GameCamera,
        Camera {
            is_active: false,
            ..default()
        },
    ));
    app
}

fn game_camera_active(app: &mut App) -> bool {
    let mut q = app
        .world_mut()
        .query_filtered::<&Camera, With<GameCamera>>();
    q.single(app.world()).unwrap().is_active
}

#[test]
fn overlay_view_modes_keep_game_camera_rendering() {
    let mut app = camera_test_app();

    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();
    assert!(game_camera_active(&mut app));

    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipViewMode, With<LocalShip>>();
        q.single_mut(app.world_mut()).unwrap().view_mode = ViewMode::Radar;
    }
    app.update();

    assert!(game_camera_active(&mut app));
}

// ── Local ship hull visibility (issue #944) ───────────────────────

use crate::server_app::LocalShipModel;

/// Headless app carrying just the local ship and the visibility system.
/// No model child yet — tests insert it when they want it to "finish
/// loading", which is the whole point of the race being reproduced.
fn hull_visibility_test_app() -> (App, Entity) {
    let mut app = App::new();
    app.add_systems(Update, toggle_ship_model_visibility);
    let ship = app
        .world_mut()
        .spawn((LocalShip, ShipViewMode::default()))
        .id();
    // A second ship with no `LocalShip` marker: every hull carries a
    // `ShipViewMode`, so the system must filter rather than assume one.
    app.world_mut().spawn(ShipViewMode::default());
    (app, ship)
}

fn set_view_mode(app: &mut App, mode: ViewMode) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipViewMode, With<LocalShip>>();
    q.single_mut(app.world_mut()).unwrap().view_mode = mode;
}

/// Mimics `server_app::decorate_local_ship_model`: the async GLB finally
/// resolves and its scene-root child is inserted, hidden by default.
fn spawn_hidden_model_child(app: &mut App, ship: Entity) -> Entity {
    let child = app
        .world_mut()
        .spawn((LocalShipModel, Visibility::Hidden))
        .id();
    app.world_mut().entity_mut(ship).add_child(child);
    child
}

fn hull_visibility(app: &mut App, child: Entity) -> Visibility {
    *app.world().entity(child).get::<Visibility>().unwrap()
}

/// The issue #944 race: the view mode flips to Cinematic *before* the
/// multi-megabyte GLB finishes loading, so the model child arrives hidden
/// after the only `Changed<ShipViewMode>` event has already fired. Nothing
/// changes the view mode again, so an edge-triggered toggle leaves the hull
/// invisible forever while the engine trails keep rendering.
#[test]
fn hull_becomes_visible_when_model_loads_after_switch_to_cinematic() {
    let (mut app, ship) = hull_visibility_test_app();
    app.update();

    set_view_mode(&mut app, ViewMode::Cinematic);
    app.update(); // model still loading — nothing to reveal yet

    let child = spawn_hidden_model_child(&mut app, ship);
    app.update(); // no further view-mode change

    assert_eq!(hull_visibility(&mut app, child), Visibility::Visible);
}

/// The ordinary ordering (model loaded first, then the switch) must keep
/// working too.
#[test]
fn hull_becomes_visible_when_cinematic_selected_after_model_loads() {
    let (mut app, ship) = hull_visibility_test_app();
    let child = spawn_hidden_model_child(&mut app, ship);
    app.update();
    assert_eq!(hull_visibility(&mut app, child), Visibility::Hidden);

    set_view_mode(&mut app, ViewMode::Cinematic);
    app.update();

    assert_eq!(hull_visibility(&mut app, child), Visibility::Visible);
}

/// Original intent preserved: outside cinematic the hull stays hidden so it
/// cannot occlude the viewscreen, whichever order things arrive in.
#[test]
fn hull_stays_hidden_in_non_cinematic_view_modes() {
    let (mut app, ship) = hull_visibility_test_app();
    let child = spawn_hidden_model_child(&mut app, ship);
    app.update();
    assert_eq!(hull_visibility(&mut app, child), Visibility::Hidden);

    for mode in [
        ViewMode::Camera(CameraView::default()),
        ViewMode::Radar,
        ViewMode::SystemChart,
        ViewMode::Comms,
    ] {
        set_view_mode(&mut app, mode);
        app.update();
        assert_eq!(hull_visibility(&mut app, child), Visibility::Hidden);
    }
}

/// Leaving cinematic hides the hull again, and a model respawned afterwards
/// (LOD swap / hull change re-inserts `Visibility::Hidden`) is re-revealed
/// without needing another view-mode change.
#[test]
fn respawned_model_is_re_revealed_while_still_in_cinematic() {
    let (mut app, ship) = hull_visibility_test_app();
    let child = spawn_hidden_model_child(&mut app, ship);
    set_view_mode(&mut app, ViewMode::Cinematic);
    app.update();
    assert_eq!(hull_visibility(&mut app, child), Visibility::Visible);

    set_view_mode(&mut app, ViewMode::Camera(CameraView::default()));
    app.update();
    assert_eq!(hull_visibility(&mut app, child), Visibility::Hidden);

    set_view_mode(&mut app, ViewMode::Cinematic);
    app.update();
    app.world_mut().entity_mut(child).despawn();
    let respawned = spawn_hidden_model_child(&mut app, ship);
    app.update();

    assert_eq!(hull_visibility(&mut app, respawned), Visibility::Visible);
}

/// Tally of frames on which the hull's `Visibility` looked dirty to a
/// downstream consumer. Must be observed from *inside* the schedule: a
/// `Changed<Visibility>` query built from `&World` after `app.update()`
/// takes `last_run = world.last_change_tick()`, which `clear_trackers()`
/// has just advanced past every write the frame made — so it reports 0
/// unconditionally and would assert nothing.
#[derive(Resource, Default)]
struct HullVisibilityDirtied(usize);

/// Stand-in for any real `Changed<Visibility>` consumer. Registered after
/// `toggle_ship_model_visibility`, its `last_run` is its own previous run,
/// so it sees exactly what a downstream system would see.
fn count_dirtied_hulls(
    mut dirtied: ResMut<HullVisibilityDirtied>,
    hulls: Query<(), (With<LocalShipModel>, Changed<Visibility>)>,
) {
    dirtied.0 += hulls.iter().count();
}

/// The write is conditional: a steady view mode must not dirty `Visibility`
/// every frame, or downstream `Changed<Visibility>` consumers churn.
/// Replacing `set_if_neq` with `*vis = wanted` makes this fail.
#[test]
fn steady_view_mode_does_not_dirty_visibility_every_frame() {
    let (mut app, ship) = hull_visibility_test_app();
    app.init_resource::<HullVisibilityDirtied>();
    app.add_systems(
        Update,
        count_dirtied_hulls.after(toggle_ship_model_visibility),
    );
    let child = spawn_hidden_model_child(&mut app, ship);
    set_view_mode(&mut app, ViewMode::Cinematic);

    app.update();
    assert_eq!(hull_visibility(&mut app, child), Visibility::Visible);
    // Sanity: the observer really does see the Hidden→Visible transition,
    // so a later count of 1 means "no churn", not "query never matched".
    assert_eq!(
        app.world().resource::<HullVisibilityDirtied>().0,
        1,
        "the one real transition should register as dirty"
    );

    // Nothing changes from here on: same view mode, same hull.
    app.update();
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<HullVisibilityDirtied>().0,
        1,
        "steady state re-dirtied Visibility; the write is unconditional"
    );
}
