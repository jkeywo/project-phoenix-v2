use super::*;
use crate::core::messages::{ActionCorrelationId, AdmittedCommand, DeliveryClass, ServerMessage};
use crate::dock::mating::DockConfig;
use crate::lobby::{server::OutboundMessage, Target};
use crate::ship::system_registry::DOCK_SYSTEM_ID;

fn config() -> DockConfig {
    DockConfig {
        moves_body: true,
        range: 200.0,
        engage_distance: 400.0,
        approach_speed: 60.0,
        mate_tolerance: 4.0,
        undock_clear_distance: 120.0,
        min_power_level: 2,
    }
}

fn control_with_id(id: &str) -> DockControl {
    DockControl::new(SystemId(id.into()), config(), PowerGroupId("dock".into()))
}

fn control() -> DockControl {
    control_with_id(DOCK_SYSTEM_ID)
}

#[test]
fn fitted_focus_only_dock_refuses_unobserved_available_target_but_undock_remains_physical() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    let mut dock = control();
    dock.available_target = Some("unobserved-partner".into());
    let operator = world
        .spawn((
            dock,
            Transform::default(),
            crate::science::broad::SensingControls(Default::default()),
            crate::core::messages::AdmittedCommands(vec![correlated_dock_command(
                SystemControlPayload::Dock,
                "unobserved",
            )]),
        ))
        .id();
    install_actual_dock_equipment(&mut world, operator);
    world.entity_mut(operator).insert(own_marker());
    world.run_system_once(handle_dock_commands).unwrap();
    let dock = world.get::<DockControl>(operator).unwrap();
    assert!(!dock.engaged());
    assert_eq!(dock.last_refusal, Some(DockRefusal::NoTarget));
    let mut dock = world.get_mut::<DockControl>(operator).unwrap();
    dock.set_engaged(true);
    dock.docked = true;
    dock.set_docking_target(Some("physical-partner".into()));
    drop(dock);
    world
        .entity_mut(operator)
        .insert(crate::core::messages::AdmittedCommands(vec![
            correlated_dock_command(SystemControlPayload::Undock, "release"),
        ]));
    world.run_system_once(handle_dock_commands).unwrap();
    let dock = world.get::<DockControl>(operator).unwrap();
    assert!(!dock.engaged() && !dock.docked);
    assert!(dock.undock_target.is_some());
}

fn install_actual_dock_equipment(world: &mut World, entity: Entity) {
    let config =
        crate::entities::include_resolve::load_entity_config("assets/entities/dock_probe.toml")
            .unwrap();
    let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
    for system in &config.ship_config.as_ref().unwrap().systems {
        sources.0.set(
            system.id.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
    }
    let damage = ShipDamage::from_config(&config).unwrap().unwrap();
    let power = ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(
            config.energy.unwrap(),
            config.ship_config.clone().unwrap(),
        )
        .unwrap(),
    );
    if world.get::<ShipPhysics>(entity).is_none() {
        let transform = world.get::<Transform>(entity).copied().unwrap_or_default();
        let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
        world.entity_mut(entity).insert(ShipPhysics {
            x: transform.translation.x,
            y: transform.translation.y,
            z: transform.translation.z,
            yaw: -yaw,
            ..Default::default()
        });
    }
    world.entity_mut(entity).insert((
        crate::ship_plugin::ShipConfigComponent(config.ship_config.unwrap()),
        sources,
        damage,
        power,
    ));
}

fn correlated_dock_command(payload: SystemControlPayload, correlation: &str) -> AdmittedCommand {
    AdmittedCommand {
        target: SystemId(DOCK_SYSTEM_ID.into()),
        payload,
        response_token: Some("helm".into()),
        feedback_correlation: Some(
            ActionCorrelationId::new(correlation).expect("valid test correlation"),
        ),
    }
}

fn feedback_outcomes(
    messages: &[OutboundMessage],
    correlation: &str,
) -> Vec<ActionFeedbackOutcome> {
    messages
        .iter()
        .filter_map(|message| {
            if message.target != Target::Token("helm".into())
                || message.delivery != DeliveryClass::Reliable
            {
                return None;
            }
            match &message.msg {
                ServerMessage::ActionFeedback {
                    correlation: actual,
                    outcome,
                } if actual.as_str() == correlation => Some(*outcome),
                _ => None,
            }
        })
        .collect()
}

