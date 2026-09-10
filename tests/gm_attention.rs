//! The GM attention queue over real authored Comms routes and real delivery.
//!
//! Every occurrence here starts life as an ordinary scripted hail delivered
//! through the ordinary Comms path (`GmAction::TransmitComms` → dialogue →
//! inbox), and every resolution is an ordinary crew action taken through a
//! Station puppet. Nothing constructs an inbox by hand.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::command_admission::{log::ShipKey, HostSlot};
use phoenix::comms::server::CommsInboxRes;
use phoenix::core::messages::{CommsMessage, StationId, SystemControlPayload};
use phoenix::entities::spawner::{EntityName, EntityUuid};
use phoenix::gm_action::*;
use phoenix::gm_attention::*;
use phoenix::gm_comms::{GmCommsContent, GmCommsTransmission};
use phoenix::lockstep::{FleetRoster, FleetShip, FleetSlotOf};
use project_phoenix as phoenix;

const WORLD: &str = "assets/worlds/probe_gm_attention.toml";

/// A world whose conversation is opened by its own script rather than by a GM,
/// so the delivered message addresses no single hull.
const FLEET_WORLD: &str = "assets/worlds/probe_gm_attention_fleet.toml";

fn args_for(world: &str) -> phoenix::headless::HeadlessArgs {
    phoenix::headless::HeadlessArgs {
        world_path: world.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1433),
        deterministic: true,
        max_ticks: 600,
        ..Default::default()
    }
}

fn seeded() -> App {
    seeded_in(WORLD)
}

fn seeded_in(world: &str) -> App {
    let mut app = phoenix::headless::build_headless_app(&args_for(world)).unwrap();
    app.insert_resource(FleetRoster::new(
        vec![FleetShip::new(HostSlot(1))],
        HostSlot(1),
    ));
    // The projection only runs on a peer that is actually presenting a GM
    // desk, exactly like the activity feed.
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

fn grant(sequence: u64, tick: u64, action: GmAction) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(4),
        sequenced_by: HostSlot(1),
        operator_id: "gm-attention".into(),
        correlation: GmActionId::new(format!("attention-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(4), sequence),
        action,
    }
}

fn enqueue(app: &mut App, action: GmAction) {
    let tick = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    let sequence = app.world().resource::<GmActionJournal>().next_sequence();
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(sequence, tick, action))
        .unwrap();
}

/// Open one authored conversation the ordinary way: a GM starts the route's
/// scripted hail toward the fleet ship.
fn open_hail(app: &mut App, speaker_name: &str, route: &str, hail: &str) {
    let sender = speaker(app, speaker_name);
    let recipient = ship(app);
    enqueue(
        app,
        GmAction::TransmitComms {
            transmission: GmCommsTransmission {
                sender,
                route: route.into(),
                recipients: vec![ShipKey(recipient)],
                content: GmCommsContent::ScriptedHail { hail: hail.into() },
            },
        },
    );
    advance(app, 6);
}

fn comms_station(app: &mut App, ship_uuid: &str) -> StationId {
    let mut query = app.world_mut().query::<(
        &EntityUuid,
        &phoenix::ship::components::ShipConfigComponent,
        Option<&phoenix::ship_plugin::HumanSeekingHosts>,
    )>();
    let (_, config, hosts) = query
        .iter(app.world())
        .find(|(uuid, _, _)| uuid.0 == ship_uuid)
        .unwrap();
    phoenix::command_admission::station_for_system(
        &config.0,
        hosts,
        &phoenix::ship::system_registry::comms_system_id(),
    )
    .unwrap()
}

/// Answer one message the ordinary way: puppet the ship's Comms Station and
/// submit the crew's own response command.
fn answer(app: &mut App, message_id: &str) {
    let ship_uuid = ship(app);
    let station = comms_station(app, &ship_uuid);
    enqueue(
        app,
        GmAction::SetStationPuppet {
            ship: ShipKey(ship_uuid.clone()),
            station: station.clone(),
            active: true,
        },
    );
    advance(app, 3);
    enqueue(
        app,
        GmAction::IssueStationCommand {
            ship: ShipKey(ship_uuid),
            station,
            target: phoenix::ship::system_registry::comms_system_id(),
            payload: phoenix::core::codec::canonical_system_command(
                &SystemControlPayload::RespondToMessage {
                    message_id: message_id.into(),
                    response_index: 0,
                },
            )
            .unwrap(),
        },
    );
    advance(app, 6);
}

