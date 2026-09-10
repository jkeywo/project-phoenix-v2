//! The GM Station-workload advisory over real worlds, real conversations and
//! real owner state (issue #1438, PRD #1419 M4).
//!
//! Every counted demand here is produced the way the running game produces it:
//! a conversation is opened by a shipped `GmAction::TransmitComms` and answered
//! by the crew's own response command through a Station puppet; a repair
//! request is filed through the same `RepairRequestQueue::push_or_merge` the
//! Channel-3 delivery handler calls; a task activation is opened through the
//! same `TaskLifecycles::begin` the lifecycle emitter calls. Nothing constructs
//! a projection by hand, and every boundary is counted in fixed simulation
//! steps, because the whole claim of the duration half of this feature is that
//! it is SIMULATION time.
//!
//! Its own binary for `snapshot_resume.rs`'s reason: `--deterministic` pins a
//! one-thread task pool, and Bevy's pools are process-global.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use phoenix::command_admission::{log::ShipKey, HostSlot};
use phoenix::comms::server::CommsInboxRes;
use phoenix::console::repair::server::{RepairQueueEntry, RepairRequestQueue};
use phoenix::core::messages::{CommsMessage, GamePhase, StationId, SystemControlPayload, SystemId};
use phoenix::core::task_lifecycle::{
    TaskLifecycles, TaskSlot, TASK_VERB_EXTERNAL_REPAIR, TASK_VERB_SCAN,
};
use phoenix::entities::spawner::{EntityName, EntityUuid};
use phoenix::gm_action::{
    GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder, SimulationPaused,
};
use phoenix::gm_comms::{GmCommsContent, GmCommsTransmission};
use phoenix::gm_workload::*;
use phoenix::lockstep::{FleetRoster, FleetShip, FleetSlotOf};
use phoenix::ship::control_source::ControlSource;
use phoenix::ship::damage::DamageTier;
use phoenix::sim_tick::SimTick;
use project_phoenix as phoenix;

/// Every shipped default: three demands, thirty simulation seconds.
const WORLD: &str = "assets/worlds/probe_gm_workload.toml";
/// Both thresholds authored down — two demands, one simulation second.
const AUTHORED_WORLD: &str = "assets/worlds/probe_gm_workload_authored.toml";
/// The workload advisory switched off, the attention queue left alone.
const QUIET_WORLD: &str = "assets/worlds/probe_gm_workload_quiet.toml";
/// One anchor and one mandatory Reach objective, so the ship's OWN Navigation
/// plots a course a human Helm has to take up. See the world's own header.
const COURSE_WORLD: &str = "assets/worlds/probe_gm_workload_course.toml";

/// All three probe worlds run at 30 Hz, so a tick count reads directly as
/// simulation seconds.
const HZ: f64 = 30.0;

fn seeded(world: &str) -> App {
    let mut app = phoenix::headless::build_headless_app(&phoenix::headless::HeadlessArgs {
        world_path: world.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        dt: 1.0 / HZ,
        seed: Some(1438),
        deterministic: true,
        max_ticks: 20000,
        ..Default::default()
    })
    .expect("the probe world builds through the ordinary headless boot");
    app.insert_resource(FleetRoster::new(
        vec![FleetShip::new(HostSlot(1))],
        HostSlot(1),
    ));
    // The advisory only runs on a peer actually presenting a GM desk.
    app.insert_resource(phoenix::gm_projection::BrowserGameMaster);
    app.add_plugins(phoenix::gm_workload::GmWorkloadPlugin);
    app.finish();
    app.cleanup();
    for _ in 0..600 {
        app.update();
        if *app.world().resource::<State<GamePhase>>().get() == GamePhase::InProgress {
            break;
        }
    }
    assert_eq!(
        *app.world().resource::<State<GamePhase>>().get(),
        GamePhase::InProgress,
        "the probe world reaches a running mission"
    );
    app
}

fn tick(app: &App) -> u64 {
    app.world().resource::<SimTick>().0
}

/// Advance exactly `steps` FIXED steps. Frames are only the vehicle.
fn step(app: &mut App, steps: u64) {
    let target = tick(app) + steps;
    let mut frames = 0;
    while tick(app) < target {
        app.update();
        frames += 1;
        assert!(frames < steps * 8 + 64, "the fixed clock is advancing");
    }
    assert_eq!(tick(app), target, "landed on the exact tick asked for");
}

fn fleet_entity(app: &mut App) -> Entity {
    let mut query = app
        .world_mut()
        .query_filtered::<Entity, With<FleetSlotOf>>();
    query
        .iter(app.world())
        .next()
        .expect("the fleet ship exists")
}

fn fleet_uuid(app: &mut App) -> String {
    let mut query = app.world_mut().query::<(&EntityUuid, &FleetSlotOf)>();
    query.iter(app.world()).next().unwrap().0 .0.clone()
}

fn speaker(app: &mut App, name: &str) -> String {
    let mut query = app.world_mut().query::<(&EntityUuid, &EntityName)>();
    query
        .iter(app.world())
        .find(|(_, entity)| entity.0 == name)
        .unwrap_or_else(|| panic!("the world authors speaker {name}"))
        .0
         .0
        .clone()
}

/// The Station that currently owns one of the fleet ship's Systems, resolved
/// exactly as command admission resolves it.
fn station_of(app: &mut App, system: &SystemId) -> StationId {
    let ship = fleet_uuid(app);
    let mut query = app.world_mut().query::<(
        &EntityUuid,
        &phoenix::ship::components::ShipConfigComponent,
        Option<&phoenix::ship_plugin::HumanSeekingHosts>,
    )>();
    let (_, config, hosts) = query
        .iter(app.world())
        .find(|(uuid, _, _)| uuid.0 == ship)
        .unwrap();
    phoenix::command_admission::station_for_system(&config.0, hosts, system)
        .unwrap_or_else(|| panic!("the hull authors a Station for {system:?}"))
}

/// Put every System this hull owns under one control source, the same way the
/// station-rating system does. A headless run backfills the player ship, and a
/// workload advisory about nobody is not what these cases are testing.
fn crew_everything(app: &mut App, source: ControlSource) {
    let entity = fleet_entity(app);
    let ids: Vec<SystemId> = app
        .world()
        .entity(entity)
        .get::<phoenix::ship::components::ShipConfigComponent>()
        .unwrap()
        .0
        .systems
        .iter()
        .map(|system| system.id.clone())
        .collect();
    let mut entity_mut = app.world_mut().entity_mut(entity);
    let mut sources = entity_mut
        .get_mut::<phoenix::ship_plugin::ShipSystemControlSources>()
        .unwrap();
    for id in ids {
        sources.0.set(id, source);
    }
}

/// Seat a connected operator at one Station and take it off Backfill, the way a
/// lobby claim does.
///
/// Needed for the Comms Station specifically: the `comms` System is
/// human-seeking, so its control source is RE-DERIVED every tick from whether
/// any Station has a live holder on a non-Backfill rating. Writing the resolver
/// directly would be overwritten on the next step, and an advisory about a seat
/// nobody is sitting at is not what these cases are about.
fn crew_station(app: &mut App, station: &StationId) {
    let entity = fleet_entity(app);
    let config = app
        .world()
        .entity(entity)
        .get::<phoenix::ship::components::ShipConfigComponent>()
        .unwrap()
        .0
        .clone();
    let rating = config
        .stations
        .iter()
        .find(|candidate| candidate.id == *station)
        .and_then(|candidate| {
            candidate
                .ratings
                .iter()
                .map(|rating| rating.name.clone())
                .find(|name| name != phoenix::ship::rating::BACKFILL_RATING)
        })
        .unwrap_or_else(|| panic!("the hull authors a crewed rating for {station:?}"));
    crew_station_on_rating(app, station, &rating);
}

