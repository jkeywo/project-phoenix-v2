//! The continuous-task lifecycle for docking, umbilical transfer and external
//! repair, driven end to end through real seeded headless runs (issue #1345,
//! PRD #1337).
//!
//! `tests/task_lifecycle_narrative.rs` proves the same contract for the two
//! #1341 tracer workflows (the scan and the tractor hold) with real
//! `RunReport` JSON/NDJSON assertions off a real admitted-command drive. That
//! file is deliberately left untouched — nothing about the tracer changed —
//! but the contract it proves is exactly what #1345 extends onto docking,
//! umbilical transfer and external repair, and until this file existed
//! nothing exercised any of the three through a real headless run: every test
//! the #1345 diff added asserted only on the in-memory `EffectQueue` a single
//! module system pushes onto, never on the narrative timeline a real seeded
//! run folds them into.
//!
//! Each probe world below is the SAME asset the module-level headless tests in
//! `tests/headless_runner.rs` already drive for their own (non-lifecycle) ACs
//! — `probe_dock.toml` (#1159), `probe_umbilical.toml` (#1160, which already
//! carries a dock AND an umbilical on one hull, so it doubles as the
//! simultaneity case), and `probe_external_repair.toml` (#1161) — driven here
//! through the identical `Dock`/`Undock`, `StartTransfer`/`StopTransfer` and
//! `DispatchExternalRepair`/`RecallExternalRepair` admitted-command paths, with
//! the assertions aimed at the narrative timeline instead.
//!
//! **AC4** ("existing behavior remains unchanged") is proved the same way
//! `task_lifecycle_narrative.rs` proves it for the tracer workflows: each of
//! the three workflows gets an identical-seeds `world_digest` comparison and a
//! JSON-vs-NDJSON `world_digest` comparison, so the timeline claims above are
//! backed by authoritative-state evidence, not narrative equality alone.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::console::weapons::beam::TacticalRadarSelection;
use project_phoenix::core::messages::{ClientMessage, SystemControlPayload, SystemId};
use project_phoenix::entities::spawner::EntityName;
use project_phoenix::headless::args::ticks_for_sim_seconds;
use project_phoenix::headless::report::{RunReport, RunTelemetry};
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};
use project_phoenix::lobby::InboundMessage;
use project_phoenix::server_app::LocalShip;
use project_phoenix::ship::state::ShipPhysics;
use project_phoenix::ship::system_registry::{
    DOCK_SYSTEM_ID, REPAIR_SYSTEM_ID, UMBILICAL_SYSTEM_ID,
};
use project_phoenix::sim_digest::world_digest;

// ── Shared plumbing ───────────────────────────────────────────────────────────

/// Every lifecycle event, as `(seq, kind, key, reason)` — `reason` empty for a
/// start beat. Mirrors `tests/task_lifecycle_narrative.rs::lifecycle_rows`.
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

fn detail_text(event: &project_phoenix::core::narrative::NarrativeEvent, key: &str) -> String {
    match event.detail.get(key) {
        Some(project_phoenix::core::narrative::NarrativeValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Every terminal reason for events whose key contains `verb_fragment`, in
/// sequence order.
fn reasons_for(report: &RunReport, verb_fragment: &str) -> Vec<String> {
    lifecycle_rows(report)
        .into_iter()
        .filter(|(_, kind, key, _)| *kind != "task_started" && key.contains(verb_fragment))
        .map(|(_, _, _, reason)| reason)
        .collect()
}

/// The whole-contract check `tests/task_lifecycle_narrative.rs` runs for the
/// tracer workflows: one start, exactly one terminal, sharing a key, the
/// terminal after the start and never silent about its reason.
fn assert_one_start_one_terminal_per_key(report: &RunReport) {
    use std::collections::BTreeMap;
    let rows = lifecycle_rows(report);
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
}

/// The reason/outcome/reason_text agreement check
/// `tests/task_lifecycle_narrative.rs` runs on every terminal event: the class
/// matches the kind, and the localizable half is always a `strings.csv` id,
/// never prose.
fn assert_reason_outcome_and_string_id_agree(report: &RunReport) {
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
}

fn operator(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the probe world spawns a local operator")
}

fn place_operator(app: &mut App, position: Vec3) {
    let entity = operator(app);
    let mut physics = app
        .world_mut()
        .get_mut::<ShipPhysics>(entity)
        .expect("the operator is a ship");
    physics.x = position.x;
    physics.y = position.y;
    physics.z = position.z;
}

fn named_entity(app: &mut App, name: &str) -> Entity {
    let mut q = app.world_mut().query::<(Entity, &EntityName)>();
    q.iter(app.world())
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("the probe world spawns '{name}'"))
}

fn move_entity(app: &mut App, entity: Entity, position: Vec3) {
    app.world_mut()
        .get_mut::<Transform>(entity)
        .expect("the entity has a transform")
        .translation = position;
}

fn despawn(app: &mut App, entity: Entity) {
    app.world_mut().entity_mut(entity).despawn();
}

/// Send one system command through the real admission path and give it the
/// ticks to arrive: drained in `PreUpdate`, admitted before `SimSet::Input`,
/// consumed the same tick.
fn send(app: &mut App, token: &str, target: SystemId, payload: SystemControlPayload) {
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg: ClientMessage::ControlSystem { target, payload },
        });
    run(app, 2);
}

