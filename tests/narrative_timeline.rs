//! The authored mission timeline, end to end (issue #1338, PRD #1337).
//!
//! `src/core/narrative.rs` and `src/narrative.rs` unit-test the vocabulary, the
//! fold and each emitter in isolation. Three claims need a whole seeded run:
//!
//! 1. **Every authored category reaches the report.** `probe_narrative.toml`
//!    authors one of each — two Objectives (one completed, one failed), a named
//!    deadline, authored beats, a Comms thread, and one MARKED entity with an
//!    authored outcome — and the run report's `narrative` field must carry all
//!    of them, with stable sequencing, the fixed sim tick/time, and String Ids
//!    rather than localized prose.
//!
//! 2. **Marking is the gate.** The same run's unmarked player hull produces no
//!    entity beat at all, and no routine simulation event (traffic, damage,
//!    repair) reaches the timeline. Without this the first claim would be
//!    satisfied by a firehose.
//!
//! 3. **Recording is inert to the authoritative outcome.** Two seeded runs
//!    produce equivalent timelines, and the SAME seeded run under `--format
//!    json` and `--format ndjson` — the flag that turns the inline stream
//!    capture on — folds to a byte-identical authoritative-state digest and
//!    reports the identical timeline. Narrative capture is a read-only
//!    projection off state the tick already decided.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use project_phoenix::headless::args::{ticks_for_sim_seconds, ReportFormat};
use project_phoenix::headless::report::RunReport;
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};
use project_phoenix::sim_digest::world_digest;

/// The probe world's own seed is authored; pinning it on the CLI is what makes
/// the run replayable (`--seed` implies the single-threaded scheduler).
const SEED: u64 = 1338;
/// Past the last authored beat (the deadline at t=6) with room for the Comms AI
/// to answer, and short enough to stay a quick test.
const SIM_SECONDS: f64 = 20.0;

fn probe_args(format: ReportFormat) -> HeadlessArgs {
    let dt = 1.0 / 30.0;
    HeadlessArgs {
        world_path: "assets/worlds/probe_narrative.toml".into(),
        ship_path: "assets/entities/alliance_courier.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(SIM_SECONDS, dt),
        seed: Some(SEED),
        deterministic: true,
        report_format: format,
        ..Default::default()
    }
}

/// One seeded run of the probe world. Returns the report and the run's final
/// authoritative-state digest.
fn probe_run(format: ReportFormat) -> (RunReport, u64) {
    let args = probe_args(format);
    let mut app = build_headless_app(&args).expect("probe_narrative world should build");
    run(&mut app, args.max_ticks);
    let digest = world_digest(app.world());
    // `wall_seconds` of 0.0 so the derived timing fields are constants rather
    // than measurements — the same trick `tests/rng_determinism.rs` uses so two
    // reports can be compared byte for byte.
    (build_report(&mut app, &args, 0.0), digest)
}

/// How many events of `kind` the timeline holds.
fn count(report: &RunReport, kind: &str) -> u64 {
    report
        .narrative
        .counts_by_kind
        .get(kind)
        .copied()
        .unwrap_or(0)
}

/// The ids recorded for one kind, in sequence order.
fn ids(report: &RunReport, kind: &str) -> Vec<String> {
    report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.kind.as_str() == kind)
        .map(|e| e.event.id.clone())
        .collect()
}

