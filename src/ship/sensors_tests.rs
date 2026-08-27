use super::*;
use crate::core::messages::*;
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
use crate::server_app::{ShipImpulse, SimOutbox};
use crate::ship::control_source::ControlSource;

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

#[derive(Resource, Default)]
struct EnqueueLog(Vec<CoordinationEnqueue>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

fn collect_enqueues(mut reader: MessageReader<CoordinationEnqueue>, mut log: ResMut<EnqueueLog>) {
    for m in reader.read() {
        log.0.push(m.clone());
    }
}

/// Test-only glue (issue #829): seed each ship's viewscreen combat_lock /
/// science_target from its `TacticalRadarSelection` / `SensorRadarSelection`
/// components before the consumers run, standing in for the radar publishers
/// + viewscreen aggregators the full app runs.
fn seed_viewscreen_from_selection(
    mut q: Query<
        (
            Option<&crate::console::weapons::TacticalRadarSelection>,
            Option<&SensorRadarSelection>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (tac, sci, mut bbs) in q.iter_mut() {
        let combat_lock = tac.and_then(|t| t.0.clone());
        let science_target = sci.and_then(|s| s.0.clone());
        let mut vbb = match bbs
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(v)) => v.clone(),
            _ => crate::core::messages::ViewscreenBlackboard::default(),
        };
        vbb.combat_lock = combat_lock;
        vbb.science_target = science_target;
        bbs.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vbb),
        );
    }
}

fn test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    // The applier (`handle_sensors_messages`) moved to SimSet::Physics
    // (issue #828), so the harness needs the production set chain for
    // AdmissionSet → Input → Physics ordering to hold.
    app.configure_sets(
        FixedUpdate,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Damage,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::PublishAggregate,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain(),
    );
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .init_resource::<EnqueueLog>()
        .init_resource::<crate::lobby::server::ShipClientConfigResource>()
        .add_plugins(ShipSensorsPlugin)
        .add_systems(
            FixedUpdate,
            seed_viewscreen_from_selection.before(crate::sim_sets::SimSet::Input),
        )
        .add_systems(PostUpdate, (collect, collect_enqueues));
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        crate::server_app::ShipSystemBlackboards::default(),
        crate::ship_plugin::ShipConfigComponent::default(),
        crate::ship_plugin::ShipSystemControlSources::default(),
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::ActiveStationRatings::default(),
        crate::ship_plugin::CoordinationQueue::default(),
        SensorRadarSelection::default(),
        // PR 7 (issue #597) — TacticalRadarSelection is now per-entity Component.
        crate::server_app::TacticalRadarSelection::default(),
        SensorsFrequencyState::default(),
        ShipImpulse(crate::ship::impulse::ImpulseState::new()),
    ));
    // One fixed step per update (issue #895): the plugin's systems run on
    // the logical tick, and each harness tick advances it once.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    app
}

fn push(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

fn tick(app: &mut App) -> Vec<OutboundMessage> {
    app.update();
    let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
    let mut out = app.world().resource::<Outbox>().0.clone();
    for (target, msg) in sim_entries {
        out.push(OutboundMessage {
            target,
            msg,
            delivery: crate::core::messages::DeliveryClass::Reliable,
        });
    }
    app.world_mut().resource_mut::<Outbox>().0.clear();
    out
}

fn start_game_with_sensors_and_tactical(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "sensors",
        ClientMessage::Identify {
            token: "sensors".into(),
            name: "Spock".into(),
        },
    );
    tick(app);
    push(
        app,
        "sensors",
        ClientMessage::SelectStation {
            station: "Sensors".into(),
        },
    );
    tick(app);
    push(
        app,
        "tactical",
        ClientMessage::Identify {
            token: "tactical".into(),
            name: "Bob".into(),
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
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "sensors", ClientMessage::SetReady { ready: true });
    push(app, "tactical", ClientMessage::SetReady { ready: true });
    tick(app);
}

#[test]
fn sensors_set_science_target_enqueues_target_designation_for_tactical() {
    let mut app = test_app();
    start_game_with_sensors_and_tactical(&mut app);

    push(
        &mut app,
        "sensors",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId(
                crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string(),
            ),
            payload: SystemControlPayload::SetScienceTarget {
                uuid: "asteroid-42".into(),
            },
        },
    );
    tick(&mut app);

    let log = app.world().resource::<EnqueueLog>();
    let enqueued = log
        .0
        .iter()
        .find(|e| matches!(&e.payload, CoordinationPayload::TargetDesignation { .. }))
        .expect("expected a TargetDesignation CoordinationEnqueue event");

    assert_eq!(
        enqueued.address,
        CoordinationAddress::Station(StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into(),
        )),
        "TargetDesignation should be enqueued for the Tactical Station"
    );
    match &enqueued.payload {
        CoordinationPayload::TargetDesignation { uuid, label } => {
            assert_eq!(uuid, "asteroid-42");
            // No EntityUuid/EntityName in this test world, so label falls
            // back to the raw uuid.
            assert_eq!(label, "asteroid-42");
        }
        other => panic!("expected TargetDesignation, got {other:?}"),
    }
}

#[test]
fn non_sensors_player_cannot_send_science_target() {
    let mut app = test_app();
    start_game_with_sensors_and_tactical(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId(
                crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string(),
            ),
            payload: SystemControlPayload::SetScienceTarget {
                uuid: "asteroid-42".into(),
            },
        },
    );
    tick(&mut app);

    let log = app.world().resource::<EnqueueLog>();
    assert!(
        !log.0
            .iter()
            .any(|e| matches!(&e.payload, CoordinationPayload::TargetDesignation { .. })),
        "non-Sensors player should not be able to enqueue a TargetDesignation"
    );
}

/// Set the LocalShip's per-entity `TacticalRadarSelection` for tests.
fn set_local_weapons_target(app: &mut App, uuid: Option<String>) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::LocalShip>>();
    if let Ok(mut wt) = q.single_mut(app.world_mut()) {
        wt.0 = uuid;
    }
}

#[test]
fn frequency_hint_emitted_when_target_changes() {
    let mut app = test_app();
    start_game_with_sensors_and_tactical(&mut app);

    set_local_weapons_target(&mut app, Some("asteroid-1".into()));
    tick(&mut app); // emits first hint

    set_local_weapons_target(&mut app, Some("asteroid-2".into()));
    let enqueue_count = {
        // Tick and count CoordinationEnqueue events written
        app.update();
        // We verify indirectly — state should update to new target
        let mut q = app
            .world_mut()
            .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry SensorsFrequencyState")
            .last_sent_target
            .clone()
    };

    assert_eq!(
        enqueue_count.as_deref(),
        Some("asteroid-2"),
        "state should track the new target after it changes"
    );
}

#[test]
fn frequency_hint_not_re_emitted_for_same_target() {
    let mut app = test_app();
    start_game_with_sensors_and_tactical(&mut app);

    set_local_weapons_target(&mut app, Some("asteroid-1".into()));
    tick(&mut app); // first emit

    let state_before = {
        let mut q = app
            .world_mut()
            .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
        q.single(app.world()).unwrap().last_sent_frequency
    };

    tick(&mut app); // second tick, same target

    let state_after = {
        let mut q = app
            .world_mut()
            .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
        q.single(app.world()).unwrap().last_sent_frequency
    };

    assert_eq!(
        state_before, state_after,
        "state should not change when target is unchanged"
    );
}