fn messages(app: &App) -> Vec<CommsMessage> {
    app.world().resource::<CommsInboxRes>().0.messages()
}

fn body_of(app: &App, body: &str) -> CommsMessage {
    messages(app)
        .into_iter()
        .find(|m| m.body == body)
        .unwrap_or_else(|| panic!("world delivers {body}"))
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

#[test]
fn a_real_unresolved_hail_becomes_an_attention_occurrence_and_recurrence_gets_a_fresh_identity() {
    let mut app = seeded();
    assert!(queue(&mut app).is_empty(), "nothing waits before a hail");

    open_hail(
        &mut app,
        "world.probe_gm_attention.speaker_default",
        "default-route",
        "default-hail",
    );
    let delivered = body_of(&app, "world.probe_gm_attention.offer_default");
    let rows = queue(&mut app);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    // Identity is the world's own minted message id, so a browser can hold a
    // snooze or a focus ring against it for the life of the condition.
    assert_eq!(row.id, format!("comms:{}", delivered.id));
    assert_eq!(row.category, GmAttentionCategory::PendingComms);
    // No authored band on this route: the system default is Attention.
    assert_eq!(row.band, GmAttentionBand::Attention);
    assert_eq!(row.reason.id, PENDING_COMMS_REASON);
    assert_eq!(
        row.reason.params.get("sender").map(String::as_str),
        Some("world.probe_gm_attention.speaker_default")
    );
    // Opening the row lands on the authored route that already speaks as this
    // sender, not on a new action surface.
    assert_eq!(row.target.route.as_deref(), Some("default-route"));
    assert_eq!(
        row.target.conversation.as_deref(),
        Some(delivered.thread_id.as_str())
    );
    assert_eq!(
        row.target.ship.as_ref().map(|ship| ship.entity_id.as_str()),
        Some(ship(&mut app).as_str())
    );

    // The identity, first-seen tick and ordering survive an ordinary republish.
    let first_seen = row.first_seen_tick;
    advance(&mut app, 30);
    let again = queue(&mut app);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].id, rows[0].id);
    assert_eq!(again[0].first_seen_tick, first_seen);

    // Answering it is a resolution: the row disappears, and the follow-up node
    // this world authors has no responses of its own, so it demands nothing.
    answer(&mut app, &delivered.id);
    let after = queue(&mut app);
    assert!(
        after
            .iter()
            .all(|row| row.id != format!("comms:{}", delivered.id)),
        "an answered conversation stops waiting: {after:?}"
    );

    // Recurrence: the same speaker, the same route, a second hail. A different
    // message, therefore a different occurrence — no stale snooze can hide it.
    open_hail(
        &mut app,
        "world.probe_gm_attention.speaker_default",
        "default-route",
        "default-hail",
    );
    let repeat = messages(&app)
        .into_iter()
        .filter(|m| m.body == "world.probe_gm_attention.offer_default")
        .find(|m| m.id != delivered.id)
        .expect("the second hail delivers its own message");
    let rows = queue(&mut app);
    assert!(rows
        .iter()
        .any(|row| row.id == format!("comms:{}", repeat.id)));
    assert!(rows
        .iter()
        .all(|row| row.id != format!("comms:{}", delivered.id)));
}

/// The ordinary scripted case: a world opens its own conversation, so nothing
/// addresses it at one hull. The row still has to explain itself — with the
/// fleet-wide sentence and no blank parameter, not the addressed sentence
/// wrapped around an empty ship name.
#[test]
fn a_world_authored_hail_addressed_at_no_hull_reads_as_fleet_wide() {
    let mut app = seeded_in(FLEET_WORLD);
    let delivered = body_of(&app, "world.probe_gm_attention.offer_fleet");
    assert!(
        delivered.recipient_ship.is_none(),
        "a world-authored open_comms addresses the fleet, not a hull"
    );

    let rows = queue(&mut app);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.id, format!("comms:{}", delivered.id));
    assert_eq!(row.category, GmAttentionCategory::PendingComms);
    assert_eq!(row.band, GmAttentionBand::Attention);
    // The fleet-wide sentence, whose only parameter is the speaker.
    assert_eq!(row.reason.id, PENDING_COMMS_FLEET_REASON);
    assert_eq!(
        row.reason.params.get("sender").map(String::as_str),
        Some("world.probe_gm_attention.speaker_fleet")
    );
    assert!(
        row.reason.params.get("ship").is_none(),
        "no ship parameter at all: {:?}",
        row.reason.params
    );
    assert!(
        row.reason.params.values().all(|value| !value.is_empty()),
        "no parameter renders blank: {:?}",
        row.reason.params
    );
    assert!(row.target.ship.is_none());
    // Unaddressed is not unroutable: opening the row still lands on the
    // authored route this speaker already speaks through.
    assert_eq!(row.target.route.as_deref(), Some("fleet-route"));
    assert_eq!(
        row.target.conversation.as_deref(),
        Some(delivered.thread_id.as_str())
    );

    // And it resolves like any other: the crew answers, the row stops waiting.
    answer(&mut app, &delivered.id);
    assert!(
        queue(&mut app).is_empty(),
        "an answered conversation stops waiting"
    );
}

