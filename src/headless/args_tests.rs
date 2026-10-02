use super::*;
use crate::logging::{LevelFilter, LogCat};

fn parse(args: &[&str]) -> HeadlessArgs {
    match parse_args(args.iter().map(|s| s.to_string())).unwrap() {
        ParseOutcome::Run(a) => *a,
        ParseOutcome::Help => panic!("expected Run, got Help"),
    }
}

fn err(args: &[&str]) -> String {
    parse_args(args.iter().map(|s| s.to_string())).unwrap_err()
}

#[test]
fn defaults_are_sixty_hz_for_sixty_seconds() {
    let a = parse(&[]);
    assert_eq!(a.dt, 1.0 / 60.0);
    // 3600 stepping ticks plus the zero-delta baseline tick.
    assert_eq!(a.max_ticks, 3601);
    assert!((a.sim_seconds() - 60.0).abs() < 1e-9);
    assert!(a.world_path.ends_with("default.toml"));
}

/// `sim_seconds()` must invert `ticks_for_sim_seconds` — this is the
/// contract the run loop and the report both lean on.
#[test]
fn tick_count_and_sim_seconds_round_trip() {
    for (secs, hz) in [(60.0, 60.0), (10.0, 30.0), (1.0, 144.0), (0.5, 20.0)] {
        let a = parse(&["--sim-seconds", &secs.to_string(), "--hz", &hz.to_string()]);
        assert!(
            (a.sim_seconds() - secs).abs() < 1e-9,
            "{secs}s at {hz}Hz round-tripped to {}",
            a.sim_seconds()
        );
    }
}

#[test]
fn help_short_circuits_before_other_arguments() {
    assert!(matches!(
        parse_args(["--help".to_string(), "--nonsense".to_string()]).unwrap(),
        ParseOutcome::Help
    ));
}

#[test]
fn hz_sets_the_timestep() {
    let a = parse(&["--hz", "120"]);
    assert!((a.dt - 1.0 / 120.0).abs() < f64::EPSILON);
    assert!((a.hz() - 120.0).abs() < 1e-9);
}

#[test]
fn dt_is_an_alternative_to_hz() {
    let a = parse(&["--dt", "0.05"]);
    assert_eq!(a.dt, 0.05);
}

#[test]
fn hz_and_dt_together_are_rejected() {
    assert!(err(&["--hz", "60", "--dt", "0.01"]).contains("give one or the other"));
}

/// The run loop only counts ticks, so `--sim-seconds` must resolve against
/// whatever `--hz` ends up being — including when `--hz` comes afterwards.
#[test]
fn sim_seconds_resolves_against_hz_in_either_order() {
    assert_eq!(parse(&["--sim-seconds", "10", "--hz", "30"]).max_ticks, 301);
    assert_eq!(parse(&["--hz", "30", "--sim-seconds", "10"]).max_ticks, 301);
}

#[test]
fn sim_seconds_rounds_up_so_the_span_is_never_short() {
    // 10s at 3Hz is 30 stepping ticks exactly; 10.1s needs 31. Both then
    // gain the baseline tick.
    assert_eq!(parse(&["--hz", "3", "--sim-seconds", "10"]).max_ticks, 31);
    assert_eq!(parse(&["--hz", "3", "--sim-seconds", "10.1"]).max_ticks, 32);
}

#[test]
fn ticks_and_sim_seconds_together_are_rejected() {
    assert!(err(&["--ticks", "10", "--sim-seconds", "10"]).contains("give one or the other"));
}

#[test]
fn log_spec_is_parsed_and_retained_verbatim() {
    let a = parse(&["--log", "info,ai=debug"]);
    assert_eq!(a.log.default_level, LevelFilter::Info);
    assert_eq!(a.log.per_cat[&LogCat::Ai], LevelFilter::Debug);
    assert_eq!(a.log_spec, "info,ai=debug");
}

/// `--log` replaces the whole config, so it must not clobber an entity
/// filter given before it.
#[test]
fn log_and_log_entity_are_order_independent() {
    for args in [
        ["--log", "ai=debug", "--log-entity", "Ironveil"],
        ["--log-entity", "Ironveil", "--log", "ai=debug"],
    ] {
        let a = parse(&args);
        assert_eq!(a.log.per_cat[&LogCat::Ai], LevelFilter::Debug);
        let f = a.log.entity_filter.as_ref().expect("entity filter dropped");
        assert_eq!(f.names, vec!["Ironveil"]);
    }
}

