//! Access-owner integration regressions. These run the installed consumer and
//! ordinary compiled System AI; Repair observations are deliberately frozen.
use super::*;
use crate::ai::state_tree_runtime::{compile_ship_trees_with_energy, ShipAiTreeRuntime};
use crate::core::messages::{ClientMessage, ConsoleId, RepairBlackboard, StationId, SystemId};
use crate::entities::config::EntityConfig;
use crate::lobby::{InboundMessage, LobbyPlugin};
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::ecs::system::RunSystemOnce;
use phoenix_model::console::{ConsoleFeed, ConsoleObservationKind};
use phoenix_model::interior::{BoundaryPhase, BoundaryStatus, TeamLocation};
use phoenix_sim_gameplay::ship::composition::SystemComponentConfig;

const HOLDER: &str = "access-engineer";
const GATE: &str = "auxiliary-hatch";
const CHANNEL: &str = "bolt-closed";

fn gate() -> SystemId {
    SystemId(GATE.into())
}

fn content() -> EntityConfig {
    let mut content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    // Exercise authored identities, not a hidden weapons-access/sealed alias.
    let ship = content.ship_config.as_mut().unwrap();
    let system = ship
        .systems
        .iter_mut()
        .find(|system| system.kind == crate::ship::system_registry::ACCESS_GATE_KIND)
        .unwrap();
    let old = std::mem::replace(&mut system.id, gate());
    let [SystemComponentConfig::AccessBoundary { sealed_channel, .. }] =
        system.components.as_mut_slice()
    else {
        panic!("authored gate composition")
    };
    *sealed_channel = CHANNEL.into();
    system.channels[0].name = CHANNEL.into();
    let tree: phoenix_sim_gameplay::ai::state_tree::AiTreeConfig =
        serde_json::from_value(serde_json::json!({
            "root":"gate", "tree":[{"id":"gate", "initial":"operate",
                "memory":{"visited":false}, "state":[{"id":"operate", "task":[
                    {"id":"remember", "action":{"kind":"remember", "key":"visited",
                        "value":{"source":"literal", "value":true}}},
                    {"id":"follow", "action":{"kind":"write", "channel":CHANNEL,
                        "value":{"source":"fact", "name":"seal_requested"}}}
                ]}]}]
        }))
        .unwrap();
    system
        .config
        .as_mut()
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("ai".into(), toml::Value::try_from(tree).unwrap());
    let layout = content.interior.as_mut().unwrap();
    let room = layout.placements.remove(&old).unwrap();
    layout.placements.insert(gate(), room);
    for connection in &mut layout.connections {
        if connection.boundary.as_ref() == Some(&old) {
            connection.boundary = Some(gate());
        }
    }
    content
}

fn fixture_with(content: EntityConfig, source: ControlSource) -> (App, Entity) {
    let ship = content.ship_config.unwrap();
    let (programs, runtimes) =
        compile_ship_trees_with_energy(&ship, content.energy.as_ref()).unwrap();
    let access = InteriorAccess::from_ship(&ship);
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        SystemId("repair".into()),
        SystemBlackboard::Repair(RepairBlackboard {
            boundaries: access.statuses(),
            ..default()
        }),
    );
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(gate(), source);
    let mut app = App::new();
    app.add_plugins((
        LobbyPlugin,
        bevy::time::TimePlugin,
        crate::server_app::AdmissionPlugin,
    ))
    .configure_sets(
        FixedUpdate,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain(),
    );
    crate::ai::host::register_ai_host_env(&mut app);
    crate::sim_tick::register_sim_tick(&mut app);
    register(&mut app);
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    let station = ship
        .system_home_station(ship.system(&gate()).unwrap())
        .unwrap()
        .clone();
    {
        let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
        sessions
            .0
            .register(HOLDER.into(), "Engineer".into())
            .unwrap();
        sessions.0.set_station(HOLDER, Some(station));
        sessions
            .0
            .register("visitor".into(), "Visitor".into())
            .unwrap();
        sessions
            .0
            .set_station("visitor", Some(StationId("helm".into())));
    }
    let entity = app
        .world_mut()
        .spawn((
            crate::server_app::LocalShip,
            crate::entities::spawner::EntityUuid("access-fixture".into()),
            ShipConfigComponent(ship),
            sources,
            access,
            boards,
            programs,
            runtimes,
            AdmittedCommands::default(),
        ))
        .id();
    (app, entity)
}

fn fixture(source: ControlSource) -> (App, Entity) {
    fixture_with(content(), source)
}

fn send(app: &mut App, token: &str, target: SystemId, payload: Control) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg: ClientMessage::ControlSystem { target, payload },
        });
}