/// Issue #873: the hand-off to the high-fidelity emitter is a
/// level-of-detail split and NOTHING else.
///
/// It used to be `AiHighFidelity && policy_for(sensors).operate_ai`, so on a
/// high-fidelity hull the ship's frequency advisory changed shape — timing,
/// and across the delivery lag its content — according to who was holding
/// the console, and could be silenced outright by the `auto_hint` rating
/// gate on the other side. Now both origins take the same path.
///
/// Asserted in both directions across two ticks (the second is where the
/// on-change debounce would let a late emit through) and for both control
/// sources, because a one-sided assertion would still pass with the
/// `operate_ai` conjunct restored.
#[test]
fn high_fidelity_ships_hand_off_regardless_of_who_holds_sensors() {
    for source in [ControlSource::Human, ControlSource::Ai] {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ai::server::AiHighFidelity);
        {
            let mut cs = app
                .world_mut()
                .entity_mut(ship)
                .take::<crate::ship_plugin::ShipSystemControlSources>()
                .unwrap();
            cs.0.set(crate::ship::system_registry::sensors_system_id(), source);
            app.world_mut().entity_mut(ship).insert(cs);
        }
        app.world_mut().resource_mut::<EnqueueLog>().0.clear();

        set_local_weapons_target(&mut app, Some("asteroid-1".into()));
        tick(&mut app);
        tick(&mut app);

        let log = app.world().resource::<EnqueueLog>();
        assert!(
            !log.0
                .iter()
                .any(|e| matches!(&e.payload, CoordinationPayload::FrequencyHint { .. })),
            "a high-fidelity hull's frequency hint belongs to \
             `tick_frequency_hint_high_fidelity` whoever holds Sensors ({source:?}); \
             emitting here too would double-send, and gating this skip on \
             `operate_ai` is the origin branch issue #873 removed"
        );
    }
}

/// The other side of the same split: a hull with no `AiHighFidelity` marker
/// is served HERE, and again regardless of origin — the immediate readout is
/// not "the human path".
#[test]
fn low_fidelity_ships_emit_here_regardless_of_who_holds_sensors() {
    for source in [ControlSource::Human, ControlSource::Ai] {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        {
            let mut cs = app
                .world_mut()
                .entity_mut(ship)
                .take::<crate::ship_plugin::ShipSystemControlSources>()
                .unwrap();
            cs.0.set(crate::ship::system_registry::sensors_system_id(), source);
            app.world_mut().entity_mut(ship).insert(cs);
        }
        app.world_mut().resource_mut::<EnqueueLog>().0.clear();

        set_local_weapons_target(&mut app, Some("asteroid-1".into()));
        tick(&mut app);

        let log = app.world().resource::<EnqueueLog>();
        let hint = log
            .0
            .iter()
            .find(|e| matches!(&e.payload, CoordinationPayload::FrequencyHint { .. }))
            .unwrap_or_else(|| panic!("expected a FrequencyHint with Sensors on {source:?}"));
        assert_eq!(
            hint.sender_origin, source,
            "sender_origin must report the live control source and be used only as a \
             delivery-routing tag"
        );
        assert_eq!(
            hint.address,
            CoordinationAddress::Station(StationId(
                crate::ship::system_registry::TACTICAL_STATION_ID.into(),
            )),
            "the hint is addressed to Tactical either way"
        );
    }
}

/// Verifies that operate_sensors_ai skips entities where Sensors is Human,
/// and runs (without panic) for entities where Sensors is Ai (issue #589 AC).
#[test]
fn operate_sensors_ai_runs_per_entity_for_ai_controlled_ships() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    // Human-controlled: operate_sensors_ai must do nothing.
    let mut human_resolver = ControlSourceResolver::new();
    human_resolver.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Human,
    );
    let human_sources = crate::ship_plugin::ShipSystemControlSources(human_resolver);
    let human_policy = human_sources
        .0
        .policy_for(&crate::ship::system_registry::sensors_system_id());
    assert!(
        !human_policy.operate_ai,
        "human Sensors should not operate AI"
    );

    // AI-controlled: operate_sensors_ai must gate and proceed.
    let mut ai_resolver = ControlSourceResolver::new();
    ai_resolver.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Ai,
    );
    let ai_sources = crate::ship_plugin::ShipSystemControlSources(ai_resolver);
    let ai_policy = ai_sources
        .0
        .policy_for(&crate::ship::system_registry::sensors_system_id());
    assert!(
        ai_policy.operate_ai,
        "AI Sensors must gate through operate_ai"
    );
}

// ── tick_sensors_threat_warning tests ──────────────────────────────────────

/// Helper: initialise a faction registry with Federation (self) and Harrow
/// (enemy) factions, register the sensor range, and spawn the local ship.
fn test_app_with_factions() -> (App, uuid::Uuid, uuid::Uuid) {
    let mut app = test_app();

    // Seed the faction registry so is_enemy works.
    let fed_uuid = uuid::Uuid::new_v4();
    let harrow_uuid = uuid::Uuid::new_v4();
    let mut reg = crate::ai::faction::FactionRegistry::new();
    reg.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: fed_uuid,
        name: "Federation".into(),
        enemies: vec![harrow_uuid],
        compliance: None,
    });
    reg.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: harrow_uuid,
        name: "Harrow".into(),
        enemies: vec![fed_uuid],
        compliance: None,
    });
    app.insert_resource(crate::entities::config_cache::FactionRegistryResource(reg));

    // Add ShipPhysics, EntityUuid, SensorsThreatState, ShipModifiers,
    // and FactionComponent to the existing test ship entity.
    let ship_uuid = uuid::Uuid::new_v4().to_string();
    let mut ship_q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    let ship = ship_q.single_mut(app.world_mut()).unwrap();
    // Threat coordination is addressed through the ship's authored topology.
    // The lightweight `ShipConfigComponent::default()` fallback does not run
    // EntityConfig's shield-arc synthesis, so this fixture must attach the same
    // composed config a real battleship spawn receives.
    let authored_ship_config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_battleship.toml",
    )
    .expect("the shipped battleship composes")
    .ship_config
    .expect("the shipped battleship declares its station topology");
    app.world_mut().entity_mut(ship).insert((
        crate::entities::spawner::EntityUuid(ship_uuid.clone()),
        SensorsThreatState::default(),
        crate::modifiers::ShipModifiers::new(),
        crate::entities::spawner::FactionComponent(fed_uuid),
        crate::ship::state::ShipPhysics::default(),
        crate::ship_plugin::ShipConfigComponent(authored_ship_config),
    ));

    (app, fed_uuid, harrow_uuid)
}

/// Spawn a hostile entity at the given position.
fn spawn_hostile(app: &mut App, uuid: &str, x: f32, z: f32, faction: uuid::Uuid) {
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid.to_string()),
        crate::entities::spawner::EntityName(format!("Hostile-{uuid}")),
        Transform::from_xyz(x, 0.0, z),
        crate::entities::spawner::FactionComponent(faction),
    ));
}

