use super::*;
use crate::core::messages::{CoordinationAddress, SystemId};
use crate::server_app::Ship;
use crate::ship::components::LastSystemTiers;
use crate::ship::control_source::ControlSource;
use crate::ship::test_support::*;

fn test_coordination_presentation() -> crate::core::messages::CoordinationPresentation {
    crate::core::messages::CoordinationPresentation::new(
        "test.coordination.title",
        "test.coordination.body",
    )
}

// ── Issue #684: Destroyed-tier alerts to Captain ─────────────────────────

#[derive(Resource, Default)]
struct CoordEnqueueBox(Vec<CoordinationEnqueue>);

fn collect_coord(
    mut reader: MessageReader<CoordinationEnqueue>,
    mut box_: ResMut<CoordEnqueueBox>,
) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

fn drain_coord(app: &mut App) -> Vec<CoordinationEnqueue> {
    let msgs = app.world().resource::<CoordEnqueueBox>().0.clone();
    app.world_mut().resource_mut::<CoordEnqueueBox>().0.clear();
    msgs
}

fn coord_test_app() -> App {
    let mut app = test_app();
    app.init_resource::<CoordEnqueueBox>()
        .add_systems(PostUpdate, collect_coord);
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(LastSystemTiers::default());
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
    for mut cfg in q.iter_mut(app.world_mut()) {
        cfg.0.coordination_lag_secs = 0.0;
    }
    app
}

fn set_captain_control_source(app: &mut App, source: ControlSource) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
    for mut cs in q.iter_mut(app.world_mut()) {
        cs.0.set(crate::ship::system_registry::captain_system_id(), source);
    }
}

#[test]
fn destroyed_crossing_emits_alert_to_captain() {
    let mut app = coord_test_app();
    let tact_sid = SystemId("tactical".into());
    set_console_hp_direct(&mut app, tact_sid, 0.0);
    tick(&mut app);
    let emitted = drain_coord(&mut app);
    let alerts: Vec<_> = emitted
        .iter()
        .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
        .collect();
    assert_eq!(alerts.len(), 1, "expected exactly one Alert");
    assert_eq!(
        alerts[0].address,
        crate::core::messages::CoordinationAddress::Station(StationId("captain".into())),
        "Alert must address the Captain Station"
    );
    assert_eq!(alerts[0].sender_label, "tactical");
    assert_eq!(
        &alerts[0].payload,
        &CoordinationPayload::Alert {
            title: "coordination.system_destroyed.title".into(),
            body: "coordination.system_destroyed.body".into(),
        },
        "the semantic Alert no longer carries Rust-composed destroyed-System English"
    );
    assert_eq!(
        alerts[0].presentation.title,
        "coordination.system_destroyed.title"
    );
    assert_eq!(
        alerts[0].presentation.body,
        "coordination.system_destroyed.body"
    );
    let title_label = alerts[0]
        .presentation
        .title_params
        .get("label")
        .expect("destroyed title must name the authored System");
    assert_eq!(
        alerts[0].presentation.body_params.get("label"),
        Some(title_label),
        "title and body use the same authored hull display label"
    );
}

#[test]
fn non_destroyed_crossing_does_not_emit_alert() {
    let mut app = coord_test_app();
    let tact_sid = SystemId("tactical".into());
    set_console_hp_direct(&mut app, tact_sid, 5.0);
    tick(&mut app);
    let emitted = drain_coord(&mut app);
    let alerts: Vec<_> = emitted
        .iter()
        .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
        .collect();
    assert_eq!(alerts.len(), 0, "no Alert for non-Destroyed crossing");
    assert!(
        emitted
            .iter()
            .any(|e| matches!(&e.payload, CoordinationPayload::RepairRequest { .. })),
        "expected a RepairRequest for Disabled crossing"
    );
}

#[test]
fn destroyed_alert_fires_once() {
    let mut app = coord_test_app();
    let tact_sid = SystemId("tactical".into());
    set_console_hp_direct(&mut app, tact_sid, 0.0);
    tick(&mut app);
    let emitted_t1 = drain_coord(&mut app);
    assert_eq!(
        emitted_t1
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
            .count(),
        1,
        "first tick must emit Alert"
    );
    tick(&mut app);
    let emitted_t2 = drain_coord(&mut app);
    assert_eq!(
        emitted_t2
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
            .count(),
        0,
        "second tick must not re-emit Alert (fire-once)"
    );
}

#[test]
fn destroyed_alert_refires_after_restore_and_re_destroy() {
    let mut app = coord_test_app();
    let tact_sid = SystemId("tactical".into());
    set_console_hp_direct(&mut app, tact_sid.clone(), 0.0);
    tick(&mut app);
    assert_eq!(
        drain_coord(&mut app)
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
            .count(),
        1,
        "first destroy must emit Alert"
    );
    set_console_hp_direct(&mut app, tact_sid.clone(), 25.0);
    tick(&mut app);
    drain_coord(&mut app);
    set_console_hp_direct(&mut app, tact_sid, 0.0);
    tick(&mut app);
    assert_eq!(
        drain_coord(&mut app)
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
            .count(),
        1,
        "re-destroy after restore must emit Alert again"
    );
}

/// Routing test helper: creates a test app without `collect_coord` (to avoid
/// interfering with the coordination event readers) and sets lag to 0.
fn routing_test_app() -> App {
    let mut app = test_app();
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(LastSystemTiers::default());
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
    for mut cfg in q.iter_mut(app.world_mut()) {
        cfg.0.coordination_lag_secs = 0.0;
    }
    app
}

#[test]
fn destroyed_alert_consumed_by_ai_captain() {
    let mut app = routing_test_app();
    start_game_with_helm_and_science(&mut app);
    set_captain_control_source(&mut app, ControlSource::Ai);
    let tact_sid = SystemId("tactical".into());
    set_console_hp_direct(&mut app, tact_sid, 0.0);
    tick(&mut app);
    tick(&mut app);
    let outbox = app.world().resource::<crate::lobby::LobbyOutbox>();
    let popups: Vec<_> = outbox
        .0
        .iter()
        .filter(|(_, msg)| {
            matches!(
                msg,
                crate::core::messages::ServerMessage::CoordinationPopup { .. }
            )
        })
        .collect();
    assert!(
        popups.is_empty(),
        "AI Captain must not produce CoordinationPopup; got {} popup(s)",
        popups.len()
    );
}

#[test]
fn destroyed_alert_shows_popup_for_human_captain() {
    let mut app = routing_test_app();
    start_game_with_helm_and_science(&mut app);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(SystemId("tactical".into()), ControlSource::Ai);
        }
    }
    let tact_sid = SystemId("tactical".into());
    set_console_hp_direct(&mut app, tact_sid, 0.0);
    // Tick 1: detect_damage_tier_crossings writes CoordinationEnqueue
    //         into the message send buffer.
    // Tick 2: buffer-swap → handle_coordination_enqueue reads and enqueues
    //         to CoordinationQueue with due_time = now + 0.
    //         process_coordination_lag reads due messages and dispatches
    //         a CoordinationPopup to the LobbyOutbox.
    // Tick 3: consumes the popup and/or allows the broadcast to flush.
    tick(&mut app);
    tick(&mut app);
    tick(&mut app);
    let outbox = app.world().resource::<crate::lobby::LobbyOutbox>();
    let has_popup = outbox.0.iter().any(|(_, msg)| {
        matches!(
            msg,
            crate::core::messages::ServerMessage::CoordinationPopup { .. }
        )
    });
    assert!(
        has_popup,
        "Human Captain must receive a CoordinationPopup for destroyed system"
    );
}

#[test]
fn a_fully_offline_station_consumes_without_popup_or_ai_handoff() {
    let mut app = routing_test_app();
    start_game_with_helm_and_science(&mut app);
    let ship = find_ship_entity(&mut app);
    let station = StationId("captain".into());
    let owned: Vec<_> = app
        .world()
        .get::<ShipConfigComponent>(ship)
        .expect("ship config")
        .0
        .systems_for_station(&station)
        .map(|system| system.id.clone())
        .collect();
    assert!(!owned.is_empty(), "fixture Station owns Systems");
    {
        let mut sources = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(ship)
            .expect("ship control sources");
        for system in owned {
            sources.0.set_offline(system, true);
        }
    }

    enqueue_coordination(
        &mut app,
        ControlSource::Ai,
        CoordinationAddress::Station(station),
        CoordinationPayload::Alert {
            title: "test.alert.title".into(),
            body: "test.alert.body".into(),
        },
    );
    tick(&mut app);
    tick(&mut app);

    assert_eq!(coordination_popups(&app), 0);
}

#[test]
fn station_control_is_resolved_live_after_enqueue_not_frozen_in_the_queue() {
    let mut app = routing_test_app();
    start_game_with_helm_and_science(&mut app);
    let ship = find_ship_entity(&mut app);
    set_captain_control_source(&mut app, ControlSource::Ai);
    app.world_mut()
        .get_mut::<CoordinationQueue>(ship)
        .expect("ship Coordination queue")
        .0
        .enqueue(coordination::QueuedCoordination {
            sender_origin: ControlSource::Ai,
            address: CoordinationAddress::Station(StationId("captain".into())),
            payload: CoordinationPayload::Alert {
                title: "test.alert.title".into(),
                body: "test.alert.body".into(),
            },
            presentation: test_coordination_presentation(),
            sender_label: "station.sensors.name".into(),
            due_time: 0.0,
        });

    // Ownership changes after enqueue but before the due delivery is routed.
    set_captain_control_source(&mut app, ControlSource::Human);
    tick(&mut app);

    assert_eq!(
        coordination_popups(&app),
        1,
        "the live human holder receives the delayed advisory; queued AI state is not frozen"
    );
    let (presentation, to_label) = app
        .world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .find_map(|(_, message)| match message {
            crate::core::messages::ServerMessage::CoordinationPopup {
                presentation,
                to_label,
                ..
            } => Some((presentation, to_label)),
            _ => None,
        })
        .expect("delayed popup");
    assert_eq!(presentation, &test_coordination_presentation());
    assert_eq!(to_label, "station.captain.name");
}

// ── Issue #737: the repair-request popup is subject to the same boundary ──
//
// `CoordinationPayload::RepairRequest` is addressed to the Station that owns
// Repair, which resolves to the Engineering holder. Before the gate, every worsening tier
// crossing handed Engineering the exact HP deficit of an arbitrary non-Core
// system with no team dispatched and no travel elapsed — the projection's
// boundary, walked around through the coordination bus.

/// Start a game with a human on the Repair station — the station that owns
/// the `repair` system on the battleship, i.e. Engineering in the role
/// sense, and therefore the recipient of every `RepairRequest` popup.
fn start_game_with_engineer(app: &mut App) {
    for (token, name, station) in [
        ("captain", "Alice", "Captain"),
        ("helm", "Hikaru", "Helm"),
        ("engineer", "Scotty", "Repair"),
    ] {
        push(
            app,
            token,
            ClientMessage::Identify {
                token: token.into(),
                name: name.into(),
            },
        );
        tick(app);
        push(
            app,
            token,
            ClientMessage::SelectStation {
                station: station.into(),
            },
        );
        tick(app);
    }
    for token in ["captain", "helm", "engineer"] {
        push(app, token, ClientMessage::SetReady { ready: true });
    }
    tick(app);
    assert_eq!(
        app.world()
            .resource::<Sessions>()
            .0
            .holder_for_station(&crate::core::messages::StationId("repair".into())),
        Some("engineer"),
        "test setup must seat a human on the station that owns `repair`"
    );
}

