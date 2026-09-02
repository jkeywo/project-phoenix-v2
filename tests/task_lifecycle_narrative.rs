//! The continuous-task lifecycle, end to end (issue #1341, PRD #1337).
//!
//! `src/core/task_lifecycle.rs` unit-tests the vocabulary, the key and the
//! registry; `src/narrative.rs` unit-tests the emitter's four decisions against
//! a bare app. Four claims need a whole seeded run of a real world, driven
//! through the real admitted-command path:
//!
//! 1. **One start and exactly one terminal per activation**, carrying the
//!    operator, the subject, a deterministic task key and a terminal reason —
//!    for BOTH tracer workflows, an instantaneous one (the scan) and a genuinely
//!    continuous one (the tractor hold). That the same two events come out of
//!    two workflows with nothing in common is the whole point: #1345, #1346,
//!    #1348 and #1350 consume this interface rather than inventing their own.
//!
//! 2. **The four terminal classes are distinct**, and the reasons under them
//!    separate a completion from a cancellation, a failure, a lost subject and
//!    the mission ending underneath a task still running.
//!
//! 3. **Repeated, simultaneous, cancelled and restarted tasks stay separately
//!    and deterministically identifiable.** The same beam holds the same hull
//!    four times over one run and the four activations carry four keys; a scan
//!    runs inside a live hold and neither ends the other.
//!
//! 4. **Recording changes nothing.** The same driven run under `--format json`
//!    and `--format ndjson` folds to a byte-identical authoritative-state digest
//!    and an identical report, and two runs of the same seed record the same
//!    timeline.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

use project_phoenix::console::weapons::beam::TacticalRadarSelection;
use project_phoenix::core::messages::{ClientMessage, SystemControlPayload, SystemId};
use project_phoenix::entities::spawner::{EntityName, EntityUuid};
use project_phoenix::headless::args::{ticks_for_sim_seconds, ReportFormat};
use project_phoenix::headless::report::RunReport;
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};
use project_phoenix::lobby::InboundMessage;
use project_phoenix::server_app::LocalShip;
use project_phoenix::ship::state::ShipPhysics;
use project_phoenix::sim_digest::world_digest;

const WORLD: &str = "assets/worlds/probe_task_lifecycle.toml";
/// The world's `player-ship` placeholder is swapped for the lobby-SELECTED
/// hull, so the selection has to be the tender or neither of its systems spawns.
const SHIP: &str = "assets/entities/lifecycle_tender.toml";
const DERELICT: &str = "world.probe_task_lifecycle.entity.derelict.name";
const DEPOT: &str = "world.probe_task_lifecycle.entity.depot.name";
const SEED: u64 = 1341;

/// An `ai:` token: admission authorises it iff the target system is
/// AI-controlled, which both of the tender's systems are while nobody is at its
/// console — the same seam a human tenure token uses from the other side
/// (AGENTS.md rule 6). Nothing downstream can tell which sent the command.
const TOKEN: &str = "ai:lifecycle-probe";

/// A uuid no entity in this world carries, so the suite is asked for a contact
/// that does not exist — the scan's `no_such_target` failure.
const PHANTOM: &str = "00000000-0000-4000-8000-0000000f0000";

fn probe_args(format: ReportFormat) -> HeadlessArgs {
    let dt = 1.0 / 30.0;
    HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: SHIP.into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        seed: Some(SEED),
        deterministic: true,
        report_format: format,
        ..Default::default()
    }
}

// ── Driving the two workflows through their real command paths ───────────────

fn operator(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the probe world spawns a local tender")
}

fn operator_uuid(app: &mut App) -> String {
    let entity = operator(app);
    app.world()
        .get::<EntityUuid>(entity)
        .expect("the tender carries a uuid")
        .0
        .clone()
}

