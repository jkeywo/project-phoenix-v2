use super::*;

fn correlated_umbilical_command(
    correlation: &str,
    payload: SystemControlPayload,
) -> crate::core::messages::AdmittedCommand {
    crate::core::messages::AdmittedCommand {
        target: umbilical_system_id(),
        payload,
        response_token: Some("engineering-holder".into()),
        feedback_correlation: Some(
            crate::core::messages::ActionCorrelationId::new(correlation)
                .expect("valid test correlation"),
        ),
    }
}

fn capacity(level: i64, ceiling: i64) -> InfrastructureCondition {
    let config = crate::infrastructure::InfrastructureConfig {
        capacities: vec![crate::infrastructure::CapacityConfig {
            id: "reserve_fuel".into(),
            amount: level,
            label: None,
            ceiling: Some(ceiling),
        }],
        ..Default::default()
    };
    InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
        &config,
    ))
}

fn umbilical() -> TransferUmbilical {
    TransferUmbilical::new(
        UmbilicalConfig {
            capacity: "reserve_fuel".into(),
            rate: 5.0,
            direction: UmbilicalDirection::Deliver,
            min_power_level: 2,
        },
        PowerGroupId("umbilical".into()),
    )
}

#[test]
fn umbilical_feedback_waits_for_the_authoritative_flow_verdict() {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .add_message::<PendingUmbilicalActionFeedback>()
        .add_message::<crate::lobby::OutboundMessage>()
        .add_systems(
            Update,
            (
                handle_umbilical_commands,
                tick_umbilical,
                finish_umbilical_action_feedback,
            )
                .chain(),
        );
    let mut dock = DockControl::new(
        crate::ship::system_registry::dock_system_id(),
        crate::dock::DockConfig {
            range: 100.0,
            engage_distance: 100.0,
            approach_speed: 1.0,
            mate_tolerance: 1.0,
            undock_clear_distance: 1.0,
            min_power_level: 1,
        },
        PowerGroupId("dock".into()),
    );
    dock.engaged = true;
    dock.docked = true;
    dock.docking_target = Some("partner-1".into());
    let power_config = crate::modifiers::power_system::PowerConfig::default();
    let power = ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_authored_groups(
            &power_config,
            &[
                crate::modifiers::power_system::AuthoredPowerGroup::at_default_floor(
                    PowerGroupId("umbilical".into()),
                    2,
                ),
            ],
        ),
    );
    let operator = app
        .world_mut()
        .spawn((
            crate::core::messages::AdmittedCommands(vec![correlated_umbilical_command(
                "umbilical-applied",
                SystemControlPayload::StartTransfer,
            )]),
            umbilical(),
            dock,
            power,
            EntityUuid("operator-1".into()),
            capacity(10, 10),
        ))
        .id();
    app.world_mut()
        .spawn((EntityUuid("partner-1".into()), capacity(0, 10)));
    let mut cursor = app
        .world()
        .resource::<Messages<crate::lobby::OutboundMessage>>()
        .get_cursor();
    let feedback_count = |messages: &[crate::lobby::OutboundMessage], correlation, expected| {
        messages
            .iter()
            .filter(|message| {
                matches!(
                    (&message.target, &message.msg),
                    (
                        crate::lobby::Target::Token(token),
                        crate::core::messages::ServerMessage::ActionFeedback {
                            correlation: actual,
                            outcome,
                        }
                    ) if token == "engineering-holder"
                        && actual.as_str() == correlation
                        && outcome == &expected
                )
            })
            .count()
    };

    app.update();
    let messages = app
        .world()
        .resource::<Messages<crate::lobby::OutboundMessage>>();
    let first: Vec<_> = cursor.read(messages).cloned().collect();
    assert_eq!(
        feedback_count(
            &first,
            "umbilical-applied",
            crate::core::messages::ActionFeedbackOutcome::Applied,
        ),
        1
    );
    assert!(app
        .world()
        .get::<TransferUmbilical>(operator)
        .is_some_and(|umbilical| umbilical.running));

    app.world_mut()
        .entity_mut(operator)
        .insert(crate::core::messages::AdmittedCommands(vec![
            correlated_umbilical_command("umbilical-stop", SystemControlPayload::StopTransfer),
        ]));
    app.update();
    let messages = app
        .world()
        .resource::<Messages<crate::lobby::OutboundMessage>>();
    let second: Vec<_> = cursor.read(messages).cloned().collect();
    assert_eq!(
        feedback_count(
            &second,
            "umbilical-stop",
            crate::core::messages::ActionFeedbackOutcome::Applied,
        ),
        1
    );

    app.world_mut()
        .get_mut::<DockControl>(operator)
        .expect("operator dock")
        .docked = false;
    app.world_mut()
        .entity_mut(operator)
        .insert(crate::core::messages::AdmittedCommands(vec![
            correlated_umbilical_command("umbilical-refused", SystemControlPayload::StartTransfer),
        ]));
    app.update();
    let messages = app
        .world()
        .resource::<Messages<crate::lobby::OutboundMessage>>();
    let third: Vec<_> = cursor.read(messages).cloned().collect();
    assert_eq!(
        feedback_count(
            &third,
            "umbilical-refused",
            crate::core::messages::ActionFeedbackOutcome::Refused,
        ),
        1
    );
}