// The fixture reads only the command publication consumed by its AI host.
fn dock_ai_scope() -> crate::ship_plugin::ShipConfigComponent {
    let mut scope = crate::ship_plugin::ShipConfigComponent(
        crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "operations"
name = "Operations"
description = "Fixture controls"
rank = "Ens."
[[console]]
id = "operations"
home_station = "operations"
screen = "fixture.html"
[[console.feed]]
source = "viewscreen"
kind = "Viewscreen"
[[system]]
id = "dock"
kind = "dock"
console = "operations"
power_group = "dock"
[[system.channel]]
name = "enabled"
kind = "boolean"

[[system.channel]]
name = "target"
kind = "entity_target"

[[system.component]]
id = "power"
kind = "power_input"

[[system.component]]
id = "mating"
kind = "dock_mating"
enabled_channel = "enabled"
target_channel = "target"
power_input = "power"

[system.component.config]
range = 200.0
engage_distance = 400.0
approach_speed = 60.0
mate_tolerance = 4.0
undock_clear_distance = 120.0
min_power_level = 2
[power_groups.dock]
label = "Dock"
"#,
            &["dock"],
        )
        .expect("explicit fixture Console scope"),
    );
    let fragment: toml::Value = toml::from_str(include_str!(
        "../../../../assets/entities/fragments/ai/dock_state_tree.toml"
    ))
    .unwrap();
    scope.0.systems[0].config = Some(fragment["system"][0]["config"].clone());
    scope
}

fn dock_tree_app() -> (App, Entity) {
    use crate::core::messages::{
        AdmittedCommands, AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus,
        ScoredObjective, ViewscreenBlackboard,
    };
    let mut app = App::new();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    app.init_resource::<crate::sim_tick::SimTick>();
    app.add_systems(Update, operate_dock_ai);
    let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
    sources.0.set(
        SystemId(DOCK_SYSTEM_ID.into()),
        crate::ship::control_source::ControlSource::Ai,
    );
    let mut dock = control();
    dock.docked = true;
    dock.set_engaged(false);
    dock.available_target = Some("berth".into());
    let mut blackboards = crate::server_app::ShipSystemBlackboards::default();
    blackboards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            scored_objectives: vec![ScoredObjective {
                id: "transfer".into(),
                score: 1.0,
                directive: AiDirective::Transfer {
                    target: "berth".into(),
                },
                source: ObjectiveSource::Mission,
                relevance: vec![SystemAffinity::Helm],
                snapshot: ObjectiveSnapshot {
                    id: "transfer".into(),
                    text: String::new(),
                    text_params: Default::default(),
                    mandatory: false,
                    status: ObjectiveStatus::Active,
                    targets: vec![],
                    source: ObjectiveSource::Mission,
                    progress: None,
                    unassigned: false,
                },
            }],
            ..Default::default()
        }),
    );
    let scope = dock_ai_scope();
    let trees = crate::ai::state_tree_runtime::compile_ship_trees(&scope.0).unwrap();
    let operator = app
        .world_mut()
        .spawn((
            EntityUuid("operator".into()),
            sources,
            scope,
            trees,
            dock,
            blackboards,
            AdmittedCommands::default(),
        ))
        .id();
    (app, operator)
}

#[test]
fn restored_docked_state_keeps_engaged_only_start_and_host_withdrawal() {
    use crate::core::messages::AdmittedCommands;
    let (mut app, operator) = dock_tree_app();
    app.update();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(operator).unwrap().0[&SystemId("dock".into())]
            .memory("owned"),
        Some(true)
    );
    let first = app
        .world_mut()
        .entity_mut(operator)
        .take::<AdmittedCommands>()
        .unwrap();
    assert_eq!(first.0.len(), 1);
    assert_eq!(first.0[0].payload, SystemControlPayload::Dock);
    app.world_mut()
        .entity_mut(operator)
        .insert(AdmittedCommands::default());
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<crate::server_app::ShipSystemBlackboards>()
        .unwrap()
        .0
        .clear();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.update();
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(operator).unwrap().0[&SystemId("dock".into())]
            .memory("owned"),
        Some(false)
    );
    assert_eq!(
        app.world()
            .entity(operator)
            .get::<AdmittedCommands>()
            .unwrap()
            .0[0]
            .payload,
        SystemControlPayload::Undock
    );
    assert!(matches!(
        app.world()
            .resource::<EffectQueue<TaskLifecycleRequest>>()
            .0
            .as_slice(),
        [TaskLifecycleRequest::End {
            reason: TaskTerminalReason::OrderWithdrawn,
            ..
        }]
    ));
}

