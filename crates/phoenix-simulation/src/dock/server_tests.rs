use super::*;
use crate::core::messages::{ActionCorrelationId, AdmittedCommand, DeliveryClass, ServerMessage};
use crate::dock::mating::DockConfig;
use crate::lobby::{server::OutboundMessage, Target};
use crate::ship::system_registry::DOCK_SYSTEM_ID;

fn config() -> DockConfig {
    DockConfig {
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

#[test]
fn restored_docked_state_keeps_engaged_only_start_and_host_withdrawal() {
    use crate::core::messages::{
        AdmittedCommands, AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus,
        ScoredObjective, ViewscreenBlackboard,
    };
    let mut app = App::new();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    app.add_systems(Update, operate_dock_ai);
    let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
    sources.0.set(
        SystemId(DOCK_SYSTEM_ID.into()),
        crate::ship::control_source::ControlSource::Ai,
    );
    let mut dock = control();
    dock.docked = true;
    dock.engaged = false;
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
    let operator = app
        .world_mut()
        .spawn((
            EntityUuid("operator".into()),
            sources,
            dock,
            blackboards,
            AdmittedCommands::default(),
        ))
        .id();
    app.update();
    assert!(app.world().entity(operator).contains::<DockAiEngaged>());
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
    app.update();
    assert!(!app.world().entity(operator).contains::<DockAiEngaged>());
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
    c.engaged = true;
    c.docked = true;
    c.docking_target = Some("berth-1".into());
    c.last_refusal = Some(DockRefusal::OutOfRange);
    let save = c.save_state();
    assert!(save.engaged);
    assert!(save.docked);
    assert_eq!(save.docking_target.as_deref(), Some("berth-1"));
}

#[test]
fn an_idle_control_saves_as_default() {
    assert_eq!(control().save_state(), DockSaveState::default());
}

#[test]
fn docked_partner_is_only_the_target_while_docked() {
    let mut c = control();
    c.docking_target = Some("berth-1".into());
    c.engaged = true;
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
    assert!(dock.engaged, "the authored Dock SystemId must be consumed");
    assert_eq!(dock.docking_target.as_deref(), Some("berth-1"));
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
        dock.engaged = engaged;
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
            Transform::from_xyz(100.0, 0.0, 0.0),
            own_marker(),
            DockControl {
                engaged: true,
                docked: false,
                docking_target: Some(TARGET.into()),
                ..control_with_id_config(DOCK_SYSTEM_ID, unpowered_config())
            },
            crate::core::messages::AdmittedCommands::default(),
        ))
        .id();
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
            .engaged,
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