/// Give the ship a hull whose entries are *declared* systems, so a tier
/// crossing resolves to a real owning station. `test_app`'s default hull
/// holds the retired coarse ids, which resolve to no `[[system]]` and would
/// therefore land in the ownerless Core bucket — the one case #737 lets
/// through.
fn give_ship_hull(app: &mut App, entries: &[(&str, f32)]) {
    let hull = crate::ship::damage::SystemHull::from_config(
        &entries
            .iter()
            .map(|(id, hp)| (SystemId((*id).into()), *hp))
            .collect::<Vec<_>>(),
    );
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::EntitySystemHull(hull));
}

/// Put `system_id` under AI control. `route_coordination` only raises a
/// popup for an AI sender talking to a human target, which is the shape the
/// leak had: an AI-run station reporting damage to a human Engineering.
fn set_ai(app: &mut App, system_id: &SystemId) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
    for mut cs in q.iter_mut(app.world_mut()) {
        cs.0.set(system_id.clone(), ControlSource::Ai);
    }
}

/// The deficit carried by the delivered `RepairRequest` popup, if any.
fn repair_popup_deficits(app: &App) -> Vec<Option<f32>> {
    app.world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .filter_map(|(_, msg)| match msg {
            crate::core::messages::ServerMessage::CoordinationPopup {
                payload: CoordinationPayload::RepairRequest { deficit, .. },
                ..
            } => Some(*deficit),
            _ => None,
        })
        .collect()
}

fn repair_popup_presentations(app: &App) -> Vec<&crate::core::messages::CoordinationPresentation> {
    app.world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .filter_map(|(_, msg)| match msg {
            crate::core::messages::ServerMessage::CoordinationPopup {
                payload: CoordinationPayload::RepairRequest { .. },
                presentation,
                ..
            } => Some(presentation),
            _ => None,
        })
        .collect()
}

fn assert_repair_presentations_are_coarse(app: &App) {
    let presentations = repair_popup_presentations(app);
    assert!(
        !presentations.is_empty(),
        "fixture must deliver Repair presentation"
    );
    for presentation in presentations {
        assert_eq!(presentation.title, "coordination.repair.title");
        assert_eq!(presentation.body, "");
        assert_eq!(
            presentation
                .title_params
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["label"],
            "Repair presentation may name only the coarse Station; protected deficit stays out"
        );
        assert!(presentation.body_params.is_empty());
    }
}

/// Put a repair team physically on site at `system_id` before the crossing.
fn place_team_on_site(app: &mut App, system_id: &SystemId) {
    use crate::modifiers::repair_teams::RepairTeams;
    let mut teams = RepairTeams::new(1);
    let mut scratch = crate::ship::damage::SystemHull::from_config(&[(system_id.clone(), 100.0)]);
    scratch.set_hp(system_id, 10.0);
    teams.dispatch(0, system_id.clone(), system_id.0.clone());
    // Travel completes → `Repairing`, which is what `on_site_systems()`
    // counts. Same state machine the wire projection reads.
    teams.tick(60.0, &mut scratch, None);
    assert!(
        teams.on_site_systems().any(|s| s == system_id),
        "test setup must actually put the team on site"
    );
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::console::repair::server::ShipRepairTeams(teams));
}

// ── Issue #873: a human-operated station feeds the backfilled AI ─────────
//
// Rule 6 on the coordination bus. A coordination fact is derived from
// authoritative system state and emitted regardless of who holds the
// sending console; `sender_origin` is stamped afterwards and used for one
// thing only — picking Consume / Popup / Suppress at delivery time. The
// emit-side halves of this live in `ship::sensors` and `console_ai::server`;
// what these tests cover is the delivery side, including the backfilled
// Tactical that the router could not see at all before this issue.

/// Seat a human on Sensors and on Tactical-adjacent nothing else, so the
/// remaining stations backfill to AI. Modelled on `start_game_with_engineer`.
fn start_game_with_sensors_officer(app: &mut App) {
    for (token, name, station) in [
        ("captain", "Alice", "Captain"),
        ("sensors", "Spock", "Sensors"),
    ] {
        push(
            app,
            token,
            ClientMessage::Identify {
                token: token.into(),
                name: name.into(),
            },
        );
        tick(app);
        push(
            app,
            token,
            ClientMessage::SelectStation {
                station: station.into(),
            },
        );
        tick(app);
    }
    for token in ["captain", "sensors"] {
        push(app, token, ClientMessage::SetReady { ready: true });
    }
    tick(app);
    assert_eq!(
        app.world()
            .resource::<Sessions>()
            .0
            .holder_for_station(&crate::core::messages::StationId("sensors".into())),
        Some("sensors"),
        "test setup must seat a human on Sensors"
    );
}

fn seat_human_on_tactical(app: &mut App) {
    push(
        app,
        "tactical",
        ClientMessage::Identify {
            token: "tactical".into(),
            name: "Uhura".into(),
        },
    );
    tick(app);
    push(
        app,
        "tactical",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    tick(app);
}

/// Put every tactical FINE system (phaser banks, torpedo tubes, the
/// magazine) on `source` — the set `any_tactical_system_operates_ai`
/// inspects, moved as one, which is what claiming or vacating the Tactical
/// station does.
fn set_tactical_fine_systems(app: &mut App, source: ControlSource) {
    let ids: Vec<SystemId> = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipConfigComponent, With<Ship>>();
        let cfg = q.single(app.world()).expect("ship config").clone();
        cfg.0
            .systems
            .iter()
            .filter(|s| {
                matches!(
                    s.kind.as_str(),
                    crate::ship::system_registry::PHASER_BANK_KIND
                        | crate::ship::system_registry::TORPEDO_TUBE_KIND
                        | crate::ship::system_registry::TORPEDO_MAGAZINE_KIND
                )
            })
            .map(|s| s.id.clone())
            .collect()
    };
    assert!(
        !ids.is_empty(),
        "the shipped hull must declare tactical fine systems for this fixture to mean anything"
    );
    for id in ids {
        set_fine_control_source(app, id, source);
    }
}

/// The shape an unmanned Tactical station backfills to.
fn backfill_tactical_to_ai(app: &mut App) {
    set_tactical_fine_systems(app, ControlSource::Ai);
}

/// Give the ship the two components the Tactical hint path needs, which
/// `test_app` does not spawn: the landing slot and the thing that moves.
fn give_ship_tactical_frequency_surface(app: &mut App) {
    let ship = find_ship_entity(app);
    app.world_mut().entity_mut(ship).insert((
        crate::ship_plugin::PendingTacticalFrequencyHint::default(),
        crate::ship::state::ShipPhaserFrequency(0.1),
    ));
}

/// Register Tactical's receiving pipeline in the sets production puts it in,
/// so the tick boundary these tests reason about is the real one: the lag
/// router emits a typed handoff in `Modifiers`, Tactical's receiver lands it
/// later in that set, and the applier reads it in the FOLLOWING tick's `Input`.
fn add_tactical_hint_applier(app: &mut App) {
    // `FixedUpdate` (issue #895): both systems join the SimSet chain in the
    // schedule it lives in, preserving the one-tick handover window between
    // the receiver (Modifiers) and next tick's Input.
    app.add_systems(
        FixedUpdate,
        (
            crate::console::weapons::receive_tactical_coordination
                .in_set(crate::sim_sets::SimSet::Modifiers)
                .after(process_coordination_lag),
            crate::console::weapons::apply_tactical_frequency_hint
                .in_set(crate::sim_sets::SimSet::Input),
        ),
    );
}

/// Arm the REAL Sensors emitter (`tick_sensors_frequency_hint`) on this
/// ship, rather than hand-writing the `CoordinationEnqueue` it would have
/// produced.
///
/// Three pieces of authoritative state, and nothing about who is sitting
/// where: the hull is made low-fidelity (that emitter serves ships without
/// `AiHighFidelity`; the high-fidelity twin
/// `tick_frequency_hint_high_fidelity` reads the same facts through the
/// operator reaction-delay model), the viewscreen's frozen Combat Lock names
/// a target, and that target carries a shield frequency to read.
///
/// Ordered `.before(handle_coordination_enqueue)` so the emit lands on the
/// bus the same tick it is written rather than at the mercy of intra-set
/// ordering.
fn arm_real_sensors_frequency_emitter(app: &mut App, target_uuid: &str, frequency: f32) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ai::server::AiHighFidelity>()
        .insert(crate::ship::sensors::SensorsFrequencyState::default());
    {
        let mut blackboards = app
            .world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
            .expect("ship carries system blackboards");
        blackboards.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            crate::core::messages::SystemBlackboard::Viewscreen(
                crate::core::messages::ViewscreenBlackboard {
                    combat_lock: Some(target_uuid.to_string()),
                    ..Default::default()
                },
            ),
        );
    }
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(target_uuid.to_string()),
        crate::ship::shields::ShipShields(
            crate::weapons::shield::ShieldSystem::default(),
            frequency,
        ),
    ));
    // `FixedUpdate` (issue #895): the emitter joins the SimSet chain in
    // the schedule the chain lives in, so its `.before` edge stays real.
    app.add_systems(
        FixedUpdate,
        crate::ship::sensors::tick_sensors_frequency_hint
            .in_set(crate::sim_sets::SimSet::Input)
            .before(handle_coordination_enqueue),
    );
}

/// Arm the REAL high-fidelity Sensors emitter — the one the PLAYER SHIP
/// runs, and therefore the one the issue is actually about.
///
/// `arm_real_sensors_frequency_emitter` above removes `AiHighFidelity` to
/// reach `tick_sensors_frequency_hint`. That is a real path, but it is the
/// path a DEMOTED NPC hull takes: `server_app::spawn_game_start_entities`
/// gives `LocalShip` `ai_high_fidelity_components()` at spawn and
/// `ai::server::lod_ai_ships` never evaluates `LocalShip`, so the player
/// hull is permanently high-fidelity. A human on the player ship's Sensors
/// is served by `tick_frequency_hint_high_fidelity` — which until this
/// fixture existed was only ever exercised in a bare-`App` fixture in
/// `console_ai::server` that stops at a collector box and never touches the
/// bus, the router, or the applier.
///
/// So: the marker STAYS, and the emitter is registered under the same
/// `ai_tick_ready` cadence production gates it with, so what gets pinned end
/// to end is the chain the player ship really takes.
fn arm_real_high_fidelity_sensors_emitter(app: &mut App, target_uuid: &str, frequency: f32) {
    let ship = find_ship_entity(app);
    assert!(
        app.world()
            .get::<crate::ai::server::AiHighFidelity>(ship)
            .is_some(),
        "this fixture is the PLAYER-ship chain: the hull must keep AiHighFidelity, \
         otherwise it is silently testing the demoted-NPC emitter instead"
    );
    {
        let mut blackboards = app
            .world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
            .expect("ship carries system blackboards");
        blackboards.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            crate::core::messages::SystemBlackboard::Viewscreen(
                crate::core::messages::ViewscreenBlackboard {
                    combat_lock: Some(target_uuid.to_string()),
                    ..Default::default()
                },
            ),
        );
    }
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(target_uuid.to_string()),
        crate::ship::shields::ShipShields(
            crate::weapons::shield::ShieldSystem::default(),
            frequency,
        ),
    ));
    crate::ai::cadence::register_ai_cadence(app);
    app.add_systems(
        Update,
        crate::console_ai::server::tick_frequency_hint_high_fidelity
            .in_set(crate::sim_sets::SimSet::Input)
            .before(handle_coordination_enqueue)
            .run_if(crate::ai::cadence::ai_tick_ready),
    );
}