fn ndjson_lines(app: &App) -> Vec<String> {
    app.world()
        .resource::<RunTelemetry>()
        .stream
        .iter()
        .filter(|l| l.contains("\"narrative\":"))
        .cloned()
        .collect()
}

// ── #1159 dock: start, cancel, range loss, target destruction, mission end ──

const BERTH: &str = "world.probe_dock.entity.berth.name";
const DOCK_TOKEN: &str = "ai:dock-lifecycle-probe";

fn dock_args(format: project_phoenix::headless::args::ReportFormat) -> HeadlessArgs {
    let dt = 1.0 / 60.0;
    HeadlessArgs {
        world_path: "assets/worlds/probe_dock.toml".into(),
        ship_path: "assets/entities/dock_probe.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        seed: Some(1345),
        deterministic: true,
        report_format: format,
        ..Default::default()
    }
}

fn send_dock(app: &mut App, payload: SystemControlPayload) {
    send(app, DOCK_TOKEN, SystemId(DOCK_SYSTEM_ID.into()), payload);
}

/// One driven dock run: commit, cancel, drift out of range, lose the target,
/// and leave a fourth hold running when the mission ends. Returns the
/// authoritative-state digest taken just before the report is built, so
/// callers can prove the lifecycle capture changed nothing (mirrors
/// `tests/task_lifecycle_narrative.rs::driven_run`).
fn driven_dock_run(format: project_phoenix::headless::args::ReportFormat) -> (RunReport, u64, App) {
    let args = dock_args(format);
    let mut app = build_headless_app(&args).expect("probe_dock world should build");
    run(&mut app, 60);
    let berth = named_entity(&mut app, BERTH);

    // 1. Dock, then an explicit Undock — the CANCELLED case.
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, berth, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    send_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 140);
    send_dock(&mut app, SystemControlPayload::Undock);
    run(&mut app, 160);

    // 2. Dock again, then drift out of range — a FAILED case.
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, berth, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    send_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 140);
    move_entity(&mut app, berth, Vec3::new(9000.0, 0.0, 0.0));
    run(&mut app, 3);

    // 3. Dock a third time, then destroy the berth — an INTERRUPTED case read
    //    as a destroyed subject.
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, berth, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    send_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 140);
    despawn(&mut app, berth);
    run(&mut app, 3);

    let digest = world_digest(app.world());
    (build_report(&mut app, &args, 0.0), digest, app)
}

#[test]
fn dock_hold_lifecycle_records_cancel_range_loss_and_target_destruction() {
    use project_phoenix::headless::args::ReportFormat;

    let (report, _digest, _app) = driven_dock_run(ReportFormat::Json);
    let rows = lifecycle_rows(&report);
    assert!(
        !rows.is_empty(),
        "the driven run exercised the dock and the timeline carries none of it: {:?}",
        report.narrative.counts_by_kind
    );
    assert_one_start_one_terminal_per_key(&report);
    assert_reason_outcome_and_string_id_agree(&report);

    // Every beat is a dock_hold on the DOCK_SYSTEM_ID.
    for (_, _, key, _) in &rows {
        assert!(
            key.contains(&format!("/{DOCK_SYSTEM_ID}/dock_hold/")),
            "unexpected task key for a dock probe run: {key}"
        );
    }

    let hold_reasons = reasons_for(&report, "/dock_hold/");
    assert_eq!(
        hold_reasons,
        vec![
            "released".to_string(),
            "out_of_range".to_string(),
            "target_destroyed".to_string()
        ],
        "each of the three closed holds ended its own way, in the order driven: {hold_reasons:?}"
    );

    // A localized reason must never reach the wire — the reason is always the
    // String Id, never the resolved English.
    let json = report.to_json();
    assert!(
        !json.contains("Target destroyed"),
        "localized English must never enter the timeline:\n{json}"
    );
}