#[test]
fn save_state_carries_the_running_intent_only() {
    let mut u = umbilical();
    u.running = true;
    u.carry = 0.4;
    u.last_refusal = Some(UmbilicalRefusal::Undocked);
    u.operator_level = Some(50);
    let save = u.save_state();
    assert!(save.running);
}

#[test]
fn an_idle_umbilical_saves_as_default() {
    assert_eq!(umbilical().save_state(), UmbilicalSaveState::default());
}

#[test]
fn restore_reseeds_running_and_clears_the_projections() {
    let mut u = umbilical();
    u.carry = 0.7;
    u.last_refusal = Some(UmbilicalRefusal::Disabled);
    u.operator_level = Some(12);
    u.restore(&UmbilicalSaveState { running: true });
    assert!(u.running);
    assert_eq!(u.carry, 0.0);
    assert_eq!(u.last_refusal, None);
    assert_eq!(u.operator_level, None);
    assert_eq!(u.activation_target, None);
}

// ── The task lifecycle (issue #1345) ─────────────────────────────────────

use crate::dock::mating::DockConfig;
use crate::infrastructure::condition::{CapacityConfig, InfrastructureConfig, InfrastructureState};

const OPERATOR: &str = "op-1";
const PARTNER: &str = "partner-1";

fn dock_config() -> DockConfig {
    DockConfig {
        range: 200.0,
        engage_distance: 400.0,
        approach_speed: 60.0,
        mate_tolerance: 4.0,
        undock_clear_distance: 120.0,
        min_power_level: 1,
    }
}

/// A docked control, mated to `PARTNER` (or idle when `docked_to` is
/// `None`) — the umbilical's own docking terms don't matter to these
/// tests, only `docked_partner()`'s answer.
fn dock(docked_to: Option<&str>) -> DockControl {
    let mut d = DockControl::new(
        SystemId(DOCK_SYSTEM_ID_FOR_TEST.into()),
        dock_config(),
        PowerGroupId("dock".into()),
    );
    if let Some(target) = docked_to {
        d.docked = true;
        d.docking_target = Some(target.into());
    }
    d
}

const DOCK_SYSTEM_ID_FOR_TEST: &str = "dock";

fn capacity_config(amount: i64, ceiling: i64) -> InfrastructureConfig {
    InfrastructureConfig {
        capacities: vec![CapacityConfig {
            id: "fuel".into(),
            amount,
            ceiling: Some(ceiling),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn flow_config() -> UmbilicalConfig {
    UmbilicalConfig {
        capacity: "fuel".into(),
        rate: 10.0,
        direction: UmbilicalDirection::Deliver,
        min_power_level: 0,
    }
}

/// A docked pair — the operator delivering `fuel` to its partner — plus the
/// lifecycle queue, ready to run `handle_umbilical_commands` and `tick_
/// umbilical` chained.
fn docked_world() -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs(1));
    app.insert_resource(time);
    app.add_systems(Update, (handle_umbilical_commands, tick_umbilical).chain());

    let operator = app
        .world_mut()
        .spawn((
            EntityUuid(OPERATOR.into()),
            umbilical_with(flow_config()),
            dock(Some(PARTNER)),
            InfrastructureCondition(InfrastructureState::from_config(&capacity_config(100, 200))),
            crate::core::messages::AdmittedCommands::default(),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid(PARTNER.into()),
        InfrastructureCondition(InfrastructureState::from_config(&capacity_config(0, 500))),
    ));
    (app, operator)
}

fn umbilical_with(config: UmbilicalConfig) -> TransferUmbilical {
    TransferUmbilical::new(config, PowerGroupId("umbilical".into()))
}

fn admit_start(app: &mut App, operator: Entity) {
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<crate::core::messages::AdmittedCommands>()
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: SystemId(UMBILICAL_SYSTEM_ID.into()),
            payload: SystemControlPayload::StartTransfer,
            response_token: None,
            feedback_correlation: None,
        });
}