#[test]
fn threat_warning_emitted_for_hostile_in_range() {
    let (mut app, _fed, harrow) = test_app_with_factions();
    spawn_hostile(&mut app, "h-1", 0.0, -200.0, harrow); // directly ahead, 200m

    let shield_address = {
        let mut ships = app.world_mut().query_filtered::<
            &crate::ship_plugin::ShipConfigComponent,
            With<crate::server_app::LocalShip>,
        >();
        let config = ships.single(app.world()).expect("the local ship config");
        crate::ship::coordination::address_for_system_kind(
            &config.0,
            crate::ship::system_registry::SHIELD_ARC_KIND,
        )
        .expect("the fixture authors one Shields Station for every shield arc")
    };

    tick(&mut app);

    let log = app.world().resource::<EnqueueLog>();
    let threat = log
        .0
        .iter()
        .find(|e| matches!(&e.payload, CoordinationPayload::ThreatBearing { .. }))
        .expect("expected a ThreatBearing CoordinationEnqueue");

    assert_eq!(
        threat.address, shield_address,
        "ThreatBearing should address the Station that owns Shields"
    );
    match &threat.payload {
        CoordinationPayload::ThreatBearing { bearing_rad, label } => {
            // Hostile at (0, -200) directly ahead → bearing ≈ 0 rad
            assert!(
                bearing_rad.abs() < 0.1,
                "bearing should be near 0 for target ahead, got {bearing_rad}"
            );
            assert!(
                label.contains("Hostile closing"),
                "label should contain threat description, got {label}"
            );
        }
        other => panic!("expected ThreatBearing, got {other:?}"),
    }
}

#[test]
fn threat_warning_debounced_for_same_threat_and_bearing() {
    let (mut app, _fed, harrow) = test_app_with_factions();
    spawn_hostile(&mut app, "h-1", 0.0, -200.0, harrow);

    tick(&mut app); // first emission

    let state = {
        let mut q = app
            .world_mut()
            .query_filtered::<&SensorsThreatState, With<crate::server_app::LocalShip>>();
        q.single(app.world()).unwrap().last_threat_uuid.clone()
    };
    assert_eq!(
        state.as_deref(),
        Some("h-1"),
        "state should track the threat uuid"
    );

    // Clear logged events
    app.world_mut().resource_mut::<EnqueueLog>().0.clear();

    tick(&mut app); // second tick, same hostile, same bearing

    let log = app.world().resource::<EnqueueLog>();
    let new_threats = log
        .0
        .iter()
        .filter(|e| matches!(&e.payload, CoordinationPayload::ThreatBearing { .. }))
        .count();
    assert_eq!(
        new_threats, 0,
        "should not re-emit ThreatBearing for the same threat and bearing"
    );
}

#[test]
fn threat_warning_not_emitted_for_out_of_range_hostile() {
    let (mut app, _fed, harrow) = test_app_with_factions();
    // Default sensor range is 500; place hostile at 1000m
    spawn_hostile(&mut app, "far-1", 0.0, -1000.0, harrow);

    tick(&mut app);

    let log = app.world().resource::<EnqueueLog>();
    let threat = log
        .0
        .iter()
        .find(|e| matches!(&e.payload, CoordinationPayload::ThreatBearing { .. }));
    assert!(
        threat.is_none(),
        "should not emit ThreatBearing for out-of-range hostile"
    );
}

#[test]
fn threat_warning_re_emitted_on_bearing_change() {
    let (mut app, _fed, harrow) = test_app_with_factions();
    spawn_hostile(&mut app, "h-1", 0.0, -200.0, harrow); // directly ahead

    tick(&mut app); // first emission, bearing ≈ 0

    // Clear logged events
    app.world_mut().resource_mut::<EnqueueLog>().0.clear();

    // Move hostile to starboard (~45°)
    let mut hostile_q = app
        .world_mut()
        .query_filtered::<&mut Transform, With<crate::entities::spawner::EntityUuid>>();
    for mut tf in hostile_q.iter_mut(app.world_mut()) {
        tf.translation.x = 200.0;
        tf.translation.z = -200.0;
    }

    tick(&mut app); // second emission — bearing changed enough

    let log = app.world().resource::<EnqueueLog>();
    let re_emitted = log
        .0
        .iter()
        .filter(|e| matches!(&e.payload, CoordinationPayload::ThreatBearing { .. }))
        .count();
    assert_eq!(
        re_emitted, 1,
        "should re-emit ThreatBearing when bearing changes materially"
    );
}

#[test]
fn threat_warning_state_cleared_when_no_threat() {
    let (mut app, _fed, harrow) = test_app_with_factions();
    spawn_hostile(&mut app, "h-1", 0.0, -200.0, harrow);

    tick(&mut app); // first emission — threat detected

    // Despawn the hostile (exclude the LocalShip)
    let mut hostile_q = app.world_mut().query_filtered::<Entity, (
        With<crate::entities::spawner::EntityUuid>,
        Without<crate::server_app::LocalShip>,
    )>();
    if let Some(hostile) = hostile_q.iter_mut(app.world_mut()).next() {
        app.world_mut().entity_mut(hostile).despawn();
    }

    // Clear logged events
    app.world_mut().resource_mut::<EnqueueLog>().0.clear();

    tick(&mut app); // tick without threat

    let state = {
        let mut q = app
            .world_mut()
            .query_filtered::<&SensorsThreatState, With<crate::server_app::LocalShip>>();
        q.single(app.world()).unwrap().last_threat_uuid.clone()
    };
    assert_eq!(
        state, None,
        "state should be cleared when no threat remains"
    );
}

// ── operate_sensors_ai tests ────────────────────────────────────────────

fn sensors_ai_test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(bevy::time::Time::<()>::default())
        .init_resource::<crate::world::server::WorldContentRuntime>()
        // Always present in production (LobbyPlugin inserts it); the
        // default carries the same default_sensors_radar_range() base the
        // old Option fallback supplied, so test ranges are unchanged.
        .init_resource::<crate::lobby::server::ShipClientConfigResource>()
        // The Sensors AI emit validates through the shared admission
        // seam, which consults Sessions for human tokens; the
        // `ai:` path only needs the resource present.
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        // The applier emits the channel-3 TargetDesignation advisory.
        .add_message::<CoordinationEnqueue>()
        // Decide-and-emit (issue #828): the decision lands in
        // `AdmittedCommands` and `handle_sensors_messages` applies it —
        // chained so the same-tick emit→apply shape of production
        // (Input → Physics) holds in the harness.
        .add_systems(
            Update,
            (
                seed_viewscreen_from_selection,
                operate_sensors_ai,
                handle_sensors_messages,
            )
                .chain(),
        );

    let mut control_sources = crate::ship_plugin::ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Ai,
    );

    app.world_mut().spawn((
        crate::server_app::Ship,
        control_sources,
        crate::server_app::ShipSystemBlackboards::default(),
        SensorRadarSelection::default(),
        crate::server_app::TacticalRadarSelection::default(),
        // Sensors range-gate on the ship's own position and radar modifier,
        // so both must be present or the query silently matches nothing and
        // every assertion below passes vacuously.
        crate::ship::state::ShipPhysics::default(),
        crate::modifiers::ShipModifiers::default(),
        // Issue #828: the AI decision flows through this ship's own
        // AdmittedCommands, applied by handle_sensors_messages.
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::ShipConfigComponent::default(),
        // The AUTHORED Sensors selector every shipped hull carries. Since
        // #885b stage 5d there is no synthesised stand-in inside
        // `operate_sensors_ai`, so a fixture that wants a ranking has to
        // attach the declaration a real hull authors — which is also what
        // makes these tests exercise shipped content rather than a Rust
        // default nobody wrote.
        SensorsTargetSelector {
            selector: crate::entities::authored_ai_pins::shipped_selector_toml("sensors")
                .to_selector()
                .expect("the shipped Sensors selector decodes"),
            power_rating: None,
        },
    ));

    app
}

