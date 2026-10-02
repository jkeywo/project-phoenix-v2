use super::*;

#[test]
fn restore_rebase_replaces_old_hud_and_republishes_unchanged_current_values() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.init_resource::<Messages<HudStateChanged>>();
    world.insert_resource(State::new(GamePhase::InProgress));
    world.run_system_once(spawn_hud_state_entity).unwrap();
    world.spawn((
        crate::server_app::LocalShip,
        crate::ship::state::ShipRedAlert(true),
        crate::ship_plugin::LastHelmInput {
            thrust: 0.6,
            ..Default::default()
        },
    ));
    world
        .resource_mut::<Messages<HudStateChanged>>()
        .write(HudStateChanged {
            json: "pre-restore HUD".into(),
        });
    rebase_hud_state(&mut world);
    let hud = &world.query::<&ViewscreenHud>().single(&world).unwrap().0;
    assert!(hud.red_alert);
    assert_eq!(hud.engine_thrust, 0.6);
    let expected = codec::encode_hud_state(hud).unwrap();
    let first: Vec<_> = world
        .resource_mut::<Messages<HudStateChanged>>()
        .drain()
        .collect();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].json, expected);
    rebase_hud_state(&mut world);
    let second: Vec<_> = world
        .resource_mut::<Messages<HudStateChanged>>()
        .drain()
        .collect();
    assert_eq!(
        second.len(),
        1,
        "an unchanged restored HUD still seeds current loops"
    );
    assert_eq!(second[0].json, expected);
}

#[test]
fn presentation_readiness_requires_terminal_preload_but_not_a_renderer() {
    let mut preload = AssetPreloadResource::default();

    assert!(
        local_presentation_ready(None),
        "a rendererless host has no presentation preload to wait for"
    );
    assert!(
        !local_presentation_ready(Some(&preload)),
        "an unstarted or in-flight preload is not terminal"
    );

    preload.started = true;
    assert!(!local_presentation_ready(Some(&preload)));

    preload.complete = true;
    assert!(local_presentation_ready(Some(&preload)));
}

#[test]
fn local_start_assets_wait_for_authoritative_rigs_without_a_renderer() {
    let rigs = crate::entities::model_markers::ModelRigReadiness::default();

    assert!(local_start_assets_ready(None, None));
    assert!(
        !local_start_assets_ready(None, Some(&rigs)),
        "rendererless does not mean canonical weapon geometry is ready"
    );

    let mut app = App::new();
    app.init_resource::<crate::entities::model_markers::ModelRigReadiness>()
        .add_systems(
            Update,
            crate::entities::model_markers::sync_authoritative_model_markers,
        );
    app.update();
    assert!(local_start_assets_ready(
        None,
        Some(
            app.world()
                .resource::<crate::entities::model_markers::ModelRigReadiness>()
        )
    ));
}

#[test]
fn host_lobby_roster_excludes_auxiliary_stations() {
    use crate::core::messages::StationId;
    use crate::lobby::stations_config::StationDef;

    let station = |id: &str, name: &str, auxiliary: bool| StationDef {
        id: StationId(id.into()),
        name: name.into(),
        description: String::new(),
        rank: String::new(),
        short_code: name.chars().take(3).collect(),
        console: None,
        ratings: vec!["Std".into()],
        human_seeking: auxiliary,
        host_order: vec![],
        visiting_rating: auxiliary.then(|| "Std".into()),
        auxiliary,
        command_target: None,
    };
    let stations = ShipStations {
        stations: vec![
            station("captain", "Captain", false),
            station("command", "Command", true),
        ],
    };
    let players = vec![
        Player {
            token: "captain-token".into(),
            name: "Ada".into(),
            connected: true,
            ready: true,
            station: Some(StationId("captain".into())),
            last_rating: None,
            spectator: false,
            afk: false,
        },
        Player {
            token: "auxiliary-token".into(),
            name: "Grace".into(),
            connected: true,
            ready: false,
            station: Some(StationId("command".into())),
            last_rating: None,
            spectator: false,
            afk: false,
        },
    ];

    let roster = claimable_lobby_roster(&stations, &players);

    assert_eq!(
        roster.stations.len(),
        1,
        "only the claimable seat becomes a card"
    );
    assert_eq!(roster.stations[0].name, "Captain");
    assert_eq!(roster.stations[0].holder_name.as_deref(), Some("Ada"));
    assert!(roster
        .stations
        .iter()
        .all(|payload| payload.name != "Command"));
    assert_eq!(
        roster.max_players, 1,
        "auxiliary Stations are not lobby slots"
    );
    assert_eq!(
        roster.crew_count, 1,
        "crew counts only claimable held seats"
    );
    assert!(
        roster.all_filled,
        "an empty auxiliary Station cannot block filled state"
    );
}