/// How many emitter runs the AUTHORED reaction delay takes, read from the
/// same two authored defaults the emitter itself reads (rule 11: no literal
/// tick count here — an authored change must move the fixture with it).
fn authored_reaction_delay_runs() -> usize {
    let delay = crate::ship::sensors::SensorsAiConfigResource::default().frequency_hint_delay_secs;
    let hz = crate::entities::config::GlobalConfig::default().ai_tick_hz;
    (delay * hz).ceil() as usize
}

fn ship_phaser_frequency(app: &mut App) -> f32 {
    let ship = find_ship_entity(app);
    app.world()
        .get::<crate::ship::state::ShipPhaserFrequency>(ship)
        .expect("ship carries a phaser frequency")
        .0
}

fn coordination_popups(app: &App) -> usize {
    app.world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .filter(|(_, msg)| {
            matches!(
                msg,
                crate::core::messages::ServerMessage::CoordinationPopup { .. }
            )
        })
        .count()
}

fn enqueue_coordination(
    app: &mut App,
    sender_origin: ControlSource,
    address: CoordinationAddress,
    payload: CoordinationPayload,
) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .resource_mut::<Messages<CoordinationEnqueue>>()
        .write(CoordinationEnqueue {
            source_entity: ship,
            sender_origin,
            address,
            payload,
            presentation: test_coordination_presentation(),
            sender_label: "Sensors".into(),
            sender_system: crate::core::messages::SystemId(String::new()),
        });
}

fn drain_chatter(app: &mut App) -> Vec<AiChatterEvent> {
    app.world_mut()
        .resource_mut::<Messages<AiChatterEvent>>()
        .drain()
        .collect()
}

#[derive(Resource, Default)]
struct DeliveredCoordinationBox(Vec<DeliveredCoordination>);

fn collect_delivered_coordination(
    mut reader: MessageReader<DeliveredCoordination>,
    mut delivered: ResMut<DeliveredCoordinationBox>,
) {
    delivered.0.extend(reader.read().cloned());
}

#[test]
fn delayed_ai_station_delivery_emits_typed_handoff_for_the_owning_module() {
    let mut app = routing_test_app();
    start_game_with_sensors_officer(&mut app);
    backfill_tactical_to_ai(&mut app);
    give_ship_tactical_frequency_surface(&mut app);
    app.init_resource::<DeliveredCoordinationBox>().add_systems(
        FixedUpdate,
        collect_delivered_coordination
            .in_set(crate::sim_sets::SimSet::Modifiers)
            .after(process_coordination_lag),
    );
    let address = CoordinationAddress::Station(StationId(
        crate::ship::system_registry::TACTICAL_STATION_ID.into(),
    ));

    enqueue_coordination(
        &mut app,
        ControlSource::Human,
        address.clone(),
        CoordinationPayload::FrequencyHint { frequency: 0.83 },
    );
    tick(&mut app);

    let ship = find_ship_entity(&mut app);
    let delivered = &app.world().resource::<DeliveredCoordinationBox>().0;
    assert_eq!(
        delivered.len(),
        1,
        "one delayed AI delivery crosses the seam"
    );
    assert_eq!(delivered[0].source_entity, ship);
    assert_eq!(delivered[0].address, address);
    assert_eq!(
        delivered[0].payload,
        CoordinationPayload::FrequencyHint { frequency: 0.83 },
        "the owning Tactical module receives the typed value unchanged"
    );
    assert_eq!(
        delivered[0].presentation,
        test_coordination_presentation(),
        "the owning module receives the producer presentation unchanged beside the typed value"
    );
}

#[test]
fn tactical_receiver_consumes_a_delivered_frequency_hint_without_router_state_mutation() {
    let mut app = routing_test_app();
    add_tactical_hint_applier(&mut app);
    start_game_with_sensors_officer(&mut app);
    backfill_tactical_to_ai(&mut app);
    give_ship_tactical_frequency_surface(&mut app);
    let ship = find_ship_entity(&mut app);

    app.world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .write(DeliveredCoordination {
            source_entity: ship,
            address: CoordinationAddress::Station(StationId(
                crate::ship::system_registry::TACTICAL_STATION_ID.into(),
            )),
            payload: CoordinationPayload::FrequencyHint { frequency: 0.61 },
            presentation: test_coordination_presentation(),
        });
    tick(&mut app);

    assert_eq!(
        app.world()
            .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
            .expect("fixture carries Tactical's pending slot")
            .0,
        Some(0.61),
        "Tactical owns the behavior behind DeliveredCoordination"
    );
}

/// AC5, end to end in ONE app. A human sits at Sensors; Tactical is unmanned
/// and backfilled to AI. The ship's own Sensors emitter derives a frequency
/// advisory from authoritative state, the bus routes it, and the AI running
/// the guns acts on it.
///
/// Nothing is hand-written here: the chain starts at the REAL emitter
/// `ship::sensors::tick_sensors_frequency_hint`, armed only with ship state
/// (a viewscreen Combat Lock on a target whose shields carry 0.83), and ends
/// at the phaser frequency. A stub `CoordinationEnqueue` would have proved
/// the routing and reaction halves while leaving the half this issue
/// actually changed — whether a human-held Sensors console emits at all —
/// asserted only in a different app, in a different module.
///
/// Before #873 this produced nothing at all, for either of two reasons:
/// the emitter stood down on a human-held console, and `SystemId("tactical")`
/// is a station key with no registered `[[system]]`, so it resolved to the
/// default `Human` policy no matter who was really running the guns — and a
/// human-origin message to a "human" target is `Suppress`. The AI Tactical
/// could not be told anything by the human sitting three feet away from it.
///
/// The origin is read back off the ship's LIVE control sources rather than
/// written as a literal, so the test would stop being about a human the
/// moment seating one stopped making Sensors human-held.
#[test]
fn human_sensors_advisory_reaches_and_moves_a_backfilled_tactical() {
    let mut app = routing_test_app();
    add_tactical_hint_applier(&mut app);
    start_game_with_sensors_officer(&mut app);
    backfill_tactical_to_ai(&mut app);
    give_ship_tactical_frequency_surface(&mut app);
    arm_real_sensors_frequency_emitter(&mut app, "harrow-raider-1", 0.83);

    assert_eq!(
        get_ship_control_sources(&mut app)
            .0
            .source_for(&crate::ship::system_registry::sensors_system_id()),
        ControlSource::Human,
        "the fixture must actually leave Sensors in human hands"
    );

    // Tick 1: tick_sensors_frequency_hint (Input) reads the Combat Lock and
    //         the target's shield frequency and writes CoordinationEnqueue →
    //         handle_coordination_enqueue queues it (lag 0) →
    //         process_coordination_lag (Modifiers) consumes it into the
    //         pending slot, because Tactical operates AI.
    // Tick 2: apply_tactical_frequency_hint (Input) folds it into the guns.
    tick(&mut app);
    tick(&mut app);

    let frequency = ship_phaser_frequency(&mut app);
    assert!(
        (frequency - 0.83).abs() < f32::EPSILON,
        "a backfilled Tactical must act on the human Sensors officer's advisory, and \
         the advisory must come from the ship's own emitter reading the locked target's \
         shields; phaser frequency is {frequency}, expected 0.83"
    );
    assert_eq!(
        coordination_popups(&app),
        0,
        "an advisory consumed by an AI station must not also raise a popup"
    );
}

/// AC3 (issue #1105): a Spectator's `SendCoordination` is dropped, while a
/// registered non-spectator's is enqueued. A large coordination lag keeps
/// the enqueued item pending so the queue length is observable before
/// delivery. Nothing is stubbed — the real `handle_coordination_enqueue`
/// runs under `ShipPlugin`, reading the live `Sessions` role that
/// `SetSpectator` set.
#[test]
fn spectator_send_coordination_is_dropped_but_crew_is_queued() {
    fn gate_app() -> App {
        let mut app = test_app();
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
        for mut cfg in q.iter_mut(app.world_mut()) {
            cfg.0.coordination_lag_secs = 100.0;
        }
        app
    }
    fn queue_len(app: &mut App) -> usize {
        let ship = find_ship_entity(app);
        app.world()
            .entity(ship)
            .get::<CoordinationQueue>()
            .expect("LocalShip carries a CoordinationQueue")
            .0
            .len()
    }
    let address = crate::core::messages::CoordinationAddress::Station(StationId(
        crate::ship::system_registry::TACTICAL_STATION_ID.into(),
    ));
    let payload = CoordinationPayload::FrequencyHint { frequency: 0.5 };

    // Registered non-spectator: enqueued.
    let mut app = gate_app();
    push(
        &mut app,
        "crew",
        ClientMessage::Identify {
            token: "crew".into(),
            name: "Officer".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "crew",
        ClientMessage::SendCoordination {
            address: address.clone(),
            payload: payload.clone(),
            presentation: test_coordination_presentation(),
        },
    );
    tick(&mut app);
    assert_eq!(
        queue_len(&mut app),
        1,
        "a registered non-spectator's coordination is queued"
    );

    // Spectator: dropped.
    let mut app = gate_app();
    push(
        &mut app,
        "spec",
        ClientMessage::Identify {
            token: "spec".into(),
            name: "Watcher".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "spec",
        ClientMessage::SetSpectator { spectator: true },
    );
    tick(&mut app);
    push(
        &mut app,
        "spec",
        ClientMessage::SendCoordination {
            address,
            payload,
            presentation: test_coordination_presentation(),
        },
    );
    tick(&mut app);
    assert_eq!(
        queue_len(&mut app),
        0,
        "a spectator's coordination is dropped"
    );
}

#[test]
fn only_ai_to_ai_coordination_reaches_viewscreen_chatter() {
    let mut human_sender = routing_test_app();
    start_game_with_sensors_officer(&mut human_sender);
    backfill_tactical_to_ai(&mut human_sender);
    give_ship_tactical_frequency_surface(&mut human_sender);
    enqueue_coordination(
        &mut human_sender,
        ControlSource::Human,
        CoordinationAddress::Station(StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into(),
        )),
        CoordinationPayload::FrequencyHint { frequency: 0.83 },
    );
    tick(&mut human_sender);
    tick(&mut human_sender);
    assert!(
        drain_chatter(&mut human_sender).is_empty(),
        "human→AI coordination belongs to the AI receiver, not viewscreen chatter"
    );

    let mut ai_sender = routing_test_app();
    start_game_with_sensors_officer(&mut ai_sender);
    backfill_tactical_to_ai(&mut ai_sender);
    give_ship_tactical_frequency_surface(&mut ai_sender);
    enqueue_coordination(
        &mut ai_sender,
        ControlSource::Ai,
        CoordinationAddress::Station(StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into(),
        )),
        CoordinationPayload::FrequencyHint { frequency: 0.83 },
    );
    tick(&mut ai_sender);
    tick(&mut ai_sender);
    let chatter = drain_chatter(&mut ai_sender);
    assert_eq!(
        chatter.len(),
        1,
        "AI→AI coordination remains visible on the viewscreen"
    );
    // Task 3: the destination is named by the owning Station alone, as a
    // `station.<id>.name` id — never the raw system key and never a
    // station+system composite. The target here is the Tactical station.
    assert_eq!(
        chatter[0].to_label, "station.tactical.name",
        "the viewscreen names the target STATION, not the bare system key"
    );
    assert_eq!(
        chatter[0].presentation,
        test_coordination_presentation(),
        "the Viewscreen receives the same producer-owned presentation that crossed the lag queue"
    );
    assert!(
        !chatter[0].to_label.contains(' '),
        "a station addressee carries no 'Station System' composite, got {:?}",
        chatter[0].to_label
    );
}