/// AC1 + AC2: every authored category appears, with stable ordering, the fixed
/// tick/time, semantic identifiers, and String Ids rather than prose.
#[test]
fn every_authored_category_reaches_the_run_report() {
    let (report, _) = probe_run(ReportFormat::Json);

    assert!(
        !report.narrative.events.is_empty(),
        "the probe world authored a whole timeline and the report carries none of it"
    );

    // ── Objectives ────────────────────────────────────────────────────────────
    assert_eq!(
        count(&report, "objective_posted"),
        3,
        "every authored Objective must be posted, including the one posted and \
         completed inside a single tick: {:?}",
        report.narrative.counts_by_kind
    );
    assert_eq!(
        ids(&report, "objective_completed"),
        vec!["sound_off".to_string(), "hold_the_channel".to_string()]
    );
    assert_eq!(
        ids(&report, "objective_failed"),
        vec!["log_the_wreck".to_string()]
    );

    // ── The deadline ──────────────────────────────────────────────────────────
    assert_eq!(
        ids(&report, "deadline_fired"),
        vec!["window_opens".to_string()]
    );

    // ── Authored beats ────────────────────────────────────────────────────────
    let beats = ids(&report, "beat_fired");
    assert!(
        beats.contains(&"probe_opening".to_string()),
        "the opening beat is missing: {beats:?}"
    );
    assert!(
        beats.contains(&"probe_deadline_beat".to_string()),
        "the deadline's own beat is missing: {beats:?}"
    );

    // ── Comms ─────────────────────────────────────────────────────────────────
    assert_eq!(
        count(&report, "comms_opened"),
        1,
        "Lyra's hail never reached the timeline: {:?}",
        report.narrative.counts_by_kind
    );
    // The choice, made by the Comms AI through the same admitted
    // `RespondToMessage` a human console submits — which is why this assertion
    // can exist in a crewless headless run at all.
    assert_eq!(
        count(&report, "comms_answered"),
        1,
        "the hail was opened but no choice was recorded: {:?}",
        report.narrative.counts_by_kind
    );
    let answered = report
        .narrative
        .events
        .iter()
        .find(|e| e.event.kind.as_str() == "comms_answered")
        .expect("checked above");
    // The two halves of a Comms beat share the thread id, so an analysis can
    // pair the hail with the answer without re-deriving the conversation.
    let opened = report
        .narrative
        .events
        .iter()
        .find(|e| e.event.kind.as_str() == "comms_opened")
        .expect("checked above");
    assert_eq!(
        answered.event.id, opened.event.id,
        "opening and answering must share the thread id"
    );
    assert!(
        answered.seq > opened.seq,
        "a thread cannot be answered before it opens"
    );
    // …and they point in OPPOSITE directions. The hail is produced by Lyra and
    // done to nobody in particular; the answer is produced by the crew and done
    // to Lyra. Recording the hailer as the answer's `source` would credit the
    // counterparty with the crew's own choice, which is exactly the reading a
    // "who did this" analysis takes off `source.entity`.
    let hailer = opened
        .event
        .source
        .entity
        .as_deref()
        .expect("the hail names its sender");
    assert_eq!(
        answered.event.target.as_deref(),
        Some(hailer),
        "the answered thread's target is the entity that was answered"
    );
    assert_ne!(
        answered.event.source.entity.as_deref(),
        Some(hailer),
        "the crew's choice must not be attributed to the hailing entity"
    );
    // The one narrative kind whose actor is a station-side decision — and so the
    // only place the `station` / `system` axes of a `NarrativeActor` are live.
    assert_eq!(
        answered.event.source.system.as_deref(),
        Some("comms"),
        "the answer is produced on the comms system: {:?}",
        answered.event.source
    );
    assert!(
        answered.event.source.station.is_some(),
        "the courier resolves a comms host station, and the answer names it: {:?}",
        answered.event.source
    );
    assert!(
        answered.event.source.entity.is_some(),
        "the answering hull is the crew's own ship: {:?}",
        answered.event.source
    );

    // ── The marked entities ───────────────────────────────────────────────────
    assert_eq!(
        ids(&report, "marked_entity_spawned"),
        vec![
            "world.probe_narrative.entity.lyra.name".to_string(),
            "world.probe_narrative.entity.wreck.name".to_string(),
        ],
        "both marked hulls arrive, in authored-id order"
    );
    assert_eq!(
        ids(&report, "marked_entity_rescued"),
        vec!["world.probe_narrative.entity.lyra.name".to_string()],
        "the authored outcome must be recorded against the authored name"
    );

    // ── Stable ordering, fixed tick, derived time ─────────────────────────────
    for (i, event) in report.narrative.events.iter().enumerate() {
        assert_eq!(
            event.seq, i as u64,
            "the monotonic sequence must be dense and in order"
        );
        assert!(
            event.sim_t >= 0.0,
            "sim time is derived from the fixed tick, never a wall clock"
        );
    }
    let ticks: Vec<u64> = report.narrative.events.iter().map(|e| e.tick).collect();
    assert!(
        ticks.windows(2).all(|w| w[0] <= w[1]),
        "the timeline must be non-decreasing in sim tick: {ticks:?}"
    );

    // ── String Ids, not prose ─────────────────────────────────────────────────
    let json = report.to_json();
    assert!(
        json.contains("\"text\":\"world.probe_narrative.objective.hold_the_channel\""),
        "the Objective's strings.csv id must pass through verbatim:\n{json}"
    );
    assert!(
        json.contains("\"label\":\"world.probe_narrative.deadline.window_opens.label\""),
        "the deadline's strings.csv id must pass through verbatim:\n{json}"
    );
    assert!(
        !json.contains("Hold the channel open"),
        "localized English must never enter the timeline:\n{json}"
    );
}