/// Seat a connected operator at one Station on a NAMED authored rating.
///
/// The rating is applied through `ship::rating::apply_rating` — the same call a
/// lobby claim makes — so which of the seat's Systems come out `Ai` and which
/// come out `Human` is decided by the SHIPPED hull TOML, not by this test. That
/// is the whole point of the `Simplified` Engineering case below: the mixed seat
/// has to be one the game actually authors.
fn crew_station_on_rating(app: &mut App, station: &StationId, rating: &str) {
    let entity = fleet_entity(app);
    let config = app
        .world()
        .entity(entity)
        .get::<phoenix::ship::components::ShipConfigComponent>()
        .unwrap()
        .0
        .clone();
    assert!(
        phoenix::ship::rating::available_ratings_for_station(&config, station).contains(&rating),
        "the hull authors the {rating} rating for {station:?}"
    );
    let token = format!("crew-{}", station.0);
    {
        let mut sessions = app.world_mut().resource_mut::<phoenix::lobby::Sessions>();
        sessions
            .0
            .register(token.clone(), format!("Operator {}", station.0))
            .expect("a fresh token");
        sessions.0.set_station(&token, Some(station.clone()));
    }
    let mut entity_mut = app.world_mut().entity_mut(entity);
    let mut ratings = entity_mut
        .get_mut::<phoenix::ship::components::ActiveStationRatings>()
        .expect("the hull carries active ratings");
    ratings.0.insert(station.clone(), rating.to_string());
    let mut sources = entity_mut
        .get_mut::<phoenix::ship_plugin::ShipSystemControlSources>()
        .unwrap();
    phoenix::ship::rating::apply_rating(&config, station, rating, &mut sources.0);
}

/// One System's live control source.
fn source_of(app: &mut App, system: &SystemId) -> ControlSource {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::ship_plugin::ShipSystemControlSources>()
        .expect("the cruiser carries control sources")
        .0
        .source_for(system)
}

/// What the Channel-3 router makes of a whole seat, asked with the router's own
/// shared rule rather than a second one written here.
fn seat_source_of(app: &mut App, station: &StationId) -> ControlSource {
    let entity = fleet_entity(app);
    let ids = systems_of(app, station);
    let sources = app
        .world()
        .entity(entity)
        .get::<phoenix::ship_plugin::ShipSystemControlSources>()
        .expect("the cruiser carries control sources");
    let policies: Vec<_> = ids.iter().map(|id| sources.0.policy_for(id)).collect();
    phoenix::ship::coordination::seat_control_source(&policies)
}

/// Mark every System one Station owns as damage-offline, the way the damage
/// sweep does. Durable, unlike a control-source write: `policy_for` returns the
/// offline policy whatever the resolver says.
fn disable_station(app: &mut App, station: &StationId) {
    let entity = fleet_entity(app);
    let ids = systems_of(app, station);
    let mut entity_mut = app.world_mut().entity_mut(entity);
    let mut sources = entity_mut
        .get_mut::<phoenix::ship_plugin::ShipSystemControlSources>()
        .unwrap();
    for id in ids {
        // Both halves of "nothing can operate this": the explicit offline
        // control source a Station Rating sets, and the damage latch the
        // console-damage sweep sets. Either alone is enough for `policy_for`;
        // together they survive whichever of the two the next tick re-derives.
        sources.0.set(id.clone(), ControlSource::Offline);
        sources.0.set_offline(id, true);
    }
}

/// Every System one Station owns RIGHT NOW, resolved exactly as command
/// admission resolves it — so a human-seeking System currently presented at
/// this seat is included and one that has gone elsewhere is not.
fn systems_of(app: &mut App, station: &StationId) -> Vec<SystemId> {
    let entity = fleet_entity(app);
    let config = app
        .world()
        .entity(entity)
        .get::<phoenix::ship::components::ShipConfigComponent>()
        .unwrap()
        .0
        .clone();
    let hosts = app
        .world()
        .entity(entity)
        .get::<phoenix::ship_plugin::HumanSeekingHosts>()
        .cloned();
    config
        .systems
        .iter()
        .filter(|system| {
            phoenix::command_admission::station_for_system(&config, hosts.as_ref(), &system.id)
                .as_ref()
                == Some(station)
        })
        .map(|system| system.id.clone())
        .collect()
}

/// Every System one Station AUTHORS, straight from the hull TOML.
///
/// `ShipConfig::systems_for_station` — the same `[[system]] station = ...`
/// membership the lobby projects into the roster pill's `station_systems`, and
/// (since #1438) the same membership the advisory reduces to a level. Distinct
/// from [`systems_of`] above, which answers the live "where is this operated
/// from" question instead.
fn authored_systems_of(app: &mut App, station: &StationId) -> Vec<SystemId> {
    let entity = fleet_entity(app);
    let config = app
        .world()
        .entity(entity)
        .get::<phoenix::ship::components::ShipConfigComponent>()
        .unwrap()
        .0
        .clone();
    config
        .systems_for_station(station)
        .map(|system| system.id.clone())
        .collect()
}

/// Mark one System damage-offline, the way the damage sweep does.
fn set_offline(app: &mut App, system: &SystemId, offline: bool) {
    let entity = fleet_entity(app);
    let mut entity_mut = app.world_mut().entity_mut(entity);
    let mut sources = entity_mut
        .get_mut::<phoenix::ship_plugin::ShipSystemControlSources>()
        .unwrap();
    sources.0.set_offline(system.clone(), offline);
}

/// Write the ship's shared Navigation goal — the same component both the human
/// admitted path and `operate_navigation_ai` write. Neither of them sends the
/// Channel-3 clearance; the exactly-once issuer observes this and does.
fn set_waypoint(app: &mut App, mode: Option<phoenix::console::navigation::WaypointMode>) {
    let entity = fleet_entity(app);
    let mut entity_mut = app.world_mut().entity_mut(entity);
    let mut waypoint = entity_mut
        .get_mut::<phoenix::console::navigation::NavigationWaypoint>()
        .expect("the cruiser carries a navigation waypoint");
    match mode {
        Some(mode) => waypoint.set(mode),
        None => {
            waypoint.clear();
        }
    }
}

/// Where the hull is right now, from the same `ShipPhysics` the AI helm steers
/// by and the workload advisory reads.
fn hull_position(app: &mut App) -> (f32, f32) {
    let entity = fleet_entity(app);
    let physics = app
        .world()
        .entity(entity)
        .get::<phoenix::ship::state::ShipPhysics>()
        .expect("the cruiser has a position");
    (physics.x, physics.z)
}

/// Put the hull somewhere. Physics is the sim's own state; a test that needs an
/// ARRIVAL without flying five thousand units at cruise speed moves the hull and
/// lets every reader of that state answer honestly.
fn place_hull(app: &mut App, x: f32, z: f32) {
    let entity = fleet_entity(app);
    let mut entity_mut = app.world_mut().entity_mut(entity);
    let mut physics = entity_mut
        .get_mut::<phoenix::ship::state::ShipPhysics>()
        .expect("the cruiser has a position");
    physics.x = x;
    physics.z = z;
    if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
        transform.translation.x = x;
        transform.translation.z = z;
    }
}

/// The hull's authored arrival tolerance — the SAME radius `ai::server`'s patrol
/// cursor advances on and `helm_ai`'s Reach completion fires on.
fn arrival_radius(app: &mut App) -> f32 {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::entities::spawner::BehaviourSection>()
        .map_or(phoenix::ai::WAYPOINT_ARRIVAL_RADIUS, |behaviour| {
            behaviour.0.waypoint_arrival_radius
        })
}

/// A free waypoint far enough from the hull that no arrival tolerance reaches
/// it, `sign` choosing which way so a replacement is genuinely somewhere else.
fn far_from_hull(app: &mut App, sign: f32) -> phoenix::console::navigation::WaypointMode {
    let (x, z) = hull_position(app);
    phoenix::console::navigation::WaypointMode::Free {
        x: x + sign * 5000.0,
        z: z - sign * 9000.0,
    }
}

/// The destination `probe_gm_workload_course.toml` authors, read from the loaded
/// world rather than copied, so the test and the world file cannot drift.
fn course_anchor(app: &App) -> (f32, f32) {
    let position = app
        .world()
        .resource::<phoenix::world::config::WorldConfig>()
        .anchors
        .get("workload_course")
        .copied()
        .expect("the course probe authors its destination anchor");
    (position[0], position[2])
}

/// Whether the ship is carrying a course at all.
fn waypoint_is_set(app: &mut App) -> bool {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::console::navigation::NavigationWaypoint>()
        .expect("the cruiser carries a navigation waypoint")
        .snapshot()
        .is_some()
}