#[test]
fn save_state_carries_engage_dock_and_target_only() {
    let mut c = control();
    c.set_engaged(true);
    c.docked = true;
    c.set_docking_target(Some("berth-1".into()));
    c.last_refusal = Some(DockRefusal::OutOfRange);
    let save = c.save_state();
    assert!(save.channels.boolean("enabled"));
    assert!(save.docked);
    assert_eq!(save.channels.target("target"), Some("berth-1"));
}

#[test]
fn an_idle_control_saves_declared_channels() {
    let save = control().save_state();
    assert!(!save.channels.boolean("enabled"));
    assert_eq!(save.channels.target("target"), None);
    assert_eq!(save.channels.0.len(), 2);
}

#[test]
fn docked_partner_is_only_the_target_while_docked() {
    let mut c = control();
    c.set_docking_target(Some("berth-1".into()));
    c.set_engaged(true);
    assert_eq!(c.docked_partner(), None, "approaching is not yet docked");
    c.docked = true;
    assert_eq!(c.docked_partner(), Some("berth-1"));
}

#[test]
fn authored_instance_id_is_the_consumed_command_target() {
    let mut app = App::new();
    app.add_systems(Update, handle_dock_commands);

    let mut dock = control_with_id("berthing-clamps");
    dock.available_target = Some("berth-1".into());
    let entity = app
        .world_mut()
        .spawn((
            dock,
            Transform::default(),
            crate::core::messages::AdmittedCommands(vec![crate::core::messages::AdmittedCommand {
                target: SystemId("berthing-clamps".into()),
                payload: SystemControlPayload::Dock,
                response_token: None,
                feedback_correlation: None,
            }]),
        ))
        .id();

    app.update();

    let dock = app.world().entity(entity).get::<DockControl>().unwrap();
    assert!(
        dock.engaged(),
        "the authored Dock SystemId must be consumed"
    );
    assert_eq!(dock.docking_target(), Some("berth-1"));
}

#[test]
fn dock_reports_one_applied_or_refused_terminal_outcome() {
    for (correlation, available, expected) in [
        (
            "dock-applied",
            Some("berth-1"),
            ActionFeedbackOutcome::Applied,
        ),
        ("dock-refused", None, ActionFeedbackOutcome::Refused),
    ] {
        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .add_systems(Update, handle_dock_commands);
        let mut dock = control();
        dock.available_target = available.map(str::to_string);
        app.world_mut().spawn((
            dock,
            Transform::default(),
            crate::core::messages::AdmittedCommands(vec![correlated_dock_command(
                SystemControlPayload::Dock,
                correlation,
            )]),
        ));
        let mut cursor = app
            .world()
            .resource::<Messages<OutboundMessage>>()
            .get_cursor();

        app.update();

        let messages: Vec<_> = cursor
            .read(app.world().resource::<Messages<OutboundMessage>>())
            .cloned()
            .collect();
        assert_eq!(feedback_outcomes(&messages, correlation), vec![expected]);
    }
}

#[test]
fn undock_reports_one_applied_or_refused_terminal_outcome() {
    for (correlation, engaged, expected) in [
        ("undock-applied", true, ActionFeedbackOutcome::Applied),
        ("undock-refused", false, ActionFeedbackOutcome::Refused),
    ] {
        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .add_systems(Update, handle_dock_commands);
        let mut dock = control();
        dock.set_engaged(engaged);
        app.world_mut().spawn((
            dock,
            Transform::default(),
            crate::core::messages::AdmittedCommands(vec![correlated_dock_command(
                SystemControlPayload::Undock,
                correlation,
            )]),
        ));
        let mut cursor = app
            .world()
            .resource::<Messages<OutboundMessage>>()
            .get_cursor();

        app.update();

        let messages: Vec<_> = cursor
            .read(app.world().resource::<Messages<OutboundMessage>>())
            .cloned()
            .collect();
        assert_eq!(feedback_outcomes(&messages, correlation), vec![expected]);
    }
}