#[test]
fn deterministic_is_off_by_default() {
    assert!(!parse(&[]).deterministic);
    assert!(parse(&["--deterministic"]).deterministic);
}

#[test]
fn seed_is_unset_by_default_and_implies_deterministic() {
    assert_eq!(parse(&[]).seed, None);
    assert!(!parse(&[]).deterministic);

    for args in [
        ["--seed", "9001", "--hz", "30"],
        ["--hz", "30", "--seed", "9001"],
    ] {
        let a = parse(&args);
        assert_eq!(a.seed, Some(9001));
        assert!(a.deterministic, "--seed must imply --deterministic");
    }
}

#[test]
fn a_non_numeric_seed_is_rejected() {
    assert!(err(&["--seed", "lucky"]).contains("whole number"));
    assert!(err(&["--seed", "-1"]).contains("whole number"));
    assert!(err(&["--seed"]).contains("requires a value"));
}

#[test]
fn report_format_is_case_insensitive_and_validated() {
    assert_eq!(
        parse(&["--report-format", "NDJSON"]).report_format,
        ReportFormat::Ndjson
    );
    assert!(err(&["--report-format", "yaml"]).contains("json"));
}

#[test]
fn bad_log_spec_surfaces_the_parser_error() {
    assert!(err(&["--log", "warpcore=debug"]).contains("warpcore"));
}

#[test]
fn unknown_flags_and_missing_values_are_errors() {
    assert!(err(&["--warp"]).contains("unknown argument"));
    assert!(err(&["--world"]).contains("requires a value"));
}

#[test]
fn non_positive_rates_are_rejected() {
    assert!(err(&["--hz", "0"]).contains("positive"));
    assert!(err(&["--dt", "-1"]).contains("positive"));
    assert!(err(&["--hz", "fast"]).contains("expects a number"));
}

// ── --side-a / --side-b (issue #844) ────────────────────────────────────

#[test]
fn sides_default_to_empty() {
    let a = parse(&[]);
    assert!(a.side_a.is_empty());
    assert!(a.side_b.is_empty());
}

#[test]
fn side_a_is_a_comma_split_list() {
    let a = parse(&["--side-a", "cruiser,courier"]);
    assert_eq!(a.side_a, vec!["cruiser", "courier"]);
    assert!(a.side_b.is_empty());
}

#[test]
fn side_b_is_a_comma_split_list() {
    let a = parse(&["--side-b", "destroyer"]);
    assert_eq!(a.side_b, vec!["destroyer"]);
    assert!(a.side_a.is_empty());
}

/// Forgiving splitting: whitespace trimmed, empty entries dropped.
#[test]
fn side_list_trims_and_drops_empties() {
    let a = parse(&["--side-a", " cruiser , , courier ,"]);
    assert_eq!(a.side_a, vec!["cruiser", "courier"]);
}

#[test]
fn a_side_longer_than_five_is_rejected() {
    assert!(err(&["--side-a", "a,b,c,d,e,f"]).contains("maximum is 5"));
    assert!(err(&["--side-b", "a,b,c,d,e,f"]).contains("maximum is 5"));
    // Exactly five is allowed.
    assert_eq!(parse(&["--side-a", "a,b,c,d,e"]).side_a.len(), 5);
}

/// `--side-a` sets the player ship, so it collides with an explicit
/// `--ship`. `--side-b` alone does not.
#[test]
fn ship_and_side_a_together_are_rejected() {
    assert!(err(&[
        "--ship",
        "assets/entities/alliance_cruiser.toml",
        "--side-a",
        "courier"
    ])
    .contains("give one or the other"));
    // --ship with only --side-b is fine (side B is all NPCs).
    let a = parse(&[
        "--ship",
        "assets/entities/alliance_cruiser.toml",
        "--side-b",
        "destroyer",
    ]);
    assert_eq!(a.side_b, vec!["destroyer"]);
}

/// The trap this closes: `--side-a cruiser --side-b destroyer` with no
/// `--world` used to load `default.toml`, which authors none of the slots
/// those flags fill — the run was a combat-free draw that looked like a
/// balance finding. Either flag alone is enough to imply the harness.
#[test]
fn sides_without_an_explicit_world_default_to_the_duel_harness() {
    assert_eq!(
        parse(&["--side-a", "cruiser", "--side-b", "destroyer"]).world_path,
        DUEL_WORLD
    );
    assert_eq!(parse(&["--side-b", "destroyer"]).world_path, DUEL_WORLD);
    // No sides → the plain default is untouched.
    assert_eq!(parse(&[]).world_path, DEFAULT_WORLD);
}