fn status(app: &App, entity: Entity) -> BoundaryStatus {
    app.world()
        .get::<InteriorAccess>(entity)
        .unwrap()
        .statuses()
        .into_iter()
        .find(|state| state.system == gate())
        .unwrap()
}

fn publish(app: &mut App, entity: Entity, states: Vec<BoundaryStatus>) {
    let mut boards = app
        .world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(entity)
        .unwrap();
    let SystemBlackboard::Repair(board) = boards.0.get_mut(&SystemId("repair".into())).unwrap()
    else {
        panic!("typed Repair publication")
    };
    board.boundaries = states;
}

fn requested_publication(app: &mut App, entity: Entity) {
    let mut observed = status(app, entity);
    observed.seal_requested = true;
    publish(app, entity, vec![observed]);
}

#[test]
fn registered_consumer_applies_only_the_current_human_owner_and_authored_target() {
    let (mut app, entity) = fixture(ControlSource::Human);
    let topology = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    let registry = app
        .world()
        .resource::<crate::command_admission::AdmittedConsumerRegistry>();
    for payload in [
        Control::SetAccessSeal { sealed: true },
        Control::SetAccessActuator { sealed: true },
    ] {
        assert_eq!(
            registry.feedback_support(&gate(), &payload, topology),
            crate::command_admission::FeedbackSupport::Supported
        );
    }
    send(
        &mut app,
        "visitor",
        gate(),
        Control::SetAccessSeal { sealed: true },
    );
    send(
        &mut app,
        HOLDER,
        SystemId("weapons-access".into()),
        Control::SetAccessSeal { sealed: true },
    );
    app.update();
    assert!(!status(&app, entity).seal_requested);
    assert!(app
        .world()
        .get::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .is_empty());
    let correlation = crate::core::messages::ActionCorrelationId::new("seal-hatch").unwrap();
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: HOLDER.into(),
            msg: ClientMessage::ControlSystemCorrelated {
                correlation: correlation.clone(),
                target: gate(),
                payload: Control::SetAccessSeal { sealed: true },
            },
        });
    app.update();
    let state = status(&app, entity);
    assert!(state.seal_requested && state.actuator_sealed);
    assert_eq!(state.phase, BoundaryPhase::Sealing);
    assert!(state.elapsed > 0.0);
    assert!(app
        .world()
        .get::<AdmittedCommands>(entity)
        .unwrap()
        .for_target(GATE)
        .any(|command| matches!(command.payload, Control::SetAccessSeal { sealed: true })));
    assert_eq!(
        app.world()
            .get::<AdmittedCommands>(entity)
            .unwrap()
            .for_target(GATE)
            .next()
            .unwrap()
            .feedback_correlation
            .as_ref(),
        Some(&correlation)
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(gate(), ControlSource::Ai);
    send(
        &mut app,
        HOLDER,
        gate(),
        Control::SetAccessSeal { sealed: false },
    );
    app.update();
    assert!(
        status(&app, entity).seal_requested,
        "previous operator cannot cancel after delegation"
    );
}

#[test]
fn ordinary_tree_reads_the_console_publication_instead_of_live_access_state() {
    let (mut app, entity) = fixture(ControlSource::Ai);
    requested_publication(&mut app, entity);
    assert!(!status(&app, entity).seal_requested);
    app.update();
    let state = status(&app, entity);
    assert!(state.actuator_sealed);
    assert!(
        !state.seal_requested,
        "AI actuator never creates the protected order"
    );
    let commands = app.world().get::<AdmittedCommands>(entity).unwrap();
    let command = commands.for_target(GATE).next().unwrap();
    assert!(matches!(
        command.payload,
        Control::SetAccessActuator { sealed: true }
    ));
    assert_eq!(command.response_token.as_deref(), Some("ai:access-fixture"));
    assert!(command.feedback_correlation.is_none());
}