/// Task 2, end to end: an AI system speaking on channel-3 is addressed FROM
/// the station that owns it, resolved at enqueue from `sender_system` — the
/// viewscreen `from_label` is a `station.<id>.name` id, never the bare
/// `chatter.sender.*` system label.
#[test]
fn a_senders_system_is_addressed_as_its_owning_station() {
    let mut app = routing_test_app();
    start_game_with_sensors_officer(&mut app);
    backfill_tactical_to_ai(&mut app);
    give_ship_tactical_frequency_surface(&mut app);
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .resource_mut::<Messages<CoordinationEnqueue>>()
        .write(CoordinationEnqueue {
            source_entity: ship,
            sender_origin: ControlSource::Ai,
            address: crate::core::messages::CoordinationAddress::Station(StationId(
                crate::ship::system_registry::TACTICAL_STATION_ID.into(),
            )),
            payload: CoordinationPayload::FrequencyHint { frequency: 0.83 },
            presentation: crate::core::messages::CoordinationPresentation::new(
                "coordination.frequency_hint.title",
                "coordination.frequency_hint.body",
            )
            .with_body_param("frequency", 0.83_f32),
            // A raw `chatter.sender.*` fallback that MUST be overridden by
            // the station the sensors system resolves to.
            sender_label: coordination::CHATTER_SENDER_SENSORS.to_string(),
            sender_system: crate::ship::system_registry::sensors_system_id(),
        });
    tick(&mut app);
    tick(&mut app);
    let chatter = drain_chatter(&mut app);
    assert_eq!(
        chatter.len(),
        1,
        "AI→AI coordination reaches the viewscreen"
    );
    assert!(
        chatter[0].from_label.starts_with("station.") && chatter[0].from_label.ends_with(".name"),
        "the sender is addressed as its owning STATION, not the \
         chatter.sender.* system label, got {:?}",
        chatter[0].from_label
    );
    assert_ne!(
        chatter[0].from_label,
        coordination::CHATTER_SENDER_SENSORS,
        "the system-level fallback label must not survive resolution"
    );
}

/// AC5 on the chain the PLAYER SHIP actually takes, end to end in ONE app.
///
/// The test above proves the low-fidelity (demoted-NPC) emitter. This one
/// proves the high-fidelity emitter, and it is the one the issue is about:
/// the player hull is permanently high-fidelity, so a human sitting at the
/// player ship's Sensors is served by `tick_frequency_hint_high_fidelity`,
/// never by `tick_sensors_frequency_hint`. Both are kept — a ship can be on
/// either side of the LOD split and both must feed a backfilled Tactical.
///
/// It also pins the consequence that reading the spec would otherwise get
/// backwards: on the player ship a HUMAN Sensors officer's advisory carries
/// the authored `frequency_hint_delay_secs` reaction delay, exactly as the
/// AI's does. That is deliberate. An advisory that is instant for a human
/// sender and delayed for an AI one is a human-vs-AI branch on a
/// coordination fact (AGENTS.md rule 6); and the "instant" path it replaced
/// delivered nothing at all, because it addressed the Tactical station key,
/// which resolves to the `Human` policy default, making a human-origin hint
/// route Human→Human = Suppress.
///
/// So the delay is asserted in both directions: silent well inside it,
/// delivered past it.
#[test]
fn human_sensors_advisory_reaches_a_backfilled_tactical_on_the_player_ships_high_fidelity_chain() {
    let mut app = routing_test_app();
    add_tactical_hint_applier(&mut app);
    start_game_with_sensors_officer(&mut app);
    backfill_tactical_to_ai(&mut app);
    give_ship_tactical_frequency_surface(&mut app);
    arm_real_high_fidelity_sensors_emitter(&mut app, "harrow-raider-1", 0.83);

    assert_eq!(
        get_ship_control_sources(&mut app)
            .0
            .source_for(&crate::ship::system_registry::sensors_system_id()),
        ControlSource::Human,
        "the fixture must actually leave Sensors in human hands"
    );
    let before = ship_phaser_frequency(&mut app);
    assert!(
        (before - 0.83).abs() > f32::EPSILON,
        "the fixture must start away from the advised frequency"
    );

    let runs = authored_reaction_delay_runs();
    // Well inside the authored reaction delay.
    for _ in 0..runs / 2 {
        tick(&mut app);
    }
    assert!(
        (ship_phaser_frequency(&mut app) - before).abs() < f32::EPSILON,
        "the authored Sensors reaction delay applies to a human sender too — half \
         of it must not be enough; phaser frequency already moved to {}",
        ship_phaser_frequency(&mut app)
    );

    // Past it, plus the router tick and the applier tick.
    for _ in 0..(runs - runs / 2 + 3) {
        tick(&mut app);
    }
    let frequency = ship_phaser_frequency(&mut app);
    assert!(
        (frequency - 0.83).abs() < f32::EPSILON,
        "on the permanently-high-fidelity PLAYER hull, a human Sensors officer's \
         advisory must still reach and move the backfilled Tactical; phaser \
         frequency is {frequency}, expected 0.83"
    );
    assert_eq!(
        coordination_popups(&app),
        0,
        "an advisory consumed by an AI station must not also raise a popup"
    );
}

/// The same delivery, from the ship's own Sensors AI. Both origins must
/// reach the backfilled Tactical identically — that symmetry is the point,
/// and asserting only the human half would let an origin branch survive on
/// the other side.
#[test]
fn ai_sensors_advisory_reaches_a_backfilled_tactical_the_same_way() {
    let mut app = routing_test_app();
    add_tactical_hint_applier(&mut app);
    start_game_with_sensors_officer(&mut app);
    backfill_tactical_to_ai(&mut app);
    give_ship_tactical_frequency_surface(&mut app);

    enqueue_coordination(
        &mut app,
        ControlSource::Ai,
        CoordinationAddress::Station(StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into(),
        )),
        CoordinationPayload::FrequencyHint { frequency: 0.83 },
    );
    tick(&mut app);
    tick(&mut app);

    let ship = find_ship_entity(&mut app);
    assert!(
        (app.world()
            .get::<crate::ship::state::ShipPhaserFrequency>(ship)
            .unwrap()
            .0
            - 0.83)
            .abs()
            < f32::EPSILON,
        "an AI-origin advisory must reach a backfilled Tactical too"
    );
    assert_eq!(
        coordination_popups(&app),
        0,
        "before #873 this AI→backfilled-Tactical hint fell through to the ownerless \
         branch and BROADCAST a popup to every connected client, because the tactical \
         station key resolves to no [[system]] and therefore no station holder"
    );
}

/// AC3. A human-held Tactical must route exactly as it did before: the live
/// Station policy resolves Human and an AI-origin advisory surfaces only to
/// that holder.
#[test]
fn a_human_held_tactical_still_routes_an_ai_advisory_to_a_popup() {
    let mut app = routing_test_app();
    start_game_with_sensors_officer(&mut app);
    seat_human_on_tactical(&mut app);
    give_ship_tactical_frequency_surface(&mut app);
    // No `backfill_tactical_to_ai` — every tactical fine system stays on
    // the default Human source.

    enqueue_coordination(
        &mut app,
        ControlSource::Ai,
        CoordinationAddress::Station(StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into(),
        )),
        CoordinationPayload::FrequencyHint { frequency: 0.83 },
    );
    tick(&mut app);
    tick(&mut app);

    let popup_targets: Vec<_> = app
        .world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .filter_map(|(target, msg)| match (target, msg) {
            (
                crate::lobby::handler::Target::Token(token),
                crate::core::messages::ServerMessage::CoordinationPopup { .. },
            ) => Some(token.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        popup_targets,
        vec!["tactical"],
        "an advisory addressed to Tactical must reach only Tactical, never Target::All"
    );
    let popup = app
        .world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .find_map(|(_, message)| match message {
            crate::core::messages::ServerMessage::CoordinationPopup {
                address, payload, ..
            } => Some((address, payload)),
            _ => None,
        })
        .expect("the Tactical holder receives one popup");
    assert_eq!(
        popup.0,
        &CoordinationAddress::Station(StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into()
        ))
    );
    assert_eq!(
        popup.1,
        &CoordinationPayload::FrequencyHint { frequency: 0.83 },
        "the human popup preserves the typed frequency through the delayed path"
    );
    assert!(
        app.world()
            .resource::<crate::lobby::LobbyOutbox>()
            .0
            .iter()
            .all(|(target, msg)| !matches!(
                (target, msg),
                (
                    crate::lobby::handler::Target::All,
                    crate::core::messages::ServerMessage::CoordinationPopup { .. },
                )
            )),
        "an addressed Tactical popup must not fall back to a broadcast"
    );
    let ship = find_ship_entity(&mut app);
    assert_eq!(
        app.world()
            .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
            .unwrap()
            .0,
        None,
        "nothing may be consumed on the AI's behalf while a human holds Tactical"
    );
}

#[test]
fn an_unknown_station_address_is_dropped_never_widened_to_all_clients() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);

    enqueue_coordination(
        &mut app,
        ControlSource::Ai,
        CoordinationAddress::Station(StationId("not-on-this-hull".into())),
        CoordinationPayload::Alert {
            title: "test.alert.title".into(),
            body: "test.alert.body".into(),
        },
    );
    tick(&mut app);
    tick(&mut app);

    assert_eq!(
        coordination_popups(&app),
        0,
        "an invalid Station address must be rejected, never widened to the Ship"
    );
    assert!(
        app.world()
            .resource::<crate::lobby::LobbyOutbox>()
            .0
            .iter()
            .all(|(target, message)| !matches!(
                (target, message),
                (
                    crate::lobby::handler::Target::All,
                    crate::core::messages::ServerMessage::CoordinationPopup { .. }
                )
            )),
        "unknown Coordination destinations must never use the legacy all-client fallback"
    );
}