#[test]
fn authored_instance_id_is_the_published_blackboard_key() {
    let mut app = App::new();
    app.add_systems(Update, publish_dock_blackboard);

    let entity = app
        .world_mut()
        .spawn((
            control_with_id("berthing-clamps"),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();

    app.update();

    let blackboards = app
        .world()
        .entity(entity)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .unwrap();
    assert!(matches!(
        blackboards.0.get(&SystemId("berthing-clamps".into())),
        Some(SystemBlackboard::Dock(_))
    ));
    assert!(
        !blackboards.0.contains_key(&dock_system_id()),
        "an arbitrary authored id must not also publish a literal dock channel"
    );
}

#[test]
fn resolve_dock_markers_takes_only_dock_prefixed_markers_in_order() {
    let rig = crate::entities::model_rig::parse_model_rig(
        r#"
            [markers.dock_fore]
            position = [0.0, 0.0, -5.0]
            direction = [0.0, 0.0, -1.0]
            [markers.engine_port]
            position = [-1.0, 0.0, 3.0]
            direction = [0.0, 0.0, 1.0]
            [markers.dock_aft]
            position = [0.0, 0.0, 5.0]
            direction = [0.0, 0.0, 1.0]
            "#,
    )
    .expect("rig parses");
    let dm = resolve_dock_markers(&rig);
    assert_eq!(dm.markers.len(), 2, "only the two dock_* markers");
    // Sorted by name: dock_aft (z=+5) then dock_fore (z=-5).
    assert!((dm.markers[0].position.z - 5.0).abs() < 1e-4);
    assert!((dm.markers[1].position.z + 5.0).abs() < 1e-4);
}

#[test]
fn a_rig_with_no_dock_markers_resolves_empty() {
    let rig = crate::entities::model_rig::parse_model_rig(
        r#"
            [markers.engine_port]
            position = [-1.0, 0.0, 3.0]
            direction = [0.0, 0.0, 1.0]
            "#,
    )
    .expect("rig parses");
    assert!(resolve_dock_markers(&rig).is_empty());
}

// ── The task lifecycle (issue #1345) ─────────────────────────────────────

const OPERATOR: &str = "operator-1";
const TARGET: &str = "target-1";

/// A config that needs no power, so the verdict turns on the mate geometry
/// and the range alone — the same trick `tractor::server`'s tests use.
fn unpowered_config() -> DockConfig {
    DockConfig {
        min_power_level: 0,
        ..config()
    }
}

fn own_marker() -> DockMarkers {
    DockMarkers {
        markers: vec![DockMarker {
            position: Vec3::ZERO,
            direction: Vec3::new(0.0, 0.0, -1.0),
        }],
    }
}

fn target_marker() -> DockMarkers {
    DockMarkers {
        markers: vec![DockMarker {
            position: Vec3::ZERO,
            direction: Vec3::new(0.0, 0.0, 1.0),
        }],
    }
}

/// An operator already ENGAGED and closing on `TARGET`, plus the target hull
/// itself, both coincident at `(100, 0, 0)` — the exact mate point for the
/// two markers above — so the very first `tick_dock` mates them.
fn engage_world() -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs(1));
    app.insert_resource(time);
    app.add_systems(Update, (handle_dock_commands, tick_dock).chain());

    let operator = app
        .world_mut()
        .spawn((
            EntityUuid(OPERATOR.into()),
            funded_power(),
            Transform::from_xyz(100.0, 0.0, 0.0),
            own_marker(),
            {
                let mut dock = control_with_id_config(DOCK_SYSTEM_ID, unpowered_config());
                dock.set_engaged(true);
                dock.set_docking_target(Some(TARGET.into()));
                dock
            },
            crate::core::messages::AdmittedCommands::default(),
        ))
        .id();
    install_actual_dock_equipment(app.world_mut(), operator);
    app.world_mut().spawn((
        EntityUuid(TARGET.into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
        target_marker(),
    ));
    (app, operator)
}

fn control_with_id_config(id: &str, cfg: DockConfig) -> DockControl {
    DockControl::new(SystemId(id.into()), cfg, PowerGroupId("dock".into()))
}

fn drain_lifecycle(app: &mut App) -> Vec<TaskLifecycleRequest> {
    std::mem::take(
        &mut app
            .world_mut()
            .resource_mut::<EffectQueue<TaskLifecycleRequest>>()
            .0,
    )
}

/// Arrival opens exactly one activation, named for the berth the mate
/// actually formed on; an unchanged hold afterwards reports nothing.
#[test]
fn arrival_opens_one_activation_and_an_unchanged_hold_is_silent() {
    let (mut app, operator) = engage_world();
    app.update();

    assert!(
        app.world()
            .entity(operator)
            .get::<DockControl>()
            .unwrap()
            .docked,
        "the two coincident markers must have mated on the first tick"
    );
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::Start {
            slot: TaskSlot::new(OPERATOR, DOCK_SYSTEM_ID, TASK_VERB_DOCK_HOLD),
            target: Some(TARGET.into()),
        }]
    );

    app.update();
    assert!(
        drain_lifecycle(&mut app).is_empty(),
        "a hold that simply continues reports nothing more"
    );
}

