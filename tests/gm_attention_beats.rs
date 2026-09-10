//! Eligible authored beats in the GM attention queue (issue #1434, PRD #1419
//! M4 story 7) over a real authored world.
//!
//! Every beat here is declared the ordinary way in
//! `assets/worlds/probe_gm_beats.toml`, every state change is made by an
//! ordinary `GmAction` through the ordinary journal, and every gate is opened
//! and closed by the world's own Rhai handlers. Nothing constructs a trigger
//! table by hand, and nothing writes a flag behind the world's back.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::command_admission::HostSlot;
use phoenix::gm_action::*;
use phoenix::gm_attention::*;
use phoenix::lockstep::{FleetRoster, FleetShip};
use project_phoenix as phoenix;

const WORLD: &str = "assets/worlds/probe_gm_beats.toml";

/// The base-world layer qualifies every beat this probe authors.
fn beat(id: &str) -> String {
    format!("base-world::{id}")
}

fn seeded() -> App {
    let args = phoenix::headless::HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1434),
        deterministic: true,
        max_ticks: 3000,
        ..Default::default()
    };
    let mut app = phoenix::headless::build_headless_app(&args).unwrap();
    app.insert_resource(FleetRoster::new(
        vec![FleetShip::new(HostSlot(1))],
        HostSlot(1),
    ));
    // The projection only runs on a peer that is actually presenting a GM desk.
    app.insert_resource(phoenix::gm_projection::BrowserGameMaster);
    app.add_plugins(GmAttentionPlugin);
    app.finish();
    app.cleanup();
    advance(&mut app, 90);
    app
}

fn advance(app: &mut App, ticks: usize) {
    for _ in 0..ticks {
        app.update();
    }
}

fn grant(sequence: u64, tick: u64, action: GmAction) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(4),
        sequenced_by: HostSlot(1),
        operator_id: "gm-beats".into(),
        correlation: GmActionId::new(format!("beat-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(4), sequence),
        action,
    }
}

/// Take one ordinary GM action through the ordinary journal, and let the world
/// settle so its handler (if any) has run.
fn act(app: &mut App, action: GmAction) {
    let tick = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    let sequence = app.world().resource::<GmActionJournal>().next_sequence();
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(sequence, tick, action))
        .unwrap();
    advance(app, 8);
}

fn fire(app: &mut App, id: &str) {
    act(app, GmAction::FireGmEvent { event: beat(id) });
}

fn queue(app: &mut App) -> Vec<GmAttentionOccurrence> {
    app.world_mut()
        .run_system_once(publish_attention_projection)
        .unwrap();
    app.world()
        .resource::<GmAttentionState>()
        .last()
        .cloned()
        .unwrap_or_default()
        .occurrences
}

/// The occurrence for one authored beat, whatever ordinal it is on.
fn row_for(rows: &[GmAttentionOccurrence], id: &str) -> Option<GmAttentionOccurrence> {
    let base = format!("{ELIGIBLE_BEAT_ID_PREFIX}{}", beat(id));
    rows.iter()
        .find(|row| row.id.starts_with(&base) && row.id[base.len()..].starts_with('#'))
        .cloned()
}

fn holds(rows: &[GmAttentionOccurrence], id: &str) -> bool {
    row_for(rows, id).is_some()
}

fn flag(app: &App, name: &str) -> i64 {
    app.world()
        .resource::<phoenix::world::server::WorldContentRuntime>()
        .flags
        .counter(name)
}

fn trigger_latches(app: &App) -> Vec<(bool, Option<f32>)> {
    app.world()
        .resource::<phoenix::world::server::WorldContentRuntime>()
        .triggers
        .iter()
        .map(|state| (state.fired, state.last_fired_elapsed))
        .collect()
}

/// AC 1: an eligible authored beat enters Attention by default, under a stable
/// qualified identity, with a reason and a link to the controls the author
/// declared.
#[test]
fn an_eligible_beat_enters_attention_with_its_qualified_identity_and_its_own_controls() {
    let mut app = seeded();
    let rows = queue(&mut app);

    let brief = row_for(&rows, "brief").expect("the manual beat is waiting to be started");
    assert_eq!(brief.category, GmAttentionCategory::EligibleBeat);
    // No authored band on this beat: the system default is Attention.
    assert_eq!(brief.band, GmAttentionBand::Attention);
    assert_eq!(brief.id, format!("event:{}#1", beat("brief")));
    // A manual beat has no other cause, and says so.
    assert_eq!(brief.reason.id, ELIGIBLE_BEAT_MANUAL_REASON);
    assert_eq!(
        brief.reason.params.get("beat").map(String::as_str),
        Some("world.probe_gm_beats.beat.brief")
    );
    let target = brief.target.event.as_ref().expect("a beat names its event");
    assert_eq!(target.id, beat("brief"));
    assert_eq!(target.label, "world.probe_gm_beats.beat.brief");
    assert_eq!(
        (target.fire, target.pause, target.skip),
        (true, false, false)
    );
    // A beat is not a conversation: it borrows none of the Comms fields.
    assert!(brief.target.route.is_none() && brief.target.conversation.is_none());

    // The automatic beat is the other sentence, and carries all three levers
    // exactly as its author declared them.
    let relief = row_for(&rows, "relief").expect("an automatic beat is also GM-operable");
    assert_eq!(relief.reason.id, ELIGIBLE_BEAT_REASON);
    assert_eq!(relief.band, GmAttentionBand::Background);
    let target = relief.target.event.as_ref().unwrap();
    assert_eq!((target.fire, target.pause, target.skip), (true, true, true));

    // Identity and first-seen tick survive an ordinary republish.
    let first_seen = brief.first_seen_tick;
    advance(&mut app, 30);
    let again = row_for(&queue(&mut app), "brief").unwrap();
    assert_eq!(again.id, brief.id);
    assert_eq!(again.first_seen_tick, first_seen);
}