/// The minted uuid of the entity carrying `name`.
fn uuid_named(app: &mut App, name: &str) -> String {
    let mut q = app.world_mut().query::<(&EntityName, &EntityUuid)>();
    let found = q
        .iter(app.world())
        .find(|(n, _)| n.0 == name)
        .map(|(_, uuid)| uuid.0.clone());
    found.unwrap_or_else(|| panic!("the probe world spawns '{name}'"))
}

/// Remove `name` from the world, the way `ctx.effects.destroy_entity` does — a
/// subject that leaves under a live hold.
fn despawn_named(app: &mut App, name: &str) {
    let mut q = app.world_mut().query::<(Entity, &EntityName)>();
    let found = q
        .iter(app.world())
        .find(|(_, n)| n.0 == name)
        .map(|(entity, _)| entity);
    let entity = found.unwrap_or_else(|| panic!("the probe world spawns '{name}'"));
    app.world_mut().entity_mut(entity).despawn();
}

/// Place the operator by writing its `ShipPhysics`, which `sync_ship_position`
/// projects into the transform the coupling reads.
fn place_operator(app: &mut App, position: Vec3) {
    let entity = operator(app);
    let mut physics = app
        .world_mut()
        .get_mut::<ShipPhysics>(entity)
        .expect("the tender is a ship");
    physics.x = position.x;
    physics.y = position.y;
    physics.z = position.z;
}

/// Set the ship's one Tactical lock, the way a `SetTarget` would.
fn set_lock(app: &mut App, uuid: &str) {
    let entity = operator(app);
    app.world_mut()
        .entity_mut(entity)
        .insert(TacticalRadarSelection(Some(uuid.to_string())));
}

/// Send one system command through the real admission path and give it the
/// ticks to arrive: drained in `PreUpdate`, admitted before `SimSet::Input`,
/// consumed in the same tick's `SimSet::Input`/`SimSet::Modifiers`.
fn send(app: &mut App, target: SystemId, payload: SystemControlPayload) {
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: TOKEN.into(),
            msg: ClientMessage::ControlSystem { target, payload },
        });
    run(app, 2);
}

fn tractor(app: &mut App, payload: SystemControlPayload) {
    send(
        app,
        SystemId(project_phoenix::ship::system_registry::TRACTOR_SYSTEM_ID.into()),
        payload,
    );
}

fn ask_for_scan(app: &mut App, uuid: &str) {
    send(
        app,
        project_phoenix::ship::system_registry::sensors_system_id(),
        SystemControlPayload::ScanTarget {
            uuid: uuid.to_string(),
        },
    );
}

/// One driven run: every case issue #1341 asks about, in one seeded world.
///
/// The sequence is deliberately fixed rather than time-based, so two runs of the
/// same format produce the same timeline event for event.
fn driven_run(format: ReportFormat) -> (RunReport, u64) {
    let args = probe_args(format);
    let mut app = build_headless_app(&args).expect("probe_task_lifecycle world should build");
    // Settle: the game-start spawn puts the tender and both subjects in the sky.
    run(&mut app, 60);
    let derelict = uuid_named(&mut app, DERELICT);
    let depot = uuid_named(&mut app, DEPOT);

    // 1. A reading that comes back — the COMPLETED case.
    ask_for_scan(&mut app, &depot);
    // 2. A contact nothing answers to — a FAILED scan, and the second activation
    //    of the same slot, so its key must differ from the first's.
    ask_for_scan(&mut app, PHANTOM);

    // 3. A hold the crew let go of — the CANCELLED case — with a scan running
    //    inside it, which is the simultaneity claim.
    set_lock(&mut app, &derelict);
    tractor(&mut app, SystemControlPayload::EngageTractor);
    ask_for_scan(&mut app, &depot);
    tractor(&mut app, SystemControlPayload::ReleaseTractor);

    // 4. The same beam on the same hull again — a repeat, which must be its own
    //    activation — dropped by flying out of the authored reach: FAILED.
    tractor(&mut app, SystemControlPayload::EngageTractor);
    place_operator(&mut app, Vec3::new(5_000.0, 0.0, 0.0));
    run(&mut app, 4);

    // 5. A third hold, ended by the subject leaving the world: INTERRUPTED, and
    //    reported as a destroyed target rather than as the "out of range" the
    //    beam itself can see.
    place_operator(&mut app, Vec3::ZERO);
    run(&mut app, 2);
    tractor(&mut app, SystemControlPayload::EngageTractor);
    despawn_named(&mut app, DERELICT);
    run(&mut app, 4);

    // 6. A fourth hold, still holding when the run stops: INTERRUPTED by the
    //    mission ending, written at the report boundary because no further tick
    //    exists to notice it.
    set_lock(&mut app, &depot);
    tractor(&mut app, SystemControlPayload::EngageTractor);
    run(&mut app, 4);

    let digest = world_digest(app.world());
    // `wall_seconds` of 0.0 so the derived timing fields are constants rather
    // than measurements, and two reports can be compared byte for byte.
    (build_report(&mut app, &args, 0.0), digest)
}