#[test]
fn dock_hold_still_running_closes_as_mission_ended_at_the_report_boundary() {
    use project_phoenix::headless::args::ReportFormat;

    let args = dock_args(ReportFormat::Json);
    let mut app = build_headless_app(&args).expect("probe_dock world should build");
    run(&mut app, 60);
    let berth = named_entity(&mut app, BERTH);
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, berth, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    send_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 140);

    // No further command: the mission ends with the dock still holding.
    let report = build_report(&mut app, &args, 0.0);
    let rows = lifecycle_rows(&report);
    assert_eq!(
        rows.len(),
        2,
        "one start, one mission-ended terminal for the still-running hold: {rows:?}"
    );
    assert_eq!(rows[0].1, "task_started");
    assert_eq!(rows[1].1, "task_interrupted");
    assert_eq!(rows[1].3, "mission_ended");
}

/// **AC4**, first half, for the dock workflow: the timeline is a pure
/// function of the seeded run, and so is the authoritative state underneath
/// it (mirrors `task_lifecycle_narrative::identical_seeds_produce_equivalent_task_timelines`).
#[test]
fn identical_seeds_produce_equivalent_dock_lifecycle_timelines() {
    use project_phoenix::headless::args::ReportFormat;

    let (first, first_digest, _) = driven_dock_run(ReportFormat::Json);
    let (second, second_digest, _) = driven_dock_run(ReportFormat::Json);
    assert!(
        !first.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
    assert_eq!(
        first.narrative, second.narrative,
        "two runs of the same seed produced different dock lifecycle timelines"
    );
    assert_eq!(
        first_digest, second_digest,
        "two runs of the same seed produced different authoritative state for the dock workflow"
    );
}

/// **AC4**, second half, for the dock workflow: lifecycle capture is a
/// read-only projection. Turning the ndjson stream on moves neither the
/// authoritative digest nor any line of the report (mirrors
/// `task_lifecycle_narrative::capturing_the_lifecycle_leaves_the_simulation_identical`).
#[test]
fn capturing_the_dock_lifecycle_leaves_the_simulation_identical() {
    use project_phoenix::headless::args::ReportFormat;

    let (json_report, json_digest, _) = driven_dock_run(ReportFormat::Json);
    let (ndjson_report, ndjson_digest, _) = driven_dock_run(ReportFormat::Ndjson);

    assert_eq!(
        json_digest, ndjson_digest,
        "enabling stream capture moved the authoritative-state digest for the dock workflow — \
         lifecycle capture must not change task timing, authority or state"
    );
    assert_eq!(
        json_report.to_json(),
        ndjson_report.to_json(),
        "the dock run report must not depend on whether stream capture was on"
    );
    assert!(
        !json_report.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
}

#[test]
fn the_dock_lifecycle_reaches_the_ndjson_stream_with_the_shared_envelope() {
    use project_phoenix::headless::args::ReportFormat;

    let args = dock_args(ReportFormat::Ndjson);
    let mut app = build_headless_app(&args).expect("probe_dock world should build");
    run(&mut app, 60);
    let berth = named_entity(&mut app, BERTH);
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, berth, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    send_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 140);
    send_dock(&mut app, SystemControlPayload::Undock);
    run(&mut app, 160);

    let report = build_report(&mut app, &args, 0.0);
    let lines = ndjson_lines(&app);
    assert!(
        !lines.is_empty(),
        "an ndjson run must carry the dock lifecycle beats inline"
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
        "the stream must carry the start beat:\n{}",
        lines.join("\n")
    );
    assert!(
        lines.iter().any(|l| l.contains("\"reason\":\"released\"")),
        "the stream must carry the release terminal:\n{}",
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

// ── #1160 umbilical: dock_hold and umbilical_flow run simultaneously ────────

const DEPOT: &str = "world.probe_umbilical.entity.depot.name";
const UMBILICAL_DOCK_TOKEN: &str = "ai:umbilical-lifecycle-dock";
const UMBILICAL_FLOW_TOKEN: &str = "ai:umbilical-lifecycle-flow";

fn umbilical_args(format: project_phoenix::headless::args::ReportFormat) -> HeadlessArgs {
    let dt = 1.0 / 60.0;
    HeadlessArgs {
        world_path: "assets/worlds/probe_umbilical.toml".into(),
        ship_path: "assets/entities/umbilical_probe.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        seed: Some(1345),
        deterministic: true,
        report_format: format,
        ..Default::default()
    }
}

fn send_umbilical_dock(app: &mut App, payload: SystemControlPayload) {
    send(
        app,
        UMBILICAL_DOCK_TOKEN,
        SystemId(DOCK_SYSTEM_ID.into()),
        payload,
    );
}

fn send_umbilical_flow(app: &mut App, payload: SystemControlPayload) {
    send(
        app,
        UMBILICAL_FLOW_TOKEN,
        SystemId(UMBILICAL_SYSTEM_ID.into()),
        payload,
    );
}

/// One driven umbilical run: dock, run a flow inside the hold, stop the flow,
/// then undock — the same simultaneity drive as
/// `a_dock_hold_and_an_umbilical_flow_run_simultaneously_and_close_independently`,
/// factored out so the digest tests below can run it twice under the same
/// seed. Returns the authoritative-state digest taken just before the report
/// is built (mirrors `tests/task_lifecycle_narrative.rs::driven_run`).
fn driven_umbilical_run(format: project_phoenix::headless::args::ReportFormat) -> (RunReport, u64) {
    let args = umbilical_args(format);
    let mut app = build_headless_app(&args).expect("probe_umbilical world should build");
    run(&mut app, 60);
    let depot = named_entity(&mut app, DEPOT);
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, depot, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);

    send_umbilical_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 160);
    send_umbilical_flow(&mut app, SystemControlPayload::StartTransfer);
    run(&mut app, 90);
    send_umbilical_flow(&mut app, SystemControlPayload::StopTransfer);
    run(&mut app, 3);
    send_umbilical_dock(&mut app, SystemControlPayload::Undock);
    run(&mut app, 200);

    let digest = world_digest(app.world());
    (build_report(&mut app, &args, 0.0), digest)
}

/// **AC3's simultaneity claim, for #1345's own pair**: a dock hold and an
/// umbilical flow are two different systems on the SAME operator, opened one
/// after the other and closed in reverse order, and neither's key nor
/// terminal touches the other's.
#[test]
fn a_dock_hold_and_an_umbilical_flow_run_simultaneously_and_close_independently() {
    use project_phoenix::headless::args::ReportFormat;

    let args = umbilical_args(ReportFormat::Json);
    let mut app = build_headless_app(&args).expect("probe_umbilical world should build");
    run(&mut app, 60);
    let depot = named_entity(&mut app, DEPOT);
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, depot, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);

    // Dock: opens the dock_hold.
    send_umbilical_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 160);
    // Start the flow: opens the umbilical_flow WHILE the dock_hold is live.
    send_umbilical_flow(&mut app, SystemControlPayload::StartTransfer);
    run(&mut app, 90);
    // Stop the flow: closes the umbilical_flow, the dock_hold still live.
    send_umbilical_flow(&mut app, SystemControlPayload::StopTransfer);
    run(&mut app, 3);
    // Undock: closes the dock_hold.
    send_umbilical_dock(&mut app, SystemControlPayload::Undock);
    run(&mut app, 200);

    let report = build_report(&mut app, &args, 0.0);
    assert_one_start_one_terminal_per_key(&report);
    assert_reason_outcome_and_string_id_agree(&report);

    let rows = lifecycle_rows(&report);
    let dock_rows: Vec<_> = rows
        .iter()
        .filter(|(_, _, key, _)| key.contains("/dock_hold/"))
        .cloned()
        .collect();
    let flow_rows: Vec<_> = rows
        .iter()
        .filter(|(_, _, key, _)| key.contains("/umbilical_flow/"))
        .cloned()
        .collect();
    assert_eq!(
        dock_rows.len(),
        2,
        "one start, one terminal for the dock hold: {dock_rows:?}"
    );
    assert_eq!(
        flow_rows.len(),
        2,
        "one start, one terminal for the umbilical flow: {flow_rows:?}"
    );
    assert_eq!(dock_rows[1].3, "released");
    assert_eq!(flow_rows[1].3, "released");

    // Simultaneity: the flow's whole window sits inside the dock hold's.
    let dock_start = dock_rows[0].0;
    let dock_end = dock_rows[1].0;
    let flow_start = flow_rows[0].0;
    let flow_end = flow_rows[1].0;
    assert!(
        dock_start < flow_start && flow_end < dock_end,
        "the umbilical flow must run entirely inside the live dock hold: \
         dock [{dock_start},{dock_end}], flow [{flow_start},{flow_end}]"
    );
}