/// Admitted sensors commands currently queued on the single test ship.
fn admitted_sensors_payloads(app: &mut App) -> Vec<SystemControlPayload> {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::core::messages::AdmittedCommands, With<crate::server_app::Ship>>(
        );
    q.single(app.world())
        .unwrap()
        .for_target(crate::ship::system_registry::SENSORS_SYSTEM_ID)
        .map(|c| c.payload.clone())
        .collect()
}

fn insert_viewscreen_objective(app: &mut App, target_name: &str, score: f32) {
    let viewscreen = crate::core::messages::ViewscreenBlackboard {
        scored_objectives: vec![crate::core::messages::ScoredObjective {
            id: format!("obj-destroy-{target_name}"),
            score,
            directive: crate::core::messages::AiDirective::Destroy {
                target: target_name.into(),
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![
                crate::core::messages::SystemAffinity::Helm,
                crate::core::messages::SystemAffinity::Weapons,
                crate::core::messages::SystemAffinity::Captain,
            ],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: format!("obj-destroy-{target_name}"),
                text: format!("Destroy {target_name}"),
                text_params: Default::default(),
                mandatory: true,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![target_name.into()],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
        ..Default::default()
    };
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<crate::server_app::Ship>>();
    let mut bbs = q
        .single_mut(app.world_mut())
        .expect("Ship must have ShipSystemBlackboards");
    bbs.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        crate::core::messages::SystemBlackboard::Viewscreen(viewscreen),
    );
}

fn insert_viewscreen_scan_objective(app: &mut App, target_name: &str, score: f32) {
    let viewscreen = crate::core::messages::ViewscreenBlackboard {
        scored_objectives: vec![crate::core::messages::ScoredObjective {
            id: format!("obj-scan-{target_name}"),
            score,
            directive: crate::core::messages::AiDirective::Scan {
                target: target_name.into(),
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Sensors],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: format!("obj-scan-{target_name}"),
                text: "world.objective.scan".into(),
                text_params: Default::default(),
                mandatory: false,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![target_name.into()],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        }],
        ..Default::default()
    };
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<crate::server_app::Ship>>();
    let mut bbs = q
        .single_mut(app.world_mut())
        .expect("the Sensors test ship");
    bbs.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        crate::core::messages::SystemBlackboard::Viewscreen(viewscreen),
    );
}

/// Issue #891 stage 2, per-host both-directions proof for the Sensors
/// target selector: an authored eligibility gated on a world flag selects
/// nothing while the flag is clear and mirrors the combat lock once it is
/// set.
#[test]
fn operate_sensors_ai_flag_guard_reads_the_world_in_both_directions() {
    let mut app = sensors_ai_test_app();
    let target = "cc000000-0000-0000-0000-0000000891aa";
    spawn_target_at(&mut app, target, 0.0, -30.0);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0 = Some(target.to_string());
    }

    // Swap in a selector whose eligibility ALSO requires the world flag.
    let cfg = crate::entities::config::FineSystemAiSelectorToml {
        param: Default::default(),
        sources: vec!["combat-lock".into()],
        horizon: 1.0e9,
        switch_margin: 0.0,
        eligibility: "candidate_fact(detectable) > 0 and flag(sensors_cleared)".into(),
        score: vec![crate::entities::config::ScoreTermToml {
            when: "candidate_fact(source_combat_lock) > 0".into(),
            weight: 1.0,
        }],
    };
    let ship = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::Ship>>();
        q.single(app.world()).unwrap()
    };
    app.world_mut()
        .entity_mut(ship)
        .insert(SensorsTargetSelector {
            selector: cfg
                .to_selector()
                .expect("flag-gated sensors selector decodes"),
            power_rating: None,
        });

    // Flag CLEAR → nothing is eligible, no science target.
    tick_sensors_ai(&mut app);
    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "with the world flag clear the eligibility must admit no candidate"
    );

    // Flag SET → the SAME eligibility admits the combat-lock mirror.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("sensors_cleared");
    tick_sensors_ai(&mut app);
    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(target),
        "with the world flag set the same eligibility must select the target"
    );
}

/// Spawn a bare targetable entity at a world position. `operate_sensors_ai`
/// range-gates on `Transform`, so a target without one is not detectable.
fn spawn_target_at(app: &mut App, uuid: &str, x: f32, z: f32) {
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid.to_string()),
        Transform::from_xyz(x, 0.0, z),
    ));
}

fn spawn_named_target_at(app: &mut App, uuid: &str, name: &str, x: f32, z: f32) {
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid.to_string()),
        crate::entities::spawner::EntityName(name.to_string()),
        Transform::from_xyz(x, 0.0, z),
    ));
}

fn get_sensors_target(app: &mut App) -> Option<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&SensorRadarSelection, With<crate::server_app::Ship>>();
    q.single(app.world()).unwrap().0.clone()
}

fn tick_sensors_ai(app: &mut App) {
    let mut time = app.world_mut().resource_mut::<bevy::time::Time>();
    time.advance_by(std::time::Duration::from_secs_f32(0.1));
    app.update();
}

#[test]
fn ai_sensors_mirrors_weapons_target() {
    let mut app = sensors_ai_test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();

    spawn_target_at(&mut app, &target_uuid, 20.0, 0.0);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
    }

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "sensors AI should mirror TacticalRadarSelection"
    );
}

#[test]
fn ai_sensors_selects_destroy_objective_when_no_weapons_target() {
    let mut app = sensors_ai_test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();

    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_viewscreen_objective(&mut app, "wave_1", 80.0);
    spawn_target_at(&mut app, &target_uuid, 40.0, 0.0);

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "sensors AI should select named Destroy objective target"
    );
}

/// Naming a target in an objective says who to engage, not that the ship can
/// see them. Before this gate the objective path resolved a name to a UUID
/// and locked it at any distance, so AI ships tracked contacts thousands of
/// units outside their own sensor range.
#[test]
fn ai_sensors_ignores_objective_target_beyond_sensor_range() {
    let mut app = sensors_ai_test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();

    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_viewscreen_objective(&mut app, "wave_1", 80.0);

    // Well beyond the default 500-unit console range the test ship uses.
    spawn_target_at(&mut app, &target_uuid, 5000.0, 0.0);

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "a named objective target outside sensor range must not be locked"
    );
}

/// An NPC must range-gate on its own `[ai_profile] sensor_range`, not on the
/// local player's console config. Borrowing the player's reach is what let
/// short-ranged Harrow hulls (sensor_range 120) see as far as the flagship.
#[test]
fn ai_sensors_uses_the_ships_own_ai_profile_range() {
    let mut app = sensors_ai_test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();

    {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::Ship>>();
        let ship = q.single(app.world()).unwrap();
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ai::server::AiProfile {
                aggression: 0.8,
                sensor_range: 120.0,
                ..Default::default()
            });
    }

    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_viewscreen_objective(&mut app, "wave_1", 80.0);

    // Inside the player's 500 console range but outside this hull's 120.
    spawn_target_at(&mut app, &target_uuid, 300.0, 0.0);

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "NPC must use its own sensor_range, not the player's console range"
    );
}