// ── Reading the timeline back ────────────────────────────────────────────────

/// Every lifecycle event, as `(seq, kind, key, reason)` — `reason` empty for a
/// start beat.
fn lifecycle_rows(report: &RunReport) -> Vec<(u64, &str, String, String)> {
    report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.kind.is_task_lifecycle())
        .map(|e| {
            (
                e.seq,
                e.event.kind.as_str(),
                e.event.id.clone(),
                detail_text(&e.event, "reason"),
            )
        })
        .collect()
}

/// One `NarrativeValue::Text` detail field, or the empty string when absent.
fn detail_text(event: &project_phoenix::core::narrative::NarrativeEvent, key: &str) -> String {
    match event.detail.get(key) {
        Some(project_phoenix::core::narrative::NarrativeValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

fn count(report: &RunReport, kind: &str) -> u64 {
    report
        .narrative
        .counts_by_kind
        .get(kind)
        .copied()
        .unwrap_or(0)
}

/// Every terminal reason recorded, in sequence order.
fn reasons(report: &RunReport) -> Vec<String> {
    lifecycle_rows(report)
        .into_iter()
        .filter(|(_, kind, _, _)| *kind != "task_started")
        .map(|(_, _, _, reason)| reason)
        .collect()
}

/// **AC1.** Every activation has one start and exactly one terminal event,
/// sharing one key, and each beat names the operator, the system, the station
/// and the subject.
#[test]
fn every_activation_has_one_start_and_exactly_one_terminal() {
    let (report, _) = driven_run(ReportFormat::Json);
    let rows = lifecycle_rows(&report);
    assert!(
        !rows.is_empty(),
        "the driven run exercised both workflows and the timeline carries none \
         of it: {:?}",
        report.narrative.counts_by_kind
    );

    let mut starts: BTreeMap<String, u64> = BTreeMap::new();
    let mut terminals: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();
    for (seq, kind, key, reason) in &rows {
        if *kind == "task_started" {
            assert!(
                starts.insert(key.clone(), *seq).is_none(),
                "a task key must be started exactly once: {key}"
            );
        } else {
            terminals
                .entry(key.clone())
                .or_default()
                .push((*seq, reason.clone()));
        }
    }
    for (key, ends) in &terminals {
        assert_eq!(
            ends.len(),
            1,
            "the whole contract: {key} ended {} times — {ends:?}",
            ends.len()
        );
        let start = starts
            .get(key)
            .unwrap_or_else(|| panic!("{key} ended without ever starting"));
        assert!(
            ends[0].0 > *start,
            "a task cannot end before it begins: {key}"
        );
        assert!(
            !ends[0].1.is_empty(),
            "every terminal event names its reason: {key}"
        );
    }
    assert_eq!(
        starts.len(),
        terminals.len(),
        "every started task must have been closed — started {:?}, closed {:?}",
        starts.keys().collect::<Vec<_>>(),
        terminals.keys().collect::<Vec<_>>()
    );

    // …and every beat carries the operator, the owning system, the station a
    // human WOULD be sitting at, and the subject.
    let operator = {
        let mut app = build_headless_app(&probe_args(ReportFormat::Json)).expect("builds");
        run(&mut app, 60);
        operator_uuid(&mut app)
    };
    for event in report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.kind.is_task_lifecycle())
    {
        assert_eq!(
            event.event.source.entity.as_deref(),
            Some(operator.as_str()),
            "the operating hull is the source of its own task: {:?}",
            event.event
        );
        let system = event
            .event
            .source
            .system
            .as_deref()
            .expect("a task names the system doing it");
        assert!(
            system == "tractor" || system == "sensors",
            "unexpected operating system {system:?}"
        );
        assert_eq!(
            event.event.source.station.as_deref(),
            Some("engineering"),
            "the tender authors both systems onto engineering: {:?}",
            event.event.source
        );
        assert!(
            event.event.target.is_some(),
            "both tracer workflows act on a subject: {:?}",
            event.event
        );
        // The key is the deterministic one, not an opaque counter: the operator,
        // the system, the verb, the subject and the activation ordinal.
        let expected = format!(
            "{}/{}/{}/{}#",
            operator,
            system,
            if system == "tractor" {
                "tractor_hold"
            } else {
                "scan"
            },
            event.event.target.as_deref().unwrap_or_default(),
        );
        assert!(
            event.event.id.starts_with(&expected),
            "the task key must be built from the operator, system, verb and \
             subject: {:?} does not start with {expected:?}",
            event.event.id
        );
    }
}

/// **AC2.** All four terminal classes appear, and the reasons under them tell a
/// completion, a cancellation, two different failures, a lost subject and the
/// mission ending apart.
#[test]
fn the_terminal_classes_and_their_reasons_are_distinct() {
    let (report, _) = driven_run(ReportFormat::Json);

    for kind in [
        "task_started",
        "task_completed",
        "task_cancelled",
        "task_failed",
        "task_interrupted",
    ] {
        assert!(
            count(&report, kind) > 0,
            "the driven run exercises every terminal class, but {kind} is \
             missing: {:?}",
            report.narrative.counts_by_kind
        );
    }

    let recorded: BTreeSet<String> = reasons(&report).into_iter().collect();
    for reason in [
        // the scan that came back
        "completed",
        // the crew letting go of the beam
        "released",
        // the suite asked for a contact that does not exist
        "no_such_target",
        // the tender flew out of the beam's authored reach
        "out_of_range",
        // the held hull left the world under the beam
        "target_destroyed",
        // the run stopped with a hold still holding
        "mission_ended",
    ] {
        assert!(
            recorded.contains(reason),
            "the {reason:?} terminal reason never appeared: {recorded:?}"
        );
    }

    // The reason and its class agree, and the localizable half of every terminal
    // event is a String Id rather than prose.
    for event in report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.kind.is_task_terminal())
    {
        let reason = detail_text(&event.event, "reason");
        let outcome = detail_text(&event.event, "outcome");
        assert_eq!(
            format!("task_{outcome}"),
            event.event.kind.as_str(),
            "the recorded class must match the kind: {:?}",
            event.event
        );
        assert_eq!(
            detail_text(&event.event, "reason_text"),
            format!("task.ended.{reason}"),
            "the reason's player-facing half is a strings.csv id: {:?}",
            event.event
        );
    }
    let json = report.to_json();
    assert!(
        !json.contains("Target destroyed"),
        "localized English must never enter the timeline:\n{json}"
    );
}

/// **AC3.** Repeated, simultaneous, cancelled and restarted activations stay
/// separately identifiable.
#[test]
fn repeated_and_simultaneous_activations_keep_separate_keys() {
    let (report, _) = driven_run(ReportFormat::Json);
    let rows = lifecycle_rows(&report);

    // The same beam held a hull four times over this run. Four starts, four
    // distinct keys, ordinals counting up — a repeat is a NEW task, never a
    // second event on the old one.
    let hold_keys: Vec<String> = rows
        .iter()
        .filter(|(_, kind, key, _)| *kind == "task_started" && key.contains("/tractor_hold/"))
        .map(|(_, _, key, _)| key.clone())
        .collect();
    assert_eq!(
        hold_keys.len(),
        4,
        "the run engages the beam four times: {hold_keys:?}"
    );
    assert_eq!(
        hold_keys.iter().collect::<BTreeSet<_>>().len(),
        4,
        "four holds, four keys: {hold_keys:?}"
    );
    let ordinals: Vec<&str> = hold_keys
        .iter()
        .map(|k| k.rsplit('#').next().expect("keys carry an ordinal"))
        .collect();
    assert_eq!(
        ordinals,
        vec!["0", "1", "2", "3"],
        "the ordinal is what separates a repeat from the activation it followed"
    );
    // Two of those holds named the SAME hull, so the ordinal is doing the
    // separating rather than the subject.
    let derelict_holds = hold_keys
        .iter()
        .filter(|k| k.starts_with(hold_keys[0].rsplit_once('#').expect("ordinal").0))
        .count();
    assert!(
        derelict_holds >= 3,
        "three of the four holds grip the same hull, so only the ordinal tells \
         them apart: {hold_keys:?}"
    );

    // Simultaneity: a whole scan ran inside a live hold, and neither ended the
    // other.
    let hold_windows: Vec<(u64, u64)> = rows
        .iter()
        .filter(|(_, kind, key, _)| *kind == "task_started" && key.contains("/tractor_hold/"))
        .filter_map(|(seq, _, key, _)| {
            rows.iter()
                .find(|(s, kind, k, _)| *kind != "task_started" && k == key && s > seq)
                .map(|(end, _, _, _)| (*seq, *end))
        })
        .collect();
    let scan_pairs: Vec<(u64, u64)> = rows
        .iter()
        .filter(|(_, kind, key, _)| *kind == "task_started" && key.contains("/scan/"))
        .filter_map(|(seq, _, key, _)| {
            rows.iter()
                .find(|(s, kind, k, _)| *kind != "task_started" && k == key && s > seq)
                .map(|(end, _, _, _)| (*seq, *end))
        })
        .collect();
    assert!(
        scan_pairs.iter().any(|(scan_start, scan_end)| {
            hold_windows
                .iter()
                .any(|(hold_start, hold_end)| hold_start < scan_start && scan_end < hold_end)
        }),
        "a scan ran to completion inside a live tractor hold, and the two must \
         be separately identifiable: holds {hold_windows:?}, scans {scan_pairs:?}"
    );

    // The cancelled hold is identifiable as cancelled, distinctly from the ones
    // that failed or were interrupted.
    let hold_reasons: Vec<String> = rows
        .iter()
        .filter(|(_, kind, key, _)| *kind != "task_started" && key.contains("/tractor_hold/"))
        .map(|(_, _, _, reason)| reason.clone())
        .collect();
    assert_eq!(
        hold_reasons,
        vec![
            "released".to_string(),
            "out_of_range".to_string(),
            "target_destroyed".to_string(),
            "mission_ended".to_string(),
        ],
        "each of the four holds ended its own way, in the order they were driven"
    );
}

/// **AC4**, first half: the timeline is a pure function of the seeded run.
#[test]
fn identical_seeds_produce_equivalent_task_timelines() {
    let (first, first_digest) = driven_run(ReportFormat::Json);
    let (second, second_digest) = driven_run(ReportFormat::Json);
    assert!(
        !first.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
    assert_eq!(
        first.narrative, second.narrative,
        "two runs with seed {SEED} produced different task timelines"
    );
    assert_eq!(
        first_digest, second_digest,
        "two runs with seed {SEED} produced different authoritative state"
    );
}

/// **AC4**, second half: lifecycle capture is a read-only projection. Turning
/// the ndjson stream on — the only runtime switch recording has — moves neither
/// the authoritative digest nor any line of the report.
#[test]
fn capturing_the_lifecycle_leaves_the_simulation_identical() {
    let (json_report, json_digest) = driven_run(ReportFormat::Json);
    let (ndjson_report, ndjson_digest) = driven_run(ReportFormat::Ndjson);

    assert_eq!(
        json_digest, ndjson_digest,
        "enabling stream capture moved the authoritative-state digest — \
         lifecycle capture must not change task timing, authority or state"
    );
    assert_eq!(
        json_report.to_json(),
        ndjson_report.to_json(),
        "the run report must not depend on whether stream capture was on"
    );
    assert!(
        !json_report.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
}

/// The NDJSON half: the same beats appear inline in the stream, in the shared
/// envelope — including the mission-end terminal written at the report boundary,
/// which is the one event no tick could have streamed for itself.
#[test]
fn the_ndjson_stream_carries_the_same_lifecycle_beats() {
    use project_phoenix::headless::report::RunTelemetry;

    let args = probe_args(ReportFormat::Ndjson);
    let mut app = build_headless_app(&args).expect("probe_task_lifecycle world should build");
    // The same drive as `driven_run`, kept here rather than shared because this
    // test needs the app afterwards, not just its report.
    run(&mut app, 60);
    let derelict = uuid_named(&mut app, DERELICT);
    let depot = uuid_named(&mut app, DEPOT);
    set_lock(&mut app, &derelict);
    tractor(&mut app, SystemControlPayload::EngageTractor);
    ask_for_scan(&mut app, &depot);
    tractor(&mut app, SystemControlPayload::ReleaseTractor);
    set_lock(&mut app, &depot);
    tractor(&mut app, SystemControlPayload::EngageTractor);
    run(&mut app, 4);

    let report = build_report(&mut app, &args, 0.0);
    let lines: Vec<String> = app
        .world()
        .resource::<RunTelemetry>()
        .stream
        .iter()
        .filter(|l| l.contains("\"narrative\":"))
        .cloned()
        .collect();

    assert!(
        !lines.is_empty(),
        "an ndjson run must carry the lifecycle beats inline"
    );
    for line in &lines {
        assert!(
            line.starts_with("{\"tick\":"),
            "every stream line uses the shared envelope: {line}"
        );
    }
    assert!(
        lines
            .iter()
            .any(|l| l.contains("\"kind\":\"task_started\"")),
        "the stream must carry the start beats:\n{}",
        lines.join("\n")
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("\"reason\":\"mission_ended\"")),
        "the report-boundary terminal must reach the stream too, or the two \
         surfaces disagree:\n{}",
        lines.join("\n")
    );

    let streamable = report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.in_timeline_stream())
        .count();
    assert_eq!(
        lines.len(),
        streamable,
        "the ndjson stream and the report's timeline must agree"
    );
}

/// The timeline stays the story, not a firehose: a world with no Objectives,
/// deadlines, beats or marked hulls records ONLY the tasks the crew ran, and one
/// pair of events per activation.
#[test]
fn only_the_tasks_the_crew_ran_reach_the_timeline() {
    let (report, _) = driven_run(ReportFormat::Json);
    for kind in report.narrative.counts_by_kind.keys() {
        assert!(
            kind.starts_with("task_"),
            "the probe authors no Objective, deadline, beat or marked hull, so \
             {kind:?} is routine simulation leaking into the story: {:?}",
            report.narrative.counts_by_kind
        );
    }
    let starts = count(&report, "task_started");
    let terminals: u64 = [
        "task_completed",
        "task_cancelled",
        "task_failed",
        "task_interrupted",
    ]
    .into_iter()
    .map(|k| count(&report, k))
    .sum();
    assert_eq!(
        starts, terminals,
        "one terminal event per start, run-wide: {:?}",
        report.narrative.counts_by_kind
    );
    assert_eq!(
        report.narrative.events.len() as u64,
        starts + terminals,
        "nothing but the lifecycle is being recorded here"
    );
}
