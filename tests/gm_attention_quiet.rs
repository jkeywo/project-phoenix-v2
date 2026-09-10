//! The quiet-time advisory over the real activity sources (issue #1436).
//!
//! Every "activity" here is an ordinary one: a Station puppet issuing an
//! authentic Station command through admission, the crew answering a scripted
//! hail through the ordinary Comms path, a GM completing an authored objective
//! through the ordinary objective reducer. Nothing sets `GmCrewActivity`
//! directly except where the test is deliberately contrasting the clock's own
//! arithmetic, and nothing builds an attention row by hand.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::command_admission::{log::ShipKey, HostSlot};
use phoenix::comms::server::CommsInboxRes;
use phoenix::core::messages::{
    AdmittedCommands, CommsMessage, StationId, SystemControlPayload, SystemId,
};
use phoenix::entities::spawner::{EntityName, EntityUuid};
use phoenix::gm_action::*;
use phoenix::gm_attention::*;
use phoenix::gm_comms::{GmCommsContent, GmCommsTransmission};
use phoenix::gm_quiet::*;
use phoenix::lockstep::{FleetRoster, FleetShip, FleetSlotOf};
use project_phoenix as phoenix;

const WORLD: &str = "assets/worlds/probe_gm_quiet.toml";

/// The world's authored interval, in simulation seconds, and at its 60 Hz tick
/// rate the number of quiet ticks that reaches it.
const QUIET_SECONDS: f32 = 2.0;
const QUIET_TICKS: u64 = 120;

fn args_for(world: &str) -> phoenix::headless::HeadlessArgs {
    phoenix::headless::HeadlessArgs {
        world_path: world.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1436),
        deterministic: true,
        max_ticks: 2000,
        ..Default::default()
    }
}

fn seeded() -> App {
    let mut app = phoenix::headless::build_headless_app(&args_for(WORLD)).unwrap();
    app.insert_resource(FleetRoster::new(
        vec![FleetShip::new(HostSlot(1))],
        HostSlot(1),
    ));
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

fn tick(app: &App) -> u64 {
    app.world().resource::<phoenix::sim_tick::SimTick>().0
}

fn ship(app: &mut App) -> String {
    let mut query = app.world_mut().query::<(&EntityUuid, &FleetSlotOf)>();
    query.iter(app.world()).next().unwrap().0 .0.clone()
}

fn speaker(app: &mut App, name: &str) -> String {
    let mut query = app.world_mut().query::<(&EntityUuid, &EntityName)>();
    query
        .iter(app.world())
        .find(|(_, entity)| entity.0 == name)
        .unwrap_or_else(|| panic!("world authors speaker {name}"))
        .0
         .0
        .clone()
}

fn grant(sequence: u64, apply_tick: u64, action: GmAction) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(4),
        sequenced_by: HostSlot(1),
        operator_id: "gm-quiet".into(),
        correlation: GmActionId::new(format!("quiet-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(HostSlot(4), sequence),
        action,
    }
}

fn enqueue(app: &mut App, action: GmAction) {
    let at = tick(app);
    let sequence = app.world().resource::<GmActionJournal>().next_sequence();
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(sequence, at, action))
        .unwrap();
}

fn station_for(app: &mut App, ship_uuid: &str, system: &SystemId) -> StationId {
    let mut query = app.world_mut().query::<(
        &EntityUuid,
        &phoenix::ship::components::ShipConfigComponent,
        Option<&phoenix::ship_plugin::HumanSeekingHosts>,
    )>();
    let (_, config, hosts) = query
        .iter(app.world())
        .find(|(uuid, _, _)| uuid.0 == ship_uuid)
        .unwrap();
    phoenix::command_admission::station_for_system(&config.0, hosts, system).unwrap()
}

/// Put a human on one Station, the ordinary way.
fn puppet(app: &mut App, station: &StationId) {
    let ship_uuid = ship(app);
    enqueue(
        app,
        GmAction::SetStationPuppet {
            ship: ShipKey(ship_uuid),
            station: station.clone(),
            active: true,
        },
    );
    advance(app, 4);
}