/// The exactly-once issuer's frontier: which waypoint generation Navigation has
/// actually sent a clearance for, whatever the routing did with it afterwards.
fn issued_generation(app: &mut App) -> Option<u64> {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::console::navigation::server::NavClearanceIssueState>()
        .expect("the cruiser carries a clearance issuer")
        .issued_generation()
}

/// Who is operating the `navigation` System right now.
///
/// Read rather than written, on purpose: `navigation` belongs to a human-seeking
/// Station, so its control source is RE-DERIVED every tick from where the crew
/// are actually sitting. A test that wrote it directly would be overwritten on
/// the next step and would be asserting about a value nothing produced.
fn navigation_source(app: &mut App) -> ControlSource {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::ship_plugin::ShipSystemControlSources>()
        .expect("the cruiser carries control sources")
        .0
        .source_for(&phoenix::ship::system_registry::navigation_system_id())
}

/// Everybody stands up. The lobby's own seat release, so the human-seeking
/// resolver re-derives from an empty table exactly as it does in a real game.
fn vacate_stations(app: &mut App) {
    let mut sessions = app.world_mut().resource_mut::<phoenix::lobby::Sessions>();
    let tokens: Vec<String> = sessions
        .0
        .players()
        .iter()
        .map(|player| player.token.clone())
        .collect();
    for token in tokens {
        sessions.0.set_station(&token, None);
    }
}

fn set_source(app: &mut App, system: &SystemId, source: ControlSource) {
    let entity = fleet_entity(app);
    let mut entity_mut = app.world_mut().entity_mut(entity);
    let mut sources = entity_mut
        .get_mut::<phoenix::ship_plugin::ShipSystemControlSources>()
        .unwrap();
    sources.0.set(system.clone(), source);
}

fn grant(sequence: u64, at: u64, action: GmAction) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(4),
        sequenced_by: HostSlot(1),
        operator_id: "gm-workload".into(),
        correlation: GmActionId::new(format!("workload-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: at,
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
    step(app, 8);
}

/// Open one authored conversation the ordinary way.
fn open_hail(app: &mut App, speaker_name: &str, route: &str, hail: &str) {
    let sender = speaker(app, speaker_name);
    let recipient = fleet_uuid(app);
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
}

fn messages(app: &App) -> Vec<CommsMessage> {
    app.world().resource::<CommsInboxRes>().0.messages()
}

fn message_id(app: &App, body: &str) -> String {
    messages(app)
        .into_iter()
        .find(|m| m.body == body)
        .unwrap_or_else(|| panic!("the world delivers {body}"))
        .id
}

/// Answer one message the ordinary way: puppet the Comms Station and submit the
/// crew's own response command.
fn answer(app: &mut App, id: &str) {
    let ship = fleet_uuid(app);
    let station = station_of(app, &phoenix::ship::system_registry::comms_system_id());
    enqueue(
        app,
        GmAction::SetStationPuppet {
            ship: ShipKey(ship.clone()),
            station: station.clone(),
            active: true,
        },
    );
    enqueue(
        app,
        GmAction::IssueStationCommand {
            ship: ShipKey(ship),
            station,
            target: phoenix::ship::system_registry::comms_system_id(),
            payload: phoenix::core::codec::canonical_system_command(
                &SystemControlPayload::RespondToMessage {
                    message_id: id.into(),
                    response_index: 0,
                },
            )
            .unwrap(),
        },
    );
}

/// The published summary, as the desk would receive it.
fn summary(app: &App) -> Vec<GmStationWorkload> {
    app.world()
        .resource::<GmWorkloadState>()
        .last()
        .cloned()
        .unwrap_or_default()
        .stations
}

/// Whether the published summary names a Station at all.
fn named(app: &App, station: &StationId) -> bool {
    summary(app).iter().any(|row| row.station_id == station.0)
}

/// Every Station the published summary reads `Offline` for.
fn offline_stations(app: &App) -> Vec<String> {
    summary(app)
        .into_iter()
        .filter(|row| row.level == GmWorkloadLevel::Offline)
        .map(|row| row.station_id)
        .collect()
}

fn row(app: &App, station: &StationId) -> GmStationWorkload {
    summary(app)
        .into_iter()
        .find(|row| row.station_id == station.0)
        .unwrap_or_else(|| panic!("the summary carries {station:?}"))
}

/// How long a Channel-3 message spends in the lag queue, in fixed steps. The
/// cruiser authors no `coordination_lag_secs`, so it takes the shipped 2.0 s.
const LAG_STEPS: u64 = 2 * HZ as u64 + 8;

/// The hull's full HP for one System.
fn max_hp(app: &mut App, system: &SystemId) -> f32 {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::entities::spawner::EntitySystemHull>()
        .expect("the cruiser carries a system hull")
        .0
        .get(system)
        .unwrap_or_else(|| panic!("the hull carries {system:?}"))
        .max
}

/// One System's live HP, as the repair sweep and the prune both read it.
fn system_hp(app: &mut App, system: &SystemId) -> f32 {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::entities::spawner::EntitySystemHull>()
        .expect("the cruiser carries a system hull")
        .0
        .get(system)
        .unwrap_or_else(|| panic!("the hull carries {system:?}"))
        .current
}

/// One System's live damage tier.
fn system_tier(app: &mut App, system: &SystemId) -> phoenix::ship::damage::DamageTier {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::entities::spawner::EntitySystemHull>()
        .expect("the cruiser carries a system hull")
        .0
        .tier_for(system)
}

/// Whether any of the hull's repair teams is off `Idle` right now — travelling
/// to a job or working one.
fn teams_busy(app: &mut App) -> bool {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<phoenix::console::repair::server::ShipRepairTeams>()
        .expect("the cruiser musters repair teams")
        .0
        .slots()
        .iter()
        .any(|slot| !matches!(slot, phoenix::core::messages::TeamSlot::Idle))
}

/// Take one of the hull's Systems down to `hp`, the way a hit does.
///
/// This is the whole point of the repair cases: nothing here writes the repair
/// queue. `damage_sync` observes the hull, enqueues a real Channel-3
/// `RepairRequest` addressed to whoever currently holds Repair, the lag router
/// routes it against live control sources, and Repair's own receiver decides
/// what to do with it. The queue entry the advisory counts is produced by that
/// path or it is not produced at all.
fn damage_system(app: &mut App, system: &SystemId, hp: f32) {
    let entity = fleet_entity(app);
    let mut entity_mut = app.world_mut().entity_mut(entity);
    let mut hull = entity_mut
        .get_mut::<phoenix::entities::spawner::EntitySystemHull>()
        .expect("the cruiser carries a system hull");
    hull.0.set_hp(system, hp);
}

/// Heal one System back to full, the way an arrived repair team does.
fn repair_system(app: &mut App, system: &SystemId) {
    let full = max_hp(app, system);
    damage_system(app, system, full);
    // The damage sweep latches an offline System; a healed one is operable
    // again, and the sweep clears the latch on its own. Restoring the source
    // here keeps a test that flipped it deliberately in charge of it.
    set_offline(app, system, false);
}

/// Get ONE real Channel-3 `RepairRequest` delivered to whoever currently holds
/// Repair, and wait out the delivery lag.
///
/// A weapon System is hit, `damage_sync` reports it, and the lag router decides
/// where that report lands. The System is AI-operated because a hit reported by
/// a person to a person is suppressed on purpose.
fn deliver_repair_request(app: &mut App) -> SystemId {
    let damaged = SystemId("phaser-fore".into());
    set_source(app, &damaged, ControlSource::Ai);
    let full = max_hp(app, &damaged);
    damage_system(app, &damaged, full * 0.5);
    step(app, LAG_STEPS);
    damaged
}

/// The ship's own repair queue, as the AI dispatcher and the advisory read it.
fn repair_queue(app: &mut App) -> Vec<RepairQueueEntry> {
    let entity = fleet_entity(app);
    app.world()
        .entity(entity)
        .get::<RepairRequestQueue>()
        .expect("the cruiser carries a repair queue")
        .entries
        .clone()
}

// ── The producers ────────────────────────────────────────────────────────────

/// Three genuinely independent conversations, opened and answered through the
/// ordinary Comms path, are three demands and then none — and each one is named
/// in the evidence rather than folded into a number.
#[test]
fn real_conversations_are_counted_once_each_and_named_in_the_evidence() {
    let mut app = seeded(WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let comms = station_of(&mut app, &phoenix::ship::system_registry::comms_system_id());
    crew_station(&mut app, &comms);
    step(&mut app, 4);
    assert_eq!(row(&app, &comms).level, GmWorkloadLevel::Underused);
    assert_eq!(row(&app, &comms).count, 0);

    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_one",
        "route-one",
        "hail-one",
    );
    let first = message_id(&app, "world.probe_gm_workload.offer_one");
    let engaged = row(&app, &comms);
    assert_eq!(engaged.level, GmWorkloadLevel::Engaged);
    assert_eq!(engaged.count, 1);
    // Evidence, not a score: the row names the conversation that produced it.
    assert_eq!(engaged.demands.len(), 1);
    assert_eq!(engaged.demands[0].key, format!("comms:{first}"));
    assert_eq!(engaged.demands[0].source, COMMS_SOURCE);
    assert_eq!(engaged.demands[0].reason.id, COMMS_REASON);

    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_two",
        "route-two",
        "hail-two",
    );
    assert_eq!(
        row(&app, &comms).count,
        2,
        "independent demands count apart"
    );

    // Republishing changes nothing: the key is the message id the world minted,
    // so the same conversation is the same demand however often it is read.
    step(&mut app, 20);
    assert_eq!(row(&app, &comms).count, 2);

    answer(&mut app, &first);
    let after = row(&app, &comms);
    assert_eq!(after.count, 1, "an answered conversation stops counting");
    assert!(!after
        .demands
        .iter()
        .any(|demand| demand.key == format!("comms:{first}")));
}