/// The handover window. `process_coordination_lag` emits the typed delivery and
/// Tactical's receiver lands it in `Modifiers`; `apply_tactical_frequency_hint`
/// reads it in the NEXT tick's `Input` — so there is exactly one tick in which a
/// human can claim Tactical after delivery resolved the addressee as AI.
///
/// The applier must therefore re-ask the router's own question,
/// `any_tactical_system_operates_ai`, and DROP the value when the answer has
/// changed. Applying it would overwrite the frequency the human just dialled
/// with an advisory addressed to nobody, and the human would have no idea
/// why their guns detuned.
///
/// Dropped, not deferred: the slot is emptied either way, so the stale hint
/// cannot re-assert itself the moment the AI takes Tactical back.
#[test]
fn a_hint_pending_when_a_human_claims_tactical_is_dropped_not_applied() {
    let mut app = routing_test_app();
    add_tactical_hint_applier(&mut app);
    start_game_with_sensors_officer(&mut app);
    backfill_tactical_to_ai(&mut app);
    give_ship_tactical_frequency_surface(&mut app);

    enqueue_coordination(
        &mut app,
        ControlSource::Human,
        CoordinationAddress::Station(StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into(),
        )),
        CoordinationPayload::FrequencyHint { frequency: 0.83 },
    );
    // Tick 1 only: the lag router has emitted the typed delivery and Tactical's
    // receiver has consumed it into the pending slot; the applier has not yet
    // run on it.
    tick(&mut app);
    let ship = find_ship_entity(&mut app);
    assert_eq!(
        app.world()
            .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
            .unwrap()
            .0,
        Some(0.83),
        "precondition: the router must have landed the hint, so what follows is \
         about the applier and not about the hint never arriving"
    );

    // A human takes Tactical inside the window.
    set_tactical_fine_systems(&mut app, ControlSource::Human);
    let dialled_by_the_human = ship_phaser_frequency(&mut app);
    tick(&mut app);

    assert!(
        (ship_phaser_frequency(&mut app) - dialled_by_the_human).abs() < f32::EPSILON,
        "a hint addressed to the AI that held Tactical a tick ago must not overwrite \
         the frequency its human successor is holding; phaser frequency moved to {}",
        ship_phaser_frequency(&mut app)
    );
    assert_eq!(
        app.world()
            .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
            .unwrap()
            .0,
        None,
        "the stale hint must be DROPPED, not left pending — otherwise it lands the \
         moment Tactical goes back to AI, long after the fact that produced it"
    );
}

/// The applier drains the slot even for a ship it cannot act on.
///
/// `apply_tactical_frequency_hint` takes everything but the slot itself as
/// `Option`. If `ShipPhaserFrequency` or `ShipConfigComponent` were required,
/// a `Ship` missing one would be filtered OUT of the query rather than
/// iterated — its pending hint would never be drained, and would then apply
/// the moment the missing component appeared, carrying a frequency from an
/// arbitrarily old tick. Every shipped spawn site attaches all of them today,
/// so this is a latent hole rather than a live bug; the point of pinning it
/// is that the doc-comment's "consumed exactly once" is then an invariant of
/// the system rather than a property of the current spawn sites.
#[test]
fn a_pending_hint_is_drained_even_on_a_ship_that_cannot_apply_it() {
    let mut app = routing_test_app();
    add_tactical_hint_applier(&mut app);
    start_game_with_sensors_officer(&mut app);
    backfill_tactical_to_ai(&mut app);
    give_ship_tactical_frequency_surface(&mut app);

    // A ship with the slot but nothing to move.
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship::state::ShipPhaserFrequency>();
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship_plugin::PendingTacticalFrequencyHint(Some(0.83)));

    tick(&mut app);
    assert_eq!(
        app.world()
            .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
            .unwrap()
            .0,
        None,
        "the slot must be drained even when the hint cannot be applied — leaving it \
         set makes the value land later, out of time with the fact that produced it"
    );

    // The frequency surface appears afterwards; the stale hint must be gone.
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::state::ShipPhaserFrequency(0.1));
    tick(&mut app);
    assert!(
        (ship_phaser_frequency(&mut app) - 0.1).abs() < f32::EPSILON,
        "a hint dropped on a previous tick must not re-assert itself once the \
         missing component appears; phaser frequency moved to {}",
        ship_phaser_frequency(&mut app)
    );
}

/// AC2, Helm half. A human on Tactical asks the backfilled Helm to come
/// about; the AI Helm must receive the request rather than have it
/// suppressed as "two humans who can just talk to each other".
#[test]
fn human_sender_advisory_is_consumed_by_a_backfilled_helm() {
    let mut app = routing_test_app();
    start_game_with_sensors_officer(&mut app);
    set_helm_control_source(&mut app, ControlSource::Ai);
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(PendingArcBearingRequest::default());

    let target_uuid = uuid::Uuid::new_v4();
    enqueue_coordination(
        &mut app,
        ControlSource::Human,
        CoordinationAddress::Station(StationId(
            crate::ship::system_registry::HELM_STATION_ID.into(),
        )),
        CoordinationPayload::ArcBearingRequest {
            uuid: target_uuid.to_string(),
            label: "Harrow Raider".into(),
            family: crate::core::messages::WeaponFamily::Phasers,
            arcs: Vec::new(),
        },
    );
    tick(&mut app);
    tick(&mut app);

    assert_eq!(
        app.world()
            .get::<PendingArcBearingRequest>(ship)
            .unwrap()
            .target,
        Some(target_uuid),
        "a backfilled Helm must act on a human-origin arc-bearing request"
    );
    assert_eq!(
        coordination_popups(&app),
        0,
        "an AI Helm consumes silently; there is no console holder to pop up at"
    );
}

// ── Issue #873: the power brownout advisory, at DELIVERY ────────────────
//
// `tick_power_brownout_advisory` used to stamp `sender_origin:
// ControlSource::Ai` as a literal. It now reads the ship's live
// `power-reactor` control source. `ship::power`'s own test asserts the tag
// at the point of emission, which is not enough to call the consequence
// deliberate: the tag is the ONLY input `route_coordination` has, so
// changing it changes who is shown the advisory. AC3 says existing
// consume/popup/suppress behaviour is unchanged, and for this advisory it is
// NOT — so the change is pinned here, on the delivery side, in both
// directions.
//
// The behaviour that changed: with a human at Power and a human at Helm, a
// brownout used to pop up at the Helm because it claimed AI origin. It now
// routes Human→Human = Suppress. That is the correct reading of the rule the
// router already implements — two humans on the same bridge talk to each
// other, the bus does not interrupt them — and a hardcoded origin is exactly
// the bug class this issue removes. It is a deliberate behaviour change, not
// an oversight, which is why it has a test of its own.

/// Arm the REAL brownout emitter on this ship: the two components its query
/// takes, the reactor on `power_source`, and the system itself in the set
/// `ShipPowerPlugin` registers it in (`SimSet::Modifiers`, ordered before
/// `process_coordination_lag` so the write and the routing of it are the
/// production order). `ShipPlugin` — the only plugin `test_app` installs —
/// does not carry `ShipPowerPlugin`, so without this the emitter would never
/// run and BOTH tests below would pass for the wrong reason.
///
/// A `CoordEnqueueBox` collector comes with it for exactly that reason: each
/// test asserts the advisory was really emitted before asserting what
/// delivery did with it.
fn arm_brownout_advisory(app: &mut App, power_source: ControlSource) {
    let ship = find_ship_entity(app);
    app.world_mut().entity_mut(ship).insert((
        crate::ship::power::ShipPowerSystem(crate::modifiers::power_system::PowerSystem::default()),
        crate::ship::power::PowerBrownoutState::default(),
    ));
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(
                crate::ship::system_registry::power_reactor_system_id(),
                power_source,
            );
        }
    }
    app.init_resource::<CoordEnqueueBox>()
        .add_systems(PostUpdate, collect_coord)
        .add_systems(
            Update,
            crate::ship::power::tick_power_brownout_advisory
                .in_set(crate::sim_sets::SimSet::Modifiers)
                .before(process_coordination_lag),
        );
}

fn brownout_advisories(app: &App) -> usize {
    app.world()
        .resource::<CoordEnqueueBox>()
        .0
        .iter()
        .filter(|e| matches!(&e.payload, CoordinationPayload::PowerBrownout { .. }))
        .count()
}

/// Force the reactor into the exhaustion-LOCK state and raise the
/// lock-changed edge the advisory now fires on. This fixture registers only
/// `tick_power_brownout_advisory` (not the `tick_power_system` integration
/// that would drive a flat battery to the lock over many ticks), and these
/// are ROUTING tests, so the lock state is set directly rather than reached.
fn drive_ship_into_brownout(app: &mut App) {
    use crate::core::messages::PowerGroupId;
    use crate::modifiers::power_system::{
        HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
    };
    let mut q = app.world_mut().query_filtered::<(
        &mut crate::ship::power::ShipPowerSystem,
        &mut crate::ship::power::PowerBrownoutState,
    ), With<Ship>>();
    for (mut ps, mut brownout) in q.iter_mut(app.world_mut()) {
        ps.0.restore(
            &[
                (PowerGroupId(HELM_POWER_GROUP.into()), 1),
                (PowerGroupId(WEAPONS_POWER_GROUP.into()), 1),
                (PowerGroupId(SHIELDS_POWER_GROUP.into()), 1),
            ],
            0.0,
            true,
        );
        brownout.locked_changed = true;
    }
}

/// The changed case. Human at Power, human at Helm → Suppress.
#[test]
fn a_human_power_officers_brownout_is_suppressed_at_a_human_helm() {
    let mut app = routing_test_app();
    // Seats a human on Helm (and Captain, and Repair).
    start_game_with_engineer(&mut app);
    arm_brownout_advisory(&mut app, ControlSource::Human);
    assert!(
        !helm_axes_operate_ai(&get_ship_control_sources(&mut app)),
        "fixture precondition: Helm must be in human hands, or the Suppress \
         branch is not the one under test"
    );

    drive_ship_into_brownout(&mut app);
    for _ in 0..4 {
        tick(&mut app);
    }

    assert!(
        brownout_advisories(&app) > 0,
        "precondition: the fixture must actually EMIT a brownout advisory, or the \
         popup assertion below passes for the wrong reason"
    );
    assert_eq!(
        coordination_popups(&app),
        0,
        "a human Power officer's brownout must not interrupt a human Helm — before \
         issue #873 the advisory stamped a literal ControlSource::Ai and took the \
         AI→human popup branch no matter who was at Power. This behaviour CHANGED, \
         deliberately: the tag now reports the live reactor control source, so \
         Human→Human routes to Suppress like every other same-origin advisory"
    );
}

/// The unchanged case, asserted alongside it so the fix cannot be mistaken
/// for "brownouts stopped being delivered".
#[test]
fn an_ai_power_brownout_still_pops_up_at_a_human_helm() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);
    arm_brownout_advisory(&mut app, ControlSource::Ai);

    drive_ship_into_brownout(&mut app);
    for _ in 0..4 {
        tick(&mut app);
    }

    assert!(
        brownout_advisories(&app) > 0,
        "precondition: the fixture must actually EMIT a brownout advisory"
    );
    assert!(
        coordination_popups(&app) > 0,
        "an AI-run reactor's brownout must still reach the human Helm exactly as it \
         did before issue #873 — only the human-at-Power case changed"
    );
}