#[test]
fn cohosted_console_requires_an_explicit_correctly_typed_repair_feed() {
    let mut content = content();
    let ship = content.ship_config.as_mut().unwrap();
    let owner = ship.system(&gate()).unwrap().console.clone().unwrap();
    let mut console = ship
        .consoles
        .iter()
        .find(|console| console.id == owner)
        .unwrap()
        .clone();
    console.id = ConsoleId("access-only".into());
    console.feeds.clear();
    console.ratings.clear();
    ship.systems
        .iter_mut()
        .find(|system| system.id == gate())
        .unwrap()
        .console = Some(console.id.clone());
    ship.consoles.push(console);
    // Both tabs still share the same home Station. That is not an information grant.
    let (mut app, entity) = fixture_with(content, ControlSource::Ai);
    requested_publication(&mut app, entity);
    let pristine = app
        .world()
        .get::<ShipAiTreeRuntime>(entity)
        .unwrap()
        .clone();
    app.update();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(entity).unwrap(),
        &pristine
    );
    assert!(!status(&app, entity).actuator_sealed);
    let set_feed = |app: &mut App, kind| {
        let mut ship = app
            .world_mut()
            .get_mut::<ShipConfigComponent>(entity)
            .unwrap();
        ship.0
            .consoles
            .iter_mut()
            .find(|console| console.id.0 == "access-only")
            .unwrap()
            .feeds = vec![ConsoleFeed {
            source: SystemId("repair".into()),
            kind,
        }];
    };
    set_feed(&mut app, ConsoleObservationKind::Power);
    app.update();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(entity).unwrap(),
        &pristine
    );
    set_feed(&mut app, ConsoleObservationKind::Repair);
    app.update();
    assert!(status(&app, entity).actuator_sealed);
    assert_ne!(
        app.world().get::<ShipAiTreeRuntime>(entity).unwrap(),
        &pristine
    );
    // Receiving the feed does not confer authority over Repair machinery.
    assert_ne!(
        app.world()
            .get::<ShipConfigComponent>(entity)
            .unwrap()
            .0
            .system(&SystemId("repair".into()))
            .unwrap()
            .console,
        Some(ConsoleId("access-only".into()))
    );
}

#[test]
fn absent_boundary_observation_or_compiled_program_does_not_invent_a_controller() {
    let (mut app, entity) = fixture(ControlSource::Ai);
    let pristine = app
        .world()
        .get::<ShipAiTreeRuntime>(entity)
        .unwrap()
        .clone();
    publish(&mut app, entity, Vec::new());
    app.world_mut().run_system_once(ai_access).unwrap();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(entity).unwrap(),
        &pristine
    );
    assert!(app
        .world()
        .get::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .is_empty());
    requested_publication(&mut app, entity);
    app.world_mut()
        .get_mut::<crate::ai::state_tree_runtime::ShipAiTreePrograms>(entity)
        .unwrap()
        .0
        .remove(&gate());
    app.world_mut().run_system_once(ai_access).unwrap();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(entity).unwrap(),
        &pristine
    );
    assert!(app
        .world()
        .get::<AdmittedCommands>(entity)
        .unwrap()
        .0
        .is_empty());
    assert!(!status(&app, entity).actuator_sealed);
}

#[test]
fn human_and_offline_modes_suspend_private_frames_then_resume_the_same_tree() {
    let (mut app, entity) = fixture(ControlSource::Human);
    requested_publication(&mut app, entity);
    let initial = app
        .world()
        .get::<ShipAiTreeRuntime>(entity)
        .unwrap()
        .clone();
    app.update();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(entity).unwrap(),
        &initial
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(gate(), ControlSource::Ai);
    app.update();
    let continued = app
        .world()
        .get::<ShipAiTreeRuntime>(entity)
        .unwrap()
        .clone();
    assert_ne!(continued, initial);
    assert!(continued.0[&gate()].last_tick.is_some());
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(gate(), ControlSource::Human);
    app.update();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(entity).unwrap(),
        &continued
    );
    {
        let mut sources = app
            .world_mut()
            .get_mut::<ShipSystemControlSources>(entity)
            .unwrap();
        sources.0.set(gate(), ControlSource::Ai);
        sources.0.set_offline(gate(), true);
    }
    app.update();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(entity).unwrap(),
        &continued
    );
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set_offline(gate(), false);
    app.update();
    let resumed = app.world().get::<ShipAiTreeRuntime>(entity).unwrap();
    assert!(resumed.0[&gate()].last_tick > continued.0[&gate()].last_tick);
    assert!(resumed.0[&gate()].root.is_some());
}

#[test]
fn stale_opening_advice_cannot_clear_a_protected_human_seal() {
    let (mut app, entity) = fixture(ControlSource::Human);
    send(
        &mut app,
        HOLDER,
        gate(),
        Control::SetAccessSeal { sealed: true },
    );
    app.update();
    // The old publication says no seal is requested and the actuator is closed,
    // provoking an actual ordinary-AI open command after delegation.
    let mut observed = status(&app, entity);
    observed.seal_requested = false;
    publish(&mut app, entity, vec![observed]);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(gate(), ControlSource::Ai);
    for _ in 0..6 {
        app.update();
    }
    let state = status(&app, entity);
    assert!(state.seal_requested);
    assert!(!state.actuator_sealed);
    assert_eq!(state.phase, BoundaryPhase::Sealed);
    assert!(app
        .world()
        .get::<AdmittedCommands>(entity)
        .unwrap()
        .for_target(GATE)
        .any(|command| matches!(
            command.payload,
            Control::SetAccessActuator { sealed: false }
        )));
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(gate(), ControlSource::Human);
    send(
        &mut app,
        HOLDER,
        gate(),
        Control::SetAccessSeal { sealed: false },
    );
    for _ in 0..6 {
        app.update();
    }
    assert_eq!(status(&app, entity).phase, BoundaryPhase::Open);
}