/// AC 2: the authored override is validated among the three bands and reaches
/// the queue; an invented one fails the world load naming the vocabulary.
#[test]
fn authored_bands_reach_the_queue_and_an_invented_one_fails_the_load() {
    let mut app = seeded();
    let rows = queue(&mut app);
    assert_eq!(
        row_for(&rows, "recall").unwrap().band,
        GmAttentionBand::Urgent
    );
    assert_eq!(
        row_for(&rows, "relief").unwrap().band,
        GmAttentionBand::Background
    );
    assert_eq!(
        row_for(&rows, "brief").unwrap().band,
        GmAttentionBand::Attention
    );

    // And an invented band never reaches a live world: the authoring host fn
    // refuses it where the scenario is compiled, so activation is blocked
    // rather than the beat landing in a band nobody chose.
    let world = std::fs::read_to_string(WORLD).unwrap();
    assert!(
        findings_for(&world).is_empty(),
        "the shipped probe compiles"
    );
    let broken = world.replace(
        ".attention_band(\"urgent\")",
        ".attention_band(\"critical\")",
    );
    let findings = findings_for(&broken);
    assert!(
        findings.iter().any(|finding| {
            finding.message.contains("attention_band 'critical'")
                && finding.message.contains("'urgent'")
                && finding.message.contains("'background'")
        }),
        "{findings:?}"
    );
    assert!(phoenix::world::validate::has_error(&findings));
}

/// Compile one world's authored scripts exactly as the loader does, and hand
/// back what it found.
fn findings_for(world_toml: &str) -> Vec<phoenix::world::validate::WorldFinding> {
    let config = phoenix::world::config::parse_world(world_toml).expect("the TOML itself parses");
    let sources: Vec<vellum_script::ScriptSource> = config
        .script_sources
        .iter()
        .enumerate()
        .map(|(index, source)| vellum_script::ScriptSource {
            path: format!("{WORLD}#script.{index}"),
            source: source.clone(),
        })
        .collect();
    phoenix::world::script::load::compile_scripts(&sources).findings
}

/// AC 3 + AC 5: the ready-to-ineligible transition, in both directions, driven
/// by the world's own authored gate.
#[test]
fn a_gated_beat_appears_when_its_gate_opens_and_leaves_when_it_closes() {
    let mut app = seeded();
    assert!(
        !holds(&queue(&mut app), "storm"),
        "a beat whose `when` reads false is not waiting on anybody"
    );

    fire(&mut app, "open_gate");
    assert_eq!(flag(&app, "storm_ready"), 1);
    let ready = row_for(&queue(&mut app), "storm").expect("the gate opened, so the beat is ready");
    assert_eq!(ready.id, format!("event:{}#1", beat("storm")));

    fire(&mut app, "close_gate");
    assert_eq!(flag(&app, "storm_ready"), 0);
    assert!(
        !holds(&queue(&mut app), "storm"),
        "the gate closed, so the beat stops asking"
    );
    // Closing the gate is not firing the beat: nothing was spent, and the
    // handler never ran.
    assert_eq!(flag(&app, "stormed"), 0);

    // And it comes back as a genuinely NEW occurrence, so a snooze taken
    // against the first one cannot hide the second.
    fire(&mut app, "open_gate");
    let again = row_for(&queue(&mut app), "storm").expect("the gate opened again");
    assert_eq!(again.id, format!("event:{}#2", beat("storm")));
}