/// AC2's other half: the timeline is not a firehose. A run of a world with no
/// combat and one marked hull records only what the scenario authored — no
/// beat for the unmarked player ship, and no kind that nothing authored.
#[test]
fn routine_simulation_never_reaches_the_timeline() {
    let (report, _) = probe_run(ReportFormat::Json);

    // Every kind present must be one this world actually authors.
    let authored: std::collections::BTreeSet<&str> = [
        "objective_posted",
        "objective_completed",
        "objective_failed",
        "deadline_fired",
        "beat_fired",
        "comms_opened",
        "comms_answered",
        "marked_entity_spawned",
        "marked_entity_rescued",
        "marked_entity_destroyed",
        "computer_message_posted",
        "computer_message_cleared",
        // The post-mission report (issue #1344): one row, and the finalized
        // beat the report-bearing ending writes.
        "report_row_updated",
        "report_finalized",
    ]
    .into_iter()
    .collect();
    for kind in report.narrative.counts_by_kind.keys() {
        assert!(
            authored.contains(kind),
            "the timeline carries {kind:?}, which nothing in probe_narrative.toml \
             authors — a routine simulation event has leaked into the story: {:?}",
            report.narrative.counts_by_kind
        );
    }

    // The player hull is unmarked, so exactly two entity spawn beats exist —
    // Lyra's and the derelict's — however many hulls the world puts in the sky.
    assert_eq!(
        count(&report, "marked_entity_spawned"),
        2,
        "marking is the gate: only the marked hulls report a spawn"
    );

    // And the whole timeline stays small. A four-beat scenario that produced
    // hundreds of events would be inferring, not recording.
    assert!(
        report.narrative.events.len() < 32,
        "a no-combat probe produced {} timeline events — something is inferring \
         story from routine simulation",
        report.narrative.events.len()
    );
}