#[test]
fn offline_opening_pauses_elapsed_progress_and_resumes_without_resetting_intent() {
    let (mut app, entity) = fixture(ControlSource::Human);
    send(
        &mut app,
        HOLDER,
        gate(),
        Control::SetAccessSeal { sealed: true },
    );
    for _ in 0..6 {
        app.update();
    }
    assert_eq!(status(&app, entity).phase, BoundaryPhase::Sealed);
    send(
        &mut app,
        HOLDER,
        gate(),
        Control::SetAccessSeal { sealed: false },
    );
    app.update();
    let paused = status(&app, entity);
    assert_eq!(paused.phase, BoundaryPhase::Opening);
    assert!(paused.elapsed > 0.0);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set_offline(gate(), true);
    for _ in 0..6 {
        app.update();
    }
    assert_eq!(status(&app, entity), paused);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set_offline(gate(), false);
    app.update();
    assert!(status(&app, entity).elapsed > paused.elapsed);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(status(&app, entity).phase, BoundaryPhase::Open);
}

#[test]
fn an_actual_repair_crossing_interlocks_the_gate_until_the_team_clears_it() {
    let content = content();
    let ship = content.ship_config.as_ref().unwrap().clone();
    let layout = content.interior.as_ref().unwrap().clone();
    let crossing_secs = layout
        .connections
        .iter()
        .find(|connection| connection.id == "engineering-weapons")
        .unwrap()
        .travel_secs;
    let count = layout.team_homes.len();
    let graph = phoenix_sim_gameplay::ship::interior::InteriorGraph::compile(&layout, &ship, count)
        .unwrap();
    let target = layout
        .placements
        .iter()
        .find(|(_, room)| room.as_str() == "weapons")
        .unwrap()
        .0
        .clone();
    let mut teams = phoenix_sim_gameplay::jobs::repair::RepairTeams::new(count);
    teams.install_layout(graph);
    teams.dispatch(0, target.clone(), "Target equipment".into());
    let mut hull = crate::ship::damage::SystemHull::from_config(&[(target.clone(), 100.0)]);
    hull.set_hp(&target, 50.0);
    teams.tick(0.2, &mut hull, Some(&ship));
    assert!(matches!(&teams.navigation().unwrap()[0].location,
        TeamLocation::Crossing { connection, .. } if connection == "engineering-weapons"));
    let (mut app, entity) = fixture_with(content, ControlSource::Human);
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::console::repair::server::ShipRepairTeams(teams));
    send(
        &mut app,
        HOLDER,
        gate(),
        Control::SetAccessSeal { sealed: true },
    );
    for _ in 0..7 {
        app.update();
    }
    let waiting = status(&app, entity);
    assert_eq!(waiting.phase, BoundaryPhase::Sealing);
    assert_eq!(waiting.elapsed, 0.0);
    let blocked = app
        .world()
        .get::<InteriorAccess>(entity)
        .unwrap()
        .blocked_connections(&layout);
    assert!(blocked.contains("engineering-weapons"));
    {
        let mut teams = app
            .world_mut()
            .get_mut::<crate::console::repair::server::ShipRepairTeams>(entity)
            .unwrap();
        // Entry is forbidden, but a committed crossing remains able to finish.
        teams
            .0
            .tick_with_access(crossing_secs, &mut hull, Some(&ship), &blocked, |_, _| 1.0);
        assert!(matches!(&teams.0.navigation().unwrap()[0].location,
            TeamLocation::Compartment { compartment } if compartment == "weapons"));
    }
    for _ in 0..6 {
        app.update();
    }
    assert_eq!(status(&app, entity).phase, BoundaryPhase::Sealed);
    // A second team cannot start a new crossing while the order is sealed.
    let mut teams = app
        .world_mut()
        .get_mut::<crate::console::repair::server::ShipRepairTeams>(entity)
        .unwrap();
    let target = layout
        .placements
        .iter()
        .find(|(_, room)| room.as_str() == "weapons")
        .unwrap()
        .0
        .clone();
    teams.0.dispatch(1, target, "Second team".into());
    teams
        .0
        .tick_with_access(0.2, &mut hull, Some(&ship), &blocked, |_, _| 1.0);
    assert!(matches!(&teams.0.navigation().unwrap()[1].location,
        TeamLocation::Compartment { compartment } if compartment == "engineering"));
    assert!(teams.0.navigation().unwrap()[1].blocked);
}