// ── motion comfort: shake_magnitude (issues #1173, #1428) ────────

#[test]
fn shake_scales_with_damage_and_saturates() {
    // Full intensity: linear up to the full-hit threshold, then clamped to
    // the max magnitude.
    let half = shake_magnitude(SHAKE_DAMAGE_FULL / 2.0, 1.0);
    assert!((half - SHAKE_MAX_MAGNITUDE * 0.5).abs() < 1e-6);
    let full = shake_magnitude(SHAKE_DAMAGE_FULL, 1.0);
    assert!((full - SHAKE_MAX_MAGNITUDE).abs() < 1e-6);
    // Beyond the threshold it does not keep growing.
    let over = shake_magnitude(SHAKE_DAMAGE_FULL * 10.0, 1.0);
    assert!((over - SHAKE_MAX_MAGNITUDE).abs() < 1e-6);
}

#[test]
fn shake_off_is_exactly_zero_at_any_damage() {
    // Issue #1428: "off" is an intensity of zero and it is absolute — the
    // AC1 guarantee for BOTH render paths, now reached by one number rather
    // than by a flag beside a scale.
    assert_eq!(shake_magnitude(SHAKE_DAMAGE_FULL, 0.0), 0.0);
    assert_eq!(shake_magnitude(SHAKE_DAMAGE_FULL * 100.0, 0.0), 0.0);
    assert_eq!(shake_magnitude(5.0, 0.0), 0.0);
}

#[test]
fn shake_intensity_dials_between_full_and_off() {
    // The gentler stop is a genuinely smaller movement, not a switch.
    let base = shake_magnitude(SHAKE_DAMAGE_FULL, 1.0);
    let half = shake_magnitude(SHAKE_DAMAGE_FULL, 0.5);
    assert!((half - base * 0.5).abs() < 1e-6);
    // Out-of-range intensity is clamped, never amplified past the max.
    let over = shake_magnitude(SHAKE_DAMAGE_FULL, 4.0);
    assert!((over - SHAKE_MAX_MAGNITUDE).abs() < 1e-6);
}

#[test]
fn no_damage_no_shake() {
    assert_eq!(shake_magnitude(0.0, 1.0), 0.0);
}

// ── motion comfort: scaled_flash_intensity (issues #1173, #1428) ─

#[test]
fn flash_passes_through_at_full_intensity() {
    // Nothing asked for less: the decayed value is untouched.
    assert_eq!(scaled_flash_intensity(1.0, 1.0), 1.0);
    assert_eq!(scaled_flash_intensity(0.42, 1.0), 0.42);
    assert_eq!(scaled_flash_intensity(0.0, 1.0), 0.0);
}

#[test]
fn flash_off_removes_the_jolt_entirely() {
    assert_eq!(scaled_flash_intensity(1.0, 0.0), 0.0);
    assert_eq!(scaled_flash_intensity(0.7, 0.0), 0.0);
}

#[test]
fn flash_intensity_dims_rather_than_switching() {
    // Issue #1428 replaced the all-or-nothing cap with a scale, so the
    // gentler stop still shows a shield hit — dimmer, not absent. That is
    // the "essential feedback survives a reduced effect" half of story 15.
    let dimmed = scaled_flash_intensity(1.0, 0.3);
    assert!((dimmed - 0.3).abs() < 1e-6);
    assert!(dimmed > 0.0, "a gentler flash is still a visible flash");
    // Out-of-range intensity cannot amplify the jolt.
    assert_eq!(scaled_flash_intensity(1.0, 4.0), 1.0);
}