/// AC 3 + AC 5: a spent one-shot resolves for ever; a repeatable beat is
/// withheld while its Fire is armed and returns as a fresh occurrence.
#[test]
fn a_once_beat_completes_and_a_repeat_beat_recurs_with_a_fresh_identity() {
    let mut app = seeded();
    let once = row_for(&queue(&mut app), "brief").unwrap();
    let repeat = row_for(&queue(&mut app), "recall").unwrap();
    assert_eq!(repeat.id, format!("event:{}#1", beat("recall")));

    fire(&mut app, "brief");
    assert_eq!(flag(&app, "briefed"), 1);
    let rows = queue(&mut app);
    assert!(
        !holds(&rows, "brief"),
        "a spent one-shot is completed, not waiting: {rows:?}"
    );
    advance(&mut app, 60);
    assert!(!holds(&queue(&mut app), "brief"), "and it stays completed");
    assert!(once.id.ends_with("#1"));

    fire(&mut app, "recall");
    assert_eq!(flag(&app, "recalled"), 1);
    // The beat is repeatable, so it is ready again — as a second occurrence.
    let again = row_for(&queue(&mut app), "recall").expect("a repeatable beat comes back");
    assert_eq!(again.id, format!("event:{}#2", beat("recall")));
    assert_eq!(again.band, GmAttentionBand::Urgent);
    assert!(
        again.first_seen_tick >= repeat.first_seen_tick,
        "a fresh occurrence starts its own wait"
    );
}

/// AC 3: paused and skip-armed are standing GM decisions, so the queue stops
/// asking about them — and resuming brings the beat back.
#[test]
fn a_paused_or_skip_armed_beat_is_withheld_and_returns_when_the_decision_is_lifted() {
    let mut app = seeded();
    assert!(holds(&queue(&mut app), "relief"));

    act(
        &mut app,
        GmAction::SetEventPaused {
            event: beat("relief"),
            active: true,
        },
    );
    assert!(
        !holds(&queue(&mut app), "relief"),
        "a GM who paused this beat has already answered the question"
    );

    act(
        &mut app,
        GmAction::SetEventPaused {
            event: beat("relief"),
            active: false,
        },
    );
    let resumed = row_for(&queue(&mut app), "relief").expect("resuming brings the beat back");
    assert_eq!(resumed.id, format!("event:{}#2", beat("relief")));

    act(
        &mut app,
        GmAction::ArmGmEventSkip {
            event: beat("relief"),
        },
    );
    assert!(
        !holds(&queue(&mut app), "relief"),
        "a GM who armed a Skip has decided how the next occurrence goes"
    );
}

/// AC 4 + AC 5: reading the queue is not a way of running the world. Publishing
/// it repeatedly fires nothing, runs no handler, moves no latch or cooldown
/// clock, and changes nothing the authoritative digest folds.
#[test]
fn inspecting_eligibility_never_fires_a_beat_or_touches_authoritative_state() {
    let mut app = seeded();
    fire(&mut app, "open_gate");
    let flags_before = ["briefed", "recalled", "stormed", "relieved"]
        .map(|name| flag(&app, name))
        .to_vec();
    assert_eq!(flags_before, vec![0, 0, 0, 0]);
    let latches_before = trigger_latches(&app);
    let digest_before = phoenix::sim_digest::world_digest(app.world());

    for _ in 0..25 {
        let rows = queue(&mut app);
        assert!(holds(&rows, "brief") && holds(&rows, "storm") && holds(&rows, "relief"));
    }

    assert_eq!(
        ["briefed", "recalled", "stormed", "relieved"]
            .map(|name| flag(&app, name))
            .to_vec(),
        flags_before,
        "no handler ran while the queue was inspected"
    );
    assert_eq!(
        trigger_latches(&app),
        latches_before,
        "no `fired` latch or cooldown clock moved"
    );
    assert_eq!(
        phoenix::sim_digest::world_digest(app.world()),
        digest_before,
        "the attention queue is not an authoritative input"
    );
    // The journal recorded exactly the one Fire this test asked for.
    assert_eq!(
        app.world()
            .resource::<GmActionJournal>()
            .applied_log()
            .entries()
            .iter()
            .filter(|logged| logged.action_kind == GmActionKind::EventControl)
            .count(),
        1
    );
}

/// AC 5: the beats sit in one list with the pending-Comms rows, oldest first
/// with a stable-id tie-break, and the list stays stable while nothing changes.
#[test]
fn the_queue_is_one_ordered_list_and_stays_stable_between_changes() {
    let mut app = seeded();
    let first = queue(&mut app);
    // Five of the six authored beats: `storm` sits behind a gate nobody opened.
    assert_eq!(first.len(), 5, "{first:?}");
    assert!(!holds(&first, "storm"));
    for pair in first.windows(2) {
        assert!(
            (pair[0].first_seen_tick, &pair[0].id) < (pair[1].first_seen_tick, &pair[1].id),
            "{pair:?}"
        );
    }
    advance(&mut app, 45);
    let second = queue(&mut app);
    assert_eq!(
        first.iter().map(|row| &row.id).collect::<Vec<_>>(),
        second.iter().map(|row| &row.id).collect::<Vec<_>>(),
        "nothing changed, so nothing moved"
    );
}