fn drain_lifecycle(app: &mut App) -> Vec<TaskLifecycleRequest> {
    std::mem::take(
        &mut app
            .world_mut()
            .resource_mut::<EffectQueue<TaskLifecycleRequest>>()
            .0,
    )
}

fn flow_slot_for_test() -> TaskSlot {
    TaskSlot::new(OPERATOR, UMBILICAL_SYSTEM_ID, TASK_VERB_UMBILICAL_FLOW)
}

/// A start that actually moves capacity opens exactly one activation; an
/// unchanged, still-flowing tick afterwards reports nothing more.
#[test]
fn a_flow_that_moves_capacity_opens_one_activation_and_then_falls_silent() {
    let (mut app, operator) = docked_world();
    admit_start(&mut app, operator);
    app.update();

    assert!(
        app.world()
            .entity(operator)
            .get::<TransferUmbilical>()
            .unwrap()
            .running
    );
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::Start {
            slot: flow_slot_for_test(),
            target: Some(PARTNER.into()),
        }]
    );

    app.update();
    assert!(
        drain_lifecycle(&mut app).is_empty(),
        "a flow that simply continues reports nothing more"
    );
}

/// `StopTransfer` on a live flow reports the cancel before clearing intent;
/// `tick_umbilical` — seeing the intent already cleared — reports nothing
/// further, so the activation gets exactly one terminal.
#[test]
fn stop_transfer_reports_released_exactly_once() {
    let (mut app, operator) = docked_world();
    admit_start(&mut app, operator);
    app.update();
    drain_lifecycle(&mut app);

    app.world_mut()
        .entity_mut(operator)
        .get_mut::<crate::core::messages::AdmittedCommands>()
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: SystemId(UMBILICAL_SYSTEM_ID.into()),
            payload: SystemControlPayload::StopTransfer,
            response_token: None,
            feedback_correlation: None,
        });
    app.update();

    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::End {
            slot: flow_slot_for_test(),
            reason: TaskTerminalReason::Released,
        }]
    );
    assert!(
        !app.world()
            .entity(operator)
            .get::<TransferUmbilical>()
            .unwrap()
            .running
    );
}

/// A start refused before it ever flows — nothing is docked to bridge to —
/// still gets a whole activation, opened and closed together, mirroring the
/// tractor's "refused before it couples" case.
#[test]
fn a_start_refused_before_it_ever_flows_still_opens_and_closes_one_activation() {
    let mut app = App::new();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs(1));
    app.insert_resource(time);
    app.add_systems(Update, (handle_umbilical_commands, tick_umbilical).chain());
    let operator = app
        .world_mut()
        .spawn((
            EntityUuid(OPERATOR.into()),
            umbilical_with(flow_config()),
            dock(None),
            crate::core::messages::AdmittedCommands::default(),
        ))
        .id();
    admit_start(&mut app, operator);
    app.update();

    let slot = flow_slot_for_test();
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![
            TaskLifecycleRequest::Start {
                slot: slot.clone(),
                target: None,
            },
            TaskLifecycleRequest::End {
                slot,
                reason: TaskTerminalReason::TargetLost,
            },
        ]
    );
}

/// A flow that WAS moving capacity and then loses its dock ends the
/// standing activation with the mapped reason — a close only, no re-opened
/// start.
#[test]
fn a_flow_that_loses_its_dock_ends_the_standing_activation() {
    let (mut app, operator) = docked_world();
    admit_start(&mut app, operator);
    app.update();
    drain_lifecycle(&mut app);

    app.world_mut()
        .entity_mut(operator)
        .get_mut::<DockControl>()
        .unwrap()
        .docked = false;
    app.update();

    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::End {
            slot: flow_slot_for_test(),
            reason: TaskTerminalReason::TargetLost,
        }],
        "the standing activation closes; nothing new opens for a flow that was already live"
    );
}