/// Undocking mid-flow ends the flow with `target_lost` (the umbilical can only
/// see that its docked partner is gone) and ends the dock hold with
/// `released` (the explicit Undock) — two different reasons for one shared
/// cause, each on its own key.
#[test]
fn undocking_mid_flow_ends_the_flow_as_target_lost_and_the_dock_as_released() {
    use project_phoenix::headless::args::ReportFormat;

    let args = umbilical_args(ReportFormat::Json);
    let mut app = build_headless_app(&args).expect("probe_umbilical world should build");
    run(&mut app, 60);
    let depot = named_entity(&mut app, DEPOT);
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, depot, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);
    send_umbilical_dock(&mut app, SystemControlPayload::Dock);
    run(&mut app, 160);
    send_umbilical_flow(&mut app, SystemControlPayload::StartTransfer);
    run(&mut app, 30);

    // Undock while the flow is still running — no StopTransfer first.
    send_umbilical_dock(&mut app, SystemControlPayload::Undock);
    run(&mut app, 5);

    let report = build_report(&mut app, &args, 0.0);
    assert_one_start_one_terminal_per_key(&report);
    assert_eq!(
        reasons_for(&report, "/umbilical_flow/"),
        vec!["target_lost".to_string()]
    );
    assert_eq!(
        reasons_for(&report, "/dock_hold/"),
        vec!["released".to_string()]
    );
}

