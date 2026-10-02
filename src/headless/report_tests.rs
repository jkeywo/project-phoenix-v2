use super::*;
use crate::core::balance::VictimKind;

/// Guards against regressing to JSON key-scraping, which reported every
/// message as `"type"` because `ServerMessage` is internally tagged.
#[test]
fn variant_name_reports_the_variant_not_the_serde_tag() {
    let name = variant_name(&ServerMessage::GameStarted);
    assert_eq!(name, "GameStarted");
    assert_ne!(name, "type");
}

/// Zeroed timings are a claim of replayability, so only the one seed tier
/// that also pins the scheduler (`--seed`, which implies `--deterministic`)
/// earns them. A world-TOML seed still runs on the default thread pool, so
/// it keeps its real, honestly-varying figures.
#[test]
fn only_a_cli_seed_zeroes_the_host_clock_timings() {
    assert_eq!(reported_wall_seconds("cli", 1.25), 0.0);
    assert_eq!(reported_wall_seconds("world", 1.25), 1.25);
    assert_eq!(reported_wall_seconds("random", 1.25), 1.25);
    // No `SimRng` in the world at all — nothing to replay, so nothing to
    // hide.
    assert_eq!(reported_wall_seconds("absent", 1.25), 1.25);
}

/// Anti-vacuity for the test above: the string it keys off is the one
/// `SeedSource` actually stamps into the report.
#[test]
fn the_reproducible_seed_source_is_the_one_the_cli_tier_reports() {
    assert_eq!(SeedSource::Cli.as_str(), "cli");
    assert_eq!(reported_wall_seconds(SeedSource::Cli.as_str(), 3.0), 0.0);
    assert_eq!(reported_wall_seconds(SeedSource::World.as_str(), 3.0), 3.0);
}

#[test]
fn report_json_is_parseable_and_carries_the_headline_numbers() {
    let report = RunReport {
        ticks: 601,
        sim_seconds: 10.0,
        seed: 42,
        seed_source: "cli".into(),
        wall_seconds: 0.5,
        ticks_per_second: 1202.0,
        final_phase: "InProgress".into(),
        game_over_reason: None,
        entity_count: 4,
        ship: Some(ShipSummary {
            name: Some("Alliance Cruiser".into()),
            x: 1.5,
            z: -2.5,
            hull_current: 90.0,
            hull_max: 100.0,
            damaged_systems: [("helm".to_string(), "Damaged".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        }),
        message_counts: [("SimState".to_string(), 100u64)].into_iter().collect(),
        damage_by_ship: BTreeMap::new(),
        // Budget-exhausted with a live closing window → timeout, carrying
        // both sides' margins (AC1 + AC2).
        outcome_report: crate::core::balance::classify(
            false,
            None,
            SideMargins::new(90.0, 100.0, 200.0, 40.0, 3.0),
            SideMargins::new(0.0, 100.0, 40.0, 200.0, 1.0),
            crate::core::report::MissionReport::default(),
        ),
        ai_doctrine: String::new(),
        station_activity: StationActivityPayload::default(),
        scenario: None,
        objective_instances: String::new(),
        narrative: Default::default(),
        console_latency: Default::default(),
    };
    let json = report.to_json();
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));
    assert_eq!(parsed["ticks"], 601);
    assert_eq!(parsed["seed"], 42);
    assert_eq!(parsed["seed_source"], "cli");
    assert_eq!(parsed["speedup_vs_realtime"], 20.0);
    assert_eq!(parsed["ship"]["name"], "Alliance Cruiser");
    assert_eq!(parsed["ship"]["damaged_systems"]["helm"], "Damaged");
    assert_eq!(parsed["message_counts"]["SimState"], 100);
    assert!(parsed["game_over_reason"].is_null());
    // An unset doctrine surface (no projection ran) renders as null, keeping
    // the report valid JSON (issue #1149).
    assert!(parsed["ai_doctrine"].is_null());
    // Outcome + per-side margins are always present.
    assert_eq!(parsed["outcome"], "timeout");
    assert_eq!(parsed["sides"]["player"]["remaining_hull_fraction"], 0.9);
    assert_eq!(parsed["sides"]["player"]["damage_dealt"], 200.0);
    assert_eq!(parsed["sides"]["enemy"]["closing_damage_rate"], 1.0);
}