#[test]
fn ai_sensors_skips_untargeted_destroy() {
    let mut app = sensors_ai_test_app();

    let viewscreen = crate::core::messages::ViewscreenBlackboard {
        scored_objectives: vec![crate::core::messages::ScoredObjective {
            id: "obj-destroy-any".into(),
            score: 80.0,
            directive: crate::core::messages::AiDirective::Destroy { target: "".into() },
            source: crate::core::messages::ObjectiveSource::Doctrine,
            relevance: vec![
                crate::core::messages::SystemAffinity::Helm,
                crate::core::messages::SystemAffinity::Weapons,
                crate::core::messages::SystemAffinity::Captain,
            ],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "obj-destroy-any".into(),
                text: "Engage hostiles".into(),
                text_params: Default::default(),
                mandatory: false,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::core::messages::ObjectiveSource::Doctrine,
            },
        }],
        ..Default::default()
    };
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<crate::server_app::Ship>>();
        let mut bbs = q
            .single_mut(app.world_mut())
            .expect("Ship must have ShipSystemBlackboards");
        bbs.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            crate::core::messages::SystemBlackboard::Viewscreen(viewscreen),
        );
    }

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "sensors AI should skip untargeted Destroy directives"
    );
}

#[test]
fn ai_sensors_prefers_weapons_target_over_objective() {
    let mut app = sensors_ai_test_app();
    let objective_uuid = uuid::Uuid::new_v4().to_string();
    let combat_uuid = uuid::Uuid::new_v4().to_string();

    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), objective_uuid.clone());
    insert_viewscreen_objective(&mut app, "wave_1", 80.0);

    spawn_target_at(&mut app, &combat_uuid, 20.0, 0.0);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0 = Some(combat_uuid.clone());
    }

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(combat_uuid.as_str()),
        "sensors AI should prefer TacticalRadarSelection over objective target"
    );
}

#[test]
fn ai_sensors_does_not_select_objective_when_weapons_target_is_some_but_entity_gone() {
    let mut app = sensors_ai_test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();

    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_viewscreen_objective(&mut app, "wave_1", 80.0);
    spawn_target_at(&mut app, &target_uuid, 30.0, 0.0);

    // TacticalRadarSelection names a UUID that no entity carries → existence check fails
    let dead_uuid = uuid::Uuid::new_v4().to_string();
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0 = Some(dead_uuid);
    }

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "sensors AI should fall through to objective when TacticalRadarSelection entity is gone"
    );
}

// ── Issue #828 tests: decide-and-emit through Admission ─────────────────

/// Issue #1139: Scan uses the same AI emit/admission seam as a human Sensors
/// command, resolves both EntityName and raw-UUID targets, and deliberately
/// retries while the objective remains active. The target is far outside the
/// normal Sensors horizon: the emitter must not steal range authority from the
/// sole science scan applier.
#[test]
fn ai_sensors_emits_admitted_scan_and_retries_without_a_range_precheck() {
    let mut app = sensors_ai_test_app();
    let named_uuid = "cc000000-0000-0000-0000-0000001139aa";
    let raw_uuid = "cc000000-0000-0000-0000-0000001139bb";
    spawn_named_target_at(&mut app, named_uuid, "Ladder Depot B", 50_000.0, 0.0);
    spawn_target_at(&mut app, raw_uuid, 60_000.0, 0.0);
    insert_viewscreen_scan_objective(&mut app, "Ladder Depot B", 52.0);

    tick_sensors_ai(&mut app);
    assert_eq!(
        admitted_sensors_payloads(&mut app),
        vec![SystemControlPayload::ScanTarget {
            uuid: named_uuid.into()
        }],
        "EntityName fallback must resolve and admission must retain the exact human ScanTarget"
    );
    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "emitting a scan must not mutate the separate Sensors target state"
    );

    // No success/refusal latch belongs to the emitter. With the objective still
    // active, the next snapshot asks the authoritative applier again.
    tick_sensors_ai(&mut app);
    assert_eq!(
        admitted_sensors_payloads(&mut app).len(),
        2,
        "an unresolved/refused scan objective must be retried"
    );

    // Replace the projected objective with a raw UUID. No name-table entry and
    // no EntityName exists for this target, so only the UUID fallback can emit.
    insert_viewscreen_scan_objective(&mut app, raw_uuid, 52.0);
    tick_sensors_ai(&mut app);
    assert_eq!(
        admitted_sensors_payloads(&mut app).last(),
        Some(&SystemControlPayload::ScanTarget {
            uuid: raw_uuid.into()
        })
    );
}

#[test]
fn scan_directive_stands_down_for_human_held_or_undeclared_sensors() {
    let target = "cc000000-0000-0000-0000-0000001139cc";

    let mut human = sensors_ai_test_app();
    spawn_target_at(&mut human, target, 20.0, 0.0);
    insert_viewscreen_scan_objective(&mut human, target, 52.0);
    {
        let mut q = human
            .world_mut()
            .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::Ship>>();
        q.single_mut(human.world_mut()).unwrap().0.set(
            crate::ship::system_registry::sensors_system_id(),
            ControlSource::Human,
        );
    }
    tick_sensors_ai(&mut human);
    assert!(admitted_sensors_payloads(&mut human).is_empty());

    let mut undeclared = sensors_ai_test_app();
    spawn_target_at(&mut undeclared, target, 20.0, 0.0);
    insert_viewscreen_scan_objective(&mut undeclared, target, 52.0);
    let ship = undeclared
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::Ship>>()
        .single(undeclared.world())
        .unwrap();
    undeclared
        .world_mut()
        .entity_mut(ship)
        .remove::<SensorsTargetSelector>();
    tick_sensors_ai(&mut undeclared);
    assert!(
        admitted_sensors_payloads(&mut undeclared).is_empty(),
        "a hull without the authored Sensors selector capability must not synthesize scan AI"
    );
}

/// The AI decision must land as an admitted `SetScienceTarget` in the
/// ship's own `AdmittedCommands` (not a direct `SensorRadarSelection` write),
/// and only on change — an unchanged decision emits nothing.
#[test]
fn ai_sensors_emits_admitted_set_science_target_on_change_only() {
    let mut app = sensors_ai_test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();

    spawn_target_at(&mut app, &target_uuid, 20.0, 0.0);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
    }

    tick_sensors_ai(&mut app);

    let payloads = admitted_sensors_payloads(&mut app);
    assert_eq!(
        payloads,
        vec![SystemControlPayload::SetScienceTarget {
            uuid: target_uuid.clone()
        }],
        "the AI decision must flow through AdmittedCommands"
    );
    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "the applier must have applied the admitted command same-tick"
    );

    // Second tick, same decision: emit-on-change means no new command.
    // (This harness has no AdmissionPlugin, so AdmittedCommands is never
    // cleared — a re-emission would grow the queue.)
    tick_sensors_ai(&mut app);
    assert_eq!(
        admitted_sensors_payloads(&mut app).len(),
        1,
        "an unchanged decision must not re-emit an admitted command"
    );
}