#[test]
fn viewscreen_motion_default_is_full_normal_motion() {
    let m = ViewscreenMotion::default();
    assert!(!m.reduced_motion);
    assert_eq!(m.shake_intensity, DEFAULT_SHAKE_INTENSITY);
    assert_eq!(m.flash_intensity, DEFAULT_FLASH_INTENSITY);
    // The default must leave both effects untouched from the pre-#1173
    // formulas.
    assert!(
        (shake_magnitude(SHAKE_DAMAGE_FULL, m.shake_intensity) - SHAKE_MAX_MAGNITUDE).abs() < 1e-6
    );
    assert_eq!(scaled_flash_intensity(0.8, m.flash_intensity), 0.8);
}

#[test]
fn following_the_preference_is_what_reduced_motion_used_to_do() {
    // The resolution `sync_viewscreen_motion` performs, stated as the rule
    // it is: an effect that has published nothing takes the preference's
    // default, and that default under reduce is exactly zero — so a build
    // whose page only ever calls `wasm_set_reduced_motion` behaves as it did
    // before issue #1428 split the lever in three.
    let following = |reduced: bool| {
        if reduced {
            REDUCED_MOTION_INTENSITY
        } else {
            DEFAULT_SHAKE_INTENSITY
        }
    };
    assert_eq!(shake_magnitude(SHAKE_DAMAGE_FULL, following(true)), 0.0);
    assert_eq!(scaled_flash_intensity(1.0, following(true)), 0.0);
    assert!(shake_magnitude(SHAKE_DAMAGE_FULL, following(false)) > 0.0);
    assert_eq!(scaled_flash_intensity(1.0, following(false)), 1.0);
}

