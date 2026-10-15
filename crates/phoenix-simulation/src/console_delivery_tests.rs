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
    app.world_mut().spawn((
        LocalShip,
        EntityUuid("own".into()),
        crate::science::broad::BroadProfile::from_config(&config)
            .unwrap()
            .unwrap(),
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
    use bevy::ecs::system::RunSystemOnce;
    let mut app = fixture();
    let old = app.world().resource::<WorldResource>().0.clone();
    let mut entities = std::collections::BTreeMap::new();
    for record in &old.entities {
        let position = record.position.unwrap();
        let bundle = (
            Transform::from_xyz(position[0], position[1], position[2]),
            crate::entities::spawner::EntityTagsSection(crate::entities::tags::parse_tags(
                &record.tags,
            )),
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
        updates: vec![(
            SystemId("sensor-radar".into()),
            SystemBlackboard::SensorRadar(Default::default()),
        )],
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
    let output = capture(&mut app).project(&raw);
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].target, Target::All);
    assert_eq!(output[0].msg, raw.msg);
}