/// When the decision changes from Some to None (target moved out of
/// range, no objective fallback), the AI emits an admitted
/// `ClearScienceTarget` and the applier clears the selection — matching
/// the old direct `sensors_target.0 = None` write.
#[test]
fn ai_sensors_clears_selection_via_admitted_clear() {
    let mut app = sensors_ai_test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();

    spawn_target_at(&mut app, &target_uuid, 20.0, 0.0);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
    }
    tick_sensors_ai(&mut app);
    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(target_uuid.as_str())
    );

    // Move the target far beyond sensor range; the weapons mirror tier
    // fails its range gate and there is no objective fallback → None.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut Transform, With<crate::entities::spawner::EntityUuid>>();
        for mut tf in q.iter_mut(app.world_mut()) {
            tf.translation.x = 50_000.0;
        }
    }
    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "the admitted ClearScienceTarget must clear the selection"
    );
    assert!(
        admitted_sensors_payloads(&mut app)
            .iter()
            .any(|p| matches!(p, SystemControlPayload::ClearScienceTarget)),
        "the clear must flow through AdmittedCommands too"
    );
}

/// Human-held Sensors refuses the `ai:` emission at admission: the
/// operate gate skips the ship, and even a direct emission attempt is
/// rejected by `validate_and_admit` (operate_ai does not hold).
#[test]
fn human_held_sensors_refuses_the_ai_emission() {
    let mut app = sensors_ai_test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();

    // Flip Sensors to Human on the test ship.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0.set(
            crate::ship::system_registry::sensors_system_id(),
            ControlSource::Human,
        );
    }
    spawn_target_at(&mut app, &target_uuid, 20.0, 0.0);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
    }

    tick_sensors_ai(&mut app);

    assert!(
        admitted_sensors_payloads(&mut app).is_empty(),
        "no ai: command may be admitted while a human holds Sensors"
    );
    assert_eq!(get_sensors_target(&mut app), None);

    // Belt and braces: the emit helper itself must be refused by the
    // admission predicate under Human control.
    let mut human_sources = crate::ship_plugin::ShipSystemControlSources::default();
    human_sources.0.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Human,
    );
    let sessions = crate::lobby::Sessions(crate::lobby::session::SessionManager::new());
    let mut admitted = crate::core::messages::AdmittedCommands::default();
    assert!(
        !crate::command_admission::ai_emit::emit_ai_command(
            None,
            crate::ship::system_registry::sensors_system_id(),
            SystemControlPayload::SetScienceTarget {
                uuid: target_uuid.clone()
            },
            &human_sources,
            &sessions,
            None,
            &mut admitted,
        ),
        "validate_and_admit must reject the ai: token when Sensors is Human"
    );
    assert!(admitted.0.is_empty());
}

// ── Issue #828 tests: per-Ship publish ──────────────────────────────────

/// Fetch a ship's published Sensors blackboard.
fn sensors_bb_of(app: &App, entity: Entity) -> SensorsBlackboard {
    let bbs = app
        .world()
        .entity(entity)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .expect("ShipSystemBlackboards");
    let key = SystemId(crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string());
    match bbs.0.get(&key).expect("Sensors blackboard") {
        SystemBlackboard::Sensors(bb) => bb.clone(),
        other => panic!("expected Sensors blackboard, got {other:?}"),
    }
}

/// Per-Ship publish (issue #828): an NPC gets its own Sensors blackboard —
/// its own science target, its own AiProfile-derived radar range — while
/// the player-only authored show/select filters stay gated on LocalShip.
#[test]
fn publish_writes_sensors_blackboards_for_every_ship_not_just_local() {
    let mut app = test_app();
    // Give the local config distinctive filters so the gating is visible.
    {
        let mut cfg = app
            .world_mut()
            .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
        cfg.0.sensors_radar_shows = vec!["ship".into()];
        cfg.0.sensors_radar_selects = vec!["hostile".into()];
    }
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::ShipSystemBlackboards::default(),
            SensorRadarSelection(Some("npc-science-target".into())),
            crate::ai::server::AiProfile {
                aggression: 0.5,
                sensor_range: 120.0,
                ..Default::default()
            },
        ))
        .id();
    app.update();

    let npc_bb = sensors_bb_of(&app, npc);
    assert_eq!(
        npc_bb.science_target_uuid.as_deref(),
        Some("npc-science-target"),
        "NPC blackboard must carry the NPC's own SensorRadarSelection"
    );
    assert!(
        (npc_bb.radar_range - 120.0).abs() < f32::EPSILON,
        "NPC radar_range must come from its own AiProfile.sensor_range, got {}",
        npc_bb.radar_range
    );
    assert!(
        npc_bb.radar_shows.is_empty() && npc_bb.radar_selects.is_empty(),
        "player-only authored filters must not leak onto NPC blackboards"
    );

    let local = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        q.single(app.world()).unwrap()
    };
    let local_bb = sensors_bb_of(&app, local);
    assert_eq!(local_bb.radar_shows, vec!["ship".to_string()]);
    assert_eq!(local_bb.radar_selects, vec!["hostile".to_string()]);
    assert_eq!(
        local_bb.radar_range,
        crate::core::messages::default_sensors_radar_range(),
        "local ship keeps the console-config range"
    );
    assert_eq!(local_bb.science_target_uuid, None);
}

/// An NPC with no AiProfile falls back to the console-config base range
/// (the same preference order as `effective_sensor_range`), scaled by its
/// own SensorRadarRange modifier when present.
#[test]
fn publish_npc_without_profile_falls_back_to_console_range() {
    let mut app = test_app();
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();
    app.update();

    let npc_bb = sensors_bb_of(&app, npc);
    assert_eq!(
        npc_bb.radar_range,
        crate::core::messages::default_sensors_radar_range(),
        "profile-less NPC falls back to the console config base range"
    );
    assert_eq!(
        npc_bb.science_target_uuid, None,
        "missing SensorRadarSelection publishes as no selection"
    );
}

// ── Issue #746 tests: independent horizon-limited hostile selection ──────

/// Build on `sensors_ai_test_app` with a Federation/Harrow faction registry
/// and give the single test ship the Federation faction, so the tier-3
/// nearest-hostile selector can judge who is an enemy.
fn sensors_ai_test_app_with_factions() -> (App, uuid::Uuid, uuid::Uuid) {
    let mut app = sensors_ai_test_app();

    let fed = uuid::Uuid::new_v4();
    let harrow = uuid::Uuid::new_v4();
    let mut reg = crate::ai::faction::FactionRegistry::new();
    reg.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: fed,
        name: "Federation".into(),
        enemies: vec![harrow],
        compliance: None,
    });
    reg.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: harrow,
        name: "Harrow".into(),
        enemies: vec![fed],
        compliance: None,
    });
    app.insert_resource(crate::entities::config_cache::FactionRegistryResource(reg));

    let ship = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::Ship>>();
        q.single(app.world()).unwrap()
    };
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::FactionComponent(fed));

    (app, fed, harrow)
}

/// Spawn a faction-bearing, targetable contact with a *parseable* UUID (the
/// nearest-hostile scan filters out ids that are not canonical UUIDs).
fn spawn_faction_contact(app: &mut App, uuid: &str, x: f32, z: f32, faction: uuid::Uuid) {
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid.to_string()),
        Transform::from_xyz(x, 0.0, z),
        crate::entities::spawner::FactionComponent(faction),
    ));
}

/// Fetch a specific ship's science selection by entity UUID (the single-ship
/// `get_sensors_target` helper cannot disambiguate a two-ship world).
fn selection_of(app: &mut App, uuid: &str) -> Option<String> {
    let mut q = app
        .world_mut()
        .query::<(&crate::entities::spawner::EntityUuid, &SensorRadarSelection)>();
    q.iter(app.world())
        .find(|(u, _)| u.0 == uuid)
        .and_then(|(_, s)| s.0.clone())
}