#[test]
fn a_withdrawn_conversation_stops_waiting() {
    let mut app = seeded();
    open_hail(
        &mut app,
        "world.probe_gm_attention.speaker_urgent",
        "urgent-route",
        "urgent-hail",
    );
    let delivered = body_of(&app, "world.probe_gm_attention.offer_urgent");
    assert_eq!(queue(&mut app).len(), 1);

    // The sender leaving is the world's own withdrawal path (a world-layer
    // unload takes it too): history is preserved, the live demand is not.
    let sender = delivered.sender_uuid.clone();
    app.world_mut()
        .resource_mut::<CommsInboxRes>()
        .0
        .orphan_sender(&sender);
    assert!(queue(&mut app).is_empty());
}

#[test]
fn authored_bands_load_and_the_queue_is_oldest_first_with_a_stable_id_tie_break() {
    let mut app = seeded();
    // Deliberately opened in reverse band order, so band grouping cannot be
    // mistaken for arrival order and arrival order cannot be mistaken for band.
    open_hail(
        &mut app,
        "world.probe_gm_attention.speaker_background",
        "background-route",
        "background-hail",
    );
    advance(&mut app, 10);
    open_hail(
        &mut app,
        "world.probe_gm_attention.speaker_default",
        "default-route",
        "default-hail",
    );
    advance(&mut app, 10);
    open_hail(
        &mut app,
        "world.probe_gm_attention.speaker_urgent",
        "urgent-route",
        "urgent-hail",
    );
    let rows = queue(&mut app);
    assert_eq!(rows.len(), 3, "{rows:?}");
    let bands: Vec<_> = rows.iter().map(|row| row.band).collect();
    assert_eq!(
        bands,
        vec![
            GmAttentionBand::Background,
            GmAttentionBand::Attention,
            GmAttentionBand::Urgent
        ],
        "the authored overrides load: background first because it arrived first"
    );
    // Oldest first across the whole queue, stable-id tie-break inside a tick.
    for pair in rows.windows(2) {
        assert!(
            (pair[0].first_seen_tick, &pair[0].id) < (pair[1].first_seen_tick, &pair[1].id),
            "{pair:?}"
        );
    }
    assert_eq!(
        rows.iter()
            .map(|row| row.target.route.clone().unwrap())
            .collect::<Vec<_>>(),
        vec!["background-route", "default-route", "urgent-route"]
    );
}

#[test]
fn an_invented_attention_band_fails_the_world_load_and_names_its_section() {
    let world = std::fs::read_to_string(WORLD).unwrap();
    assert!(phoenix::world::config::parse_world(&world).is_ok());
    let broken = world.replace(
        "attention_band = \"urgent\"",
        "attention_band = \"critical\"",
    );
    let error = phoenix::world::config::parse_world(&broken).unwrap_err();
    assert!(
        error.contains("[[gm_comms_route]] 'urgent-route'")
            && error.contains("attention_band 'critical'")
            && error.contains("'background'"),
        "{error}"
    );
}

/// The queue is peer-local presentation: it must not reach the authoritative
/// digest, whatever it is holding.
#[test]
fn the_attention_queue_is_not_an_authoritative_input() {
    let mut app = seeded();
    let before = phoenix::sim_digest::world_digest(app.world());
    open_hail(
        &mut app,
        "world.probe_gm_attention.speaker_urgent",
        "urgent-route",
        "urgent-hail",
    );
    let with_occurrence = phoenix::sim_digest::world_digest(app.world());
    assert!(!queue(&mut app).is_empty());
    // Publishing the projection changes nothing the digest folds.
    assert_eq!(
        with_occurrence,
        phoenix::sim_digest::world_digest(app.world())
    );
    // The hail itself of course did change authoritative state; that is the
    // point of the comparison above being taken after it.
    assert_ne!(before, with_occurrence);
}
