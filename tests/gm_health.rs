//! Public technical health on a real Game Master desk (issue #1437).
//!
//! Every fact here comes from the machinery that actually owns it: real
//! `Sessions` records made by the ordinary lobby registration path, a real
//! [`LockstepSession`] barrier with real watermarks and a real departure, the
//! real [`SimulationPaused`] resource, and a real headless world with a real
//! hull and its authored Stations. Nothing hand-builds a projection.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::command_admission::HostSlot;
use phoenix::core::messages::StationId;
use phoenix::entities::spawner::EntityUuid;
use phoenix::gm_action::SimulationPaused;
use phoenix::gm_attention::{
    publish_attention_projection, GmAttentionBand, GmAttentionCategory, GmAttentionOccurrence,
    GmAttentionPlugin, GmAttentionState,
};
use phoenix::gm_health::*;
use phoenix::lobby::Sessions;
use phoenix::lockstep::{FleetLockstep, FleetRoster, FleetShip, FleetSlotOf, LockstepSession};
use project_phoenix as phoenix;

/// The #1433 probe world: one crewed hull plus three authored Comms routes, so
/// the technical rows and the advisory ones can be seen in the same queue.
const WORLD: &str = "assets/worlds/probe_gm_attention.toml";

const HELM: &str = "helm";
const DELAY: u64 = 6;

fn args() -> phoenix::headless::HeadlessArgs {
    phoenix::headless::HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1437),
        deterministic: true,
        max_ticks: 600,
        ..Default::default()
    }
}

/// A booted GM-presenting peer whose fleet is `slots`, seen from `local`.
fn seeded(local: HostSlot, slots: &[HostSlot]) -> App {
    let mut app = phoenix::headless::build_headless_app(&args()).unwrap();
    let ships = slots.iter().map(|&host| FleetShip::new(host)).collect();
    app.insert_resource(FleetRoster::new(ships, local));
    app.insert_resource(FleetLockstep(LockstepSession::new(
        local,
        slots.iter().copied(),
        DELAY,
    )));
    // The projections only run on a peer that is actually presenting a GM desk.
    app.insert_resource(phoenix::gm_projection::BrowserGameMaster);
    app.add_plugins((GmHealthPlugin, GmAttentionPlugin));
    app.finish();
    app.cleanup();
    advance(&mut app, 90);
    app
}

fn solo() -> App {
    seeded(HostSlot(1), &[HostSlot(1)])
}

fn advance(app: &mut App, ticks: usize) {
    for _ in 0..ticks {
        app.update();
    }
}

/// The uuid of the hull the frozen roster put in the world.
fn ship(app: &mut App) -> String {
    let mut query = app.world_mut().query::<(&EntityUuid, &FleetSlotOf)>();
    query.iter(app.world()).next().unwrap().0 .0.clone()
}

/// Sample the health projection the way the Host Channel does.
fn health(app: &mut App) -> GmHealthProjection {
    app.world_mut()
        .run_system_once(publish_health_projection)
        .unwrap();
    app.world()
        .resource::<GmHealthWatch>()
        .last()
        .cloned()
        .unwrap_or_default()
}