/// Tier 3 selects a hostile only — a closer ally or neutral contact is never
/// designated. AC: hostility.
#[test]
fn ai_sensors_independently_selects_nearest_hostile_only() {
    let (mut app, fed, harrow) = sensors_ai_test_app_with_factions();
    let neutral_faction = uuid::Uuid::new_v4(); // not an enemy of Federation

    let ally = uuid::Uuid::new_v4().to_string();
    let neutral = uuid::Uuid::new_v4().to_string();
    let enemy = uuid::Uuid::new_v4().to_string();
    // Ally and neutral are *closer* than the enemy: proximity must not win
    // over hostility.
    spawn_faction_contact(&mut app, &ally, 10.0, 0.0, fed);
    spawn_faction_contact(&mut app, &neutral, 20.0, 0.0, neutral_faction);
    spawn_faction_contact(&mut app, &enemy, 60.0, 0.0, harrow);

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(enemy.as_str()),
        "sensors AI must independently pick the hostile, not the closer ally/neutral"
    );
    // And the selection reaches Tactical through the normal admitted path.
    assert!(
        admitted_sensors_payloads(&mut app).iter().any(|p| matches!(
            p,
            SystemControlPayload::SetScienceTarget { uuid } if uuid == &enemy
        )),
        "the independent selection must flow through an admitted SetScienceTarget"
    );
}

/// The independent tier is the *fallback*: an in-range combat lock (tier 1)
/// still wins over a nearest hostile. Tactical authority is not displaced —
/// Sensors mirrors what Tactical designates.
#[test]
fn ai_sensors_combat_lock_outranks_independent_hostile() {
    let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
    let locked = uuid::Uuid::new_v4().to_string();
    let nearer_hostile = uuid::Uuid::new_v4().to_string();

    // A nearer hostile the independent tier would otherwise choose…
    spawn_faction_contact(&mut app, &nearer_hostile, 15.0, 0.0, harrow);
    // …and a farther hostile that Tactical has locked.
    spawn_faction_contact(&mut app, &locked, 80.0, 0.0, harrow);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::TacticalRadarSelection, With<crate::server_app::Ship>>();
        q.single_mut(app.world_mut()).unwrap().0 = Some(locked.clone());
    }

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(locked.as_str()),
        "combat-lock mirror must outrank the independent nearest-hostile fallback"
    );
}

/// Damaging the sensor-radar hull system shrinks the `SensorRadarRange`
/// modifier, collapsing the horizon inward until a previously-visible
/// hostile falls outside it and is dropped. AC: horizon damage scaling.
#[test]
fn ai_sensors_drops_hostile_when_sensor_radar_damage_shrinks_horizon() {
    use crate::entities::spawner::EntitySystemHull;
    use crate::ship::damage::{ConsoleTierConfig, SystemHull};
    use crate::ship::system_registry::sensor_radar_system_id;
    use bevy::ecs::system::RunSystemOnce;

    let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
    let enemy = uuid::Uuid::new_v4().to_string();
    // Inside the healthy ~500 console horizon, but beyond the ~417 horizon a
    // Damaged sensor-radar leaves (500 × 1/1.2 ≈ 417).
    spawn_faction_contact(&mut app, &enemy, 450.0, 0.0, harrow);

    tick_sensors_ai(&mut app);
    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(enemy.as_str()),
        "a hostile at 450 is inside the undamaged horizon"
    );

    // Damage the sensor-radar to the Damaged tier (10/20 HP, below the 75%
    // threshold) and re-run the real damage→modifier translator.
    let tier_config = ConsoleTierConfig {
        damaged_threshold_pct: 0.75,
        disabled_threshold_pct: 0.25,
        debuff_magnitude: 0.20,
    };
    let ship = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::Ship>>();
        q.single(app.world()).unwrap()
    };
    let mut hull =
        SystemHull::from_config_with_tiers(&[(sensor_radar_system_id(), 20.0, tier_config)]);
    hull.set_hp(&sensor_radar_system_id(), 10.0);
    app.world_mut()
        .entity_mut(ship)
        .insert(EntitySystemHull(hull));
    app.world_mut()
        .run_system_once(crate::modifiers::coordination::apply_radar_damage_modifiers)
        .unwrap();

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "the damage-shrunken horizon must drop the now-out-of-range hostile"
    );
    assert!(
        admitted_sensors_payloads(&mut app)
            .iter()
            .any(|p| matches!(p, SystemControlPayload::ClearScienceTarget)),
        "the drop must flow through an admitted ClearScienceTarget"
    );
}

/// A designated hostile that despawns is dropped via an admitted
/// `ClearScienceTarget`. AC: target loss.
#[test]
fn ai_sensors_clears_when_selected_hostile_despawns() {
    let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
    let enemy = uuid::Uuid::new_v4().to_string();
    spawn_faction_contact(&mut app, &enemy, 100.0, 0.0, harrow);

    tick_sensors_ai(&mut app);
    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(enemy.as_str())
    );

    // Despawn the hostile.
    let hostile_entity = {
        let mut q = app
            .world_mut()
            .query::<(Entity, &crate::entities::spawner::EntityUuid)>();
        q.iter(app.world())
            .find(|(_, u)| u.0 == enemy)
            .map(|(e, _)| e)
            .unwrap()
    };
    app.world_mut().entity_mut(hostile_entity).despawn();

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "a despawned hostile must be cleared"
    );
    assert!(
        admitted_sensors_payloads(&mut app)
            .iter()
            .any(|p| matches!(p, SystemControlPayload::ClearScienceTarget)),
        "target loss must flow through an admitted ClearScienceTarget"
    );
}

/// Two AI Sensors ships each select their own nearest hostile, gated by
/// their own position and horizon — no cross-ship leakage. AC: per-ship
/// isolation.
#[test]
fn ai_sensors_two_ships_select_their_own_hostiles() {
    let (mut app, fed, harrow) = sensors_ai_test_app_with_factions();

    // Ship A is the helper's ship, sitting at the origin. Give it an id.
    let ship_a_uuid = uuid::Uuid::new_v4().to_string();
    {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::Ship>>();
        let ship_a = q.single(app.world()).unwrap();
        app.world_mut()
            .entity_mut(ship_a)
            .insert(crate::entities::spawner::EntityUuid(ship_a_uuid.clone()));
    }

    // Ship B, same faction, 1000 units away on +X.
    let ship_b_uuid = uuid::Uuid::new_v4().to_string();
    let mut control_sources = crate::ship_plugin::ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Ai,
    );
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::entities::spawner::EntityUuid(ship_b_uuid.clone()),
        control_sources,
        crate::server_app::ShipSystemBlackboards::default(),
        SensorRadarSelection::default(),
        crate::server_app::TacticalRadarSelection::default(),
        crate::ship::state::ShipPhysics {
            x: 1000.0,
            ..Default::default()
        },
        crate::modifiers::ShipModifiers::default(),
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::ShipConfigComponent::default(),
        crate::entities::spawner::FactionComponent(fed),
        // Ship B needs its own authored selector, same as ship A: the
        // declaration is per-entity and there is no synthesised fallback.
        SensorsTargetSelector {
            selector: crate::entities::authored_ai_pins::shipped_selector_toml("sensors")
                .to_selector()
                .expect("the shipped Sensors selector decodes"),
            power_rating: None,
        },
    ));

    // A hostile beside each ship; each lies far outside the other's horizon.
    let enemy_a = uuid::Uuid::new_v4().to_string();
    let enemy_b = uuid::Uuid::new_v4().to_string();
    spawn_faction_contact(&mut app, &enemy_a, 50.0, 0.0, harrow);
    spawn_faction_contact(&mut app, &enemy_b, 1050.0, 0.0, harrow);

    tick_sensors_ai(&mut app);

    assert_eq!(
        selection_of(&mut app, &ship_a_uuid).as_deref(),
        Some(enemy_a.as_str()),
        "ship A must pick the hostile inside its own horizon"
    );
    assert_eq!(
        selection_of(&mut app, &ship_b_uuid).as_deref(),
        Some(enemy_b.as_str()),
        "ship B must pick its own hostile — no cross-ship leakage"
    );
}