/// `Undock` on a mated dock reports the cancel before clearing intent, and
/// `tick_dock` — seeing the intent already cleared this same tick — reports
/// nothing further, so the activation gets exactly one terminal.
#[test]
fn undock_reports_released_exactly_once() {
    let (mut app, operator) = engage_world();
    app.update();
    drain_lifecycle(&mut app);

    app.world_mut()
        .entity_mut(operator)
        .get_mut::<crate::core::messages::AdmittedCommands>()
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: SystemId(DOCK_SYSTEM_ID.into()),
            payload: SystemControlPayload::Undock,
            response_token: None,
            feedback_correlation: None,
        });
    app.update();

    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::End {
            slot: TaskSlot::new(OPERATOR, DOCK_SYSTEM_ID, TASK_VERB_DOCK_HOLD),
            reason: TaskTerminalReason::Released,
        }],
        "exactly one terminal, and it is a cancellation, not a failure"
    );
    assert!(
        !app.world()
            .entity(operator)
            .get::<DockControl>()
            .unwrap()
            .docked
    );
}

/// An engage refused before it ever mates — the berth is out of the authored
/// range from the very first tick — still gets a whole activation: a start
/// and its own terminal, adjacent on one slot, mirroring the tractor's
/// "refused before it couples" case.
#[test]
fn a_dock_refused_before_it_mates_still_opens_and_closes_one_activation() {
    let (mut app, operator) = engage_world();
    // Move the operator far beyond the authored range before the first tick,
    // so the mate never forms.
    // Move the canonical physical owner; Transform alone is a stale display
    // pose and deliberately cannot move accepted Dock equipment.
    app.world_mut().get_mut::<ShipPhysics>(operator).unwrap().x = 9000.0;
    app.world_mut()
        .entity_mut(operator)
        .insert(Transform::from_xyz(9000.0, 0.0, 0.0));
    app.update();

    let slot = TaskSlot::new(OPERATOR, DOCK_SYSTEM_ID, TASK_VERB_DOCK_HOLD);
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![
            TaskLifecycleRequest::Start {
                slot: slot.clone(),
                target: Some(TARGET.into()),
            },
            TaskLifecycleRequest::End {
                slot,
                reason: TaskTerminalReason::OutOfRange,
            },
        ]
    );
    assert!(
        !app.world()
            .entity(operator)
            .get::<DockControl>()
            .unwrap()
            .engaged(),
        "a refused engage drops the intent with the (never-formed) mate"
    );
}

/// A mate that drifts out of range AFTER forming ends the standing
/// activation with the mapped reason — a close only, no re-opened start.
#[test]
fn a_mate_that_drifts_out_of_range_ends_the_standing_activation() {
    let (mut app, operator) = engage_world();
    app.update();
    drain_lifecycle(&mut app);
    assert!(
        app.world()
            .entity(operator)
            .get::<DockControl>()
            .unwrap()
            .docked
    );

    // Move the canonical physical owner; Transform alone is a stale display
    // pose and deliberately cannot move accepted Dock equipment.
    app.world_mut().get_mut::<ShipPhysics>(operator).unwrap().x = 9000.0;
    app.world_mut()
        .entity_mut(operator)
        .insert(Transform::from_xyz(9000.0, 0.0, 0.0));
    app.update();

    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::End {
            slot: TaskSlot::new(OPERATOR, DOCK_SYSTEM_ID, TASK_VERB_DOCK_HOLD),
            reason: TaskTerminalReason::OutOfRange,
        }],
        "the standing activation closes; nothing new opens for a hold that was already live"
    );
}