/// Sample the attention queue AFTER the health projection, exactly as the
/// registered system order does.
fn queue(app: &mut App) -> Vec<GmAttentionOccurrence> {
    health(app);
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

/// Seat a real human at Helm through the ordinary Session registration path.
fn seat(app: &mut App, token: &str, name: &str) {
    let mut sessions = app.world_mut().resource_mut::<Sessions>();
    sessions.0.register(token.into(), name.into()).unwrap();
    sessions.0.set_station(token, Some(StationId(HELM.into())));
}

fn drop_connection(app: &mut App, token: &str) {
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .disconnect(token);
}

fn reconnect(app: &mut App, token: &str) {
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .reconnect(token);
}

/// Declare `slot`'s watermark for the tick this host is about to run, exactly as
/// an arriving mesh frame from a peer that is keeping pace would.
fn keep_pace(app: &mut App, slot: HostSlot) {
    let now = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    let mut fleet = app.world_mut().resource_mut::<FleetLockstep>();
    let ready = fleet.0.ready_through(now);
    fleet.0.observe(slot, ready);
}

/// How many health republishes are sitting in the message buffer.
fn published(app: &mut App) -> usize {
    app.world_mut()
        .run_system_once(
            |mut reader: MessageReader<phoenix::console_bridge::GmHealthChanged>| {
                reader.read().count()
            },
        )
        .unwrap()
}

fn station_rows(projection: &GmHealthProjection) -> Vec<(&str, GmHealthState)> {
    projection
        .stations
        .iter()
        .map(|row| (row.station_id.0.as_str(), row.state))
        .collect()
}

/// A real human dropping off a real Station produces BOTH treatments — the
/// Urgent row a facilitator triages and the persistent technical warning they
/// cannot narrow away from — and a reconnect resolves both.
#[test]
fn a_dropped_station_human_is_urgent_attention_and_a_technical_alert_until_they_return() {
    let mut app = solo();
    seat(&mut app, "crew-1", "Morgan");

    let connected = health(&mut app);
    assert_eq!(
        station_rows(&connected),
        vec![(HELM, GmHealthState::Live)],
        "a seated, connected human reads as connected"
    );
    assert!(connected.alerts.is_empty(), "nothing is wrong yet");
    assert!(
        queue(&mut app)
            .iter()
            .all(|row| row.category != GmAttentionCategory::StationHealth),
        "and the queue carries no technical row"
    );

    drop_connection(&mut app, "crew-1");
    let hull = ship(&mut app);
    let lost = health(&mut app);
    assert_eq!(
        station_rows(&lost),
        vec![(HELM, GmHealthState::Disconnected)]
    );
    assert_eq!(lost.alerts.len(), 1, "one technical warning, not a storm");
    let alert = &lost.alerts[0];
    assert_eq!(alert.kind, GmHealthAlertKind::StationDisconnected);
    assert_eq!(alert.severity, GmHealthState::Disconnected);
    assert_eq!(alert.reason.id, STATION_DISCONNECTED_REASON);
    assert_eq!(alert.reason.params.get("operator").unwrap(), "Morgan");
    assert_eq!(alert.station.as_ref().unwrap().0, HELM);
    assert_eq!(alert.ship.as_ref().unwrap().entity_id, hull);
    assert_eq!(lost.worst_state(), GmHealthState::Disconnected);

    // The same fact, once, in the queue — and Urgent, decided by the system.
    let rows = queue(&mut app);
    let technical: Vec<&GmAttentionOccurrence> = rows
        .iter()
        .filter(|row| row.category == GmAttentionCategory::StationHealth)
        .collect();
    assert_eq!(technical.len(), 1);
    assert_eq!(technical[0].band, GmAttentionBand::Urgent);
    assert_eq!(technical[0].reason.id, STATION_DISCONNECTED_REASON);
    assert_eq!(
        technical[0].target.ship.as_ref().unwrap().entity_id,
        hull,
        "opening the row selects the hull that lost its human"
    );
    assert_eq!(
        technical[0].id,
        format!("health:{}", alert.id),
        "one fact, one identity, in both treatments"
    );

    reconnect(&mut app, "crew-1");
    let back = health(&mut app);
    assert_eq!(station_rows(&back), vec![(HELM, GmHealthState::Live)]);
    assert!(back.alerts.is_empty(), "the reconnect resolves the warning");
    assert!(
        queue(&mut app)
            .iter()
            .all(|row| row.category != GmAttentionCategory::StationHealth),
        "and the Urgent row with it"
    );
}

/// The ordinary mid-game backfill. The lobby lets a live human claim a seat
/// whose previous holder merely dropped, and leaves the departed record still
/// naming that seat — so the panel must report the SEAT, resolved to its
/// connected holder, and not one row per stale record. Getting this wrong puts
/// a false, permanently unresolvable Disconnected warning on the one region a
/// facilitator cannot filter, snooze or hold away.
#[test]
fn a_seat_backfilled_by_a_live_human_reads_as_theirs_and_clears_the_warning() {
    let mut app = solo();
    seat(&mut app, "crew-1", "Morgan");
    drop_connection(&mut app, "crew-1");
    assert_eq!(
        health(&mut app).alerts.len(),
        1,
        "the vacated seat is a warning while it is genuinely empty"
    );

    // A second real human takes the seat Morgan's record still points at.
    seat(&mut app, "crew-2", "Alex");

    let backfilled = health(&mut app);
    assert_eq!(
        station_rows(&backfilled),
        vec![(HELM, GmHealthState::Live)],
        "one row for one seat, and the seat is being flown"
    );
    assert_eq!(
        backfilled.stations[0].operator, "Alex",
        "named for the human actually on it, not the departed one"
    );
    assert!(
        backfilled.alerts.is_empty(),
        "a crewed seat is not a technical failure"
    );
    assert_ne!(backfilled.worst_state(), GmHealthState::Disconnected);
    assert!(
        queue(&mut app)
            .iter()
            .all(|row| row.category != GmAttentionCategory::StationHealth),
        "and nothing Urgent is left standing against it"
    );

    // Should the replacement drop too, the seat is empty again — reported under
    // the operator who actually left it, not the one who left before them.
    drop_connection(&mut app, "crew-2");
    let lost = health(&mut app);
    assert_eq!(
        station_rows(&lost),
        vec![(HELM, GmHealthState::Disconnected)]
    );
    assert_eq!(lost.alerts.len(), 1, "one seat, one warning");
    assert_eq!(
        lost.alerts[0].reason.params.get("operator").unwrap(),
        "Alex"
    );
}

/// A second outage is a NEW occurrence, not the old one aging on: its identity
/// changes, so a snooze taken against the first cannot hide the second.
#[test]
fn a_second_outage_mints_a_fresh_occurrence_rather_than_reviving_the_old_one() {
    let mut app = solo();
    seat(&mut app, "crew-1", "Morgan");
    drop_connection(&mut app, "crew-1");
    let first = health(&mut app).alerts[0].id.clone();
    reconnect(&mut app, "crew-1");
    assert!(health(&mut app).alerts.is_empty());
    drop_connection(&mut app, "crew-1");
    let second = health(&mut app).alerts[0].id.clone();
    assert_ne!(first, second, "a recurrence is a different occurrence");
    assert!(first.starts_with("station-disconnected:"));
    assert!(second.starts_with("station-disconnected:"));
}

/// A whole peer leaving the barrier (issue #1119) loses the hull it was flying,
/// and says so about the hull rather than about a slot number.
#[test]
fn a_departed_peer_is_reported_as_the_hull_it_was_flying() {
    // This desk is slot 2 and watches slot 1's hull.
    let mut app = seeded(HostSlot(2), &[HostSlot(1), HostSlot(2)]);
    let hull = ship(&mut app);
    assert!(health(&mut app).alerts.is_empty());

    app.world_mut()
        .resource_mut::<FleetLockstep>()
        .0
        .depart(HostSlot(1));
    let lost = health(&mut app);
    assert_eq!(lost.alerts.len(), 1);
    assert_eq!(lost.alerts[0].kind, GmHealthAlertKind::ShipPeerLost);
    assert_eq!(lost.alerts[0].reason.id, SHIP_PEER_LOST_REASON);
    assert_eq!(lost.alerts[0].ship.as_ref().unwrap().entity_id, hull);
    let departed = lost
        .peers
        .iter()
        .find(|peer| {
            peer.ship
                .as_ref()
                .is_some_and(|ship| ship.entity_id == hull)
        })
        .expect("the hull's peer has a row");
    assert_eq!(departed.state, GmHealthState::Disconnected);
    assert_eq!(
        departed.behind_ticks, None,
        "a departed peer has no meaningful watermark to report"
    );
    // Urgent in the queue too, because a hull with nobody flying it is exactly
    // what a facilitator has to decide about.
    let rows = queue(&mut app);
    let technical: Vec<_> = rows
        .iter()
        .filter(|row| row.category == GmAttentionCategory::StationHealth)
        .collect();
    assert_eq!(technical.len(), 1);
    assert_eq!(technical[0].band, GmAttentionBand::Urgent);
}

/// The four states are told apart on a real barrier: a peer inside the agreed
/// input delay is fine, one past it is behind, and the same lag while the world
/// is deliberately held is a pause rather than a fault.
#[test]
fn paused_stale_and_live_are_distinguished_by_the_barriers_own_threshold() {
    let mut app = seeded(HostSlot(1), &[HostSlot(1), HostSlot(2)]);
    let now = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    let ready_through = now + DELAY;

    // Slot 2 has declared everything this host has: exactly level.
    app.world_mut()
        .resource_mut::<FleetLockstep>()
        .0
        .observe(HostSlot(2), ready_through);
    let level = health(&mut app);
    let peer_two = |projection: &GmHealthProjection| {
        projection
            .peers
            .iter()
            .find(|peer| !peer.local)
            .cloned()
            .expect("the remote peer has a row")
    };
    assert_eq!(peer_two(&level).state, GmHealthState::Live);
    assert_eq!(peer_two(&level).behind_ticks, Some(0));
    assert_eq!(
        level.input_delay_ticks,
        Some(DELAY),
        "the panel reports the tolerance it judges against"
    );

    // Still inside the barrier's own tolerance: not a fault, and not reported
    // as one. `stall_at` would still let this tick run.
    let mut app = seeded(HostSlot(1), &[HostSlot(1), HostSlot(2)]);
    let now = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    app.world_mut()
        .resource_mut::<FleetLockstep>()
        .0
        .observe(HostSlot(2), now);
    let inside = health(&mut app);
    assert_eq!(peer_two(&inside).behind_ticks, Some(DELAY));
    assert_eq!(peer_two(&inside).state, GmHealthState::Live);

    // One tick past it — the point the barrier itself starts withholding.
    let mut app = seeded(HostSlot(1), &[HostSlot(1), HostSlot(2)]);
    let now = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    app.world_mut()
        .resource_mut::<FleetLockstep>()
        .0
        .observe(HostSlot(2), now - 1);
    let behind = health(&mut app);
    assert_eq!(peer_two(&behind).behind_ticks, Some(DELAY + 1));
    assert_eq!(peer_two(&behind).state, GmHealthState::Stale);
    assert_eq!(behind.worst_state(), GmHealthState::Stale);
    assert!(
        behind.alerts.is_empty(),
        "a lagging peer is a state, not a persistent failure banner"
    );

    // The SAME lag, while the operator holds the world, is a pause.
    app.world_mut().insert_resource(SimulationPaused(true));
    let held = health(&mut app);
    assert!(held.paused);
    assert_eq!(peer_two(&held).state, GmHealthState::Paused);
    assert_eq!(held.worst_state(), GmHealthState::Paused);
}

/// A pause must not be able to hide a real loss.
#[test]
fn a_paused_world_still_reports_a_station_that_lost_its_human() {
    let mut app = solo();
    seat(&mut app, "crew-1", "Morgan");
    app.world_mut().insert_resource(SimulationPaused(true));
    assert_eq!(
        station_rows(&health(&mut app)),
        vec![(HELM, GmHealthState::Paused)]
    );
    drop_connection(&mut app, "crew-1");
    let lost = health(&mut app);
    assert_eq!(
        station_rows(&lost),
        vec![(HELM, GmHealthState::Disconnected)]
    );
    assert_eq!(lost.alerts.len(), 1);
    assert_eq!(lost.worst_state(), GmHealthState::Disconnected);
}

/// Nothing private crosses the Host Channel. The assertion is made against the
/// exact JSON the encoder puts on the wire, not against the Rust struct.
#[test]
fn the_wire_carries_no_session_token_no_fleet_slot_and_no_transport_id() {
    let mut app = seeded(HostSlot(1), &[HostSlot(1), HostSlot(2)]);
    seat(&mut app, "session-token-abcdef", "Morgan");
    drop_connection(&mut app, "session-token-abcdef");
    app.world_mut()
        .resource_mut::<FleetLockstep>()
        .0
        .depart(HostSlot(2));
    let projection = health(&mut app);
    assert!(!projection.alerts.is_empty(), "there is something to leak");
    let json = phoenix::core::codec::encode_gm_health_projection(&projection).unwrap();

    assert!(
        !json.contains("session-token-abcdef"),
        "the Session token must never reach a GM page: {json}"
    );
    for forbidden in ["\"slot\"", "HostSlot", "\"host\"", "incarnation", "\"leg\""] {
        assert!(
            !json.contains(forbidden),
            "technical transport identity {forbidden} must not be projected: {json}"
        );
    }
    // What DOES cross is the public identity a facilitator can act on.
    assert!(
        json.contains("Morgan"),
        "the public display name is the point"
    );
    assert!(json.contains("station-disconnected:"));
}

/// The technical treatment is system-defined: an authored advisory band on the
/// route table moves that route's Comms rows and reaches nothing here.
#[test]
fn an_authored_attention_band_cannot_reband_or_suppress_the_technical_row() {
    let mut app = solo();
    // The probe world authors `attention_band = "background"` on one route, so
    // a world that CAN lower a band is already loaded.
    let world = app
        .world()
        .resource::<phoenix::world::config::WorldConfig>();
    assert!(
        world
            .gm_comms_routes
            .iter()
            .any(|route| route.attention_band.as_deref() == Some("background")),
        "the probe world authors a lowered advisory band"
    );
    seat(&mut app, "crew-1", "Morgan");
    drop_connection(&mut app, "crew-1");
    let technical: Vec<_> = queue(&mut app)
        .into_iter()
        .filter(|row| row.category == GmAttentionCategory::StationHealth)
        .collect();
    assert_eq!(technical.len(), 1);
    assert_eq!(
        technical[0].band,
        GmAttentionBand::Urgent,
        "no authored key anywhere lowers a technical row"
    );
}

/// Routine simulation is not news. The projection republishes on what a Game
/// Master reads, never on the tick advancing — otherwise the banner region
/// would re-announce itself every frame.
#[test]
fn an_ordinary_running_tick_does_not_republish_the_panel() {
    let mut app = solo();
    seat(&mut app, "crew-1", "Morgan");
    health(&mut app);
    published(&mut app);
    for _ in 0..8 {
        app.update();
    }
    assert_eq!(
        published(&mut app),
        0,
        "eight quiet ticks published nothing new"
    );
    drop_connection(&mut app, "crew-1");
    app.update();
    assert!(published(&mut app) > 0, "a real loss does publish");
}

/// The same claim where it can actually be broken: a fleet with a REMOTE peer,
/// so the projection carries a live `behind_ticks` watermark. A solo fleet has
/// no remote watermark at all, so it cannot exercise the path that would
/// republish at tick rate — the panel would then re-announce itself every frame
/// to a facilitator who already knows (PRD #1418).
#[test]
fn a_multi_peer_fleet_keeping_pace_does_not_republish_either() {
    let peer = HostSlot(2);
    let mut app = seeded(HostSlot(1), &[HostSlot(1), peer]);
    seat(&mut app, "crew-1", "Morgan");

    // Settle into the steady state: the peer declares its watermark for every
    // tick this host runs, so its lag is a constant rather than a growing one.
    for _ in 0..4 {
        keep_pace(&mut app, peer);
        app.update();
    }
    let steady = health(&mut app);
    assert_eq!(steady.peers.len(), 2, "the remote peer is really a row");
    let remote = steady
        .peers
        .iter()
        .find(|row| !row.local)
        .expect("a remote peer row");
    assert!(
        remote.behind_ticks.is_some(),
        "the remote row carries a real watermark measure, or this test proves nothing"
    );
    assert_eq!(remote.state, GmHealthState::Live, "and it is keeping pace");

    published(&mut app);
    let before = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    let mut republished = 0;
    for _ in 0..8 {
        keep_pace(&mut app, peer);
        app.update();
        republished += published(&mut app);
    }
    let after = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    assert!(
        after > before,
        "the simulation must actually be running for this to mean anything"
    );
    assert_eq!(
        republished,
        0,
        "{} ordinary ticks in a two-peer fleet published nothing new",
        after - before
    );

    // And the test can still see a republish: let the peer actually fall behind
    // far enough to change what a Game Master reads.
    for _ in 0..(DELAY as usize + 2) {
        app.update();
    }
    assert!(
        published(&mut app) > 0,
        "a peer that really falls behind does publish"
    );
}