#[test]
fn repair_popup_withholds_exact_non_core_deficit_before_a_team_arrives() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);

    // helm-radar is owned by Helm — non-Core, and no team dispatched.
    let radar = SystemId("helm-radar".into());
    give_ship_hull(&mut app, &[("helm-radar", 100.0), ("repair", 100.0)]);
    set_ai(&mut app, &radar);
    set_console_hp_direct(&mut app, radar.clone(), 1.0);
    tick(&mut app);
    tick(&mut app);
    tick(&mut app);

    let deficits = repair_popup_deficits(&app);
    assert!(
        !deficits.is_empty(),
        "Engineering must still be told the system needs attention"
    );
    assert!(
        deficits.iter().all(|d| d.is_none()),
        "the exact HP deficit of a non-Core system must not reach Engineering \
         before a team is on site; got {deficits:?}"
    );
    assert_repair_presentations_are_coarse(&app);
}

#[test]
fn repair_popup_carries_the_exact_deficit_once_a_team_is_on_site() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);

    let radar = SystemId("helm-radar".into());
    give_ship_hull(&mut app, &[("helm-radar", 100.0), ("repair", 100.0)]);
    set_ai(&mut app, &radar);
    place_team_on_site(&mut app, &radar);
    set_console_hp_direct(&mut app, radar.clone(), 1.0);
    tick(&mut app);
    tick(&mut app);
    tick(&mut app);

    let deficits = repair_popup_deficits(&app);
    assert!(
        deficits.iter().any(|d| d.is_some()),
        "a team on site is the information gate opening; got {deficits:?}"
    );
    assert_repair_presentations_are_coarse(&app);
}

// ── Issue #879: the ship-wide intent-advisory broadcast ──────────────────
//
// Every payload above is addressed to ONE console. An intent advisory is
// addressed to the ship: it goes to every human seat, so a crew that has
// lost seats to Backfill still shares one picture of what the automation is
// doing. These cover the delivery half; the routing rule itself is the pure
// `coordination::broadcast_to_ship`, and the "when does anything get
// emitted at all" half is `ship::intent_narration*`.

/// Put every system the named station owns on `source` — what claiming or
/// vacating that seat does to the ship's control sources.
fn set_station_systems_source(app: &mut App, station: &str, source: ControlSource) {
    let ids: Vec<SystemId> = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipConfigComponent, With<Ship>>();
        let cfg = q.single(app.world()).expect("ship config").0.clone();
        cfg.systems
            .iter()
            .filter(|s| s.station.as_ref().map(|st| st.0.as_str()) == Some(station))
            .map(|s| s.id.clone())
            .collect()
    };
    assert!(
        !ids.is_empty(),
        "the shipped hull must give `{station}` systems for this fixture to mean anything"
    );
    for id in ids {
        set_fine_control_source(app, id, source);
    }
}

/// The session tokens that received an intent-advisory popup, in order.
fn intent_popup_tokens(app: &App) -> Vec<String> {
    app.world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .filter_map(|(target, msg)| match (target, msg) {
            (
                crate::lobby::handler::Target::Token(token),
                crate::core::messages::ServerMessage::CoordinationPopup {
                    payload: CoordinationPayload::IntentAdvisory { .. },
                    ..
                },
            ) => Some(token.clone()),
            _ => None,
        })
        .collect()
}

fn enqueue_intent_advisory(app: &mut App, sender_origin: ControlSource) {
    enqueue_coordination(
        app,
        sender_origin,
        CoordinationAddress::Ship,
        CoordinationPayload::IntentAdvisory {
            kind: crate::core::messages::IntentKind::TargetSwitched,
            subject: Some("Harrow Raider".into()),
            generation: 7,
        },
    );
}

/// AC: a backfilled seat's advisory reaches every human seat on the source
/// ship, and no AI seat — even one that still carries a session token.
///
/// Three humans are seated (Captain, Helm, Repair) and the Repair station's
/// systems are then put on AI, which is the shape a seat has while it is
/// backfilled. The advisory must reach the two human seats and stop at the
/// AI one; the addressed path could not express this at all, because it
/// delivers to exactly one console.
#[test]
fn an_intent_advisory_reaches_every_human_seat_and_no_ai_seat() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);
    set_station_systems_source(&mut app, "repair", ControlSource::Ai);
    backfill_tactical_to_ai(&mut app);

    enqueue_intent_advisory(&mut app, ControlSource::Ai);
    tick(&mut app);
    tick(&mut app);

    assert_eq!(
        intent_popup_tokens(&app),
        vec!["captain".to_string(), "helm".to_string()],
        "every HUMAN seat on the ship, in authored station order — and not \
         the backfilled Repair seat, whose holder token is still there"
    );
}

/// Addressing, not payload knowledge, selects the whole-Ship path. An Alert is
/// normally Station-addressed; carrying it on `Ship` must still fan it out to
/// every connected human seat in deterministic authored order.
#[test]
fn a_ship_address_broadcasts_a_non_intent_payload_in_authored_order() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);
    backfill_tactical_to_ai(&mut app);

    enqueue_coordination(
        &mut app,
        ControlSource::Ai,
        CoordinationAddress::Ship,
        CoordinationPayload::Alert {
            title: "test.alert.title".into(),
            body: "test.alert.body".into(),
        },
    );
    tick(&mut app);
    tick(&mut app);

    let tokens: Vec<_> = app
        .world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .filter_map(|(target, message)| match (target, message) {
            (
                crate::lobby::handler::Target::Token(token),
                crate::core::messages::ServerMessage::CoordinationPopup {
                    payload: CoordinationPayload::Alert { .. },
                    ..
                },
            ) => Some(token.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        tokens,
        vec![
            "captain".to_string(),
            "helm".to_string(),
            "engineer".to_string()
        ],
        "the Ship address alone selects deterministic fan-out; payload kind is irrelevant"
    );
}

/// The inverse regression: an Intent payload carried on a Station address is
/// still a one-Station message. Looking at the variant would leak it across the
/// bridge despite the explicit address.
#[test]
fn a_station_address_keeps_an_intent_payload_at_one_station() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);

    enqueue_coordination(
        &mut app,
        ControlSource::Ai,
        CoordinationAddress::Station(StationId("captain".into())),
        CoordinationPayload::IntentAdvisory {
            kind: crate::core::messages::IntentKind::TargetSwitched,
            subject: Some("Harrow Raider".into()),
            generation: 9,
        },
    );
    tick(&mut app);
    tick(&mut app);

    assert_eq!(
        intent_popup_tokens(&app),
        vec!["captain".to_string()],
        "an Intent variant cannot override its explicit Station address"
    );
}

/// The broadcast inherits the delivery matrix rather than replacing it: a
/// human-held seat's advisory is suppressed at every human seat, exactly as
/// human→human channel-3 traffic always has been.
#[test]
fn a_human_seats_intent_advisory_is_suppressed_across_the_ship() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);

    enqueue_intent_advisory(&mut app, ControlSource::Human);
    tick(&mut app);
    tick(&mut app);

    assert!(
        intent_popup_tokens(&app).is_empty(),
        "two officers on the same bridge talk to each other; the matrix has \
         said so since #494 and the broadcast does not get to disagree"
    );
}

/// Delivery is the existing transient popup surface and nothing else — no
/// durable log is written anywhere on the way.
#[test]
fn an_intent_advisory_is_delivered_verbatim_through_the_popup_surface() {
    let mut app = routing_test_app();
    start_game_with_engineer(&mut app);
    backfill_tactical_to_ai(&mut app);

    enqueue_intent_advisory(&mut app, ControlSource::Ai);
    tick(&mut app);
    tick(&mut app);

    let payloads: Vec<CoordinationPayload> = app
        .world()
        .resource::<crate::lobby::LobbyOutbox>()
        .0
        .iter()
        .filter_map(|(_, msg)| match msg {
            crate::core::messages::ServerMessage::CoordinationPopup { payload, .. } => {
                Some(payload.clone())
            }
            _ => None,
        })
        .collect();
    assert!(
        payloads.iter().all(|p| *p
            == CoordinationPayload::IntentAdvisory {
                kind: crate::core::messages::IntentKind::TargetSwitched,
                subject: Some("Harrow Raider".into()),
                generation: 7,
            }),
        "the advisory rides the CoordinationPopup surface unchanged; got {payloads:?}"
    );
    assert!(
        !payloads.is_empty(),
        "precondition: something was delivered"
    );
}

// ── Human-seeking hosts (issue #984) ─────────────────────────────────────

/// The four shipped alliance hulls, and the station each one AUTHORS as the
/// home of its comms and navigation systems.
///
/// THREE OF FOUR disagree with the naive `StationId(system_id.0)` string
/// match, and that coincidence on the remaining one is exactly what hid the
/// `CommsState` bug: the destroyer and courier declare no `comms` STATION at
/// all, so a literal `StationId("comms")` found no holder and those two
/// hulls silently never received a `CommsState`. Every resolution goes
/// through `station_for_system`; nothing casts a `SystemId` to a
/// `StationId`.
const SEEKING_HULLS: &[(&str, &str, &str)] = &[
    // hull, comms system's station, navigation system's station
    ("alliance_battleship", "comms", "navigation"),
    ("alliance_cruiser", "comms", "navigation"),
    ("alliance_destroyer", "comms", "navigation"),
    ("alliance_courier", "captain", "captain"),
];

fn hull_ship_config(stem: &str) -> crate::ship::config::ShipConfig {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets/entities")
        .join(format!("{stem}.toml"));
    let key = path.to_string_lossy().replace('\\', "/");
    crate::entities::include_resolve::load_entity_config(&key)
        .unwrap_or_else(|e| panic!("{stem} must parse: {e}"))
        .ship_config
        .unwrap_or_else(|| panic!("{stem} must declare a ShipConfig"))
}

/// A `ShipPlugin` app whose LocalShip wears a REAL shipped hull, booted the
/// way production boots one — `seed_boot_ratings` with everything on
/// Backfill — and then crewed at `manned`: each named station gets a
/// connected holder, a non-Backfill active rating, and its own systems set
/// Human, which is what a seated officer looks like from here.
fn seeking_config_app(config: crate::ship::config::ShipConfig, manned: &[&str]) -> App {
    let mut app = test_app();
    let ship = find_ship_entity(&mut app);
    let (mut resolver, mut active) = crate::ship::rating::seed_boot_ratings(&config, |_| {
        crate::ship::rating::BACKFILL_RATING.to_string()
    });
    for (idx, station) in manned.iter().enumerate() {
        let sid = crate::core::messages::StationId((*station).into());
        for system in config.systems_for_station(&sid) {
            resolver.set(system.id.clone(), ControlSource::Human);
        }
        active.insert(sid.clone(), "Manual".to_string());
        let token = format!("officer-{station}");
        let mut sessions = app.world_mut().resource_mut::<Sessions>();
        sessions
            .0
            .register(token.clone(), format!("Officer {idx}"))
            .expect("a fresh token registers");
        sessions.0.set_station(&token, Some(sid));
    }
    app.world_mut().entity_mut(ship).insert((
        ShipConfigComponent(config),
        ShipSystemControlSources(resolver),
        ActiveStationRatings(active),
    ));
    app
}

fn seeking_app(stem: &str, manned: &[&str]) -> App {
    seeking_config_app(hull_ship_config(stem), manned)
}