#[test]
fn report_json_is_parseable_with_no_ship() {
    let report = RunReport {
        ticks: 1,
        sim_seconds: 0.0,
        seed: 7,
        seed_source: "world".into(),
        wall_seconds: 0.0,
        ticks_per_second: 0.0,
        final_phase: "GameOver".into(),
        game_over_reason: Some("hull breach".into()),
        entity_count: 0,
        ship: None,
        message_counts: BTreeMap::new(),
        damage_by_ship: BTreeMap::new(),
        // Reached GameOver via the player-death latch → defeat.
        outcome_report: crate::core::balance::classify(
            true,
            Some(crate::core::balance::Outcome::Defeat),
            SideMargins::default(),
            SideMargins::default(),
            crate::core::report::MissionReport::default(),
        ),
        ai_doctrine: String::new(),
        station_activity: StationActivityPayload::default(),
        scenario: None,
        objective_instances: String::new(),
        narrative: Default::default(),
        console_latency: Default::default(),
    };
    let parsed: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert!(parsed["ship"].is_null());
    assert_eq!(parsed["game_over_reason"], "hull breach");
    assert_eq!(parsed["outcome"], "defeat");
    assert!(report.ended_in_game_over());
}

/// The per-ship section is built from the balance-event log, so it has to
/// survive into the report for ships the world no longer contains.
#[test]
fn report_json_carries_per_ship_damage_ledgers() {
    let events = [
        BalanceEvent::DamageApplied {
            attacker: Some("player".into()),
            victim: "raider".into(),
            victim_kind: VictimKind::Ship,
            weapon: "fore_phaser".into(),
            amount: 20.0,
            shield_absorbed: 5.0,
            hull_damage: 15.0,
            system_hit: None,
        },
        BalanceEvent::DamageApplied {
            attacker: Some("raider".into()),
            victim: "player".into(),
            victim_kind: VictimKind::Ship,
            weapon: "torpedo".into(),
            amount: 9.0,
            shield_absorbed: 9.0,
            hull_damage: 0.0,
            system_hit: None,
        },
    ];
    let names: BTreeMap<String, String> = [("player".to_string(), "Ironveil".to_string())]
        .into_iter()
        .collect();
    let report = RunReport {
        ticks: 10,
        sim_seconds: 1.0,
        seed: 0,
        seed_source: "random".into(),
        wall_seconds: 1.0,
        ticks_per_second: 10.0,
        final_phase: "InProgress".into(),
        game_over_reason: None,
        entity_count: 2,
        ship: None,
        message_counts: BTreeMap::new(),
        damage_by_ship: crate::core::balance::aggregate_damage(events.iter(), &names),
        outcome_report: crate::core::balance::classify(
            false,
            None,
            SideMargins::default(),
            SideMargins::default(),
            crate::core::report::MissionReport::default(),
        ),
        ai_doctrine: String::new(),
        station_activity: StationActivityPayload::default(),
        scenario: None,
        objective_instances: String::new(),
        narrative: Default::default(),
        console_latency: Default::default(),
    };
    let json = report.to_json();
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));

    assert_eq!(parsed["damage_by_ship"]["player"]["name_id"], "Ironveil");
    assert_eq!(parsed["damage_by_ship"]["player"]["damage_dealt"], 20.0);
    assert_eq!(parsed["damage_by_ship"]["player"]["damage_taken"], 9.0);
    assert_eq!(parsed["damage_by_ship"]["raider"]["damage_dealt"], 9.0);
    assert_eq!(parsed["damage_by_ship"]["raider"]["damage_taken"], 20.0);
    assert!(parsed["damage_by_ship"]["raider"]["name_id"].is_null());
}