#[test]
fn an_explicit_intensity_outranks_the_preference_in_both_directions() {
    // The point of the `Option`: a published value is the operator's, and a
    // published `0.0` is a choice rather than an absence. Both are honoured
    // over whatever the machine's preference would have defaulted to.
    let resolve = |published: Option<f32>, reduced: bool| {
        let following = if reduced {
            REDUCED_MOTION_INTENSITY
        } else {
            DEFAULT_SHAKE_INTENSITY
        };
        published.unwrap_or(following).clamp(0.0, 1.0)
    };
    // Keep the shake on a machine whose OS asked to reduce motion.
    assert!(shake_magnitude(SHAKE_DAMAGE_FULL, resolve(Some(1.0), true)) > 0.0);
    // Turn it off on a machine whose OS asked for nothing.
    assert_eq!(
        shake_magnitude(SHAKE_DAMAGE_FULL, resolve(Some(0.0), false)),
        0.0
    );
    // Publishing nothing follows.
    assert_eq!(shake_magnitude(SHAKE_DAMAGE_FULL, resolve(None, true)), 0.0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_decorative_band_the_hud_overlay_is_stamped_from_follows_the_same_rule() {
    // The third effect has no uniform and no camera, but the native
    // Viewscreen still has to be told about it: its frame, readout and
    // red-alert vignette are a separate DOCUMENT, and
    // `panes::ultralight::cache_hud_state` stamps that document from this
    // resource. Resolved here, beside the two the renderer owns, so a
    // display following the machine cannot end up with a glow that
    // disagrees with the shader behind it.
    use crate::server::bridge::{
        clear_native_effect_intensities, set_native_effect_intensities,
        NATIVE_EFFECT_LATCH_TEST_LOCK,
    };
    use bevy::ecs::system::RunSystemOnce;

    let _serialised = NATIVE_EFFECT_LATCH_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_native_effect_intensities();

    let mut app = App::new();
    app.init_resource::<ViewscreenMotion>();
    let resolved = |app: &mut App| {
        app.world_mut().run_system_once(sync_viewscreen_motion).ok();
        app.world().resource::<ViewscreenMotion>().clone()
    };

    // Nothing published, and a machine that asked for nothing: full.
    let motion = resolved(&mut app);
    assert_eq!(motion.decorative_intensity, DEFAULT_DECORATIVE_INTENSITY);

    // An explicit off is the operator's, and beats the machine either way.
    set_native_effect_intensities(Some(100), Some(100), Some(0));
    assert_eq!(resolved(&mut app).decorative_intensity, 0.0);

    // Following, on a machine that asked to reduce, is exactly zero — what
    // `data-reduced-motion` did on its own before the lever was split.
    clear_native_effect_intensities();
    app.world_mut()
        .resource_mut::<ViewscreenMotion>()
        .reduced_motion = true;
    assert_eq!(
        resolved(&mut app).decorative_intensity,
        REDUCED_MOTION_INTENSITY
    );

    // …and an explicit KEEP survives that same machine, which is the
    // direction a one-lever build could not express.
    set_native_effect_intensities(None, None, Some(100));
    assert_eq!(resolved(&mut app).decorative_intensity, 1.0);

    clear_native_effect_intensities();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_native_latch_carries_a_choice_and_a_reset() {
    use crate::server::bridge::{
        published_effect_intensities, set_native_effect_intensities, NATIVE_EFFECT_LATCH_TEST_LOCK,
    };

    // The latch is process-global, and the host-lobby test that seeds it
    // from a saved record shares it; one lock keeps the two off each other.
    let _serialised = NATIVE_EFFECT_LATCH_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Whole percent in, fractions out — the shape the setting already
    // crosses the page/host bridge in (issue #1428).
    set_native_effect_intensities(Some(30), Some(0), Some(40));
    let (shake, flash, decorative) = published_effect_intensities();
    assert!((shake.expect("a published shake") - 0.3).abs() < 1e-6);
    assert_eq!(
        flash,
        Some(0.0),
        "a published zero is a choice, not an absence"
    );
    // The third effect rides the same latch but is nobody's uniform: it is
    // the band the native HUD overlay's document is stamped with.
    assert!((decorative.expect("a published band") - 0.4).abs() < 1e-6);

    // A per-setting reset publishes nothing again, so the effect goes back
    // to following the machine rather than sticking at its last number.
    set_native_effect_intensities(None, None, None);
    assert_eq!(published_effect_intensities(), (None, None, None));
}

// ── compute_hud_state ────────────────────────────────────────────

#[test]
fn compute_hud_state_nominal() {
    let physics = ShipPhysics::default();
    let state = compute_hud_state(
        false,
        &physics,
        100.0,
        100.0,
        0.0,
        false,
        &GamePhase::InProgress,
        None,
        None,
        None,
        None,
    );
    assert_eq!(state.heading, 0);
    assert_eq!(state.hull_pct, 100);
    // The condition rides the wire as a string id (issue #975), resolved on
    // the client; Rust's contract is the id, not the English word.
    assert_eq!(state.condition, "server.hud_nominal");
    assert!(!state.red_alert);
    assert_eq!(state.engine_thrust, 0.0);
    assert!(!state.phaser_firing);
    assert!(state.game_over_message.is_none());
}

#[test]
fn compute_hud_state_alert_and_partial_hull() {
    let physics = ShipPhysics {
        yaw: std::f32::consts::FRAC_PI_2,
        ..Default::default()
    };
    let state = compute_hud_state(
        true,
        &physics,
        50.0,
        100.0,
        0.75,
        true,
        &GamePhase::InProgress,
        None,
        None,
        None,
        None,
    );
    assert_eq!(state.heading, 90);
    assert_eq!(state.hull_pct, 50);
    assert_eq!(state.condition, "server.hud_alert");
    assert!(state.red_alert);
    assert!((state.engine_thrust - 0.75).abs() < f32::EPSILON);
    assert!(state.phaser_firing);
}

#[test]
fn compute_hud_state_engine_thrust_propagated() {
    let physics = ShipPhysics::default();
    let state = compute_hud_state(
        false,
        &physics,
        100.0,
        100.0,
        0.5,
        false,
        &GamePhase::InProgress,
        None,
        None,
        None,
        None,
    );
    assert_eq!(state.engine_thrust, 0.5);
}

#[test]
fn compute_hud_state_game_over_ship_destroyed() {
    use crate::server_app::GameOverReason;
    let physics = ShipPhysics::default();
    // The built-in death sites latch the string id (issue #977);
    // `compute_hud_state` passes it through, `localiseTree` resolves it to
    // "Ship Destroyed" on the client. No English is composed here.
    let reason = GameOverReason(Some("server.game_over.ship_destroyed".into()), None);
    let state = compute_hud_state(
        false,
        &physics,
        0.0,
        100.0,
        0.0,
        false,
        &GamePhase::GameOver,
        Some(&reason),
        None,
        None,
        None,
    );
    assert_eq!(
        state.game_over_message.as_deref(),
        Some("server.game_over.ship_destroyed")
    );
}

/// The Viewscreen frames the ending the way a phone does: the declared
/// side and the scenario's title ride the final HUD state, and neither is
/// published while the mission still runs.
#[test]
fn compute_hud_state_frames_the_ending_with_outcome_and_scenario() {
    use crate::server_app::GameOverReason;
    let physics = ShipPhysics::default();
    let reason = GameOverReason(
        Some("world.falling_skyway.game_over.mission_complete".into()),
        Some(crate::core::balance::Outcome::Victory),
    );
    let title = "world.falling_skyway.global.title";

    let live = compute_hud_state(
        false,
        &physics,
        80.0,
        100.0,
        0.0,
        false,
        &GamePhase::InProgress,
        Some(&reason),
        None,
        None,
        Some(title),
    );
    assert!(live.game_over_outcome.is_none());
    assert!(live.scenario_title.is_none());

    let ended = compute_hud_state(
        false,
        &physics,
        80.0,
        100.0,
        0.0,
        false,
        &GamePhase::GameOver,
        Some(&reason),
        None,
        None,
        Some(title),
    );
    // `balance::Outcome::as_str`, the spelling ServerMessage::GameOver uses.
    assert_eq!(ended.game_over_outcome.as_deref(), Some("victory"));
    // An id, resolved by the host channel; Rust composes no English here.
    assert_eq!(ended.scenario_title.as_deref(), Some(title));

    // An ending that declared no side publishes none: the frame decides
    // ENDED from that absence, never from the closing prose.
    let undeclared = GameOverReason(Some("The channel went quiet.".into()), None);
    let ended = compute_hud_state(
        false,
        &physics,
        80.0,
        100.0,
        0.0,
        false,
        &GamePhase::GameOver,
        Some(&undeclared),
        None,
        None,
        Some(title),
    );
    assert!(ended.game_over_outcome.is_none());
}

/// Issue #1344: the Viewscreen shows the SAME rows a phone does, in the
/// same order, and carries no score for the same reason the phone's wire
/// row has no field to put one in.
#[test]
fn compute_hud_state_carries_the_post_mission_report_at_game_over() {
    use crate::core::report::{MissionReport, ReportRow, ReportRowState};
    use crate::server_app::GameOverReason;

    let physics = ShipPhysics::default();
    let reason = GameOverReason(
        Some("world.falling_skyway.game_over.lark_collision".into()),
        Some(crate::core::balance::Outcome::Defeat),
    );
    let mut report = MissionReport::default();
    report.set_row(ReportRow {
        id: "lyra".into(),
        heading_id: "world.falling_skyway.report.lyra.heading".into(),
        outcome_id: "world.falling_skyway.report.lyra.saved".into(),
        state: ReportRowState::Saved,
        score: 6,
    });

    // While the mission runs there is nothing to report on yet, even though
    // the row is already written.
    let live = compute_hud_state(
        false,
        &physics,
        80.0,
        100.0,
        0.0,
        false,
        &GamePhase::InProgress,
        Some(&reason),
        None,
        Some(&report),
        None,
    );
    assert!(live.game_over_report.is_empty());

    let ended = compute_hud_state(
        false,
        &physics,
        80.0,
        100.0,
        0.0,
        false,
        &GamePhase::GameOver,
        Some(&reason),
        None,
        Some(&report),
        None,
    );
    assert_eq!(ended.game_over_report.len(), 1);
    assert_eq!(ended.game_over_report[0].id, "lyra");
    assert_eq!(
        ended.game_over_report[0].heading,
        "world.falling_skyway.report.lyra.heading"
    );
    assert_eq!(ended.game_over_report[0].state, "saved");
    // Every text field is a String Id the host channel localises; Rust
    // composes no English on this surface.
    assert!(ended.game_over_report[0]
        .outcome
        .starts_with("world.falling_skyway.report."));
}

/// A scenario that authored no report ends exactly as it always did.
#[test]
fn compute_hud_state_reports_nothing_for_a_scenario_without_a_report() {
    use crate::server_app::GameOverReason;
    let physics = ShipPhysics::default();
    let reason = GameOverReason(Some("server.game_over.ship_destroyed".into()), None);
    let state = compute_hud_state(
        false,
        &physics,
        0.0,
        100.0,
        0.0,
        false,
        &GamePhase::GameOver,
        Some(&reason),
        None,
        Some(&crate::core::report::MissionReport::default()),
        None,
    );
    assert!(state.game_over_report.is_empty());
    assert_eq!(
        state.game_over_message.as_deref(),
        Some("server.game_over.ship_destroyed")
    );
}

#[test]
fn compute_hud_state_game_over_scenario_message() {
    use crate::server_app::GameOverReason;
    let physics = ShipPhysics::default();
    let reason = GameOverReason(Some("VICTORY: All enemies eliminated.".into()), None);
    let state = compute_hud_state(
        false,
        &physics,
        50.0,
        100.0,
        0.0,
        false,
        &GamePhase::GameOver,
        Some(&reason),
        None,
        None,
        None,
    );
    assert_eq!(
        state.game_over_message.as_deref(),
        Some("VICTORY: All enemies eliminated.")
    );
}

// ── computer_message passthrough (issue #1342) ────────────────────

#[test]
fn compute_hud_state_carries_the_active_computer_message() {
    let physics = ShipPhysics::default();
    let msg = ComputerMessageWire {
        id: "hail_debris".into(),
        text: "world.probe.computer_message.text".into(),
        severity: "advisory".into(),
        station: Some("tactical".into()),
    };
    let state = compute_hud_state(
        false,
        &physics,
        100.0,
        100.0,
        0.0,
        false,
        &GamePhase::InProgress,
        None,
        Some(msg.clone()),
        None,
        None,
    );
    assert_eq!(state.computer_message, Some(msg));
}

#[test]
fn to_computer_message_wire_reduces_the_authoritative_state() {
    use crate::core::computer_message::{ComputerMessageSeverity, ComputerMessageState};
    use crate::core::messages::StationId;
    let state = ComputerMessageState {
        id: "charge_ready".into(),
        text: "world.probe.computer_message.charge".into(),
        severity: ComputerMessageSeverity::Critical,
        station: Some(StationId("tactical".into())),
        shown_tick: 0,
        expires_tick: 480,
    };
    let wire = to_computer_message_wire(&state);
    assert_eq!(wire.id, "charge_ready");
    assert_eq!(wire.text, "world.probe.computer_message.charge");
    assert_eq!(wire.severity, "critical");
    assert_eq!(wire.station.as_deref(), Some("tactical"));
}

#[test]
fn game_over_hud_push_never_carries_a_computer_message() {
    // AC2: mission end clears the banner. The final push forces `None`
    // regardless of what the resource holds, rather than racing
    // `clear_active_computer_message`'s own `OnEnter` system.
    let physics = ShipPhysics::default();
    let state = compute_hud_state(
        false,
        &physics,
        0.0,
        100.0,
        0.0,
        false,
        &GamePhase::GameOver,
        None,
        None,
        None,
        None,
    );
    assert!(state.computer_message.is_none());
}

// ── yaw_to_compass_bearing ───────────────────────────────────────

#[test]
fn bearing_zero_yaw_is_zero() {
    assert_eq!(yaw_to_compass_bearing(0.0), 0);
}

#[test]
fn bearing_quarter_turn_is_ninety() {
    // +π/2 = right turn (clockwise) → ship faces East → 090°
    assert_eq!(yaw_to_compass_bearing(std::f32::consts::FRAC_PI_2), 90);
}

#[test]
fn bearing_half_turn_is_one_eighty() {
    assert_eq!(yaw_to_compass_bearing(std::f32::consts::PI), 180);
}

#[test]
fn bearing_three_quarter_turn_is_two_seventy() {
    // 3*π/2 = three-quarter clockwise turn → ship faces West → 270°
    assert_eq!(
        yaw_to_compass_bearing(3.0 * std::f32::consts::FRAC_PI_2),
        270
    );
}

#[test]
fn bearing_full_turn_wraps_to_zero() {
    assert_eq!(yaw_to_compass_bearing(std::f32::consts::TAU), 0);
}

#[test]
fn bearing_negative_yaw_wraps_positive() {
    // -π/2 = left turn (counter-clockwise) → ship faces West → 270°
    assert_eq!(yaw_to_compass_bearing(-std::f32::consts::FRAC_PI_2), 270);
}

#[test]
fn bearing_multi_turn_yaw_wraps() {
    // 2.5 turns clockwise: 2τ + π/2 → same as π/2 → 090°
    let yaw = 2.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
    assert_eq!(yaw_to_compass_bearing(yaw), 90);
}

#[test]
fn bearing_rounds_359_5_to_zero_not_360() {
    // -0.5° (tiny left turn) → 359.5°, rounds to 360 then wraps to 0.
    let yaw = (-0.5_f32).to_radians();
    assert_eq!(yaw_to_compass_bearing(yaw), 0);
}
#[test]
fn selected_sensor_report_hud_follows_view_selection_and_conceal() {
    use crate::gm_information::reports::{ReportPolicy, ReportSample, ReportState};
    use bevy::ecs::system::RunSystemOnce;
    let mut app = App::new();
    app.insert_resource(State::new(GamePhase::InProgress));
    app.insert_resource(crate::sim_tick::SimTick(12));
    let mut content = crate::world::server::WorldContentRuntime::default();
    content
        .contact_information
        .reports
        .entry("observer".into())
        .or_default()
        .insert(
            "target".into(),
            ReportState {
                policy: ReportPolicy {
                    delay_ticks: 2,
                    position_step_mm: 0,
                    hide_identity: true,
                },
                next_tick: Some(14),
                pending: None,
                presented: Some(ReportSample {
                    observed_tick: 10,
                    name: "console.sensors.basic_contact".into(),
                    position_mm: [0, 0, 0],
                }),
            },
        );
    app.insert_resource(content);
    let mut view = crate::ship::state::ShipViewMode::default();
    view.view_mode = crate::core::messages::ViewMode::SensorsRadar;
    let observer = app
        .world_mut()
        .spawn((
            crate::server_app::LocalShip,
            crate::entities::spawner::EntityUuid("observer".into()),
            view,
            crate::ship::sensors::SensorRadarSelection(Some("target".into())),
        ))
        .id();
    app.world_mut()
        .run_system_once(spawn_hud_state_entity)
        .unwrap();
    let read = |app: &mut App| {
        app.world_mut()
            .run_system_once(recompute_hud_state)
            .unwrap();
        app.world_mut()
            .query::<&ViewscreenHud>()
            .single(app.world())
            .unwrap()
            .0
            .sensor_report
            .clone()
    };
    let report = read(&mut app).unwrap();
    assert_eq!(report.age_ticks, 2);
    assert_eq!(report.observed_tick, 10);
    app.world_mut()
        .entity_mut(observer)
        .get_mut::<crate::ship::state::ShipViewMode>()
        .unwrap()
        .view_mode = crate::core::messages::ViewMode::Camera(Default::default());
    assert!(read(&mut app).is_none());
    app.world_mut()
        .entity_mut(observer)
        .get_mut::<crate::ship::state::ShipViewMode>()
        .unwrap()
        .view_mode = crate::core::messages::ViewMode::ScienceRadar;
    assert!(read(&mut app).is_some());
    crate::gm_contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "target",
        crate::gm_contact::ContactMode::Conceal,
    );
    assert!(read(&mut app).is_none());
}