/// A repair request DELIVERED to a human Repair seat is the demand; the repair
/// TEAM the crew then dispatch is automatic progress and adds nothing. Two
/// systems of the same Station worsening is one demand, because the ship's own
/// queue merges on the Station. And it ends when the damage it names is gone.
///
/// Every step of that goes through the shipped path: real hull damage, a real
/// `damage_sync` enqueue, the real lag router, and Repair's own receiver.
#[test]
fn a_repair_request_delivered_to_a_human_seat_is_actionable_once_and_ends_with_its_damage() {
    let mut app = seeded(WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let repair_station = station_of(
        &mut app,
        &phoenix::ship::system_registry::repair_system_id(),
    );
    crew_station(&mut app, &repair_station);
    // The damaged Systems report as AI: a hit reported BY a person TO a person
    // is suppressed on purpose (they can say it out loud), so an AI-operated
    // weapon suite is what makes this a request anybody is actually asked.
    let phaser = SystemId("phaser-fore".into());
    let sibling = SystemId("phaser-aft".into());
    let engine = SystemId("helm-engine-port".into());
    for system in [&phaser, &sibling, &engine] {
        set_source(&mut app, system, ControlSource::Ai);
    }
    step(&mut app, 4);
    let before = row(&app, &repair_station).count;
    assert!(
        repair_queue(&mut app).is_empty(),
        "an undamaged hull owes nothing"
    );

    // One hit on the weapons suite. Nothing is written by hand.
    let phaser_full = max_hp(&mut app, &phaser);
    damage_system(&mut app, &phaser, phaser_full * 0.5);
    step(&mut app, LAG_STEPS);
    assert_eq!(
        repair_queue(&mut app).len(),
        1,
        "the delivered request is the ship's own record of what is owed"
    );
    let demanded = row(&app, &repair_station);
    assert_eq!(demanded.count, before + 1);
    assert_eq!(demanded.demands.len(), 1);
    assert_eq!(demanded.demands[0].source, REPAIR_SOURCE);
    assert_eq!(demanded.demands[0].reason.id, REPAIR_REASON);
    // The tier travels as a String Table id, not as raw Rust Debug output.
    assert_eq!(
        demanded.demands[0].reason.params.get("tier"),
        Some(&tier_string_id(DamageTier::Damaged).to_string())
    );

    // A second System of the SAME Station taking a hit. The queue merges on the
    // Station, so the crew are being asked for one trip, not two.
    let sibling_full = max_hp(&mut app, &sibling);
    damage_system(&mut app, &sibling, sibling_full * 0.1);
    step(&mut app, LAG_STEPS);
    assert_eq!(
        row(&app, &repair_station).count,
        before + 1,
        "one demand however many of that Station's Systems are hit"
    );

    // A different Station is a different demand.
    let engine_full = max_hp(&mut app, &engine);
    damage_system(&mut app, &engine, engine_full * 0.5);
    step(&mut app, LAG_STEPS);
    assert_eq!(row(&app, &repair_station).count, before + 2);

    // Now the crew dispatch: an external-repair activation and a science scan
    // open on this hull. Both are ordinary automatic progress and neither is a
    // demand, even though a human started them.
    let ship = fleet_uuid(&mut app);
    let at = tick(&app);
    {
        let mut lifecycles = app.world_mut().resource_mut::<TaskLifecycles>();
        lifecycles.begin(
            TaskSlot::new(ship.clone(), "repair", TASK_VERB_EXTERNAL_REPAIR),
            Some("ally-hull".into()),
            Some(repair_station.0.clone()),
            at,
        );
        lifecycles.begin(
            TaskSlot::new(ship, "sensors", TASK_VERB_SCAN),
            Some("contact".into()),
            None,
            at,
        );
    }
    step(&mut app, 4);
    assert_eq!(
        row(&app, &repair_station).count,
        before + 2,
        "running work is not a demand merely because a human started it"
    );
    assert!(!task_verb_counts(TASK_VERB_SCAN));

    // Repaired. The seat is HUMAN throughout — before #1438 the only pruner ran
    // behind the AI control gate, so this entry would have stood for ever.
    for system in [&phaser, &sibling, &engine] {
        repair_system(&mut app, system);
    }
    step(&mut app, 4);
    assert!(
        repair_queue(&mut app).is_empty(),
        "a request ends when the damage it names does, whoever is at the seat"
    );
    assert_eq!(row(&app, &repair_station).count, before);
}

/// A request filed while the seat was Backfill still ends when its damage does
/// after a person takes the seat.
///
/// This is the failure the seat-independent prune exists for: the entry is
/// created through the AI arm, the seat then flips to Human, and the pruner that
/// used to live inside `operate_repair_ai` would never look at this hull again.
#[test]
fn a_backfill_to_human_flip_still_ends_a_repair_request_whose_damage_is_gone() {
    let mut app = seeded(WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let repair_station = station_of(
        &mut app,
        &phoenix::ship::system_registry::repair_system_id(),
    );
    let phaser = SystemId("phaser-fore".into());
    set_source(&mut app, &phaser, ControlSource::Ai);
    // Backfill the WHOLE Engineering seat — Repair shares it with the power
    // Systems, and a seat is Backfill only when nobody is at any of it. The
    // request is then CONSUMED by the AI, which is the arm that has always
    // written the queue.
    for system in systems_of(&mut app, &repair_station) {
        set_source(&mut app, &system, ControlSource::Ai);
    }
    step(&mut app, 4);
    assert_eq!(row(&app, &repair_station).level, GmWorkloadLevel::Backfill);

    let full = max_hp(&mut app, &phaser);
    damage_system(&mut app, &phaser, full * 0.5);
    step(&mut app, LAG_STEPS);
    assert_eq!(
        repair_queue(&mut app).len(),
        1,
        "the AI arm queued it, as it always has"
    );
    // A Backfill seat is not a workload: the AI's dispatch is the AI's work.
    assert_eq!(row(&app, &repair_station).level, GmWorkloadLevel::Backfill);

    // Somebody takes the seat.
    for system in systems_of(&mut app, &repair_station) {
        set_source(&mut app, &system, ControlSource::Human);
    }
    step(&mut app, 4);
    assert_eq!(
        row(&app, &repair_station).count,
        1,
        "the outstanding request is now a demand on the person who inherited it"
    );

    // And the damage is dealt with.
    repair_system(&mut app, &phaser);
    step(&mut app, 4);
    assert!(
        repair_queue(&mut app).is_empty(),
        "the flip does not strand the entry"
    );
    assert_eq!(row(&app, &repair_station).count, 0);
    assert_eq!(row(&app, &repair_station).level, GmWorkloadLevel::Underused);
}

/// A Navigation clearance DELIVERED to a human Helm is one demand on that
/// Station, keyed by its request generation — so re-pointing the course is a NEW
/// demand rather than the old one persisting.
///
/// Nothing here writes a waypoint: the hull has somewhere to be, its own
/// Navigation ranks the authored destination and writes the shared waypoint, and
/// the exactly-once issuer clears the Helm onto it. That is the only shape this
/// demand has in a running game — a course a person plotted goes to a person who
/// can be spoken to, and the router suppresses it.
#[test]
fn a_navigation_clearance_delivered_to_a_human_helm_is_one_demand_keyed_by_generation() {
    let mut app = seeded(COURSE_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let steering = phoenix::ship::system_registry::helm_steering_system_id();
    let helm = station_of(&mut app, &steering);
    // Deliberately NOT crewing a seat. `navigation` is a human-seeking Station
    // whose host order reaches the Helm, so the moment anybody sits down it
    // comes and sits with them — and then Navigation and Helm are one person,
    // which the router suppresses (the case below).
    step(&mut app, 8);
    assert_eq!(navigation_source(&mut app), ControlSource::Ai);
    assert!(
        waypoint_is_set(&mut app),
        "the hull's own Navigation plotted the authored destination"
    );

    let flying = row(&app, &helm);
    assert_eq!(flying.count, 1);
    assert_eq!(flying.demands.len(), 1);
    assert_eq!(flying.demands[0].source, NAVIGATION_SOURCE);
    assert_eq!(flying.demands[0].reason.id, NAVIGATION_REASON);
    let first_key = flying.demands[0].key.clone();

    // Still the same standing order however long it is left: one demand, one
    // key, not one per tick.
    step(&mut app, 30);
    assert_eq!(row(&app, &helm).count, 1);
    assert_eq!(row(&app, &helm).demands[0].key, first_key);

    // The course is re-pointed. Whoever moved it, the issuer mints a new
    // generation, so what the Helm is being asked is a NEW request rather than
    // the first one quietly re-aimed.
    let elsewhere = far_from_hull(&mut app, -1.0);
    set_waypoint(&mut app, Some(elsewhere));
    step(&mut app, 8);
    let replaced = row(&app, &helm);
    assert_eq!(replaced.count, 1, "a replacement replaces");
    assert_ne!(replaced.demands[0].key, first_key);
}

/// A course the ship's Navigation WITHDRAWS ends the demand with it.
///
/// The objective is completed through the mission's own objective manager, which
/// takes the destination away; `operate_navigation_ai` then has no eligible
/// winner and clears the course it set. Navigation owns that clear, start to
/// finish — the advisory only stops counting.
#[test]
fn a_withdrawn_course_ends_the_clearance_demand() {
    let mut app = seeded(COURSE_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let steering = phoenix::ship::system_registry::helm_steering_system_id();
    let helm = station_of(&mut app, &steering);
    step(&mut app, 8);
    assert_eq!(row(&app, &helm).count, 1);

    app.world_mut()
        .resource_mut::<phoenix::world::server::ObjectiveManagerRes>()
        .0
        .complete("obj-workload-course");
    step(&mut app, 12);
    assert!(
        !waypoint_is_set(&mut app),
        "Navigation withdrew its own course"
    );
    assert_eq!(row(&app, &helm).count, 0);
    assert_eq!(row(&app, &helm).level, GmWorkloadLevel::Underused);
}

/// Two people at one table do not need a clearance popup, and the router
/// suppresses it. What is never delivered is never a demand.
///
/// The exactly-once issuer latches `issued_generation` whatever the helm's
/// control state, so this is precisely the case a producer keyed on that latch
/// would have counted — a Helm operator told they owe an answer to somebody
/// sitting next to them.
#[test]
fn a_clearance_between_two_people_is_suppressed_and_never_counted() {
    let mut app = seeded(COURSE_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let steering = phoenix::ship::system_registry::helm_steering_system_id();
    let helm = station_of(&mut app, &steering);
    // Somebody sits at the Helm. `navigation` is human-seeking and its host
    // order reaches the Helm, so the same person now plots the course AND flies
    // it: a human sender to a human recipient.
    crew_station(&mut app, &helm);
    step(&mut app, 2);
    assert_eq!(navigation_source(&mut app), ControlSource::Human);

    let course = far_from_hull(&mut app, 1.0);
    set_waypoint(&mut app, Some(course));
    step(&mut app, LAG_STEPS);
    // The clearance WAS issued — the frontier moved — and still nobody is owed
    // anything, because the delivery was suppressed.
    assert!(
        issued_generation(&mut app).is_some(),
        "the exactly-once issuer latched regardless of routing"
    );
    assert_eq!(
        row(&app, &helm).count,
        0,
        "a human Navigation and a human Helm can simply talk"
    );

    // The operator steps away from the Helm seat. Navigation has nowhere left to
    // seek a person, falls to the hull's own AI, plots the authored destination,
    // and NOW the Helm — whose Systems are still under a person — is genuinely
    // being asked to take a course up.
    vacate_stations(&mut app);
    step(&mut app, 12);
    assert_eq!(navigation_source(&mut app), ControlSource::Ai);
    assert_eq!(row(&app, &helm).count, 1);
}

/// The clearance demand auto-completes when the hull ARRIVES, and stays
/// complete — it does not come back at the Helm when the hull flies on past the
/// tolerance. The COURSE itself is never touched: Navigation still owns it.
#[test]
fn arriving_completes_the_clearance_demand_without_clearing_the_course() {
    let mut app = seeded(COURSE_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let steering = phoenix::ship::system_registry::helm_steering_system_id();
    let helm = station_of(&mut app, &steering);
    step(&mut app, 8);
    assert_eq!(navigation_source(&mut app), ControlSource::Ai);
    assert_eq!(row(&app, &helm).count, 1, "a course nobody has flown");

    let ship = fleet_uuid(&mut app);
    assert_eq!(
        app.world().resource::<GmWorkloadWatch>().nav_arrival(&ship),
        None
    );

    // Fly it. The hull is placed inside the SAME arrival tolerance the AI helm's
    // patrol cursor advances on and the Reach completion fires on — nothing new
    // is authored here.
    let (goal_x, goal_z) = course_anchor(&app);
    let radius = arrival_radius(&mut app);
    place_hull(&mut app, goal_x + radius * 0.25, goal_z);
    step(&mut app, 4);
    assert_eq!(
        row(&app, &helm).count,
        0,
        "arriving answers the clearance; no acknowledge button exists"
    );
    assert!(
        waypoint_is_set(&mut app),
        "and the COURSE is untouched — Navigation still owns setting and clearing it"
    );

    // Fly straight on through. The demand does not resurrect.
    place_hull(&mut app, goal_x + radius * 6.0, goal_z);
    step(&mut app, 4);
    assert_eq!(
        row(&app, &helm).count,
        0,
        "a flown clearance stays flown when the hull carries on"
    );

    // A NEW course is a new request, and it is owed afresh.
    let elsewhere = far_from_hull(&mut app, -1.0);
    set_waypoint(&mut app, Some(elsewhere));
    step(&mut app, 8);
    assert_eq!(row(&app, &helm).count, 1, "a replacement is owed afresh");
}

/// The arrival latch is history a restored world cannot recompute, so it rides
/// in the snapshot beside the overload stopwatch.
#[test]
fn a_flown_clearance_survives_capture_and_restore() {
    let mut app = seeded(COURSE_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let steering = phoenix::ship::system_registry::helm_steering_system_id();
    let helm = station_of(&mut app, &steering);
    step(&mut app, 8);
    let (goal_x, goal_z) = course_anchor(&app);
    place_hull(&mut app, goal_x, goal_z);
    step(&mut app, 4);

    let ship = fleet_uuid(&mut app);
    let flown = app
        .world()
        .resource::<GmWorkloadWatch>()
        .nav_arrival(&ship)
        .expect("the hull is standing on its own waypoint");
    assert_eq!(row(&app, &helm).count, 0);

    let snapshot = phoenix::snapshot::capture(app.world_mut());
    assert_eq!(snapshot.gm_station_workload.nav_arrival(&ship), Some(flown));

    // Wipe it the way a fresh peer would have it, then restore.
    app.world_mut().insert_resource(GmWorkloadWatch::default());
    phoenix::snapshot::restore(app.world_mut(), &snapshot);
    assert_eq!(
        app.world().resource::<GmWorkloadWatch>().nav_arrival(&ship),
        Some(flown),
        "a resumed mission remembers the course that was flown"
    );
}

/// Source state this build cannot attribute is excluded, not guessed at: an
/// activation on a System no Station of this hull owns, and one whose verb the
/// inventory has never heard of, both contribute nothing anywhere.
#[test]
fn unattributed_and_unsupported_source_state_is_excluded() {
    let mut app = seeded(WORLD);
    crew_everything(&mut app, ControlSource::Human);
    step(&mut app, 2);
    let before: u32 = summary(&app).iter().map(|row| row.count).sum();

    let ship = fleet_uuid(&mut app);
    let at = tick(&app);
    {
        let mut lifecycles = app.world_mut().resource_mut::<TaskLifecycles>();
        // A System this hull does not have: no Station owns it, so there is
        // nobody the demand could belong to.
        lifecycles.begin(
            TaskSlot::new(ship.clone(), "quantum-loom", TASK_VERB_SCAN),
            None,
            None,
            at,
        );
        // A verb the inventory has never been told about.
        lifecycles.begin(
            TaskSlot::new(ship.clone(), "repair", "teleport_the_admiral"),
            None,
            None,
            at,
        );
        // And one belonging to some other hull entirely.
        lifecycles.begin(
            TaskSlot::new("some-other-hull", "repair", TASK_VERB_EXTERNAL_REPAIR),
            None,
            None,
            at,
        );
    }
    step(&mut app, 4);
    let after: u32 = summary(&app).iter().map(|row| row.count).sum();
    assert_eq!(after, before, "nothing unattributed reached a Station");
}

// ── Levels, thresholds and the duration ──────────────────────────────────────

/// The authored count and duration are both honoured, the exact boundary is the
/// authored one, and falling below the count ends the overload AND resets its
/// timer rather than leaving it banked.
#[test]
fn overload_needs_the_authored_count_held_for_the_authored_duration() {
    let mut app = seeded(AUTHORED_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let comms = station_of(&mut app, &phoenix::ship::system_registry::comms_system_id());
    crew_station(&mut app, &comms);
    step(&mut app, 2);
    assert_eq!(row(&app, &comms).overload_count, 2, "the authored count");
    assert_eq!(row(&app, &comms).overload_secs, 1, "the authored duration");

    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_one",
        "route-one",
        "hail-one",
    );
    assert_eq!(row(&app, &comms).level, GmWorkloadLevel::Engaged);
    let ship = fleet_uuid(&mut app);
    assert_eq!(
        app.world()
            .resource::<GmWorkloadWatch>()
            .ticks(&ship, &comms.0),
        0,
        "below the count, nothing is banked"
    );

    // Reaching the count starts the clock but is not yet the answer.
    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_two",
        "route-two",
        "hail-two",
    );
    let banked = app
        .world()
        .resource::<GmWorkloadWatch>()
        .ticks(&ship, &comms.0);
    assert!(banked > 0 && banked < 30, "mid-duration, banked {banked}");
    assert_eq!(
        row(&app, &comms).level,
        GmWorkloadLevel::Engaged,
        "at the count but not yet for the duration"
    );

    // One authored second at 30 Hz is thirty fixed steps. Walk to the step
    // before the boundary, then across it.
    step(&mut app, 30 - banked - 1);
    assert_eq!(
        app.world()
            .resource::<GmWorkloadWatch>()
            .ticks(&ship, &comms.0),
        29
    );
    assert_eq!(row(&app, &comms).level, GmWorkloadLevel::Engaged);
    step(&mut app, 1);
    assert_eq!(
        app.world()
            .resource::<GmWorkloadWatch>()
            .ticks(&ship, &comms.0),
        30
    );
    assert_eq!(row(&app, &comms).level, GmWorkloadLevel::Overloaded);
    assert_eq!(row(&app, &comms).sustained_secs, 1);

    // Dropping below the count ends the overload and resets the timer: the next
    // overload has to be earned over the whole duration again.
    let first = message_id(&app, "world.probe_gm_workload.offer_one");
    answer(&mut app, &first);
    assert_eq!(row(&app, &comms).level, GmWorkloadLevel::Engaged);
    assert_eq!(
        app.world()
            .resource::<GmWorkloadWatch>()
            .ticks(&ship, &comms.0),
        0,
        "the timer resets rather than banking"
    );
}

/// A world may retune the thresholds but may not author a rule that fires
/// before anything has happened, and "off" is its own separate switch.
#[test]
fn a_zero_threshold_fails_the_world_load_and_names_the_switch_instead() {
    let base = std::fs::read_to_string(AUTHORED_WORLD).unwrap();
    let zero_count = base.replace("workload_overload_count = 2", "workload_overload_count = 0");
    let error = phoenix::world::config::parse_world(&zero_count)
        .expect_err("a zero count is refused at load");
    assert!(error.contains("workload_overload_count"), "{error}");
    assert!(error.contains("workload_disabled"), "{error}");

    let zero_secs = base.replace(
        "workload_overload_secs = 1.0",
        "workload_overload_secs = 0.0",
    );
    let error = phoenix::world::config::parse_world(&zero_secs)
        .expect_err("a zero duration is refused at load");
    assert!(error.contains("workload_overload_secs"), "{error}");

    // And the shipped defaults are what an unauthored world gets.
    let plain = phoenix::world::config::parse_world(&std::fs::read_to_string(WORLD).unwrap())
        .expect("the unauthored probe world loads");
    assert_eq!(
        plain.gm_attention.workload_overload_count(),
        DEFAULT_OVERLOAD_COUNT
    );
    assert_eq!(
        plain.gm_attention.workload_overload_ticks(30.0),
        30 * DEFAULT_OVERLOAD_SECS as u64
    );
}

// ── Ownership: Backfill, mixed, and offline ──────────────────────────────────

/// A fully AI-operated Station says Backfill instead of a count; a mixed one is
/// a human seat that counts only the demands its human half is being asked for.
#[test]
fn a_backfilled_station_says_so_and_a_mixed_one_counts_only_its_human_half() {
    let mut app = seeded(WORLD);
    let comms_system = phoenix::ship::system_registry::comms_system_id();
    let repair_system = phoenix::ship::system_registry::repair_system_id();
    let comms = station_of(&mut app, &comms_system);
    let engineering = station_of(&mut app, &repair_system);
    assert_ne!(
        comms, engineering,
        "the cruiser puts Comms and Repair on different Stations"
    );

    crew_everything(&mut app, ControlSource::Human);
    crew_station(&mut app, &comms);
    crew_station(&mut app, &engineering);
    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_one",
        "route-one",
        "hail-one",
    );
    deliver_repair_request(&mut app);
    step(&mut app, 2);
    assert_eq!(row(&app, &comms).count, 1);
    assert_eq!(row(&app, &engineering).count, 1);
    assert_eq!(row(&app, &engineering).level, GmWorkloadLevel::Engaged);

    // MIXED: the Repair System alone stops being human-operable while the
    // Power Systems beside it stay under a person. The seat is still a human
    // seat — somebody is sitting there — but the repair request is no longer
    // something they can act on, so it is not counted against them.
    let engineering_systems = systems_of(&mut app, &engineering);
    assert!(
        engineering_systems.len() > 1,
        "the Engineering seat owns more than one System"
    );
    set_offline(&mut app, &repair_system, true);
    step(&mut app, 2);
    let mixed = row(&app, &engineering);
    assert_eq!(mixed.level, GmWorkloadLevel::Underused);
    assert_eq!(
        mixed.count, 0,
        "a mixed Station counts only the human-required subset"
    );
    // And the Comms Station beside it is untouched: this is per-Station, not
    // per-ship.
    assert_eq!(row(&app, &comms).count, 1);

    // Give Repair back and the same request is a demand again — the demand
    // itself never went anywhere, only the requirement for a person did.
    set_offline(&mut app, &repair_system, false);
    step(&mut app, 2);
    assert_eq!(row(&app, &engineering).count, 1);
    assert_eq!(row(&app, &engineering).level, GmWorkloadLevel::Engaged);

    // Backfill the WHOLE Engineering Station. It stops reporting a count at
    // all: "0 demands" about an AI-operated seat would be an observation about
    // nobody, and the work is now the AI's.
    for id in &engineering_systems {
        set_source(&mut app, id, ControlSource::Ai);
    }
    step(&mut app, 2);
    let backfilled = row(&app, &engineering);
    assert_eq!(backfilled.level, GmWorkloadLevel::Backfill);
    assert_eq!(backfilled.count, 0);
    assert!(
        backfilled.demands.is_empty(),
        "a backfilled seat shows no human evidence"
    );
    assert_eq!(row(&app, &comms).count, 1, "and its neighbour is untouched");
}

/// The cruiser's SHIPPED `Simplified` Engineering rating is the mixed seat this
/// game actually authors: `automated_systems = ["repair"]`, so
/// `ship::rating::apply_rating` puts `repair` under the AI and leaves the power
/// Systems beside it under the person. Its manual copy promises exactly that —
/// "Simplified rating lets the AI run the repair teams so you concentrate on
/// power allocation."
///
/// Two things follow, and this case pins both:
///
/// * the seat still accepts human input (the power Systems do), so the Channel-3
///   router calls a `RepairRequest` addressed there a Popup — and since #1438
///   the peer-identical `HumanRouted` arm writes the ship's
///   `RepairRequestQueue`, which is what the AI repair System the rating created
///   then dispatches against. Before #1438 the queue stayed empty on this
///   seat and the rating's promise was silently broken;
/// * the advisory does NOT count that request as a demand on the person: the
///   `repair` System it is addressed to is AI-operated, so it is the AI's work,
///   and the seat reads Underused.
///
/// Nothing here hand-sets a control source on the Engineering seat. The rating
/// name and the hull TOML are the whole input.
#[test]
fn a_simplified_engineering_seat_dispatches_ai_repair_without_counting_as_human_work() {
    let mut app = seeded(WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let repair = phoenix::ship::system_registry::repair_system_id();
    let engineering = station_of(&mut app, &repair);
    // Comms is human-seeking: with its own seat empty it comes and sits at
    // whichever Station has a holder, and this case is about Engineering.
    let comms = station_of(&mut app, &phoenix::ship::system_registry::comms_system_id());
    crew_station(&mut app, &comms);
    crew_station_on_rating(&mut app, &engineering, "Simplified");
    step(&mut app, 4);

    // The mix is the HULL's, read back from the live resolver.
    assert_eq!(
        source_of(&mut app, &repair),
        ControlSource::Ai,
        "the shipped Simplified rating automates `repair`"
    );
    let siblings: Vec<SystemId> = systems_of(&mut app, &engineering)
        .into_iter()
        .filter(|id| *id != repair)
        .collect();
    assert!(
        !siblings.is_empty(),
        "the Engineering seat owns more than the repair System"
    );
    assert!(
        siblings
            .iter()
            .any(|id| source_of(&mut app, id) == ControlSource::Human),
        "and leaves the power Systems with the person"
    );
    // Which is exactly the question the Channel-3 router asks about the seat:
    // somebody is there, so a request addressed here routes to a PERSON.
    assert_eq!(
        seat_source_of(&mut app, &engineering),
        ControlSource::Human,
        "a mixed seat is a human seat to the router — the Popup arm, not the Ai arm"
    );

    // A real hit on a real System. `damage_sync` crosses `phaser-fore` into
    // Damaged and files a genuine Channel-3 `RepairRequest` at the Station that
    // holds `repair`; nothing below writes the queue by hand. Only just into the
    // tier, so a 0.5 HP/s team can finish inside the test.
    let damaged = SystemId("phaser-fore".into());
    set_source(&mut app, &damaged, ControlSource::Ai);
    let full = max_hp(&mut app, &damaged);
    damage_system(&mut app, &damaged, full * 0.70);
    let hit = system_hp(&mut app, &damaged);
    step(&mut app, LAG_STEPS);

    assert_eq!(
        repair_queue(&mut app).len(),
        1,
        "the human-routed request is recorded as a thing still owed"
    );
    // …and it is NOT a demand on the person sitting there.
    let mixed = row(&app, &engineering);
    assert_eq!(
        mixed.count, 0,
        "the repair System is AI-operated, so its request is the AI's work"
    );
    assert!(mixed.demands.is_empty(), "and shows no human evidence");
    assert_eq!(
        mixed.level,
        GmWorkloadLevel::Underused,
        "the seat itself is still a person's seat, merely under-asked"
    );

    // The AI repair the rating created now acts on it: a team leaves, crosses,
    // and heals the hit until the damage the request named is gone.
    let mut dispatched = false;
    let mut healed = false;
    for _ in 0..40 {
        step(&mut app, 30);
        dispatched |= teams_busy(&mut app);
        if repair_queue(&mut app).is_empty() {
            healed = true;
            break;
        }
    }
    assert!(
        dispatched,
        "the AI repair System dispatched a team off Idle"
    );
    assert!(
        healed,
        "and the request ended, because the damage it named did"
    );
    assert!(
        system_hp(&mut app, &damaged) > hit,
        "the hull is actually repaired, not just dequeued"
    );
    assert_eq!(
        phoenix::ship::damage::DamageTier::Operational,
        system_tier(&mut app, &damaged),
        "back to Operational"
    );

    // Throughout, the advisory never claimed this as somebody's workload.
    assert_eq!(row(&app, &engineering).count, 0);
    assert_eq!(row(&app, &engineering).level, GmWorkloadLevel::Underused);
}

/// A Station nothing can operate is neither underused nor backfilled.
#[test]
fn a_station_nothing_can_operate_reads_offline() {
    let mut app = seeded(WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let engineering = station_of(
        &mut app,
        &phoenix::ship::system_registry::repair_system_id(),
    );
    crew_station(&mut app, &engineering);
    // Comms is human-seeking: with its own seat empty it would come and sit at
    // the only crewed Station on the hull, and this case is about a Station
    // with nothing operable, not about where Comms went.
    let comms = station_of(&mut app, &phoenix::ship::system_registry::comms_system_id());
    crew_station(&mut app, &comms);
    deliver_repair_request(&mut app);
    step(&mut app, 2);
    assert!(row(&app, &engineering).level.counts_people());

    // Damage takes the whole seat out. It is neither underused (nobody is being
    // under-asked) nor backfilled (no AI is operating it either).
    disable_station(&mut app, &engineering);
    step(&mut app, 2);
    assert_eq!(row(&app, &engineering).level, GmWorkloadLevel::Offline);
    assert_eq!(row(&app, &engineering).count, 0);
    assert!(row(&app, &engineering).demands.is_empty());
}

// ── Human-seeking migration: a Station that is nobody's seat ───────────

/// A Station every one of whose AUTHORED Systems has migrated to a
/// human-seeking host is not a seat anybody holds, so the summary does not name
/// it at all — and its work is counted once, at the seat actually presenting it.
///
/// The stock Alliance Cruiser authors three of these and they are the ORDINARY
/// case, not a corner. `navigation` and `command` are human-seeking Stations
/// (`host_order = ["comms", "captain", …]` and `["captain"]`); the `comms`
/// SYSTEM is human-seeking with no `seek_order`, so with its own seat empty it
/// goes and sits at whichever seat is crewed. Each of the three authors exactly
/// one System, so reading membership from the LIVE host map left each of them
/// with the EMPTY set the moment anybody sat down — and an empty set reduces to
/// `Offline`. All three therefore read "No System here can be operated", which
/// is false, contradicts the roster pill on the same row, and contradicts
/// `GmWorkloadLevel::Offline`'s own meaning. Membership is authored; the live
/// map only says where the work is being presented.
///
/// Nothing here hand-sets a control source: every seat is taken the way a lobby
/// claim takes it, through `crew_station`, and every migration is the hull's own
/// authored seek rules doing what they do in a real game.
#[test]
fn a_station_whose_systems_have_all_migrated_is_left_out_of_the_summary() {
    let mut app = seeded(WORLD);
    let captain = StationId("captain".into());
    let comms = StationId("comms".into());
    let navigation = StationId("navigation".into());
    let command = StationId("command".into());
    let engineering = station_of(
        &mut app,
        &phoenix::ship::system_registry::repair_system_id(),
    );

    let visiting: Vec<(StationId, SystemId)> = [&navigation, &command, &comms]
        .into_iter()
        .map(|station| {
            let mut authored = authored_systems_of(&mut app, station);
            assert_eq!(
                authored.len(),
                1,
                "the cruiser gives {station:?} exactly one authored System"
            );
            (station.clone(), authored.remove(0))
        })
        .collect();

    // One person, in the Captain's chair. The hull's own seek rules send all
    // three visiting Stations' Systems to them.
    crew_station(&mut app, &captain);
    step(&mut app, 4);
    for (station, system) in &visiting {
        assert_eq!(
            station_of(&mut app, system),
            captain,
            "{system:?} is presented at the Captain's seat"
        );
        assert!(
            !named(&app, station),
            "{station:?} is nobody's seat while its only System is hosted elsewhere"
        );
    }
    assert!(named(&app, &captain), "the host seat IS a seat");
    assert_eq!(row(&app, &captain).level, GmWorkloadLevel::Underused);
    assert!(
        offline_stations(&app).is_empty(),
        "a migrated Station works perfectly well; nothing here is inoperable"
    );

    // The migrated demand is counted at the host, once.
    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_one",
        "route-one",
        "hail-one",
    );
    step(&mut app, 2);
    let host = row(&app, &captain);
    assert_eq!(host.count, 1, "the Captain is carrying the Comms traffic");
    assert_eq!(host.level, GmWorkloadLevel::Engaged);
    assert!(
        host.demands
            .iter()
            .any(|demand| demand.source == COMMS_SOURCE),
        "and the evidence says which conversation"
    );
    assert!(
        !named(&app, &comms),
        "the vacated Comms Station does not name it a second time"
    );

    // Somebody sits at Comms. Its own System comes home, so the Station is a
    // seat again and the conversation is theirs, not the Captain's.
    crew_station(&mut app, &comms);
    step(&mut app, 4);
    let comms_system = visiting
        .iter()
        .find(|(station, _)| station == &comms)
        .expect("Comms is one of the three")
        .1
        .clone();
    assert_eq!(station_of(&mut app, &comms_system), comms);
    assert!(named(&app, &comms));
    let seat = row(&app, &comms);
    assert_eq!(seat.level, GmWorkloadLevel::Engaged);
    assert_eq!(seat.count, 1);
    assert_eq!(row(&app, &captain).count, 0, "handed over, not duplicated");
    assert_eq!(row(&app, &captain).level, GmWorkloadLevel::Underused);
    // Navigation simply moved along its authored `host_order` to the Comms seat
    // and Command is still the Captain's: neither is a seat of its own.
    assert!(!named(&app, &navigation));
    assert!(!named(&app, &command));

    // Offline is reserved for a Station whose OWN authored Systems are all
    // damage-disabled, and it is the only thing in the whole summary that
    // reads it.
    crew_station(&mut app, &engineering);
    step(&mut app, 2);
    assert!(row(&app, &engineering).level.counts_people());
    disable_station(&mut app, &engineering);
    step(&mut app, 2);
    assert_eq!(row(&app, &engineering).level, GmWorkloadLevel::Offline);
    assert_eq!(offline_stations(&app), vec![engineering.0.clone()]);
}

// ── The off switch, and restore ──────────────────────────────────────────────

/// The disable silences this advisory and nothing else: the world still has a
/// live pending conversation and the ordinary Comms inbox still holds it.
#[test]
fn the_disable_silences_the_workload_advisory_alone() {
    let mut app = seeded(QUIET_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_one",
        "route-one",
        "hail-one",
    );
    step(&mut app, 4);
    assert!(
        summary(&app).is_empty(),
        "a silenced advisory publishes an empty summary, not a stale one"
    );
    assert!(
        app.world().resource::<GmWorkloadWatch>().is_empty(),
        "and banks no elapsed overload while silenced"
    );
    // The conversation it would have counted is demonstrably still live.
    assert!(messages(&app)
        .iter()
        .any(|message| message.body == "world.probe_gm_workload.offer_one"));
}

/// A paused world does not age, so a Station cannot serve its overload duration
/// while nothing is running.
///
/// The seam is the one pause actually uses — `Time<Virtual>` stops, which
/// starves the fixed accumulator — so no fixed step starts and the stopwatch has
/// nothing to count. Four hundred paused FRAMES is far more wall clock than the
/// authored duration; the boundary must still land where the authored duration
/// says it does once the world resumes.
#[test]
fn a_paused_simulation_banks_no_overload_and_resumes_on_the_authored_boundary() {
    let mut app = seeded(AUTHORED_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let comms = station_of(&mut app, &phoenix::ship::system_registry::comms_system_id());
    crew_station(&mut app, &comms);
    let ship = fleet_uuid(&mut app);

    // The authored world overloads at TWO demands held for ONE simulation
    // second — thirty fixed steps at this world's 30 Hz.
    let row_now = row(&app, &comms);
    assert_eq!(row_now.overload_count, 2);
    assert_eq!(row_now.overload_secs, 1);
    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_one",
        "route-one",
        "hail-one",
    );
    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_two",
        "route-two",
        "hail-two",
    );
    step(&mut app, 4);
    assert_eq!(row(&app, &comms).count, 2, "at the authored count");
    assert_eq!(
        row(&app, &comms).level,
        GmWorkloadLevel::Engaged,
        "at the count but not yet through the duration"
    );
    let before_tick = tick(&app);
    let banked = app
        .world()
        .resource::<GmWorkloadWatch>()
        .ticks(&ship, &comms.0);
    assert!(banked > 0, "the Station is mid-duration");
    assert!(banked < HZ as u64, "and has not served it yet");

    app.world_mut().insert_resource(SimulationPaused(true));
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    for _ in 0..400 {
        app.update();
    }
    assert_eq!(
        tick(&app),
        before_tick,
        "a paused world takes no fixed step"
    );
    assert_eq!(
        app.world()
            .resource::<GmWorkloadWatch>()
            .ticks(&ship, &comms.0),
        banked,
        "and therefore banks no elapsed overload"
    );
    assert_eq!(
        row(&app, &comms).level,
        GmWorkloadLevel::Engaged,
        "four hundred paused frames do not overload anybody"
    );

    app.world_mut().insert_resource(SimulationPaused(false));
    app.world_mut().resource_mut::<Time<Virtual>>().unpause();

    // One step short of the authored duration is still Engaged...
    step(&mut app, HZ as u64 - banked - 1);
    assert_eq!(
        row(&app, &comms).level,
        GmWorkloadLevel::Engaged,
        "the boundary is exact, and this is one step short of it"
    );
    // ...and the step that completes it is the one that says Overloaded.
    step(&mut app, 1);
    assert_eq!(
        app.world()
            .resource::<GmWorkloadWatch>()
            .ticks(&ship, &comms.0),
        HZ as u64,
        "exactly the authored duration in fixed steps, pause included at zero"
    );
    assert_eq!(row(&app, &comms).level, GmWorkloadLevel::Overloaded);
}

/// Elapsed overload is history, so it survives a capture and restore rather
/// than forgiving a Station because somebody happened to save.
#[test]
fn the_sustained_overload_stopwatch_survives_capture_and_restore() {
    let mut app = seeded(AUTHORED_WORLD);
    crew_everything(&mut app, ControlSource::Human);
    let comms = station_of(&mut app, &phoenix::ship::system_registry::comms_system_id());
    crew_station(&mut app, &comms);
    let ship = fleet_uuid(&mut app);
    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_one",
        "route-one",
        "hail-one",
    );
    open_hail(
        &mut app,
        "world.probe_gm_workload.speaker_two",
        "route-two",
        "hail-two",
    );
    step(&mut app, 10);
    let banked = app
        .world()
        .resource::<GmWorkloadWatch>()
        .ticks(&ship, &comms.0);
    assert!(banked > 0, "the Station is mid-duration");

    let snapshot = phoenix::snapshot::capture(app.world_mut());
    assert_eq!(snapshot.gm_station_workload.ticks(&ship, &comms.0), banked);

    // Wipe it the way a fresh peer would have it, then restore.
    app.world_mut().insert_resource(GmWorkloadWatch::default());
    assert_eq!(
        app.world()
            .resource::<GmWorkloadWatch>()
            .ticks(&ship, &comms.0),
        0
    );
    phoenix::snapshot::restore(app.world_mut(), &snapshot);
    assert_eq!(
        app.world()
            .resource::<GmWorkloadWatch>()
            .ticks(&ship, &comms.0),
        banked,
        "a resumed Station keeps the wait it had served"
    );
}