/// The always-on station-activity series serialises into the report as a
/// per-station, per-bucket, per-control-source object (issue #1147). This is
/// the schema the balance-runs merge folds and the report-integration tests
/// assert on — a run's report carries it whether or not any debug flag was
/// set, because `build_report` reads the tracker's projection directly.
#[test]
fn report_json_carries_the_station_activity_series() {
    use crate::debug::payload::{StationActivityBucket, StationActivityEntry};

    let payload = StationActivityPayload {
        schema_version: crate::debug::payload::DEBUG_SCHEMA_VERSION,
        bucket_ticks: 900,
        bucket_secs: 15.0,
        buckets: vec![StationActivityBucket {
            start_tick: 0,
            stations: vec![
                StationActivityEntry {
                    station: "helm".into(),
                    human: 12,
                    ai: 3,
                    offline: 0,
                },
                StationActivityEntry {
                    station: "weapons".into(),
                    human: 0,
                    ai: 21,
                    offline: 0,
                },
            ],
        }],
    };
    let report = RunReport {
        ticks: 900,
        sim_seconds: 15.0,
        seed: 1,
        seed_source: "cli".into(),
        wall_seconds: 0.0,
        ticks_per_second: 0.0,
        final_phase: "InProgress".into(),
        game_over_reason: None,
        entity_count: 2,
        ship: None,
        message_counts: BTreeMap::new(),
        damage_by_ship: BTreeMap::new(),
        outcome_report: crate::core::balance::classify(
            false,
            None,
            SideMargins::default(),
            SideMargins::default(),
            crate::core::report::MissionReport::default(),
        ),
        ai_doctrine: String::new(),
        station_activity: payload,
        scenario: None,
        objective_instances: String::new(),
        narrative: Default::default(),
        console_latency: Default::default(),
    };
    let json = report.to_json();
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));

    let sa = &parsed["station_activity"];
    assert_eq!(sa["schema_version"], 1);
    assert_eq!(sa["bucket_ticks"], 900);
    assert_eq!(sa["bucket_secs"], 15.0);
    let stations = &sa["buckets"][0]["stations"];
    // Sorted by station id: helm before weapons, split by control source.
    assert_eq!(stations[0]["station"], "helm");
    assert_eq!(stations[0]["human"], 12);
    assert_eq!(stations[0]["ai"], 3);
    assert_eq!(stations[1]["station"], "weapons");
    assert_eq!(stations[1]["ai"], 21);
    assert_eq!(stations[1]["human"], 0);
}

/// The authored mission timeline (issue #1338) reaches the report JSON with
/// its sequence, fixed tick, derived time, semantic ids and String Ids — and
/// the per-kind census beside it, so a reader gets the shape of the story
/// without re-walking the array.
#[test]
fn report_json_carries_the_narrative_timeline() {
    use crate::core::narrative::{
        fold_narrative, NarrativeEvent, NarrativeKind, NarrativeValue, StampedNarrativeEvent,
    };

    let events = vec![
        StampedNarrativeEvent {
            seq: 0,
            tick: 60,
            sim_t: 1.0,
            event: NarrativeEvent::new(NarrativeKind::ObjectivePosted, "hold_the_channel")
                .text("text", "world.probe_narrative.objective.hold_the_channel")
                .detail("mandatory", NarrativeValue::Flag(true)),
        },
        StampedNarrativeEvent {
            seq: 1,
            tick: 240,
            sim_t: 4.0,
            event: NarrativeEvent::new(NarrativeKind::ObjectiveCompleted, "hold_the_channel"),
        },
        StampedNarrativeEvent {
            seq: 2,
            tick: 360,
            sim_t: 6.0,
            event: NarrativeEvent::new(NarrativeKind::BeatFired, "probe_deadline_beat"),
        },
    ];

    let report = RunReport {
        ticks: 360,
        sim_seconds: 6.0,
        seed: 1338,
        seed_source: SeedSource::Cli.as_str().to_string(),
        wall_seconds: 0.0,
        ticks_per_second: 0.0,
        final_phase: "InProgress".into(),
        game_over_reason: None,
        entity_count: 2,
        ship: None,
        message_counts: BTreeMap::new(),
        damage_by_ship: BTreeMap::new(),
        narrative: fold_narrative(&events),
        outcome_report: crate::core::balance::classify(
            false,
            None,
            SideMargins::default(),
            SideMargins::default(),
            crate::core::report::MissionReport::default(),
        ),
        ai_doctrine: String::new(),
        station_activity: StationActivityPayload::default(),
        scenario: None,
        objective_instances: String::new(),
        console_latency: Default::default(),
    };
    let json = report.to_json();
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("report is not valid JSON: {e}\n{json}"));

    let narrative = &parsed["narrative"];
    assert_eq!(narrative["count"], 3);
    assert_eq!(narrative["counts_by_kind"]["objective_posted"], 1);
    assert_eq!(narrative["counts_by_kind"]["objective_completed"], 1);
    assert_eq!(narrative["counts_by_kind"]["beat_fired"], 1);

    let first = &narrative["events"][0];
    assert_eq!(first["seq"], 0);
    assert_eq!(first["tick"], 60);
    assert_eq!(first["sim_t"], 1.0);
    assert_eq!(first["kind"], "objective_posted");
    assert_eq!(first["id"], "hold_the_channel");
    // The String Id, never the rendered English.
    assert_eq!(
        first["detail"]["text"],
        "world.probe_narrative.objective.hold_the_channel"
    );
    assert_eq!(first["detail"]["mandatory"], true);
    assert!(first["source"].is_null());
    assert!(first["target"].is_null());

    assert_eq!(narrative["events"][2]["kind"], "beat_fired");
    assert_eq!(narrative["events"][2]["id"], "probe_deadline_beat");
}

