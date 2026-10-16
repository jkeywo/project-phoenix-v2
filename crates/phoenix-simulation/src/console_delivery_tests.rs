use super::*;
use crate::core::codec::JsonCodec;
use crate::core::messages::{
    DeliveryClass, EntityStateSnapshot, GamePhase, SimSnapshot, SystemBlackboard, SystemId,
};
use bevy::ecs::system::SystemState;

fn entity(uuid: &str, tags: &[&str]) -> EntitySnapshot {
    EntitySnapshot {
        uuid: uuid.into(),
        id: Some(format!("private-script-{uuid}")),
        name: Some(format!("name-{uuid}")),
        position: Some(match uuid {
            "own" => [-2.0, 0.0, -4.0],
            "chart" => [12.0, 0.0, 34.0],
            _ => [123.0, 7.0, 456.0],
        }),
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        radius: Some(9.0),
        hull_fraction: Some(0.37),
        shield_fraction: Some(0.42),
        infrastructure: Some(crate::core::messages::InfrastructureSnapshot {
            condition_fraction: 0.13,
            flags: vec![],
            capacities: vec![(format!("secret-cargo-{uuid}"), 917)],
        }),
        ..Default::default()
    }
}

fn fixture() -> App {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = App::new();
    let mut sessions = crate::lobby::session::SessionManager::new();
    for token in ["science", "helm", "spectator", "unseated", "gone"] {
        sessions.register(token.into(), token.into()).unwrap();
    }
    sessions.set_station(
        "science",
        Some(crate::core::messages::StationId("science".into())),
    );
    sessions.set_station(
        "helm",
        Some(crate::core::messages::StationId("helm".into())),
    );
    sessions.set_spectator("spectator", true);
    sessions.disconnect("gone");
    app.insert_resource(Sessions(sessions));
    app.insert_resource(WorldResource(WorldData {
        entities: vec![
            entity("own", &["ship"]),
            entity("hidden", &["ship"]),
            entity("obstruction", &["planet"]),
            entity("chart", &["planet", CHARTED_TAG]),
            entity("concealed-chart", &["planet", CHARTED_TAG]),
        ],
        scenario_title: "public-title".into(),
        scenario_description: "public-description".into(),
    }));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime
        .contact_overrides
        .entry("own".into())
        .or_default()
        .insert(
            "concealed-chart".into(),
            crate::gm::operations::contact::ContactMode::Conceal,
        );
    app.insert_resource(runtime);
    let profile = crate::science::broad::BroadProfile::from_config(&config)
        .unwrap()
        .unwrap();
    let tracks = crate::science::broad::BroadTracks::new(&profile);
    let ship = config.ship_config.clone().unwrap();
    let controls =
        crate::science::broad::SensingControls(config.sensing.as_ref().unwrap().control.clone());
    let mut client = crate::core::messages::ShipClientConfig::default();
    client.system_kinds = ship
        .systems
        .iter()
        .map(|system| (system.id.0.clone(), system.kind.clone()))
        .collect();
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(client));
    let hosts = crate::ship::components::ConsoleHosts(
        ship.consoles
            .iter()
            .map(|console| phoenix_model::console::ConsoleHostSnapshot {
                console: console.id.clone(),
                host: console.home_station.clone(),
                rating: String::new(),
            })
            .collect(),
    );
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        SystemId("sensor-radar".into()),
        SystemBlackboard::SensorRadar(Default::default()),
    );
    boards.0.insert(
        SystemId("tactical-radar".into()),
        SystemBlackboard::TacticalRadar(Default::default()),
    );
    boards.0.insert(
        SystemId("helm-radar".into()),
        SystemBlackboard::Helm(Default::default()),
    );
    app.world_mut().spawn((
        LocalShip,
        EntityUuid("own".into()),
        profile,
        tracks,
        controls,
        crate::ship::components::ShipConfigComponent(ship),
        hosts,
        boards,
    ));
    app
}

fn capture(app: &mut App) -> CapturedConsoleDelivery {
    let mut state = SystemState::<ConsoleDeliveryProjection>::new(app.world_mut());
    state.get(app.world()).capture()
}

fn packet(msg: ServerMessage) -> OutboundMessage {
    OutboundMessage {
        target: Target::All,
        msg,
        delivery: DeliveryClass::Reliable,
    }
}

fn phone<'a>(messages: &'a [OutboundMessage], token: &str) -> Option<&'a ServerMessage> {
    messages
        .iter()
        .find_map(|out| (out.target == Target::Token(token.into())).then_some(&out.msg))
}

fn encoded(message: &ServerMessage) -> String {
    let wire = JsonCodec.encode_server(message).unwrap();
    assert_eq!(JsonCodec.decode_server(&wire).unwrap(), *message);
    wire
}

fn assert_no_hidden(message: &ServerMessage) {
    let wire = encoded(message);
    for secret in [
        "hidden",
        "obstruction",
        "concealed-chart",
        "secret-cargo-chart",
        "private-script-chart",
        "123.0",
        "456.0",
    ] {
        assert!(!wire.contains(secret), "{secret} escaped in {wire}");
    }
}

fn world_message(app: &App) -> ServerMessage {
    ServerMessage::WorldSetup {
        world: app.world().resource::<WorldResource>().0.clone(),
    }
}

fn local_ship(app: &mut App) -> Entity {
    let mut query = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
    query.single(app.world()).unwrap()
}

fn observe(app: &mut App, owner: &str, tick: u64, paid: f32) -> String {
    use crate::science::broad::{BroadMeasurement, TrackIdentity};
    let entity = local_ship(app);
    let owner = SystemId(owner.into());
    let profile = app
        .world()
        .get::<crate::science::broad::BroadProfile>(entity)
        .unwrap()
        .0[&owner]
        .profile
        .clone();
    let mut states = app
        .world_mut()
        .get_mut::<crate::science::broad::BroadTracks>(entity)
        .unwrap();
    let state = states.0.get_mut(&owner).unwrap();
    let samples = [BroadMeasurement {
        elevation: None,
        observation: None,
        source: "hidden".into(),
        selectable: true,
        position: [20.0, -40.0],
        support: 1.0,
        identity: Some(TrackIdentity {
            name: "Recorded public identity".into(),
            faction: None,
            references: Vec::new(),
        }),
        classification: None,
        assessment: None,
    }];
    state
        .advance(
            &profile,
            tick,
            0.5,
            paid,
            if paid > 0.0 { &samples } else { &[] },
        )
        .unwrap();
    let contact = state.tracks.keys().next().unwrap().clone();
    let public = state.projection(&profile, tick, 0.5).unwrap();
    drop(states);
    let mut boards = app
        .world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap();
    match boards.0.get_mut(&owner).unwrap() {
        SystemBlackboard::SensorRadar(board) => board.tracks = Some(public),
        SystemBlackboard::TacticalRadar(board) => board.tracks = Some(public),
        SystemBlackboard::Helm(board) => board.tracks = Some(public),
        _ => unreachable!(),
    }
    contact
}