#[test]
fn composed_channels_drive_existing_commands_and_survive_restore() {
    let mut system: crate::ship::config::SystemInstanceConfig = toml::from_str(
        r#"
        id = "custom-clamps"
        kind = "dock"
        power_group = "dock"
    "#,
    )
    .unwrap();
    system.channels = vec![
        CommandChannelConfig {
            name: "hold".into(),
            kind: CommandChannelKind::Boolean,
            initial: None,
        },
        CommandChannelConfig {
            name: "berth".into(),
            kind: CommandChannelKind::EntityTarget,
            initial: None,
        },
    ];
    system.components = vec![
        phoenix_sim_gameplay::ship::composition::SystemComponentConfig::PowerInput {
            id: "supply".into(),
        },
        phoenix_sim_gameplay::ship::composition::SystemComponentConfig::DockMating {
            id: "mating".into(),
            enabled_channel: "hold".into(),
            target_channel: "berth".into(),
            power_input: "supply".into(),
            config: config(),
        },
    ];
    let mut control = DockControl::from_system(&system);
    control
        .channels
        .set("hold", CommandChannelValue::Boolean(true))
        .unwrap();
    control
        .channels
        .set(
            "berth",
            CommandChannelValue::EntityTarget(Some("berth-1".into())),
        )
        .unwrap();
    assert!(control.engaged());
    assert_eq!(control.docking_target(), Some("berth-1"));
    control.docked = true;
    let encoded = serde_json::to_string(&control.save_state()).unwrap();
    let save = serde_json::from_str(&encoded).unwrap();
    let mut restored = DockControl::from_system(&system);
    restored.restore(&save);
    assert_eq!(restored.docked_partner(), Some("berth-1"));
    assert_eq!(restored.channels, control.channels);
    restored.set_engaged(false);
    restored.set_docking_target(None);
    assert!(!restored.channels.boolean("hold"));
    assert_eq!(restored.channels.target("berth"), None);
}

#[test]
fn resumed_clearance_finishes_the_same_manoeuvre() {
    let mut original = control();
    original.undock_target = Some(Vec3::new(0.0, 0.0, 120.0));
    let mut restored = control();
    restored.restore(
        &serde_json::from_str(&serde_json::to_string(&original.save_state()).unwrap()).unwrap(),
    );
    assert_eq!(restored.undock_target, original.undock_target);
    let pose = Pose {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };
    let expected = decide_dock(
        Entity::PLACEHOLDER,
        "ship",
        &original,
        pose,
        &[],
        2,
        false,
        &[],
        1.0,
    );
    let actual = decide_dock(
        Entity::PLACEHOLDER,
        "ship",
        &restored,
        pose,
        &[],
        2,
        false,
        &[],
        1.0,
    );
    assert_eq!(actual.undock_target, expected.undock_target);
    assert_eq!(actual.new_pose, expected.new_pose);
    assert!(actual.new_pose.is_some());
}

#[test]
fn availability_uses_current_conditions_not_historical_refusal() {
    let mut dock = control();
    dock.last_refusal = Some(DockRefusal::OutOfRange);
    assert_eq!(dock.availability_refusal(2, false, true), None);
    assert_eq!(
        dock.availability_refusal(1, false, true),
        Some(DockRefusal::Unpowered)
    );
    assert_eq!(
        dock.availability_refusal(2, true, true),
        Some(DockRefusal::Disabled)
    );
    assert_eq!(
        dock.availability_refusal(2, false, false),
        Some(DockRefusal::NoDockMarkers)
    );
}