/// The ship's-computer message's full lifecycle (issue #1342), through the
/// real script boundary: `debris_notice` (t=1, 2s) EXPIRES on its own,
/// `charge_ready` (t=4, 10s, Station "tactical") is SUPERSEDED by
/// `wreck_cleared` at t=8 rather than expiring — proving "a new message
/// supersedes it immediately" — and `wreck_cleared` itself is simply SHOWN
/// and outlives the run.
#[test]
fn the_ships_computer_message_lifecycle_reaches_the_run_report() {
    let (report, _) = probe_run(ReportFormat::Json);

    assert_eq!(
        ids(&report, "computer_message_posted"),
        vec![
            "debris_notice".to_string(),
            "charge_ready".to_string(),
            "wreck_cleared".to_string(),
        ],
        "all three messages must be shown, in authored order: {:?}",
        report.narrative.counts_by_kind
    );

    let cleared: Vec<&project_phoenix::core::narrative::StampedNarrativeEvent> = report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.kind.as_str() == "computer_message_cleared")
        .collect();
    assert_eq!(
        cleared.len(),
        2,
        "debris_notice expires and charge_ready is superseded — two clears, \
         not three: wreck_cleared outlives the run: {:?}",
        report.narrative.counts_by_kind
    );

    let expired = cleared
        .iter()
        .find(|e| e.event.id == "debris_notice")
        .expect("debris_notice's short duration must expire it");
    assert_eq!(
        expired.event.detail.get("reason"),
        Some(&project_phoenix::core::narrative::NarrativeValue::Text(
            "expired".into()
        ))
    );

    let superseded = cleared
        .iter()
        .find(|e| e.event.id == "charge_ready")
        .expect("charge_ready must be superseded by wreck_cleared, not expire");
    assert_eq!(
        superseded.event.detail.get("reason"),
        Some(&project_phoenix::core::narrative::NarrativeValue::Text(
            "superseded".into()
        ))
    );
    assert_eq!(
        superseded.event.detail.get("superseded_by"),
        Some(&project_phoenix::core::narrative::NarrativeValue::Text(
            "wreck_cleared".into()
        ))
    );

    // The shown event's own detail: String Id text, severity, duration, and
    // the optional Station cue — never localized prose.
    let charge_ready = report
        .narrative
        .events
        .iter()
        .find(|e| {
            e.event.kind.as_str() == "computer_message_posted" && e.event.id == "charge_ready"
        })
        .expect("checked above");
    assert_eq!(
        charge_ready.event.detail.get("text"),
        Some(&project_phoenix::core::narrative::NarrativeValue::Text(
            "world.probe_narrative.computer_message.charge".into()
        ))
    );
    assert_eq!(
        charge_ready.event.detail.get("severity"),
        Some(&project_phoenix::core::narrative::NarrativeValue::Text(
            "critical".into()
        ))
    );
    assert_eq!(
        charge_ready.event.detail.get("duration_secs"),
        Some(&project_phoenix::core::narrative::NarrativeValue::Int(10))
    );
    assert_eq!(
        charge_ready.event.detail.get("station"),
        Some(&project_phoenix::core::narrative::NarrativeValue::Text(
            "tactical".into()
        ))
    );

    // Ordering: shown before its own clear, and the supersession happens
    // strictly after charge_ready was shown.
    let shown_charge_ready = charge_ready.seq;
    assert!(superseded.seq > shown_charge_ready);
    let shown_debris = report
        .narrative
        .events
        .iter()
        .find(|e| {
            e.event.kind.as_str() == "computer_message_posted" && e.event.id == "debris_notice"
        })
        .expect("checked above")
        .seq;
    assert!(expired.seq > shown_debris);

    // Localized English must never enter the timeline — only the String Id.
    let json = report.to_json();
    assert!(
        json.contains("\"world.probe_narrative.computer_message.charge\""),
        "the message's strings.csv id must pass through verbatim:\n{json}"
    );
    assert!(
        !json.contains("Tactical charge nearing safety threshold"),
        "localized English must never enter the timeline:\n{json}"
    );
}

/// A scripted removal is the engine's OTHER death path, and it writes no
/// `BalanceEvent` — an authorial removal must never enter the combat ledger. So
/// the timeline records it from the scripted-removal signal instead, and only
/// when the author claimed nothing else for that hull.
///
/// Both halves in one run, through the real script boundary: at t=8 the world
/// calls `destroy_entity` on the derelict (no authored outcome) and on Lyra
/// (declared `rescued` at t=4).
#[test]
fn a_scripted_removal_records_a_death_only_when_the_author_claimed_nothing_else() {
    let (report, _) = probe_run(ReportFormat::Json);

    assert_eq!(
        ids(&report, "marked_entity_destroyed"),
        vec!["world.probe_narrative.entity.wreck.name".to_string()],
        "the derelict the script removed must not vanish from the timeline \
         unremarked, and Lyra's rescue must not be re-read as a death: {:?}",
        report.narrative.counts_by_kind
    );

    // The death is the derelict's LAST word — it arrives after its spawn, and
    // after the rescue that spared Lyra the same fate.
    let spawn = report
        .narrative
        .events
        .iter()
        .find(|e| {
            e.event.kind.as_str() == "marked_entity_spawned"
                && e.event.id == "world.probe_narrative.entity.wreck.name"
        })
        .expect("the derelict is marked and spawns");
    let death = report
        .narrative
        .events
        .iter()
        .find(|e| e.event.kind.as_str() == "marked_entity_destroyed")
        .expect("asserted above");
    assert!(
        death.seq > spawn.seq && death.tick > spawn.tick,
        "a hull cannot die before it arrives: {spawn:?} / {death:?}"
    );

    // And the combat ledger did not move. `DamageLedger::death` is written from
    // `BalanceEvent::EntityDestroyed` and from nothing else, so two hulls
    // leaving the world with every ledger's `death` still `None` is the proof
    // that the narrative death above came from the scripted-removal signal —
    // and that a rescue-by-despawn cannot be counted as a destruction by
    // anything reading the balance stream.
    assert!(
        report
            .damage_by_ship
            .values()
            .all(|ledger| ledger.death.is_none()),
        "a scripted removal is not a kill and must leave the combat ledger \
         alone: {:?}",
        report.damage_by_ship
    );
}