/// **AC4**, first half, for the umbilical workflow.
#[test]
fn identical_seeds_produce_equivalent_umbilical_lifecycle_timelines() {
    use project_phoenix::headless::args::ReportFormat;

    let (first, first_digest) = driven_umbilical_run(ReportFormat::Json);
    let (second, second_digest) = driven_umbilical_run(ReportFormat::Json);
    assert!(
        !first.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
    assert_eq!(
        first.narrative, second.narrative,
        "two runs of the same seed produced different umbilical lifecycle timelines"
    );
    assert_eq!(
        first_digest, second_digest,
        "two runs of the same seed produced different authoritative state for the umbilical workflow"
    );
}

/// **AC4**, second half, for the umbilical workflow: lifecycle capture is a
/// read-only projection over the dock+flow simultaneity drive.
#[test]
fn capturing_the_umbilical_lifecycle_leaves_the_simulation_identical() {
    use project_phoenix::headless::args::ReportFormat;

    let (json_report, json_digest) = driven_umbilical_run(ReportFormat::Json);
    let (ndjson_report, ndjson_digest) = driven_umbilical_run(ReportFormat::Ndjson);

    assert_eq!(
        json_digest, ndjson_digest,
        "enabling stream capture moved the authoritative-state digest for the umbilical \
         workflow — lifecycle capture must not change task timing, authority or state"
    );
    assert_eq!(
        json_report.to_json(),
        ndjson_report.to_json(),
        "the umbilical run report must not depend on whether stream capture was on"
    );
    assert!(
        !json_report.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
}

// ── #1161 external repair: dispatch, recall, range loss, mission end ───────

const ALLY: &str = "world.probe_external_repair.entity.ally.name";
const REPAIR_TOKEN: &str = "ai:external-repair-lifecycle-probe";

fn external_repair_args(format: project_phoenix::headless::args::ReportFormat) -> HeadlessArgs {
    let dt = 1.0 / 60.0;
    HeadlessArgs {
        world_path: "assets/worlds/probe_external_repair.toml".into(),
        ship_path: "assets/entities/repair_tender.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(60.0, dt),
        seed: Some(1345),
        deterministic: true,
        report_format: format,
        ..Default::default()
    }
}

fn set_repair_lock(app: &mut App, uuid: Option<String>) {
    let op = operator(app);
    app.world_mut()
        .entity_mut(op)
        .insert(TacticalRadarSelection(uuid));
}

fn send_repair(app: &mut App, payload: SystemControlPayload) {
    send(
        app,
        REPAIR_TOKEN,
        SystemId(REPAIR_SYSTEM_ID.into()),
        payload,
    );
}

fn ally_uuid(app: &mut App) -> String {
    use project_phoenix::entities::spawner::EntityUuid;
    let mut q = app.world_mut().query::<(&EntityName, &EntityUuid)>();
    q.iter(app.world())
        .find(|(n, _)| n.0 == ALLY)
        .map(|(_, uuid)| uuid.0.clone())
        .expect("the probe world spawns the ally")
}

/// One driven external-repair run: dispatch, then an explicit recall.
/// Returns the authoritative-state digest taken just before the report is
/// built (mirrors `tests/task_lifecycle_narrative.rs::driven_run`).
fn driven_external_repair_run(
    format: project_phoenix::headless::args::ReportFormat,
) -> (RunReport, u64) {
    let args = external_repair_args(format);
    let mut app = build_headless_app(&args).expect("probe_external_repair world should build");
    run(&mut app, 60);
    let ally = ally_uuid(&mut app);
    place_operator(&mut app, Vec3::ZERO);
    set_repair_lock(&mut app, Some(ally));
    run(&mut app, 1);
    send_repair(&mut app, SystemControlPayload::DispatchExternalRepair);
    run(&mut app, 2);
    send_repair(&mut app, SystemControlPayload::RecallExternalRepair);
    run(&mut app, 4);

    let digest = world_digest(app.world());
    (build_report(&mut app, &args, 0.0), digest)
}

#[test]
fn external_repair_lifecycle_records_dispatch_recall_and_range_loss() {
    use project_phoenix::headless::args::ReportFormat;

    let args = external_repair_args(ReportFormat::Json);
    let mut app = build_headless_app(&args).expect("probe_external_repair world should build");
    run(&mut app, 60);
    let ally = ally_uuid(&mut app);

    // 1. Dispatch, then an explicit Recall — the CANCELLED case.
    place_operator(&mut app, Vec3::ZERO);
    set_repair_lock(&mut app, Some(ally.clone()));
    run(&mut app, 1);
    send_repair(&mut app, SystemControlPayload::DispatchExternalRepair);
    run(&mut app, 2);
    send_repair(&mut app, SystemControlPayload::RecallExternalRepair);
    run(&mut app, 2);

    // 2. Dispatch again, then drift out of the authored reach — a FAILED case.
    send_repair(&mut app, SystemControlPayload::DispatchExternalRepair);
    run(&mut app, 2);
    place_operator(&mut app, Vec3::new(50_000.0, 0.0, 0.0));
    run(&mut app, 3);

    // 3. A third dispatch, still committed when the run stops — INTERRUPTED
    //    by the mission ending.
    place_operator(&mut app, Vec3::ZERO);
    run(&mut app, 2);
    send_repair(&mut app, SystemControlPayload::DispatchExternalRepair);
    run(&mut app, 2);

    let report = build_report(&mut app, &args, 0.0);
    assert_one_start_one_terminal_per_key(&report);
    assert_reason_outcome_and_string_id_agree(&report);

    for (_, _, key, _) in lifecycle_rows(&report) {
        assert!(
            key.contains(&format!("/{REPAIR_SYSTEM_ID}/external_repair/")),
            "unexpected task key for an external-repair probe run: {key}"
        );
    }

    assert_eq!(
        reasons_for(&report, "/external_repair/"),
        vec![
            "released".to_string(),
            "out_of_range".to_string(),
            "mission_ended".to_string(),
        ],
        "the three dispatches ended their own way, in the order driven"
    );
}

/// **AC4**, first half, for the external-repair workflow.
#[test]
fn identical_seeds_produce_equivalent_external_repair_lifecycle_timelines() {
    use project_phoenix::headless::args::ReportFormat;

    let (first, first_digest) = driven_external_repair_run(ReportFormat::Json);
    let (second, second_digest) = driven_external_repair_run(ReportFormat::Json);
    assert!(
        !first.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
    assert_eq!(
        first.narrative, second.narrative,
        "two runs of the same seed produced different external-repair lifecycle timelines"
    );
    assert_eq!(
        first_digest, second_digest,
        "two runs of the same seed produced different authoritative state for the \
         external-repair workflow"
    );
}

/// **AC4**, second half, for the external-repair workflow: lifecycle capture
/// is a read-only projection.
#[test]
fn capturing_the_external_repair_lifecycle_leaves_the_simulation_identical() {
    use project_phoenix::headless::args::ReportFormat;

    let (json_report, json_digest) = driven_external_repair_run(ReportFormat::Json);
    let (ndjson_report, ndjson_digest) = driven_external_repair_run(ReportFormat::Ndjson);

    assert_eq!(
        json_digest, ndjson_digest,
        "enabling stream capture moved the authoritative-state digest for the \
         external-repair workflow — lifecycle capture must not change task timing, \
         authority or state"
    );
    assert_eq!(
        json_report.to_json(),
        ndjson_report.to_json(),
        "the external-repair run report must not depend on whether stream capture was on"
    );
    assert!(
        !json_report.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
}