// ── Issue #776 tests: data-driven selector + authored power rating ───────

/// AC2/AC7: an authored selector gates eligibility on the ship's own
/// authored power rating via `self_fact(power_rating)`. An under-rated ship
/// selects nothing even with a hostile in horizon; raising the rating makes
/// the same contact eligible and the pick flows through an admitted
/// `SetScienceTarget` (an observable output).
#[test]
fn ai_sensors_selector_gates_on_authored_power_rating() {
    let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
    let enemy = uuid::Uuid::new_v4().to_string();
    spawn_faction_contact(&mut app, &enemy, 60.0, 0.0, harrow);

    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
    cfg.param.insert("min_rating".into(), 5.0);
    cfg.eligibility = "candidate_fact(detectable) > 0 and candidate_fact(hostile) > 0 \
         and self_fact(power_rating) >= param(min_rating)"
        .into();
    let selector = cfg.to_selector().unwrap();

    let ship = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::Ship>>();
        q.single(app.world()).unwrap()
    };

    // Under-rated: nothing eligible.
    app.world_mut()
        .entity_mut(ship)
        .insert(SensorsTargetSelector {
            selector: selector.clone(),
            power_rating: Some(3.0),
        });
    tick_sensors_ai(&mut app);
    assert_eq!(
        get_sensors_target(&mut app),
        None,
        "an under-rated ship must select nothing under the authored gate"
    );

    // Sufficiently rated: the same contact is now eligible and selected.
    app.world_mut()
        .entity_mut(ship)
        .insert(SensorsTargetSelector {
            selector,
            power_rating: Some(6.0),
        });
    tick_sensors_ai(&mut app);
    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(enemy.as_str()),
        "raising the rating above the floor makes the contact eligible"
    );
    assert!(
        admitted_sensors_payloads(&mut app).iter().any(|p| matches!(
            p,
            SystemControlPayload::SetScienceTarget { uuid } if uuid == &enemy
        )),
        "the selection must flow through an admitted SetScienceTarget"
    );
}

/// AC6: a selected contact drives the existing advisory `TargetDesignation`
/// on the channel-3 bus for Tactical — the same applier a human selection
/// uses. Here an AI ship independently designates its nearest hostile.
#[test]
fn ai_sensors_selection_drives_target_designation_advisory() {
    use crate::core::messages::CoordinationPayload;
    let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
    // The advisory is emitted by `handle_sensors_messages`; add a sink.
    app.init_resource::<EnqueueLog>().add_systems(
        bevy::app::Update,
        collect_enqueues.after(handle_sensors_messages),
    );
    let enemy = uuid::Uuid::new_v4().to_string();
    spawn_faction_contact(&mut app, &enemy, 60.0, 0.0, harrow);

    tick_sensors_ai(&mut app);

    assert_eq!(
        get_sensors_target(&mut app).as_deref(),
        Some(enemy.as_str())
    );
    let log = app.world().resource::<EnqueueLog>();
    assert!(
        log.0.iter().any(|e| matches!(
            &e.payload,
            CoordinationPayload::TargetDesignation { uuid, .. } if uuid == &enemy
        )),
        "the AI selection must designate its contact to Tactical (channel-3 advisory)"
    );
}

// ── publish_sensor_radar_blackboard: selected-target alert (issue #749) ──────

/// Minimal app that runs only `publish_sensor_radar_blackboard`, plus a
/// scanning ship whose `SensorRadarSelection` we drive directly. Returns the
/// scanning ship's `Entity` so the caller can read back its blackboard.
fn alert_publisher_app() -> (App, Entity) {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.add_systems(Update, publish_sensor_radar_blackboard);
    let scanner = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::ShipSystemBlackboards::default(),
            SensorRadarSelection::default(),
        ))
        .id();
    (app, scanner)
}

/// Read the `selected_target_alert` replica off a ship's sensor-radar blackboard.
fn published_alert(app: &App, ship: Entity) -> Option<bool> {
    match app
        .world()
        .entity(ship)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .and_then(|bbs| {
            bbs.0
                .get(&crate::ship::system_registry::sensor_radar_system_id())
                .cloned()
        }) {
        Some(SystemBlackboard::SensorRadar(bb)) => bb.selected_target_alert,
        _ => panic!("sensor-radar blackboard missing"),
    }
}

fn set_selection(app: &mut App, ship: Entity, uuid: Option<&str>) {
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<SensorRadarSelection>()
        .unwrap()
        .0 = uuid.map(|s| s.to_string());
}

#[test]
fn sensor_radar_alert_none_when_no_selection() {
    let (mut app, scanner) = alert_publisher_app();
    app.update();
    assert_eq!(
        published_alert(&app, scanner),
        None,
        "no selection → no alert field"
    );
}

#[test]
fn sensor_radar_alert_reports_selected_ship_red_alert() {
    let (mut app, scanner) = alert_publisher_app();
    // A capable ship target, currently at red alert.
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::entities::spawner::EntityUuid("enemy-1".into()),
        crate::ship::state::ShipRedAlert(true),
    ));
    set_selection(&mut app, scanner, Some("enemy-1"));
    app.update();
    assert_eq!(
        published_alert(&app, scanner),
        Some(true),
        "selected capable ship at red alert → Some(true)"
    );
}

#[test]
fn sensor_radar_alert_reports_capable_but_calm_ship() {
    let (mut app, scanner) = alert_publisher_app();
    // Capable but not alerted — the distinct Some(false) case.
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::entities::spawner::EntityUuid("enemy-2".into()),
        crate::ship::state::ShipRedAlert(false),
    ));
    set_selection(&mut app, scanner, Some("enemy-2"));
    app.update();
    assert_eq!(
        published_alert(&app, scanner),
        Some(false),
        "selected capable ship not at red alert → Some(false)"
    );
}

#[test]
fn sensor_radar_alert_none_for_non_ship_target() {
    let (mut app, scanner) = alert_publisher_app();
    // An asteroid: carries a uuid but NOT ShipRedAlert and NOT the Ship
    // marker → no capability → no alert field (the no-leak boundary).
    app.world_mut()
        .spawn(crate::entities::spawner::EntityUuid("asteroid-9".into()));
    set_selection(&mut app, scanner, Some("asteroid-9"));
    app.update();
    assert_eq!(
        published_alert(&app, scanner),
        None,
        "non-ship contact has no red-alert capability → no alert field"
    );
}