fn host_of(app: &mut App, system: &crate::core::messages::SystemId) -> Option<String> {
    let ship = find_ship_entity(app);
    app.world()
        .entity(ship)
        .get::<HumanSeekingHosts>()
        .and_then(|h| h.host_for(system))
        .map(|s| s.0.clone())
}

fn source_of(app: &mut App, system: &crate::core::messages::SystemId) -> ControlSource {
    let ship = find_ship_entity(app);
    app.world()
        .entity(ship)
        .get::<ShipSystemControlSources>()
        .expect("the ship carries control sources")
        .0
        .source_for(system)
}

fn station_assignment(
    app: &mut App,
    station: &crate::core::messages::StationId,
) -> crate::ship::coordination::VisitingStationAssignment {
    let ship = find_ship_entity(app);
    app.world()
        .entity(ship)
        .get::<VisitingStationHosts>()
        .and_then(|hosts| hosts.assignment_for(station))
        .cloned()
        .expect("the complete visiting Station has a live assignment")
}

#[test]
fn live_scenario_floor_raises_effective_visiting_rating_and_control_depth() {
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "navigation"
name = "Navigation"
description = ""
rank = ""
human_seeking = true
host_order = ["captain"]
visiting_rating = "Visit"
[[station.rating]]
name = "Floor"
automated_systems = []
[[station.rating]]
name = "Visit"
automated_systems = ["navigation"]

[[station]]
id = "captain"
name = "Captain"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "navigation"
kind = "navigation"
station = "navigation"
"#,
        &["navigation"],
    )
    .expect("the adapter fixture is valid authored hull data");
    let mut app = seeking_config_app(config, &["captain"]);
    let navigation_station = crate::core::messages::StationId("navigation".into());
    let navigation_system = crate::core::messages::SystemId("navigation".into());

    tick(&mut app);
    let baseline = station_assignment(&mut app, &navigation_station);
    assert_eq!(
        baseline.host,
        Some(crate::core::messages::StationId("captain".into()))
    );
    assert_eq!(baseline.rating, "Visit");
    assert_eq!(source_of(&mut app, &navigation_system), ControlSource::Ai);

    app.world_mut()
        .insert_resource(crate::world::config::WorldConfig {
            scenario_detail_floor: vec!["navigation".into()],
            ..Default::default()
        });
    tick(&mut app);

    let ship = find_ship_entity(&mut app);
    assert!(
        app.world()
            .entity(ship)
            .get::<ScenarioDetailFloor>()
            .expect("LocalShip requires the live scenario-floor input")
            .0
            .contains(&navigation_system),
        "the production writer resolves the active world's kind selector onto this hull"
    );

    let raised = station_assignment(&mut app, &navigation_station);
    assert_eq!(raised.rating, "Floor");
    assert_eq!(raised.host, baseline.host);
    assert_eq!(
        source_of(&mut app, &navigation_system),
        ControlSource::Human
    );
    assert_eq!(
        host_of(&mut app, &navigation_system).as_deref(),
        Some("captain")
    );
}

#[test]
fn an_ineligible_direct_holder_is_skipped_as_a_visiting_host() {
    // AC2 (issue #1103) at the Bevy adapter. Captain is crewed and would host
    // the visiting navigation station, but that holder reports itself
    // INELIGIBLE for navigation. The resolver must skip it and fall the
    // station to AI — never touching the settings or the reason.
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "navigation"
name = "Navigation"
description = ""
rank = ""
human_seeking = true
host_order = ["captain"]
visiting_rating = "Visit"
[[station.rating]]
name = "Floor"
automated_systems = []
[[station.rating]]
name = "Visit"
automated_systems = []

[[station]]
id = "captain"
name = "Captain"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "navigation"
kind = "navigation"
station = "navigation"
"#,
        &["navigation"],
    )
    .expect("valid authored hull data");
    let mut app = seeking_config_app(config, &["captain"]);
    let navigation_station = crate::core::messages::StationId("navigation".into());
    let navigation_system = crate::core::messages::SystemId("navigation".into());

    // Baseline: the eligible captain (default TRUE) hosts navigation and
    // operates it as a human.
    tick(&mut app);
    assert_eq!(
        station_assignment(&mut app, &navigation_station).host,
        Some(crate::core::messages::StationId("captain".into())),
        "baseline: the eligible captain hosts the visiting navigation station"
    );
    assert_eq!(
        source_of(&mut app, &navigation_system),
        ControlSource::Human
    );

    // The captain's holder now reports itself ineligible for navigation.
    {
        let mut sessions = app.world_mut().resource_mut::<Sessions>();
        sessions.0.set_eligibility(
            "officer-captain",
            std::collections::HashSet::from([navigation_station.clone()]),
        );
    }
    tick(&mut app);

    assert_eq!(
        station_assignment(&mut app, &navigation_station).host,
        None,
        "an ineligible holder is skipped; the visiting station falls to AI"
    );
    assert_eq!(source_of(&mut app, &navigation_system), ControlSource::Ai);
}

#[test]
fn an_afk_direct_holder_is_skipped_as_a_visiting_host_and_returns_on_leave() {
    // AC3/AC4 (issue #1104) at the Bevy adapter. The Captain is crewed and
    // hosts the visiting navigation station; when that holder steps AFK the
    // resolver must skip them and fall the station to AI — deterministically,
    // per tick — and re-include them the tick after they return.
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "navigation"
name = "Navigation"
description = ""
rank = ""
human_seeking = true
host_order = ["captain"]
visiting_rating = "Visit"
[[station.rating]]
name = "Floor"
automated_systems = []
[[station.rating]]
name = "Visit"
automated_systems = []

[[station]]
id = "captain"
name = "Captain"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "navigation"
kind = "navigation"
station = "navigation"
"#,
        &["navigation"],
    )
    .expect("valid authored hull data");
    let mut app = seeking_config_app(config, &["captain"]);
    let navigation_station = crate::core::messages::StationId("navigation".into());
    let navigation_system = crate::core::messages::SystemId("navigation".into());

    // Baseline: the present captain hosts navigation and operates it.
    tick(&mut app);
    assert_eq!(
        station_assignment(&mut app, &navigation_station).host,
        Some(crate::core::messages::StationId("captain".into())),
        "baseline: the present captain hosts the visiting navigation station"
    );
    assert_eq!(
        source_of(&mut app, &navigation_system),
        ControlSource::Human
    );

    // The captain steps AFK.
    {
        let mut sessions = app.world_mut().resource_mut::<Sessions>();
        sessions.0.set_afk("officer-captain", true);
    }
    tick(&mut app);
    assert_eq!(
        station_assignment(&mut app, &navigation_station).host,
        None,
        "an AFK holder is skipped; the visiting station falls to AI"
    );
    assert_eq!(source_of(&mut app, &navigation_system), ControlSource::Ai);

    // The captain returns — the pure per-tick recompute re-includes them
    // with no stored state (AC4).
    {
        let mut sessions = app.world_mut().resource_mut::<Sessions>();
        sessions.0.set_afk("officer-captain", false);
    }
    tick(&mut app);
    assert_eq!(
        station_assignment(&mut app, &navigation_station).host,
        Some(crate::core::messages::StationId("captain".into())),
        "leaving AFK re-includes the eligible visiting host on the next tick"
    );
    assert_eq!(
        source_of(&mut app, &navigation_system),
        ControlSource::Human
    );
}

#[test]
fn shipped_combat_test_floor_resolves_through_destroyer_hull_and_production_writer() {
    let config = hull_ship_config("alliance_destroyer");
    let mut app = seeking_config_app(config, &["tactical"]);
    let world =
        crate::world::config::parse_world(include_str!("../../assets/worlds/combat_test.toml"))
            .expect("the shipped root world parses");
    assert_eq!(world.scenario_detail_floor, vec!["navigation"]);
    app.insert_resource(world);

    tick(&mut app);

    let navigation_station = crate::core::messages::StationId("navigation".into());
    let navigation_system = crate::core::messages::SystemId("navigation".into());
    let assignment = station_assignment(&mut app, &navigation_station);
    assert_eq!(
        assignment.host,
        Some(crate::core::messages::StationId("tactical".into()))
    );
    assert_eq!(
        assignment.rating, "Std",
        "Combat Test raises the Destroyer's authored Simplified visiting baseline"
    );
    assert_eq!(
        source_of(&mut app, &navigation_system),
        ControlSource::Human
    );
}

/// The `SystemId`-vs-`StationId` regression, pinned on every shipped hull.
/// Nothing here goes near the live seek: it asserts what the four hulls
/// AUTHOR, and that the literal the addressing used to hardcode names no
/// station at all on two of them.
#[test]
fn every_shipped_hull_authors_seeking_comms_and_navigation_on_the_station_it_declares() {
    for (stem, comms_station, nav_station) in SEEKING_HULLS {
        let config = hull_ship_config(stem);
        for (system_id, expected) in [
            (
                crate::ship::system_registry::comms_system_id(),
                comms_station,
            ),
            (
                crate::ship::system_registry::navigation_system_id(),
                nav_station,
            ),
        ] {
            let system = config
                .system(&system_id)
                .unwrap_or_else(|| panic!("{stem} must declare {:?}", system_id.0));
            if (stem == &"alliance_destroyer"
                && (system_id.0 == "navigation" || system_id.0 == "comms"))
                || (stem == &"alliance_cruiser" && system_id.0 == "navigation")
            {
                assert!(
                    config
                        .station(&crate::core::messages::StationId(system_id.0.clone()))
                        .is_some_and(|station| station.human_seeking),
                    "Destroyer {:?} seeks as one complete Station",
                    system_id.0
                );
                assert!(
                    !system.human_seeking,
                    "the retired System overlay stays retired"
                );
            } else {
                assert!(
                    system.human_seeking,
                    "{stem}: {:?} keeps legacy System seeking until migrated",
                    system_id.0
                );
            }
            assert_eq!(
                crate::command_admission::station_for_system(&config, None, &system_id),
                Some(crate::core::messages::StationId((*expected).into())),
                "{stem}: {:?} must resolve to its authored station",
                system_id.0
            );
        }
        let naive =
            crate::core::messages::StationId(crate::ship::system_registry::COMMS_SYSTEM_ID.into());
        if *comms_station != crate::ship::system_registry::COMMS_SYSTEM_ID {
            assert!(
                config.station(&naive).is_none(),
                "{stem}: this hull homes comms on {comms_station:?} and declares NO \
                 comms station — the old StationId(\"comms\") literal resolved to \
                 nobody here, which is why CommsState never arrived"
            );
        }
    }
}