#[test]
fn dock_tree_suspends_private_memory_on_human_and_offline_then_resumes_withdrawal() {
    use crate::core::messages::AdmittedCommands;
    use crate::ship::control_source::ControlSource;
    let (mut app, operator) = dock_tree_app();
    app.update();
    let saved = app
        .world()
        .get::<ShipAiTreeRuntime>(operator)
        .unwrap()
        .clone();
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(operator)
        .unwrap()
        .0
        .clear();
    for source in [ControlSource::Human, ControlSource::Offline] {
        app.world_mut()
            .get_mut::<crate::ship_plugin::ShipSystemControlSources>(operator)
            .unwrap()
            .0
            .set(SystemId("dock".into()), source);
        app.world_mut()
            .get_mut::<AdmittedCommands>(operator)
            .unwrap()
            .0
            .clear();
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
        app.update();
        assert_eq!(app.world().get::<ShipAiTreeRuntime>(operator), Some(&saved));
        assert!(app
            .world()
            .get::<AdmittedCommands>(operator)
            .unwrap()
            .0
            .is_empty());
    }
    // A wire round trip during human control retains ownership and call memory.
    let restored: ShipAiTreeRuntime =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    app.world_mut().entity_mut(operator).insert(restored);
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(operator)
        .unwrap()
        .0
        .set(SystemId("dock".into()), ControlSource::Ai);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    app.update();
    assert_eq!(
        app.world().get::<AdmittedCommands>(operator).unwrap().0[0].payload,
        SystemControlPayload::Undock
    );
}

#[test]
fn dock_tree_cannot_read_unbound_orders_or_undock_unowned_human_work() {
    use crate::core::messages::AdmittedCommands;
    let (mut app, operator) = dock_tree_app();
    app.world_mut()
        .get_mut::<crate::ship_plugin::ShipConfigComponent>(operator)
        .unwrap()
        .0
        .consoles[0]
        .feeds
        .clear();
    app.update();
    assert!(app
        .world()
        .get::<AdmittedCommands>(operator)
        .unwrap()
        .0
        .is_empty());
    assert_eq!(
        app.world().get::<ShipAiTreeRuntime>(operator).unwrap().0[&SystemId("dock".into())]
            .memory("owned"),
        Some(false)
    );
    assert!(app.world().get::<DockControl>(operator).unwrap().docked);
}

#[test]
fn custom_tree_stop_with_live_order_is_not_reported_as_order_withdrawal() {
    use crate::core::messages::AdmittedCommands;
    use phoenix_sim_gameplay::ai::state_tree::{AiTreeConfig, BoolSource, TaskAction, TaskDef};
    let (mut app, operator) = dock_tree_app();
    let mut scope = dock_ai_scope();
    let mut tree: AiTreeConfig = scope.0.systems[0].config.as_ref().unwrap()["ai"]
        .clone()
        .try_into()
        .unwrap();
    // An authored last writer stops the operation for its own reason while
    // the directive remains present. It still uses normal Undock admission.
    tree.tree
        .iter_mut()
        .find(|tree| tree.id == "dock-actions")
        .unwrap()
        .state[0]
        .task
        .push(TaskDef {
            id: "custom-stop".into(),
            when: vec![],
            action: TaskAction::Write {
                channel: "enabled".into(),
                value: BoolSource::Literal { value: false },
            },
        });
    scope.0.systems[0].config.as_mut().unwrap()["ai"] = toml::Value::try_from(tree).unwrap();
    let trees = crate::ai::state_tree_runtime::compile_ship_trees(&scope.0).unwrap();
    app.world_mut().entity_mut(operator).insert((scope, trees));
    // Controller-only fixture leaves the physical hold active between decisions:
    // first adopts it; the second proves previously owned work isn't mislabeled.
    for tick in [0, 1] {
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        app.world_mut()
            .get_mut::<AdmittedCommands>(operator)
            .unwrap()
            .0
            .clear();
        app.update();
        assert_eq!(
            app.world()
                .get::<AdmittedCommands>(operator)
                .unwrap()
                .0
                .len(),
            1
        );
        assert_eq!(
            app.world().get::<AdmittedCommands>(operator).unwrap().0[0].payload,
            SystemControlPayload::Undock
        );
        assert_eq!(
            app.world().get::<ShipAiTreeRuntime>(operator).unwrap().0[&SystemId("dock".into())]
                .memory("owned"),
            Some(true)
        );
        assert!(
            app.world()
                .resource::<EffectQueue<TaskLifecycleRequest>>()
                .0
                .is_empty(),
            "a live order has not been withdrawn"
        );
    }
}

/// Real finite local reserve for the lifecycle consumer under test. No refill
/// or alternate actuation path is installed by these bare owner fixtures.
fn funded_power() -> crate::ship::power::ShipPowerSystem {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(
            config.energy.unwrap(),
            config.ship_config.unwrap(),
        )
        .unwrap(),
    )
}

#[path = "separation_tests.rs"]
mod separation_tests;