/// A run that authored no story still reports the field, with an explicit
/// zero — so a consumer never has to distinguish "absent" from "nothing
/// happened", the same contract `station_activity` carries.
#[test]
fn report_json_carries_an_empty_narrative_timeline() {
    let report = RunReport {
        ticks: 1,
        sim_seconds: 0.0,
        seed: 0,
        seed_source: "absent".into(),
        wall_seconds: 0.0,
        ticks_per_second: 0.0,
        final_phase: "InProgress".into(),
        game_over_reason: None,
        entity_count: 0,
        ship: None,
        message_counts: BTreeMap::new(),
        damage_by_ship: BTreeMap::new(),
        narrative: Default::default(),
        outcome_report: crate::core::balance::classify(
            false,
            None,
            SideMargins::default(),
            SideMargins::default(),
            crate::core::report::MissionReport::default(),
        ),
        ai_doctrine: String::new(),
        station_activity: StationActivityPayload::default(),
        scenario: None,
        objective_instances: String::new(),
        console_latency: Default::default(),
    };
    let parsed: serde_json::Value = serde_json::from_str(&report.to_json())
        .unwrap_or_else(|e| panic!("report is not valid JSON: {e}"));
    assert_eq!(parsed["narrative"]["count"], 0);
    assert!(parsed["narrative"]["events"].as_array().unwrap().is_empty());
}

/// The console-latency surface (issue #1169) reaches the report JSON with
/// its per-action distributions, and a run that measured nothing says so
/// with an empty list rather than being absent.
#[test]
fn report_json_carries_the_console_latency_distributions() {
    use crate::debug::ConsoleLatencyTracker;

    let mut tracker = ConsoleLatencyTracker::default();
    for ms in [4.0, 8.0, 12.0, 40.0] {
        tracker.record_host("FirePhaser", ms);
    }

    let mut report = RunReport {
        ticks: 10,
        sim_seconds: 1.0,
        seed: 7,
        seed_source: SeedSource::Cli.as_str().to_string(),
        wall_seconds: 0.0,
        ticks_per_second: 0.0,
        final_phase: "InProgress".into(),
        game_over_reason: None,
        entity_count: 1,
        ship: None,
        message_counts: BTreeMap::new(),
        damage_by_ship: BTreeMap::new(),
        outcome_report: crate::core::balance::classify(
            false,
            None,
            SideMargins::default(),
            SideMargins::default(),
            crate::core::report::MissionReport::default(),
        ),
        ai_doctrine: String::new(),
        station_activity: StationActivityPayload::default(),
        scenario: None,
        objective_instances: String::new(),
        narrative: Default::default(),
        console_latency: Default::default(),
    };

    // An unmeasured run: present, versioned, and empty.
    let parsed: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(parsed["console_latency"]["schema_version"], 1);
    assert_eq!(
        parsed["console_latency"]["actions"]
            .as_array()
            .expect("actions is a list")
            .len(),
        0,
        "a run with the flag off reports no measurements, not a missing field"
    );

    report.console_latency = tracker.report();
    let parsed: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    let entry = &parsed["console_latency"]["actions"][0];
    assert_eq!(entry["surface"], "SimHost");
    assert_eq!(entry["action"], "FirePhaser");
    // Nearest-rank over [4, 8, 12, 40]: p50 = 2nd, p75 = 3rd, max = 4th.
    assert_eq!(entry["admit_to_broadcast"]["p50_ms"], 8.0);
    assert_eq!(entry["admit_to_broadcast"]["p75_ms"], 12.0);
    assert_eq!(entry["admit_to_broadcast"]["max_ms"], 40.0);
    // The host cannot observe a client's input event, so it must not claim to.
    assert!(
        entry.get("input_to_send").is_none(),
        "a host-measured entry must not carry client segments"
    );
}