/// Issue #1338's first acceptance criterion asks for posted/completed/failed as
/// DISTINCT recorded transitions. `post` adds `sound_off` and completes it in
/// the same handler — one fixed tick, one end-of-tick status, two beats — so
/// this is the end-to-end proof that a transition cannot be swallowed by
/// sharing a tick with the next one.
#[test]
fn an_objective_posted_and_completed_in_one_tick_records_both_transitions() {
    let (report, _) = probe_run(ReportFormat::Json);

    let sound_off: Vec<(&str, u64, u64)> = report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.id == "sound_off")
        .map(|e| (e.event.kind.as_str(), e.seq, e.tick))
        .collect();
    assert_eq!(
        sound_off.len(),
        2,
        "both transitions must be recorded: {:?}",
        report.narrative.events
    );
    assert_eq!(sound_off[0].0, "objective_posted");
    assert_eq!(sound_off[1].0, "objective_completed");
    assert!(
        sound_off[0].1 < sound_off[1].1,
        "the posting must sequence before the completion: {sound_off:?}"
    );
    assert_eq!(
        sound_off[0].2, sound_off[1].2,
        "both happened on the same fixed tick, which is the whole point: \
         {sound_off:?}"
    );
}

/// AC3, first half: two runs of the same seed produce equivalent timelines.
#[test]
fn identical_seeds_produce_equivalent_timelines() {
    let (first, _) = probe_run(ReportFormat::Json);
    let (second, _) = probe_run(ReportFormat::Json);
    assert!(
        !first.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );
    assert_eq!(
        first.narrative, second.narrative,
        "two runs with seed {SEED} produced different mission timelines"
    );
}

/// AC3, second half: turning the ndjson stream capture on — the only runtime
/// switch narrative recording has — changes neither the authoritative digest nor
/// the recorded timeline. The report differs in nothing that the simulation
/// decided.
#[test]
fn enabling_stream_capture_leaves_the_digest_and_the_timeline_identical() {
    let (json_report, json_digest) = probe_run(ReportFormat::Json);
    let (ndjson_report, ndjson_digest) = probe_run(ReportFormat::Ndjson);

    assert_eq!(
        json_digest, ndjson_digest,
        "enabling narrative/balance stream capture moved the authoritative-state \
         digest — capture must be a read-only projection off state the tick \
         already decided"
    );
    assert_eq!(
        json_report.narrative, ndjson_report.narrative,
        "the recorded timeline must not depend on the report format"
    );
    // Anti-vacuity for the equality above: the ndjson run really did capture a
    // stream, so the two runs were genuinely doing different amounts of work.
    assert!(
        !json_report.narrative.events.is_empty(),
        "an empty timeline would make this comparison vacuous"
    );

    // And so is every other line of the report — the outcome classification and
    // the combat ledgers included, which narrative recording must never touch.
    // `wall_seconds` is passed as 0.0 for both, so the only way these can differ
    // is if the simulation itself moved.
    assert_eq!(
        json_report.to_json(),
        ndjson_report.to_json(),
        "the run report must not depend on whether stream capture was on"
    );
}

/// The NDJSON half of AC2: the same beats appear inline in the stream, in the
/// shared `{"tick":..,"sim_t":..,"narrative":{..}}` envelope, in tick order
/// beside the balance events.
#[test]
fn the_ndjson_stream_carries_the_same_beats() {
    use project_phoenix::headless::report::RunTelemetry;

    let args = probe_args(ReportFormat::Ndjson);
    let mut app = build_headless_app(&args).expect("probe_narrative world should build");
    run(&mut app, args.max_ticks);
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
        "an ndjson run must carry the narrative beats inline"
    );
    for line in &lines {
        assert!(
            line.starts_with("{\"tick\":"),
            "every stream line uses the shared envelope: {line}"
        );
        assert!(
            line.contains("\"seq\":"),
            "and carries its sequence: {line}"
        );
    }
    assert!(
        lines.iter().any(
            |l| l.contains("\"kind\":\"beat_fired\"") && l.contains("\"id\":\"probe_opening\"")
        ),
        "the opening beat must appear in the stream:\n{}",
        lines.join("\n")
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("\"kind\":\"deadline_fired\"")),
        "the deadline must appear in the stream:\n{}",
        lines.join("\n")
    );

    // And the report built from the same run carries the same count, so the two
    // surfaces cannot drift.
    let report = build_report(&mut app, &args, 0.0);
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

    // Issue #1344's stream policy, proved on a real run: a re-scored report row
    // is a RATE, so `report_row_updated` is folded but never streamed, while the
    // finalized beat — which happens once — is both.
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("\"kind\":\"report_row_updated\"")),
        "a re-scored report row must stay out of the ndjson stream:\n{}",
        lines.join("\n")
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("\"kind\":\"report_finalized\"")),
        "the finalized beat happens once and belongs in the stream:\n{}",
        lines.join("\n")
    );
    assert_eq!(
        count(&report, "report_row_updated"),
        1,
        "the folded timeline still sees the row move: {:?}",
        report.narrative.counts_by_kind
    );
}