/// Issue one authentic Station command through the ordinary admission path.
fn operate(app: &mut App, station: &StationId, target: SystemId, payload: SystemControlPayload) {
    let ship_uuid = ship(app);
    enqueue(
        app,
        GmAction::IssueStationCommand {
            ship: ShipKey(ship_uuid),
            station: station.clone(),
            target,
            payload: phoenix::core::codec::canonical_system_command(&payload).unwrap(),
        },
    );
    advance(app, 6);
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

fn quiet_rows(app: &mut App) -> Vec<GmAttentionOccurrence> {
    queue(app)
        .into_iter()
        .filter(|row| row.category == GmAttentionCategory::QuietTime)
        .collect()
}

/// Run the simulation until the authored interval has passed with nothing
/// happening, then read the queue.
fn go_quiet(app: &mut App) -> Vec<GmAttentionOccurrence> {
    let from = app
        .world()
        .resource::<GmCrewActivity>()
        .last_activity_tick();
    while tick(app) < from + QUIET_TICKS + 2 {
        app.update();
    }
    quiet_rows(app)
}

fn messages(app: &App) -> Vec<CommsMessage> {
    app.world().resource::<CommsInboxRes>().0.messages()
}

#[test]
fn a_quiet_session_gets_exactly_one_background_row_that_explains_the_interval() {
    let mut app = seeded();
    let rows = go_quiet(&mut app);
    assert_eq!(rows.len(), 1, "one row, not one per source: {rows:?}");
    let row = &rows[0];
    assert_eq!(row.band, GmAttentionBand::Background);
    assert_eq!(row.reason.id, QUIET_REASON);
    // The sentence names the INTERVAL and nothing else. No operator, no
    // Station, no keypress count, nothing inferred about anybody.
    assert_eq!(
        row.reason.params.get("seconds").map(String::as_str),
        Some("2"),
    );
    assert_eq!(row.reason.params.len(), 1, "{:?}", row.reason.params);
    // Nowhere to go: a lull is the session's, not a ship's or a conversation's.
    assert_eq!(row.target, GmAttentionTarget::default());
    assert!(row.id.starts_with(QUIET_ID_PREFIX), "{}", row.id);

    // The wire spelling the page filters and styles on, through the real
    // encoder the Host Channel uses — an empty `target` and all.
    let encoded = phoenix::core::codec::encode_gm_attention_projection(
        &phoenix::gm_attention::GmAttentionProjection {
            occurrences: rows.clone(),
        },
    )
    .expect("the projection encodes");
    assert!(encoded.contains("\"category\":\"quiet_time\""), "{encoded}");
    assert!(encoded.contains("\"target\":{}"), "{encoded}");
}

#[test]
fn age_alone_never_escalates_the_row() {
    let mut app = seeded();
    let first = go_quiet(&mut app);
    assert_eq!(first[0].band, GmAttentionBand::Background);
    let opened_on = first[0].first_seen_tick;
    let id = first[0].id.clone();

    // Ten times the interval later it is the same row, in the same band. The
    // desk may have aged it; the queue has not promoted it.
    for _ in 0..QUIET_TICKS * 10 {
        app.update();
    }
    let later = quiet_rows(&mut app);
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].band, GmAttentionBand::Background);
    assert_eq!(later[0].id, id, "a lull that never ended is one occurrence");
    assert_eq!(later[0].first_seen_tick, opened_on);
}

#[test]
fn a_completed_comms_response_resolves_the_row_and_restarts_the_timing() {
    let mut app = seeded();
    assert_eq!(go_quiet(&mut app).len(), 1);
    let first_id = quiet_rows(&mut app)[0].id.clone();

    // A real hail, answered by the ship's own Comms Station through the
    // ordinary admitted `RespondToMessage` the console sends.
    let sender = speaker(&mut app, "world.probe_gm_quiet.speaker");
    let recipient = ship(&mut app);
    enqueue(
        &mut app,
        GmAction::TransmitComms {
            transmission: GmCommsTransmission {
                sender,
                route: "quiet-route".into(),
                recipients: vec![ShipKey(recipient.clone())],
                content: GmCommsContent::ScriptedHail {
                    hail: "quiet-hail".into(),
                },
            },
        },
    );
    advance(&mut app, 6);
    let delivered = messages(&app)
        .into_iter()
        .find(|m| m.body == "world.probe_gm_quiet.offer")
        .expect("the world delivers its offer");

    let comms = phoenix::ship::system_registry::comms_system_id();
    let station = station_for(&mut app, &recipient, &comms);
    puppet(&mut app, &station);
    operate(
        &mut app,
        &station,
        comms,
        SystemControlPayload::RespondToMessage {
            message_id: delivered.id.clone(),
            response_index: 0,
        },
    );

    // The answer landed, so the lull ended.
    assert!(
        quiet_rows(&mut app).is_empty(),
        "answering the crew's own Comms is meaningful activity",
    );
    // ... and the next lull is a DIFFERENT occurrence, so a snooze taken
    // against the first cannot hide it.
    let second = go_quiet(&mut app);
    assert_eq!(second.len(), 1);
    assert_ne!(second[0].id, first_id);
}