fn board_packet(app: &mut App, sources: &[&str], target: Target) -> OutboundMessage {
    let entity = local_ship(app);
    let boards = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0;
    let sources: Vec<_> = sources
        .iter()
        .map(|source| SystemId((*source).into()))
        .collect();
    OutboundMessage {
        target,
        delivery: DeliveryClass::Snapshot,
        msg: ServerMessage::BlackboardUpdate {
            console_scope: Some(sources.clone()),
            console_scope_tick: Some(20),
            updates: sources
                .iter()
                .map(|source| (source.clone(), boards[source].clone()))
                .map(|(id, board)| (id, board.into()))
                .collect(),
            presentation_generation: Some(2),
        },
    }
}

fn only_board(message: &ServerMessage) -> &SystemBlackboard {
    let ServerMessage::BlackboardUpdate { updates, .. } = message else {
        panic!("typed publication");
    };
    assert_eq!(updates.len(), 1);
    &updates[0].1
}

fn shared_board(message: &ServerMessage) -> &crate::core::messages::SharedSystemBlackboard {
    let ServerMessage::BlackboardUpdate { updates, .. } = message else {
        panic!("typed publication")
    };
    assert_eq!(updates.len(), 1);
    &updates[0].1
}

#[test]
fn shared_delivery_copies_only_rewritten_boards_and_retains_original_message() {
    use std::sync::Arc;
    let mut app = fixture();
    let contact = observe(&mut app, "sensor-radar", 0, 1.0);
    let unchanged = board_packet(&mut app, &["sensor-radar"], Target::All);
    let projection = capture(&mut app);
    let output = projection.project(&unchanged);
    assert!(Arc::ptr_eq(
        shared_board(&unchanged.msg),
        shared_board(phone(&output, "science").unwrap())
    ));
    assert!(Arc::ptr_eq(
        shared_board(&unchanged.msg),
        shared_board(phone(&output, crate::console_bridge::LOCAL_CONSOLE_TOKEN).unwrap())
    ));

    let entity = local_ship(&mut app);
    {
        let mut boards = app
            .world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
            .unwrap();
        let SystemBlackboard::SensorRadar(board) =
            boards.0.get_mut(&SystemId("sensor-radar".into())).unwrap()
        else {
            unreachable!()
        };
        board.selected_target = Some("hidden".into());
        board.selected_target_alert = Some(true);
    }
    let original = board_packet(&mut app, &["sensor-radar"], Target::All);
    let original_json = JsonCodec.encode_server(&original.msg).unwrap();
    let original_copy = original.clone();
    assert!(Arc::ptr_eq(
        shared_board(&original.msg),
        shared_board(&original_copy.msg)
    ));
    let output = projection.project(&original);
    let phone_message = phone(&output, "science").unwrap();
    assert!(!Arc::ptr_eq(
        shared_board(&original.msg),
        shared_board(phone_message)
    ));
    let SystemBlackboard::SensorRadar(board) = only_board(phone_message) else {
        unreachable!()
    };
    assert_eq!(board.selected_target.as_deref(), Some(contact.as_str()));
    assert_eq!(board.selected_target_alert, Some(true));
    assert!(Arc::ptr_eq(
        shared_board(&original.msg),
        shared_board(phone(&output, crate::console_bridge::LOCAL_CONSOLE_TOKEN).unwrap())
    ));
    assert_eq!(
        JsonCodec.encode_server(&original.msg).unwrap(),
        original_json
    );
    assert_eq!(
        JsonCodec.encode_server(&original_copy.msg).unwrap(),
        original_json
    );
    assert_eq!(
        JsonCodec.encode_server(&unchanged.msg).unwrap(),
        JsonCodec
            .encode_server(phone(&projection.project(&unchanged), "science").unwrap())
            .unwrap()
    );

    // An already-sanitized retained packet is shared, rather than copied merely
    // because its variant supports private references.
    let sanitized = OutboundMessage {
        target: Target::Token("science".into()),
        msg: phone_message.clone(),
        delivery: DeliveryClass::Snapshot,
    };
    let repeated = projection.project(&sanitized);
    assert!(Arc::ptr_eq(
        shared_board(&sanitized.msg),
        shared_board(phone(&repeated, "science").unwrap())
    ));
}