/// The destroyer's authored `host_order` for its two complete human-seeking
/// Stations (issue #984, #1097, #1098). Not a restatement of the TOML: it
/// also pins that each order is a permutation of the hull's OTHER stations
/// and that it DIFFERS from the derived one, which is the only reason to
/// author it at all. Neither Comms nor Navigation carries a legacy
/// `[[system]] seek_order` any more — both are complete Stations now.
#[test]
fn the_destroyer_authors_engineering_second_in_its_seek_order() {
    let config = hull_ship_config("alliance_destroyer");
    let authored_stations: Vec<&str> = config.stations.iter().map(|s| s.id.0.as_str()).collect();
    assert_eq!(
        authored_stations,
        vec![
            "captain",
            "helm",
            "tactical",
            "navigation",
            "comms",
            "engineering",
            // The auxiliary Command station (issue #1107): mounted and
            // resolved like any other, so it takes its authored place in the
            // roster, but `auxiliary = true` keeps it off the lobby's seat
            // list.
            "command"
        ],
        "the [[station]] array is the lobby's row order and the broadcast \
         router's fan-out order — the seek order does not touch it"
    );

    let comms = config
        .station(&crate::core::messages::StationId("comms".into()))
        .expect("the destroyer declares a comms Station");
    let order: Vec<&str> = comms.host_order.iter().map(|s| s.0.as_str()).collect();
    assert_eq!(
        order,
        vec!["tactical", "engineering", "captain", "helm"],
        "comms: Tactical owns it, then Engineering — John's ruling. Navigation \
         was dropped now that it too is an auxiliary visiting station: a \
         visiting station cannot host another visiting station."
    );
    // Every OTHER station a visiting Comms could land on is covered — but the
    // auxiliary stations (Navigation, Comms itself, and Command, issue #1107)
    // are not candidate hosts: a visiting station never lands on an auxiliary
    // surface, so they are correctly absent from the order and the count.
    let host_candidates = config
        .stations
        .iter()
        .filter(|s| !s.auxiliary && s.id.0 != "comms")
        .count();
    assert_eq!(
        order.len(),
        host_candidates,
        "comms: the order covers every OTHER non-auxiliary seat, so no \
         reachable seat is unreachable"
    );
    assert_ne!(
        order,
        authored_stations
            .iter()
            .filter(|s| **s != "comms" && **s != "command" && **s != "navigation")
            .copied()
            .collect::<Vec<_>>(),
        "comms: an order identical to the authored station list would be \
         the derived walk written out, and worth nothing"
    );
}

/// The authored order, live: Tactical empty and three other seats crewed.
/// The DERIVED walk would hand both systems to the Captain (first authored
/// station); the authored one hands them to Engineering. Navigation still
/// remains AI-operated at its authored Simplified visiting rating until a
/// scenario raises its detail floor.
#[test]
fn the_destroyers_seek_order_prefers_engineering_over_the_captain() {
    let mut app = seeking_app("alliance_destroyer", &["captain", "helm", "engineering"]);
    tick(&mut app);

    for system_id in [
        crate::ship::system_registry::comms_system_id(),
        crate::ship::system_registry::navigation_system_id(),
    ] {
        assert_eq!(
            host_of(&mut app, &system_id).as_deref(),
            Some("engineering"),
            "{:?}: the authored order promotes Engineering ahead of the \
             Captain, whose attention is meant to stay on the whole board",
            system_id.0
        );
    }
    assert_eq!(
        source_of(&mut app, &crate::ship::system_registry::comms_system_id()),
        ControlSource::Human,
        "Comms remains fully human-operated for its visiting host"
    );
    assert_eq!(
        source_of(
            &mut app,
            &crate::ship::system_registry::navigation_system_id()
        ),
        ControlSource::Ai,
        "Navigation stays AUTO at its authored Simplified visiting rating without a scenario floor"
    );
}

/// Owner-first, on real hulls: with the seeking system's OWN station crewed,
/// that is where it hosts — the hull's Comms officer keeps their console
/// even though `comms` is the LAST authored station on two of these hulls.
#[test]
fn a_seeking_system_hosts_on_its_own_station_when_that_seat_is_crewed() {
    for (stem, comms_station, nav_station) in SEEKING_HULLS {
        for (system_id, owner) in [
            (
                crate::ship::system_registry::comms_system_id(),
                comms_station,
            ),
            (
                crate::ship::system_registry::navigation_system_id(),
                nav_station,
            ),
        ] {
            let mut app = seeking_app(stem, &[owner]);
            tick(&mut app);
            assert_eq!(
                host_of(&mut app, &system_id).as_deref(),
                Some(*owner),
                "{stem}: {:?} must host on its own crewed station",
                system_id.0
            );
            assert_eq!(
                source_of(&mut app, &system_id),
                ControlSource::Human,
                "{stem}: {:?} must accept human input once a human hosts it",
                system_id.0
            );
        }
    }
}

/// The whole point of the mechanism: with nobody at the comms or navigation
/// seats but a human on the bridge, both consoles follow the human. The
/// battleship is the sharpest case — its `comms` and `navigation` stations
/// each own exactly one system, and it is the seeking one.
#[test]
fn seeking_systems_follow_the_only_human_on_the_bridge() {
    let mut app = seeking_app("alliance_battleship", &["captain"]);
    tick(&mut app);
    for system_id in [
        crate::ship::system_registry::comms_system_id(),
        crate::ship::system_registry::navigation_system_id(),
    ] {
        assert_eq!(
            host_of(&mut app, &system_id).as_deref(),
            Some("captain"),
            "{:?} must follow the seated Captain",
            system_id.0
        );
        assert_eq!(source_of(&mut app, &system_id), ControlSource::Human);
    }
}

/// ...and it must not STEAL the console: with both seats crewed, owner-first
/// leaves comms with the Comms officer even though `captain` is authored
/// first and `comms` last.
#[test]
fn an_earlier_authored_human_does_not_take_the_console_off_its_own_officer() {
    let mut app = seeking_app("alliance_battleship", &["captain", "comms"]);
    tick(&mut app);
    assert_eq!(
        host_of(&mut app, &crate::ship::system_registry::comms_system_id()).as_deref(),
        Some("comms"),
    );
    assert_eq!(
        host_of(
            &mut app,
            &crate::ship::system_registry::navigation_system_id()
        )
        .as_deref(),
        Some("captain"),
        "navigation's own station is empty, so it falls through to the Captain"
    );
}

/// Determinism (issue #984 §2.4). A headless or replayed run has an EMPTY
/// `Sessions`, so no seat ever has a holder, every seek returns `None`, and
/// the resolver writes the `Ai` `seed_boot_ratings` already set. No host map
/// entry is minted. Nothing on any determinism-suite path changes — proved
/// here mechanically rather than argued.
#[test]
fn an_empty_session_map_leaves_every_shipped_hull_exactly_as_booted() {
    for (stem, _, _) in SEEKING_HULLS {
        let mut app = seeking_app(stem, &[]);
        let before = {
            let ship = find_ship_entity(&mut app);
            app.world()
                .entity(ship)
                .get::<ShipSystemControlSources>()
                .unwrap()
                .clone()
        };
        tick(&mut app);
        tick(&mut app);
        let ship = find_ship_entity(&mut app);
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<ShipSystemControlSources>()
                .unwrap(),
            &before,
            "{stem}: with nobody connected the resolver must be a no-op"
        );
        for system_id in [
            crate::ship::system_registry::comms_system_id(),
            crate::ship::system_registry::navigation_system_id(),
        ] {
            assert_eq!(
                source_of(&mut app, &system_id),
                ControlSource::Ai,
                "{stem}: {:?} stays AI-operated with no human anywhere",
                system_id.0
            );
            assert_eq!(
                host_of(&mut app, &system_id),
                None,
                "{stem}: no human host is recorded when there is no human"
            );
        }
    }
}

/// The resolver re-asserts EVERY tick. `apply_rating` rewrites every system
/// its station owns whenever a lobby event fires — including a sought one —
/// so a resolver that only ran on a change of its own inputs would be
/// silently undone.
#[test]
fn a_rating_event_that_backfills_the_host_station_is_re_asserted_next_tick() {
    let mut app = seeking_app("alliance_destroyer", &["captain"]);
    tick(&mut app);
    let comms = crate::ship::system_registry::comms_system_id();
    assert_eq!(host_of(&mut app, &comms).as_deref(), Some("captain"));

    // What `handle_station_rating_change` does on a lobby event: re-apply
    // the Captain's rating, which sets every captain-owned system — comms
    // among them, since the seek put it there — back to that rating's answer.
    {
        let ship = find_ship_entity(&mut app);
        let mut entity = app.world_mut().entity_mut(ship);
        let mut sources = entity.get_mut::<ShipSystemControlSources>().unwrap();
        sources.0.set(comms.clone(), ControlSource::Ai);
    }
    tick(&mut app);
    assert_eq!(
        source_of(&mut app, &comms),
        ControlSource::Human,
        "the seek must re-assert its answer, not latch on first resolution"
    );
}

/// The admission consequence, end to end on the hull that needs it most:
/// the destroyer homes comms on `tactical`, nobody is sitting there, and the
/// Captain — who holds no station the comms system authors — is nonetheless
/// the admitted operator, because the seek made them its host.
#[test]
fn the_sought_host_is_the_token_admission_accepts_for_the_comms_system() {
    let mut app = seeking_app("alliance_destroyer", &["captain"]);
    tick(&mut app);
    let ship = find_ship_entity(&mut app);
    let world = app.world();
    let config = world.entity(ship).get::<ShipConfigComponent>().unwrap();
    let sources = world
        .entity(ship)
        .get::<ShipSystemControlSources>()
        .unwrap();
    let hosts = world.entity(ship).get::<HumanSeekingHosts>();
    let sessions = world.resource::<Sessions>();
    let payload = crate::core::messages::SystemControlPayload::ClearComms;
    let comms = crate::ship::system_registry::comms_system_id();

    assert!(
        crate::command_admission::is_command_authorized(
            "officer-captain",
            &comms,
            &payload,
            sources,
            sessions,
            &config.0,
            hosts,
        ),
        "the sought host's token must be admitted to the system it now hosts"
    );
    assert!(
        !crate::command_admission::is_command_authorized(
            "officer-captain",
            &comms,
            &payload,
            sources,
            sessions,
            &config.0,
            None,
        ),
        "and without the host map it must NOT be — otherwise this test would \
         pass on the authored station alone and prove nothing"
    );
}

#[test]
fn only_the_live_resolved_host_can_command_complete_navigation() {
    let mut app = seeking_app("alliance_destroyer", &["tactical", "captain"]);
    app.world_mut()
        .insert_resource(crate::world::config::WorldConfig {
            scenario_detail_floor: vec!["navigation".into()],
            ..Default::default()
        });
    tick(&mut app);
    let navigation = crate::ship::system_registry::navigation_system_id();
    let payload = crate::core::messages::SystemControlPayload::ClearNavigationWaypoint;

    let authorized = |app: &mut App, token: &str| {
        let ship = find_ship_entity(app);
        let world = app.world();
        crate::command_admission::is_command_authorized(
            token,
            &navigation,
            &payload,
            world
                .entity(ship)
                .get::<ShipSystemControlSources>()
                .unwrap(),
            world.resource::<Sessions>(),
            &world.entity(ship).get::<ShipConfigComponent>().unwrap().0,
            world.entity(ship).get::<HumanSeekingHosts>(),
        )
    };
    assert!(authorized(&mut app, "officer-tactical"));
    assert!(!authorized(&mut app, "officer-captain"));

    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .disconnect("officer-tactical");
    tick(&mut app);
    assert!(
        !authorized(&mut app, "officer-tactical"),
        "the stale host is refused"
    );
    assert!(
        authorized(&mut app, "officer-captain"),
        "the next authored host takes authority"
    );
}