#[test]
fn objective_progress_resolves_the_row() {
    let mut app = seeded();
    assert_eq!(go_quiet(&mut app).len(), 1);

    // The palette entry names no recipients, so the whole fleet is its scope
    // and the request carries the same (empty) vocabulary.
    enqueue(
        &mut app,
        GmAction::ObjectiveAction {
            objective: "gm-quiet-check".into(),
            verb: phoenix::gm_objective::ObjectiveVerb::Activate,
            recipients: vec![],
        },
    );
    advance(&mut app, 8);
    assert!(
        quiet_rows(&mut app).is_empty(),
        "an objective actually entering a lifecycle state is progress",
    );
}

#[test]
fn a_no_op_press_the_system_refused_is_not_activity() {
    let mut app = seeded();
    assert_eq!(go_quiet(&mut app).len(), 1);
    let id = quiet_rows(&mut app)[0].id.clone();

    // Answering a message id the inbox does not hold: an authentic Station
    // command, correctly admitted, that the Comms System then refuses. Nothing
    // changed, so the session is still quiet.
    let recipient = ship(&mut app);
    let comms = phoenix::ship::system_registry::comms_system_id();
    let station = station_for(&mut app, &recipient, &comms);
    puppet(&mut app, &station);
    operate(
        &mut app,
        &station,
        comms,
        SystemControlPayload::RespondToMessage {
            message_id: "no-such-message".into(),
            response_index: 0,
        },
    );

    let rows = quiet_rows(&mut app);
    assert_eq!(rows.len(), 1, "a refused press is not effective control");
    assert_eq!(rows[0].id, id, "and it did not restart the occurrence");
}

#[test]
fn ai_noise_never_resets_the_clock_but_a_sustained_human_control_does() {
    let mut app = seeded();
    app.init_resource::<AiProbe>();
    app.add_systems(
        FixedUpdate,
        emit_ai_steering.after(phoenix::command_admission::AdmissionSet),
    );

    // The ship's own AI works the helm for a full interval and a half, through
    // the real `emit_ai_command` seam, into the real `AdmittedCommands` channel
    // the adapter reads. The session is still quiet: an AI flying the ship is
    // exactly the lull a Game Master wants told about.
    app.world_mut().resource_mut::<AiProbe>().commands = QUIET_TICKS as usize * 2;
    let rows = go_quiet(&mut app);
    assert!(
        app.world().resource::<AiProbe>().admitted > 0,
        "the AI probe must actually be admitting commands",
    );
    assert_eq!(rows.len(), 1, "AI traffic is not crew activity: {rows:?}");
    let during_ai = rows[0].id.clone();

    // Now a human holds the throttle over from the Helm Station. One sustained
    // control, no semantic action, no terminal result to settle — and the lull
    // ends.
    let hull = ship(&mut app);
    let thrust = SystemId(phoenix::ship::system_registry::HELM_THRUST_SYSTEM_ID.into());
    let station = station_for(&mut app, &hull, &thrust);
    puppet(&mut app, &station);
    operate(
        &mut app,
        &station,
        thrust.clone(),
        SystemControlPayload::SetThrust { value: 0.6 },
    );
    assert!(
        quiet_rows(&mut app).is_empty(),
        "a hand on the throttle is effective control",
    );

    // A throttle RETURNED to neutral is the rest position, not somebody
    // working: it must not hold the next lull off.
    operate(
        &mut app,
        &station,
        thrust,
        SystemControlPayload::SetThrust { value: 0.0 },
    );
    let next = go_quiet(&mut app);
    assert_eq!(next.len(), 1);
    assert_ne!(
        next[0].id, during_ai,
        "a second lull is a second occurrence"
    );
}