#[test]
fn machinery_reference_boards_keep_physical_work_but_expose_only_held_contact_and_scan_facts() {
    let mut app = fixture();
    let alias = observe(&mut app, "sensor-radar", 0, 1.0);
    let owner = SystemId("sensor-radar".into());
    let originals = [
        SystemBlackboard::Dock(crate::core::messages::DockBlackboard {
            available: true,
            available_target: Some("hidden".into()),
            available_target_name: Some("SECRET".into()),
            engaged: true,
            docked: true,
            docked_to: Some("hidden".into()),
            docked_to_name: Some("SECRET".into()),
            ..Default::default()
        }),
        SystemBlackboard::Tractor(crate::core::messages::TractorBlackboard {
            engaged: true,
            coupled_target: Some("hidden".into()),
            coupled_target_name: Some("SECRET".into()),
            ..Default::default()
        }),
        SystemBlackboard::Transporter(crate::core::messages::TransporterBlackboard {
            selected_contact: Some("hidden".into()),
            selected_contact_name: Some("SECRET".into()),
            discovered: true,
            civilians_remaining: 917,
            transporting: true,
            progress: 0.25,
            ..Default::default()
        }),
    ];
    for original in &originals {
        let mut board = original.clone();
        capture(&mut app).references.board(&owner, &mut board);
        let wire = serde_json::to_string(&board).unwrap();
        assert!(!wire.contains("hidden") && !wire.contains("SECRET") && !wire.contains("917"));
        assert!(wire.contains(&alias));
        match board {
            SystemBlackboard::Dock(board) => {
                assert!(board.engaged && board.docked && board.available);
                assert!(board.docked_to_name.is_none());
            }
            SystemBlackboard::Tractor(board) => assert!(board.engaged),
            SystemBlackboard::Transporter(board) => {
                assert!(board.transporting);
                assert_eq!(board.progress, 0.25);
                assert!(!board.discovered);
                assert_eq!(board.civilians_remaining, 0);
            }
            _ => unreachable!(),
        }
    }
    // A General structure reading contains no personnel quantity. Even with
    // that held reading, a supplied live count must remain unavailable.
    let entity = local_ship(&mut app);
    let scan_key = crate::science::scan_blackboard_key_for(&SystemId("sensors".into()));
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            scan_key,
            SystemBlackboard::Scan(crate::core::messages::ScanBlackboard {
                owner: Some(SystemId("sensors".into())),
                reading: Some(crate::core::messages::ScanReadingSnapshot {
                    subject_uuid: alias.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        );
    let mut board = originals[2].clone();
    capture(&mut app).references.board(&owner, &mut board);
    let SystemBlackboard::Transporter(board) = board else {
        unreachable!()
    };
    assert!(!board.discovered);
    assert_eq!(board.civilians_remaining, 0);
    observe(&mut app, "sensor-radar", 1, 0.0);
    for mut board in originals {
        capture(&mut app).references.board(&owner, &mut board);
        let wire = serde_json::to_string(&board).unwrap();
        assert!(!wire.contains("hidden") && !wire.contains("SECRET") && !wire.contains(&alias));
        match board {
            SystemBlackboard::Dock(board) => {
                assert!(board.docked && board.engaged && !board.available)
            }
            SystemBlackboard::Tractor(board) => assert!(board.engaged),
            SystemBlackboard::Transporter(board) => {
                assert!(board.transporting);
                assert_eq!(board.progress, 0.25);
                assert!(!board.discovered);
                assert_eq!(board.civilians_remaining, 0);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn helm_aliases_require_the_actual_instrument_publication_not_the_motion_aggregate() {
    let mut app = fixture();
    let alias = observe(&mut app, "helm-radar", 0, 1.0);
    let entity = local_ship(&mut app);
    let mut board = SystemBlackboard::Viewscreen(crate::core::messages::ViewscreenBlackboard {
        last_attacker_uuid: Some("hidden".into()),
        ..Default::default()
    });
    capture(&mut app)
        .references
        .board(&SystemId("helm-radar".into()), &mut board);
    let SystemBlackboard::Viewscreen(projected) = &board else {
        unreachable!()
    };
    assert_eq!(
        projected.last_attacker_uuid.as_deref(),
        Some(alias.as_str())
    );

    let mut boards = app
        .world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap();
    let measured = boards.0.remove(&SystemId("helm-radar".into())).unwrap();
    boards
        .0
        .insert(crate::ship::system_registry::helm_station_key(), measured);
    drop(boards);
    let SystemBlackboard::Viewscreen(projected) = &mut board else {
        unreachable!()
    };
    projected.last_attacker_uuid = Some("hidden".into());
    capture(&mut app)
        .references
        .board(&SystemId("helm-radar".into()), &mut board);
    let SystemBlackboard::Viewscreen(projected) = board else {
        unreachable!()
    };
    assert!(projected.last_attacker_uuid.is_none());
}

#[test]
fn typed_viewscreen_aliases_use_its_own_console_not_the_strongest_cohosted_receiver() {
    use phoenix_model::console::{
        ConsoleConfig, ConsoleFeed, ConsoleHostSnapshot, ConsoleObservationKind,
    };
    let mut app = fixture();
    let sensor = observe(&mut app, "sensor-radar", 0, 1.0);
    let tactical = observe(&mut app, "tactical-radar", 0, 1.0);
    let entity = local_ship(&mut app);
    let private = crate::core::messages::ConsoleId("private-view".into());
    {
        let mut config = app
            .world_mut()
            .get_mut::<crate::ship::components::ShipConfigComponent>(entity)
            .unwrap();
        config.0.consoles.push(ConsoleConfig {
            id: private.clone(),
            home_station: Some(crate::core::messages::StationId("science".into())),
            screen: Some("fixture.html".into()),
            ..Default::default()
        });
        config
            .0
            .systems
            .iter_mut()
            .find(|system| system.id == crate::ship::system_registry::viewscreen_system_id())
            .unwrap()
            .console = Some(private.clone());
    }
    app.world_mut()
        .get_mut::<crate::ship::components::ConsoleHosts>(entity)
        .unwrap()
        .0
        .push(ConsoleHostSnapshot {
            console: private.clone(),
            host: Some(crate::core::messages::StationId("science".into())),
            rating: "Std".into(),
        });
    let original = SystemBlackboard::Viewscreen(crate::core::messages::ViewscreenBlackboard {
        combat_lock: Some("hidden".into()),
        science_target: Some("hidden".into()),
        last_attacker_uuid: Some("hidden".into()),
        ..Default::default()
    });
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            crate::ship::system_registry::viewscreen_system_id(),
            original.clone(),
        );
    let raw = board_packet(&mut app, &["viewscreen"], Target::Token("science".into()));
    let projected = capture(&mut app).project(&raw);
    let SystemBlackboard::Viewscreen(board) = only_board(phone(&projected, "science").unwrap())
    else {
        unreachable!()
    };
    assert!(
        board.science_target.is_none()
            && board.combat_lock.is_none()
            && board.last_attacker_uuid.is_none(),
        "cohosted Science's own radar is not private-view's knowledge"
    );
    // A wrong typed feed still grants no knowledge.
    {
        let mut config = app
            .world_mut()
            .get_mut::<crate::ship::components::ShipConfigComponent>(entity)
            .unwrap();
        config
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id == private)
            .unwrap()
            .feeds
            .push(ConsoleFeed {
                source: SystemId("sensor-radar".into()),
                kind: ConsoleObservationKind::TacticalRadar,
            });
    }
    let projected = capture(&mut app).project(&raw);
    let SystemBlackboard::Viewscreen(board) = only_board(phone(&projected, "science").unwrap())
    else {
        unreachable!()
    };
    assert!(board.science_target.is_none());
    {
        let mut config = app
            .world_mut()
            .get_mut::<crate::ship::components::ShipConfigComponent>(entity)
            .unwrap();
        config
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id == private)
            .unwrap()
            .feeds[0]
            .kind = ConsoleObservationKind::SensorRadar;
    }
    let projected = capture(&mut app).project(&raw);
    let visible = phone(&projected, "science").unwrap();
    let SystemBlackboard::Viewscreen(board) = only_board(visible) else {
        unreachable!()
    };
    assert_eq!(board.science_target.as_deref(), Some(sensor.as_str()));
    assert!(board.combat_lock.is_none());
    assert_eq!(board.last_attacker_uuid.as_deref(), Some(sensor.as_str()));
    assert_no_hidden(visible);
    {
        let mut config = app
            .world_mut()
            .get_mut::<crate::ship::components::ShipConfigComponent>(entity)
            .unwrap();
        config
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id == private)
            .unwrap()
            .feeds
            .push(ConsoleFeed {
                source: SystemId("tactical-radar".into()),
                kind: ConsoleObservationKind::TacticalRadar,
            });
    }
    let projected = capture(&mut app).project(&raw);
    let SystemBlackboard::Viewscreen(board) = only_board(phone(&projected, "science").unwrap())
    else {
        unreachable!()
    };
    assert_eq!(board.combat_lock.as_deref(), Some(tactical.as_str()));
    assert_eq!(board.science_target.as_deref(), Some(sensor.as_str()));
    assert_eq!(
        board.last_attacker_uuid.as_deref(),
        Some(tactical.as_str()),
        "the explicitly bound combat instrument is preferred only when this Console permits it"
    );
    assert_eq!(
        app.world()
            .get::<crate::server_app::ShipSystemBlackboards>(entity)
            .unwrap()
            .0[&crate::ship::system_registry::viewscreen_system_id()],
        original,
        "no internal actuator references are changed"
    );
}

#[test]
fn typed_weapons_use_recorded_identity_and_lost_contacts_clear_without_reading_scene_truth() {
    let mut app = fixture();
    let contact = observe(&mut app, "tactical-radar", 0, 1.0);
    let entity = local_ship(&mut app);
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(
            crate::ship::system_registry::tactical_station_key(),
            SystemBlackboard::Weapons(crate::core::messages::WeaponsBlackboard {
                target_uuid: Some("hidden".into()),
                locked_target: Some("hidden".into()),
                target_name: Some("SECRET LIVE NAME".into()),
                ..Default::default()
            }),
        );
    let raw = board_packet(&mut app, &["tactical"], Target::Token("science".into()));
    let public = {
        let mut boards = app
            .world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
            .unwrap();
        let SystemBlackboard::TacticalRadar(board) = boards
            .0
            .get_mut(&SystemId("tactical-radar".into()))
            .unwrap()
        else {
            unreachable!()
        };
        board.tracks.take()
    };
    let projected = capture(&mut app).project(&raw);
    let SystemBlackboard::Weapons(board) = only_board(phone(&projected, "science").unwrap()) else {
        unreachable!()
    };
    assert!(
        board.target_uuid.is_none(),
        "private current routing without a published measurement grants no alias"
    );
    {
        let mut boards = app
            .world_mut()
            .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
            .unwrap();
        let SystemBlackboard::TacticalRadar(board) = boards
            .0
            .get_mut(&SystemId("tactical-radar".into()))
            .unwrap()
        else {
            unreachable!()
        };
        board.tracks = public;
    }
    let projected = capture(&mut app).project(&raw);
    let SystemBlackboard::Weapons(board) = only_board(phone(&projected, "science").unwrap()) else {
        unreachable!()
    };
    assert_eq!(board.target_uuid.as_deref(), Some(contact.as_str()));
    assert_eq!(board.locked_target, board.target_uuid);
    assert!(board.target_name.is_none());
    observe(&mut app, "tactical-radar", 1, 1.0);
    let projected = capture(&mut app).project(&raw);
    let visible = phone(&projected, "science").unwrap();
    assert_no_hidden(visible);
    assert!(!encoded(visible).contains("SECRET LIVE NAME"));
    let SystemBlackboard::Weapons(board) = only_board(visible) else {
        unreachable!()
    };
    assert_eq!(
        board.target_name.as_deref(),
        Some("Recorded public identity")
    );
    // The same sanctioned opaque reference is idempotent on an outbound copy.
    let idempotent = capture(&mut app).project(
        &projected
            .iter()
            .find(|out| out.target == Target::Token("science".into()))
            .unwrap()
            .clone(),
    );
    assert_eq!(phone(&idempotent, "science"), Some(visible));
    observe(&mut app, "tactical-radar", 2, 0.0);
    let projected = capture(&mut app).project(&raw);
    let SystemBlackboard::Weapons(board) = only_board(phone(&projected, "science").unwrap()) else {
        unreachable!()
    };
    assert!(
        board.target_uuid.is_none() && board.target_name.is_none() && board.locked_target.is_none()
    );
}

#[test]
fn typed_receipts_are_capped_before_aliasing_and_legacy_targets_do_not_broadcast_to_ungranted_consoles(
) {
    let mut app = fixture();
    let contact = observe(&mut app, "tactical-radar", 0, 1.0);
    let raw = board_packet(&mut app, &["sensor-radar"], Target::All);
    let output = capture(&mut app).project(&raw);
    let ServerMessage::BlackboardUpdate {
        console_scope,
        updates,
        ..
    } = phone(&output, "unseated").unwrap()
    else {
        unreachable!()
    };
    assert!(console_scope.as_ref().unwrap().is_empty() && updates.is_empty());
    assert_eq!(
        only_board(phone(&output, "science").unwrap()),
        &SystemBlackboard::SensorRadar(Default::default())
    );
    let legacy = packet(ServerMessage::TargetLock {
        uuid: "hidden".into(),
        locked: true,
    });
    let output = capture(&mut app).project(&legacy);
    assert_eq!(
        phone(&output, "science"),
        Some(&ServerMessage::TargetLock {
            uuid: contact,
            locked: true
        })
    );
    assert!(phone(&output, "unseated").is_none());
}

#[test]
fn typed_sensors_maps_keep_permitted_aliases_without_disclosing_a_decoy_specific_list() {
    let mut app = fixture();
    let contact = observe(&mut app, "sensor-radar", 0, 1.0);
    let entity = local_ship(&mut app);
    let ghost = "__gm_ghost:own:reported";
    let board = crate::core::messages::SensorsBlackboard {
        science_target_uuid: Some("hidden".into()),
        contact_ghosts: vec![EntitySnapshot {
            uuid: ghost.into(),
            name: Some("Reported point".into()),
            position: Some([1.0, 0.0, 2.0]),
            ..Default::default()
        }],
        contact_classifications: [
            ("hidden".into(), "Recorded classification".into()),
            ("unobserved-secret".into(), "UNSEEN".into()),
            (ghost.into(), "Reported class".into()),
        ]
        .into_iter()
        .collect(),
        contact_overrides: [(
            "concealed-chart".into(),
            phoenix_model::wire::ContactMode::Conceal,
        )]
        .into_iter()
        .collect(),
        contact_reports: [(
            "hidden".into(),
            Some(phoenix_model::wire::SensorReport {
                observation: Some(phoenix_model::observation::ObservationMetadata::at(
                    0,
                    phoenix_model::observation::ObservationSubject::Entity {
                        uuid: "hidden".into(),
                    },
                    phoenix_model::observation::ObservationProvenance {
                        kind: phoenix_model::observation::ObservationSource::SensorsReport,
                        system: None,
                        label: "console.sensors.report_source".into(),
                    },
                    Default::default(),
                )),
                observed_tick: 0,
                age_ticks: 3,
                source: "console.sensors.report_source".into(),
                name: "Reported label".into(),
                position_mm: [1000, 0, 2000],
            }),
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .insert(SystemId("sensors".into()), SystemBlackboard::Sensors(board));
    let raw = board_packet(&mut app, &["sensors"], Target::Token("science".into()));
    let projected = capture(&mut app).project(&raw);
    let visible = phone(&projected, "science").unwrap();
    assert_no_hidden(visible);
    assert!(!encoded(visible).contains("unobserved-secret"));
    let SystemBlackboard::Sensors(board) = only_board(visible) else {
        unreachable!()
    };
    assert_eq!(board.science_target_uuid.as_deref(), Some(contact.as_str()));
    assert_eq!(
        board.contact_classifications[&contact],
        "Recorded classification"
    );
    assert!(!board.contact_classifications.contains_key(ghost));
    assert!(board.contact_overrides.is_empty());
    assert!(board.contact_ghosts.is_empty());
    assert!(!encoded(visible).contains("__gm_ghost"));
    let SystemBlackboard::Sensors(original) = only_board(&raw.msg) else {
        unreachable!()
    };
    assert_eq!(
        original.contact_ghosts[0].position,
        Some([1.0, 0.0, 2.0]),
        "the raw host/GM source is not mutated"
    );
    let report = board.contact_reports[&contact].as_ref().unwrap();
    assert_eq!(report.age_ticks, 3);
    assert_eq!(report.position_mm, [1000, 0, 2000]);
    assert_eq!(
        report.observation.as_ref().unwrap().subject,
        phoenix_model::observation::ObservationSubject::Entity { uuid: contact }
    );
}

#[test]
fn actual_broad_hull_projects_initial_world_and_reconnect_at_the_same_encoded_boundary() {
    let mut app = fixture();
    let source = world_message(&app);
    let projection = capture(&mut app);
    let outputs = projection.project(&packet(source.clone()));
    assert_eq!(
        outputs.len(),
        5,
        "host plus four connected ordinary Sessions"
    );
    assert_eq!(
        phone(&outputs, crate::console_bridge::LOCAL_CONSOLE_TOKEN),
        Some(&source)
    );
    assert!(phone(&outputs, "gone").is_none());
    for token in ["science", "helm", "spectator", "unseated"] {
        let projected = phone(&outputs, token).unwrap();
        assert_no_hidden(projected);
        let ServerMessage::WorldSetup { world } = projected else {
            panic!("world projection");
        };
        assert_eq!(
            world
                .entities
                .iter()
                .map(|e| e.uuid.as_str())
                .collect::<Vec<_>>(),
            ["own", "chart"]
        );
        assert_eq!(world.scenario_title, "public-title");
        assert_eq!(
            world.entities[0].infrastructure,
            entity("own", &["ship"]).infrastructure
        );
        assert!(
            world.entities[1].hull_fraction.is_none() && world.entities[1].infrastructure.is_none()
        );
        assert_eq!(world.entities[1].position, Some([12.0, 0.0, 34.0]));
    }
    let mut welcome = crate::lobby::handler::welcome_message(
        &app.world().resource::<Sessions>().0,
        &GamePhase::InProgress,
        Some(&app.world().resource::<WorldResource>().0),
        &crate::lobby::stations_config::ShipStations {
            stations: vec![],
            consoles: vec![],
        },
        &Default::default(),
        &Default::default(),
        &[],
    );
    // A real reconnect Welcome is targeted, not a global reset.
    let outputs = projection.project(&OutboundMessage {
        target: Target::Token("science".into()),
        msg: welcome.clone(),
        delivery: DeliveryClass::Reliable,
    });
    assert_eq!(outputs.len(), 1);
    assert_no_hidden(&outputs[0].msg);
    let ServerMessage::Welcome { state, .. } = &mut welcome else {
        unreachable!()
    };
    state.world = None;
    assert_eq!(projection.project(&packet(welcome.clone()))[1].msg, welcome);
}

#[test]
fn live_delta_never_exposes_foreign_position_health_flags_or_shield_frequency() {
    let mut app = fixture();
    let projection = capture(&mut app);
    let state = |uuid: &str| EntityStateSnapshot {
        uuid: uuid.into(),
        position: entity(uuid, &[]).position,
        yaw: Some(0.7),
        hull_fraction: Some(0.37),
        shield_fraction: Some(0.42),
        flags: vec![],
        shields: Some(vec![]),
        shield_freq: Some(0.923),
        warp_out_remaining_secs: Some(7.0),
    };
    let mut snapshot: SimSnapshot = JsonCodec
        .decode_server(r#"{"type":"SimState","data":{"snapshot":{}}}"#)
        .map(|msg| {
            let ServerMessage::SimState { snapshot } = msg else {
                unreachable!()
            };
            snapshot
        })
        .unwrap();
    snapshot.entity_states = ["own", "hidden", "obstruction", "chart", "concealed-chart"]
        .map(state)
        .to_vec();
    let raw = ServerMessage::SimState { snapshot };
    let outputs = projection.project(&OutboundMessage {
        target: Target::AllExcept("helm".into()),
        msg: raw.clone(),
        delivery: DeliveryClass::Snapshot,
    });
    assert!(phone(&outputs, "helm").is_none());
    assert!(outputs
        .iter()
        .all(|out| out.delivery == DeliveryClass::Snapshot));
    assert_eq!(
        phone(&outputs, crate::console_bridge::LOCAL_CONSOLE_TOKEN),
        Some(&raw)
    );
    let projected = phone(&outputs, "science").unwrap();
    assert_no_hidden(projected);
    let ServerMessage::SimState { snapshot } = projected else {
        unreachable!()
    };
    assert_eq!(snapshot.entity_states.len(), 2);
    assert_eq!(snapshot.entity_states[0], state("own"));
    let chart = &snapshot.entity_states[1];
    assert_eq!(chart.position, state("chart").position);
    assert!(
        chart.hull_fraction.is_none() && chart.shields.is_none() && chart.shield_freq.is_none()
    );
}

#[test]
fn runtime_spawn_and_terminal_envelopes_never_disclose_hidden_actor_keys() {
    let mut app = fixture();
    for uuid in ["hidden", "obstruction", "concealed-chart"] {
        let projection = capture(&mut app);
        let spawned = projection.project(&packet(ServerMessage::EntitySpawned {
            snapshot: app
                .world()
                .resource::<WorldResource>()
                .0
                .entities
                .iter()
                .find(|e| e.uuid == uuid)
                .unwrap()
                .clone(),
        }));
        assert_eq!(
            spawned.len(),
            1,
            "only local renderer receives hidden spawn"
        );
        assert!(phone(&spawned, "science").is_none());
    }
    let mut chart = entity("new-chart", &["moon", CHARTED_TAG]);
    chart.position = Some([1.0, 0.0, 2.0]);
    let outputs =
        capture(&mut app).project(&packet(ServerMessage::EntitySpawned { snapshot: chart }));
    assert_no_hidden(phone(&outputs, "science").unwrap());
    for removal in [
        ServerMessage::EntityDespawned {
            uuid: "hidden".into(),
        },
        ServerMessage::AsteroidDestroyed {
            uuid: "hidden".into(),
        },
        ServerMessage::StationDestroyed {
            uuid: "hidden".into(),
        },
    ] {
        app.world_mut()
            .resource_mut::<WorldResource>()
            .0
            .entities
            .retain(|entity| entity.uuid != "chart");
        let output = capture(&mut app).project(&packet(removal));
        assert!(
            phone(&output, "science").is_none(),
            "an unseen death has no phone frame"
        );
    }
    let own_death = packet(ServerMessage::EntityDespawned { uuid: "own".into() });
    assert_eq!(
        phone(&capture(&mut app).project(&own_death), "science"),
        Some(&own_death.msg)
    );
}

#[test]
fn actual_reconciliation_refreshes_only_a_removed_visible_chart_landmark_once() {
    use crate::server_app::{
        LastBroadcastEntityHealth, LastBroadcastEntityPositions, SimOutbox, TrackedEntities,
    };
    let mut app = fixture();
    let old = app.world().resource::<WorldResource>().0.clone();
    let mut entities = std::collections::BTreeMap::new();
    for record in &old.entities {
        let position = record.position.unwrap();
        let bundle = (
            Transform::from_xyz(position[0], position[1], position[2]),
            crate::entities::spawner::EntityTagsSection(record.tags.clone()),
        );
        let entity = if record.uuid == "own" {
            let mut local = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
            let entity = local.single(app.world()).unwrap();
            app.world_mut().entity_mut(entity).insert(bundle);
            entity
        } else {
            app.world_mut()
                .spawn((EntityUuid(record.uuid.clone()), bundle))
                .id()
        };
        entities.insert(record.uuid.clone(), entity);
    }
    app.insert_resource(TrackedEntities {
        reported: old
            .entities
            .iter()
            .map(|record| record.uuid.clone())
            .collect(),
        seeded: true,
    });
    app.init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<LastBroadcastEntityHealth>()
        .init_resource::<SimOutbox>();
    let reconcile = |app: &mut App| {
        app.world_mut()
            .run_system_cached(crate::server_app::reconcile_runtime_entities)
            .unwrap();
        let messages = app.world_mut().resource_mut::<SimOutbox>().drain();
        let projection = capture(app);
        messages
            .into_iter()
            .flat_map(|out| {
                projection.project(&OutboundMessage {
                    target: out.target,
                    msg: out.message,
                    delivery: out.delivery,
                })
            })
            .collect::<Vec<_>>()
    };
    for uuid in ["hidden", "concealed-chart"] {
        app.world_mut().despawn(entities[uuid]);
        assert!(
            phone(&reconcile(&mut app), "science").is_none(),
            "{uuid} must not reveal even removal timing"
        );
    }
    app.world_mut().despawn(entities["chart"]);
    let output = reconcile(&mut app);
    let phone_frames = output
        .iter()
        .filter(|out| out.target == Target::Token("science".into()))
        .collect::<Vec<_>>();
    assert_eq!(phone_frames.len(), 1);
    assert_eq!(phone_frames[0].delivery, DeliveryClass::Reliable);
    assert_no_hidden(&phone_frames[0].msg);
    let ServerMessage::WorldSetup { world } = &phone_frames[0].msg else {
        panic!("one current public roster");
    };
    assert_eq!(
        world
            .entities
            .iter()
            .map(|entity| entity.uuid.as_str())
            .collect::<Vec<_>>(),
        ["own"]
    );
    assert!(
        reconcile(&mut app).is_empty(),
        "chart refresh is not a new periodic publisher"
    );
}

#[test]
fn visual_events_keep_only_source_labelled_own_launches_without_hidden_targets() {
    let mut app = fixture();
    let projection = capture(&mut app);
    let own = ServerMessage::BlasterFired {
        bank: "fore".into(),
        source_uuid: "own".into(),
        projectile_id: "own-bolt".into(),
        x: 1.0,
        y: 0.0,
        pitch: 0.0,
        z: 2.0,
        heading: 0.3,
        visual_scale: 1.0,
    };
    assert_eq!(
        phone(&projection.project(&packet(own.clone())), "science"),
        Some(&own)
    );
    let mut foreign = own.clone();
    let ServerMessage::BlasterFired { source_uuid, .. } = &mut foreign else {
        unreachable!()
    };
    *source_uuid = "hidden".into();
    let cases = [
        foreign,
        ServerMessage::BeamStarted {
            bank: "fore".into(),
            source_uuid: "own".into(),
            target_uuid: "hidden".into(),
        },
        ServerMessage::BeamEnded {
            bank: "fore".into(),
            source_uuid: "hidden".into(),
            target_uuid: "own".into(),
        },
        ServerMessage::PhaserFired {
            bank: "fore".into(),
            target_uuid: "hidden".into(),
        },
        ServerMessage::BlasterHit {
            bank: "fore".into(),
            projectile_id: "foreign-bolt".into(),
            target_uuid: "own".into(),
        },
        ServerMessage::TorpedoLaunched {
            uuid: "foreign-round".into(),
            tube: "fore".into(),
            x: 123.0,
            y: 7.0,
            z: 456.0,
            heading: 0.7,
        },
        ServerMessage::TorpedoDestroyed {
            uuid: "foreign-round".into(),
        },
    ];
    for message in cases {
        let output = projection.project(&packet(message.clone()));
        assert!(
            phone(&output, "science").is_none(),
            "visual truth cannot reach phone: {message:?}"
        );
        assert_eq!(
            phone(&output, crate::console_bridge::LOCAL_CONSOLE_TOKEN),
            Some(&message)
        );
    }
    let chart_beam = ServerMessage::BeamStarted {
        bank: "fore".into(),
        source_uuid: "own".into(),
        target_uuid: "chart".into(),
    };
    assert_eq!(
        phone(&projection.project(&packet(chart_beam.clone())), "science"),
        Some(&chart_beam)
    );
}

#[test]
fn existing_scoped_console_publications_and_host_class_gm_channels_are_unchanged() {
    let mut app = fixture();
    let projection = capture(&mut app);
    // This is already scoped by the real publication owner. The egress helper
    // must not pool/augment it using another mounted Console's observations.
    let board = packet(ServerMessage::BlackboardUpdate {
        console_scope: Some(vec![SystemId("sensor-radar".into())]),
        console_scope_tick: Some(17),
        updates: ([(
            SystemId("sensor-radar".into()),
            SystemBlackboard::SensorRadar(Default::default()),
        )])
        .into_iter()
        .map(|(id, board)| (id, board.into()))
        .collect(),
        presentation_generation: Some(2),
    });
    let mut board = board;
    board.target = Target::Token("science".into());
    let outputs = projection.project(&board);
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].target, board.target);
    assert_eq!(outputs[0].msg, board.msg);
    assert_eq!(encoded(&outputs[0].msg), encoded(&board.msg));
    crate::gm::continuity::solo::bind_standalone_game_master(app.world_mut()).unwrap();
    assert!(crate::gm::continuity::solo::local_gm_operator(app.world()).is_some());
    let raw = world_message(&app);
    let addressed = OutboundMessage {
        target: Target::Token(crate::console_bridge::LOCAL_CONSOLE_TOKEN.into()),
        msg: raw.clone(),
        delivery: DeliveryClass::Reliable,
    };
    let output = capture(&mut app).project(&addressed);
    assert_eq!(output.len(), 1);
    assert_eq!(encoded(&output[0].msg), encoded(&raw));
    // A client-chosen gm-looking token is never a privileged route.
    let forged = OutboundMessage {
        target: Target::Token("gm:forged".into()),
        ..addressed
    };
    assert!(capture(&mut app).project(&forged).is_empty());
}

#[test]
fn legacy_spawn_weapon_and_modifier_envelopes_cannot_reintroduce_hidden_actor_keys() {
    let mut app = fixture();
    let projection = capture(&mut app);
    let raw_spawns = [
        ServerMessage::StationSpawned {
            uuid: "hidden".into(),
            name: "name-hidden".into(),
            position: [123.0, 7.0, 456.0],
            shape: "sphere".into(),
            radius: 4.0,
            hull_integrity: 91.0,
        },
        ServerMessage::AsteroidSpawned {
            uuid: "hidden".into(),
            x: 123.0,
            y: 7.0,
            z: 456.0,
            config_path: "secret-template".into(),
            max_hp: 91,
            current_hp: 37,
            radius: 4.0,
            radar_icon: Some("asteroid".into()),
            radar_colour: None,
            radar_size: None,
        },
        ServerMessage::TargetLock {
            uuid: "hidden".into(),
            locked: true,
        },
        ServerMessage::ModifierAdded {
            source: ModifierSource::RegionEffect {
                uuid: uuid::Uuid::nil(),
            },
            slot: crate::core::messages::ModifierSlot::SensorRadarRange,
            bonus: -0.2,
        },
    ];
    for message in raw_spawns {
        let output = projection.project(&packet(message.clone()));
        assert!(phone(&output, "science").is_none());
        assert_eq!(
            phone(&output, crate::console_bridge::LOCAL_CONSOLE_TOKEN),
            Some(&message)
        );
    }
    let message = ServerMessage::WeaponsUpdate {
        target_uuid: Some("hidden".into()),
        target_name: Some("name-hidden".into()),
        banks: vec![],
        tubes: vec![],
        torpedo_count: 2,
        phaser_mode: crate::core::messages::PhaserMode::Auto,
        blasters: vec![],
        phaser_frequency: 0.5,
    };
    let output = projection.project(&packet(message));
    let visible = phone(&output, "science").unwrap();
    assert_no_hidden(visible);
    let ServerMessage::WeaponsUpdate {
        target_uuid,
        target_name,
        torpedo_count,
        ..
    } = visible
    else {
        unreachable!()
    };
    assert!(target_uuid.is_none() && target_name.is_none());
    assert_eq!(*torpedo_count, 2);
    let sequence = [
        packet(world_message(&app)),
        packet(ServerMessage::GameStarted),
        packet(ServerMessage::TargetLock {
            uuid: "own".into(),
            locked: false,
        }),
    ];
    let delivered = sequence
        .iter()
        .flat_map(|out| projection.project(out))
        .filter(|out| out.target == Target::All || out.target == Target::Token("science".into()))
        .map(|out| std::mem::discriminant(&out.msg))
        .collect::<Vec<_>>();
    assert_eq!(
        delivered,
        sequence
            .iter()
            .map(|out| std::mem::discriminant(&out.msg))
            .collect::<Vec<_>>()
    );
}

#[test]
fn unfitted_legacy_hosts_preserve_existing_delivery_and_projection_does_not_write_simulation() {
    let mut app = fixture();
    let world_before = encoded(&world_message(&app));
    let overrides_before = app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .contact_overrides
        .clone();
    let projection = capture(&mut app);
    let raw = packet(world_message(&app));
    let _ = projection.project(&raw);
    assert_eq!(encoded(&world_message(&app)), world_before);
    assert_eq!(
        app.world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        overrides_before
    );
    let mut query = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
    let local = query.single(app.world()).unwrap();
    app.world_mut()
        .entity_mut(local)
        .remove::<crate::science::broad::BroadProfile>();
    app.world_mut()
        .entity_mut(local)
        .remove::<crate::science::broad::SensingControls>();
    app.world_mut()
        .resource_mut::<crate::lobby::server::ShipClientConfigResource>()
        .0
        .system_kinds
        .clear();
    let output = capture(&mut app).project(&raw);
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].target, Target::All);
    assert_eq!(output[0].msg, raw.msg);
}

#[test]
fn focused_only_instruments_and_accepted_sensor_topology_never_enable_raw_delivery() {
    let mut app = fixture();
    let local = local_ship(&mut app);
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    app.world_mut()
        .entity_mut(local)
        .remove::<crate::science::broad::BroadProfile>()
        .remove::<crate::science::broad::SensingControls>()
        .insert(crate::science::ShipScanRecord::from_config(&config).unwrap());
    // The real focused owner alone activates privacy even in a reduced host
    // that does not install the accepted client configuration resource.
    app.world_mut()
        .remove_resource::<crate::lobby::server::ShipClientConfigResource>();
    let raw = packet(world_message(&app));
    let projected = capture(&mut app).project(&raw);
    assert_no_hidden(phone(&projected, "science").unwrap());
    assert_eq!(raw.msg, world_message(&app));
    app.world_mut()
        .entity_mut(local)
        .remove::<crate::science::ShipScanRecord>()
        .insert(crate::science::passive::PassiveProfile::from_config(&config).unwrap());
    assert_no_hidden(phone(&capture(&mut app).project(&raw), "science").unwrap());
    // Accepted topology also covers the gap before/after a live owner exists.
    app.world_mut()
        .entity_mut(local)
        .remove::<crate::science::passive::PassiveProfile>();
    let mut accepted = crate::core::messages::ShipClientConfig::default();
    accepted
        .system_kinds
        .insert("actual-focused".into(), "sensors".into());
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(accepted));
    assert_no_hidden(phone(&capture(&mut app).project(&raw), "science").unwrap());
}

#[test]
fn actual_local_hull_removal_keeps_accepted_privacy_without_guessing_an_observer() {
    let mut app = fixture();
    let local = local_ship(&mut app);
    app.init_resource::<crate::server_app::SimOutbox>();
    let _ = crate::gm::operations::despawn::remove_entity(app.world_mut(), local);
    let raw = packet(world_message(&app));
    let projection = capture(&mut app);
    let projected = projection.project(&raw);
    let ServerMessage::WorldSetup { world } = phone(&projected, "science").unwrap() else {
        unreachable!()
    };
    assert!(
        world.entities.is_empty(),
        "lost observer scope cannot disclose chart Conceal or foreign truth"
    );
    assert!(
        phone(
            &projection.project(&packet(ServerMessage::EntityDespawned {
                uuid: "own".into()
            })),
            "science"
        )
        .is_none(),
        "missing scope does not guess a terminal UUID"
    );
    let own_end = packet(ServerMessage::ShipDestroyed);
    let end = projection.project(&own_end);
    assert_eq!(end.len(), 1);
    assert_eq!(end[0].target, own_end.target);
    assert_eq!(end[0].msg, own_end.msg);
    assert_eq!(
        phone(&projected, crate::console_bridge::LOCAL_CONSOLE_TOKEN),
        Some(&raw.msg)
    );
}

#[test]
fn explicit_combat_binding_selects_its_measured_record_among_independent_tactical_instruments() {
    use crate::science::broad::{BroadProfile, BroadTrackState, BroadTracks, SensingControls};
    let mut app = fixture();
    let local = local_ship(&mut app);
    let second = SystemId("tactical-second".into());
    {
        let mut config = app
            .world_mut()
            .get_mut::<crate::ship::components::ShipConfigComponent>(local)
            .unwrap();
        let mut system = config
            .0
            .system(&SystemId("tactical-radar".into()))
            .unwrap()
            .clone();
        system.id = second.clone();
        config.0.systems.push(system);
    }
    let mut instrument = app.world().get::<BroadProfile>(local).unwrap().0
        [&SystemId("tactical-radar".into())]
        .clone();
    instrument.profile.system = second.clone();
    let state = BroadTrackState::new(&instrument.profile);
    app.world_mut()
        .get_mut::<BroadProfile>(local)
        .unwrap()
        .0
        .insert(second.clone(), instrument);
    app.world_mut()
        .get_mut::<BroadTracks>(local)
        .unwrap()
        .0
        .insert(second.clone(), state);
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(local)
        .unwrap()
        .0
        .insert(
            second.clone(),
            SystemBlackboard::TacticalRadar(Default::default()),
        );
    let first = observe(&mut app, "tactical-radar", 0, 1.0);
    let other = observe(&mut app, &second.0, 0, 1.0);
    assert_ne!(first, other);
    let raw = packet(ServerMessage::TargetLock {
        uuid: "hidden".into(),
        locked: true,
    });
    let selected = |app: &mut App| {
        let out = capture(app).project(&raw);
        match phone(&out, "science") {
            Some(ServerMessage::TargetLock { uuid, .. }) => Some(uuid.clone()),
            None => None,
            _ => unreachable!(),
        }
    };
    assert_eq!(selected(&mut app), Some(first));
    app.world_mut()
        .get_mut::<SensingControls>(local)
        .unwrap()
        .0
        .combat_lock = Some(second.clone());
    assert!(
        selected(&mut app).is_none(),
        "binding an actuator does not grant Science receipt of its radar"
    );
    {
        use phoenix_model::console::{ConsoleFeed, ConsoleObservationKind};
        let mut config = app
            .world_mut()
            .get_mut::<crate::ship::components::ShipConfigComponent>(local)
            .unwrap();
        let science = config
            .0
            .console_for_system(&SystemId("sensor-radar".into()))
            .unwrap()
            .id
            .clone();
        config
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id == science)
            .unwrap()
            .feeds
            .push(ConsoleFeed {
                source: second,
                kind: ConsoleObservationKind::TacticalRadar,
            });
    }
    assert_eq!(selected(&mut app), Some(other));
    app.world_mut()
        .get_mut::<SensingControls>(local)
        .unwrap()
        .0
        .combat_lock = None;
    assert!(
        selected(&mut app).is_none(),
        "explicitly unbound is not a unique-kind guess"
    );
    app.world_mut()
        .entity_mut(local)
        .remove::<SensingControls>();
    assert!(
        selected(&mut app).is_none(),
        "two unbound publishers are ambiguous"
    );
}

#[test]
fn outgoing_navigation_uses_its_exact_picture_and_preserves_only_public_chart_anchors() {
    let mut app = fixture();
    let handle = observe(&mut app, "tactical-radar", 0, 1.0);
    let projection = capture(&mut app).references;
    let mut board = SystemBlackboard::Navigation(crate::core::messages::NavigationBlackboard {
        navigation_waypoint: Some(crate::core::messages::WaypointSnapshot {
            x: 12.0,
            z: 34.0,
            source_uuid: Some("hidden".into()),
        }),
        ..Default::default()
    });
    projection.navigation(&SystemId("navigation".into()), &mut board, |_| false);
    let SystemBlackboard::Navigation(value) = &board else {
        unreachable!()
    };
    assert_eq!(
        value
            .navigation_waypoint
            .as_ref()
            .unwrap()
            .source_uuid
            .as_deref(),
        Some(handle.as_str())
    );
    assert!(!serde_json::to_string(&board).unwrap().contains("hidden"));
    let SystemBlackboard::Navigation(value) = &mut board else {
        unreachable!()
    };
    value.navigation_waypoint.as_mut().unwrap().source_uuid = Some("charted-public".into());
    projection.navigation(&SystemId("navigation".into()), &mut board, |id| {
        id == "charted-public"
    });
    let SystemBlackboard::Navigation(value) = &mut board else {
        unreachable!()
    };
    assert_eq!(
        value
            .navigation_waypoint
            .as_ref()
            .unwrap()
            .source_uuid
            .as_deref(),
        Some("charted-public")
    );
    value.navigation_waypoint.as_mut().unwrap().source_uuid = Some("unseen-private".into());
    projection.navigation(&SystemId("navigation".into()), &mut board, |_| false);
    let SystemBlackboard::Navigation(value) = &board else {
        unreachable!()
    };
    assert_eq!(
        value.navigation_waypoint.as_ref().unwrap().source_uuid,
        None
    );
    assert_eq!(value.navigation_waypoint.as_ref().unwrap().x, 12.0);
}

#[test]
fn legacy_navigation_projection_preserves_its_anchor() {
    let projection = ReferenceProjection::default();
    let mut board = SystemBlackboard::Navigation(crate::core::messages::NavigationBlackboard {
        navigation_waypoint: Some(crate::core::messages::WaypointSnapshot {
            x: 1.0,
            z: 2.0,
            source_uuid: Some("legacy-anchor".into()),
        }),
        ..Default::default()
    });
    let original = board.clone();
    projection.navigation(&SystemId("navigation".into()), &mut board, |_| false);
    assert_eq!(board, original);
}
#[path = "console_delivery_weapon_targets_tests.rs"]
mod weapon_assignment_views;