// ── The post-mission report, end to end (issue #1344) ──────────────────────

/// AC1 and AC4 on a whole seeded run: the probe world writes one row and then
/// declares a DEFEAT, and the report-bearing ending outranks it — the run
/// classifies `reported`, carries the row's String Ids in authored order, and
/// carries the hidden score and a total equal to the visible rows' scores.
#[test]
fn a_report_bearing_ending_classifies_as_reported_and_carries_its_rows() {
    use project_phoenix::core::balance::RunOutcome;
    use project_phoenix::core::report::ReportRowState;

    let (report, _) = probe_run(ReportFormat::Json);

    assert_eq!(
        report.outcome_report.outcome,
        RunOutcome::Reported,
        "a declared defeat that carries a report is reported, not a defeat"
    );

    let rows = report.outcome_report.report.rows();
    assert_eq!(rows.len(), 1, "one authored row: {rows:?}");
    assert_eq!(rows[0].id, "lyra");
    // String Ids, never prose — the same rule the timeline follows.
    assert_eq!(
        rows[0].heading_id,
        "world.probe_narrative.report.lyra.heading"
    );
    assert_eq!(
        rows[0].outcome_id,
        "world.probe_narrative.report.lyra.saved"
    );
    assert_eq!(rows[0].state, ReportRowState::Saved);

    // AC4: the signed score, and a total equal to the visible rows' scores.
    assert_eq!(rows[0].score, 6);
    assert_eq!(report.outcome_report.report.total(), 6);

    // The declared outcome is not lost — it stopped being the frame, which is
    // not the same thing. It is still latched for anything that wants it.
    let json = report.to_json();
    let parsed: serde_json::Value =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("run report is not valid JSON: {e}"));
    assert_eq!(parsed["outcome"], "reported");
    assert_eq!(parsed["report"]["rows"][0]["state"], "saved");
    assert_eq!(parsed["report"]["rows"][0]["score"], 6);
    assert_eq!(parsed["report"]["total"], 6);
    // The run really did reach the scripted ending — the reason string itself is
    // TAKEN by the broadcast (it is per-ending display text and the next round
    // must not inherit it), so the phase is what proves it here.
    assert_eq!(parsed["final_phase"], "GameOver");
}

/// The finalized beat names the ending it closed and carries the shape of the
/// report — the count and the hidden total — so an after-action reader can
/// check the rows it went on to read against what the run said it wrote.
#[test]
fn the_finalized_beat_names_the_ending_and_the_totals() {
    use project_phoenix::core::narrative::NarrativeValue;

    let (report, _) = probe_run(ReportFormat::Json);
    let finalized: Vec<_> = report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.kind.as_str() == "report_finalized")
        .collect();
    assert_eq!(finalized.len(), 1, "finalized fires exactly once");
    assert_eq!(
        finalized[0].event.id,
        "world.probe_narrative.game_over.closed"
    );
    assert_eq!(
        finalized[0].event.detail.get("rows"),
        Some(&NarrativeValue::Int(1))
    );
    assert_eq!(
        finalized[0].event.detail.get("total"),
        Some(&NarrativeValue::Int(6))
    );
    // It is the LAST thing the timeline says, because the mission is over.
    assert_eq!(
        report
            .narrative
            .events
            .last()
            .map(|e| e.event.kind.as_str()),
        Some("report_finalized")
    );
}