/// How many more ticks the fixture's AI should work the helm for, and how many
/// commands it actually got admitted.
#[derive(Resource, Default)]
struct AiProbe {
    commands: usize,
    admitted: usize,
}

/// Emit one AI helm command per tick through the production AI seam.
///
/// `.after(AdmissionSet)` for the reason every AI decide system is: admission
/// clears and refills `AdmittedCommands` at the top of each tick, so anything
/// pushed before it is wiped.
fn emit_ai_steering(
    sessions: Res<phoenix::lobby::Sessions>,
    mut ships: Query<
        (
            Option<&EntityUuid>,
            &phoenix::ship::components::ShipSystemControlSources,
            Option<&phoenix::ship::components::ShipConfigComponent>,
            &mut AdmittedCommands,
        ),
        With<FleetSlotOf>,
    >,
    mut probe: ResMut<AiProbe>,
) {
    if probe.commands == 0 {
        return;
    }
    probe.commands -= 1;
    for (uuid, sources, config, mut admitted) in ships.iter_mut() {
        if phoenix::command_admission::ai_emit::emit_ai_command(
            uuid,
            SystemId(phoenix::ship::system_registry::HELM_THRUST_SYSTEM_ID.into()),
            SystemControlPayload::SetThrust { value: 0.9 },
            sources,
            &sessions,
            config,
            &mut admitted,
        ) {
            probe.admitted += 1;
        }
    }
}

#[test]
fn pause_freezes_the_quiet_clock_because_it_freezes_the_simulation() {
    let mut app = seeded();
    // Pause well short of the interval.
    app.world_mut()
        .insert_resource(phoenix::gm_action::SimulationPaused(true));
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    let paused_at = tick(&app);

    // Many frames of real time pass. The fixed schedule is starved, so the
    // simulation clock the advisory measures does not move.
    for _ in 0..(QUIET_TICKS as usize * 3) {
        app.update();
    }
    assert_eq!(tick(&app), paused_at, "a paused sim takes no fixed steps");
    assert!(
        quiet_rows(&mut app).is_empty(),
        "a paused session cannot age into a lull",
    );

    // Resume, and the same lull it was in the middle of completes normally.
    app.world_mut()
        .insert_resource(phoenix::gm_action::SimulationPaused(false));
    app.world_mut().resource_mut::<Time<Virtual>>().unpause();
    assert_eq!(go_quiet(&mut app).len(), 1);
}

#[test]
fn the_interval_is_an_edge_not_a_window() {
    let mut app = seeded();
    let from = app
        .world()
        .resource::<GmCrewActivity>()
        .last_activity_tick();
    let hz = app
        .world()
        .resource::<phoenix::world::config::WorldConfig>()
        .global
        .sim_tick_hz;
    assert_eq!(hz, 60.0, "the probe world runs at the default rate");

    // One tick short of the authored interval: still working hours.
    while tick(&app) < from + QUIET_TICKS - 1 {
        app.update();
    }
    assert!(
        quiet_rows(&mut app).is_empty(),
        "{} of {QUIET_TICKS} ticks quiet",
        tick(&app) - from,
    );
    while tick(&app) < from + QUIET_TICKS {
        app.update();
    }
    assert_eq!(quiet_rows(&mut app).len(), 1, "the interval has elapsed");
}

#[test]
fn a_world_that_switches_the_advisory_off_never_publishes_it() {
    let mut app = seeded();
    // The same world, with the independent disable set: the interval is
    // untouched and unreachable.
    app.world_mut()
        .resource_mut::<phoenix::world::config::WorldConfig>()
        .gm_attention
        .quiet_time_disabled = true;
    for _ in 0..QUIET_TICKS * 4 {
        app.update();
    }
    assert!(quiet_rows(&mut app).is_empty());
    assert_eq!(
        app.world()
            .resource::<phoenix::world::config::WorldConfig>()
            .gm_attention
            .quiet_time_secs,
        QUIET_SECONDS,
        "the duration override is a separate field and is still there",
    );
}