/// An explicit `--world` still wins: a user may have authored their own
/// duel-shaped world. Order-independent, like `--seed`/`--deterministic`.
#[test]
fn an_explicit_world_wins_over_the_duel_default() {
    assert_eq!(
        parse(&[
            "--side-a",
            "cruiser",
            "--world",
            "assets/worlds/combat_test.toml"
        ])
        .world_path,
        "assets/worlds/combat_test.toml"
    );
    assert_eq!(
        parse(&[
            "--world",
            "assets/worlds/combat_test.toml",
            "--side-b",
            "destroyer"
        ])
        .world_path,
        "assets/worlds/combat_test.toml"
    );
}

// ── Console-latency measurement (issue #1169) ────────────────────────────

/// Measurement is opt-in: an ordinary run must take no wall-clock reading,
/// which is what makes "capture off costs nothing" true rather than a claim.
#[test]
fn console_latency_measurement_is_off_by_default() {
    assert!(!parse(&[]).console_latency);
}

#[test]
fn console_latency_parses() {
    assert!(parse(&["--console-latency"]).console_latency);
}

/// Asking for a perf capture asks for every metric the budget compares,
/// including `sim.console_ack` — otherwise a CI job would have to be edited
/// to keep producing it.
#[test]
fn perf_capture_implies_console_latency() {
    let a = parse(&["--perf-capture", "target/perf/capture.json"]);
    assert!(
        a.console_latency,
        "--perf-capture must imply the measurement its metric is built from"
    );
}

// ── Replay flags (issue #901) ────────────────────────────────────────────

#[test]
fn replay_flags_default_to_off() {
    let a = parse(&[]);
    assert_eq!(a.record_path, None);
    assert_eq!(a.replay_path, None);
    assert_eq!(
            a.digest_every, 0,
            "periodic hashing must be off unless asked for, so a run that did not ask for it pays nothing"
        );
}

#[test]
fn record_and_digest_every_parse() {
    let a = parse(&[
        "--record",
        "run.ron",
        "--seed",
        "7",
        "--digest-every",
        "120",
    ]);
    assert_eq!(a.record_path.as_deref(), Some("run.ron"));
    assert_eq!(a.digest_every, 120);
    assert_eq!(a.seed, Some(7));
}

/// An artifact whose seed came from the OS names a run nothing can
/// re-derive, so it must fail at argument time rather than after the run.
#[test]
fn recording_without_a_seed_is_refused() {
    assert!(err(&["--record", "run.ron"]).contains("--seed"));
}

#[test]
fn recording_and_replaying_at_once_is_refused() {
    assert!(
        err(&["--record", "a.ron", "--replay", "b.ron", "--seed", "1"])
            .contains("one or the other")
    );
}

/// A replay takes its whole setup from the artifact. A flag that would set
/// the same thing is rejected rather than accepted and quietly ignored.
#[test]
fn a_replay_refuses_the_flags_the_artifact_already_decides() {
    for extra in [
        vec!["--world", "assets/worlds/patrol.toml"],
        vec!["--ship", "assets/entities/alliance_cruiser.toml"],
        vec!["--seed", "3"],
        vec!["--side-a", "cruiser"],
        vec!["--side-b", "destroyer"],
        vec!["--ticks", "100"],
        vec!["--sim-seconds", "10"],
        vec!["--dt", "0.02"],
        vec!["--hz", "30"],
    ] {
        let mut argv = vec!["--replay", "run.ron"];
        argv.extend(extra.iter());
        let message = err(&argv);
        assert!(
            message.contains("--replay"),
            "{extra:?} should be refused alongside --replay; got {message:?}"
        );
    }
}

#[test]
fn recording_and_perf_capture_together_are_refused() {
    assert!(err(&[
        "--record",
        "run.ron",
        "--seed",
        "1",
        "--perf-capture",
        "cap.json"
    ])
    .contains("--perf-capture"));
}

#[test]
fn digest_every_rejects_a_non_number() {
    assert!(err(&["--digest-every", "often"]).contains("--digest-every"));
}