#[test]
fn a_restored_snapshot_starts_its_quiet_clock_at_the_restore_point() {
    let mut app = seeded();
    assert_eq!(go_quiet(&mut app).len(), 1);
    let before = quiet_rows(&mut app)[0].id.clone();

    // The restore hook is what a snapshot/join calls once `SimTick` has been
    // moved to the captured one.
    let restored_tick = tick(&app) + 5_000;
    app.world_mut()
        .insert_resource(phoenix::sim_tick::SimTick(restored_tick));
    phoenix::gm_quiet::rebase_after_restore(app.world_mut());

    // The continuation does not open on a five-thousand-tick lull it never had.
    assert!(
        quiet_rows(&mut app).is_empty(),
        "the clock rebased onto the restored tick",
    );
    assert_eq!(
        app.world()
            .resource::<GmCrewActivity>()
            .last_activity_tick(),
        restored_tick,
    );

    // And when the restored session does fall quiet, that is a new occurrence.
    let after = go_quiet(&mut app);
    assert_eq!(after.len(), 1);
    assert_ne!(after[0].id, before);
}

#[test]
fn the_advisory_never_reaches_the_authoritative_state() {
    let mut app = seeded();
    assert_eq!(go_quiet(&mut app).len(), 1);
    // A quiet row is a peer-local reading of state the simulation already owns.
    // Publishing it takes no GM action, writes no journal entry and changes
    // nothing the digest folds.
    let journal_before = app.world().resource::<GmActionJournal>().len();
    let digest_before = phoenix::sim_digest::world_digest(app.world());
    for _ in 0..30 {
        queue(&mut app);
    }
    assert_eq!(
        app.world().resource::<GmActionJournal>().len(),
        journal_before
    );
    assert_eq!(
        phoenix::sim_digest::world_digest(app.world()),
        digest_before,
    );
}

// ── Authored settings ────────────────────────────────────────────────────────

/// The interval and the disable are two independent fields on the SHARED
/// `[gm_attention]` table, and an unusable interval is refused at load rather
/// than becoming an advisory that fires every tick or never.
#[test]
fn the_authored_interval_and_its_independent_disable_load_or_fail_at_the_world_file() {
    use phoenix::world::config::parse_world;

    let defaulted = parse_world(
        "[global]
seed = 7
",
    )
    .expect("TOML should parse");
    assert_eq!(
        defaulted.gm_attention.quiet_time_secs,
        phoenix::gm_quiet::DEFAULT_QUIET_SECONDS,
    );
    assert!(!defaulted.gm_attention.quiet_time_disabled);
    assert_eq!(defaulted.gm_attention.quiet_time_ticks(60.0), 7_200);

    let authored = parse_world(
        "[gm_attention]
quiet_time_secs = 90.0
",
    )
    .expect("a positive interval is an override, not a disable");
    assert_eq!(authored.gm_attention.quiet_time_secs, 90.0);
    assert!(
        !authored.gm_attention.quiet_time_disabled,
        "changing the interval must not switch the advisory off",
    );
    // Nor the other way round: the interval survives the disable, so switching
    // the advisory back on keeps the value the author chose.
    let off = parse_world(
        "[gm_attention]
quiet_time_secs = 90.0
quiet_time_disabled = true
",
    )
    .expect("both may be authored together");
    assert_eq!(off.gm_attention.quiet_time_secs, 90.0);
    assert!(off.gm_attention.quiet_time_disabled);
    // And neither field disturbs the queue's other advisory.
    assert_eq!(
        off.gm_attention.idle_npc_grace_secs,
        phoenix::gm_attention::DEFAULT_IDLE_NPC_GRACE_SECS,
    );

    for value in ["0.0", "-1.0", "nan", "inf"] {
        let source = format!(
            "[gm_attention]
quiet_time_secs = {value}
"
        );
        let error = parse_world(&source).expect_err("the interval must be positive and finite");
        assert!(
            error.contains("[gm_attention]")
                && error.contains("quiet_time_secs")
                && error.contains("quiet_time_disabled"),
            "the load error must name the table, the field and the disable for {value}; got:              {error}",
        );
    }
}
