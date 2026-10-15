use super::*;

fn transfer_power() -> ShipPowerSystem {
    let base = crate::modifiers::power_system::PowerSystem::default();
    let mut hardware = base.config().clone();
    let mut ship = base.ship_config().clone();
    let previous = hardware.consumers[0].system.clone();
    hardware.consumers[0].system = umbilical_system_id();
    let system = ship
        .systems
        .iter_mut()
        .find(|system| system.id == previous)
        .unwrap();
    system.id = umbilical_system_id();
    system.kind = crate::ship::system_registry::UMBILICAL_KIND.into();
    ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(hardware, ship).unwrap(),
    )
}

#[test]
fn partial_energy_near_source_limit_pays_only_the_resource_actually_moved() {
    let (mut app, operator) = docked_world();
    app.init_resource::<EffectQueue<crate::resources::server::ResourceTransfer>>();
    app.world_mut().entity_mut(operator).insert((
        InfrastructureCondition(InfrastructureState::from_config(&capacity_config(2, 200))),
        transfer_power(),
    ));
    let mut power = app
        .world_mut()
        .get_mut::<ShipPowerSystem>(operator)
        .unwrap();
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&umbilical_system_id())
        .unwrap()
        .energy = 0.05;
    drop(power);
    let mut flow = app
        .world_mut()
        .get_mut::<TransferUmbilical>(operator)
        .unwrap();
    flow.config.rate = 20.0;
    flow.running = true;
    drop(flow);
    app.update();
    let transfers = &app
        .world()
        .resource::<EffectQueue<crate::resources::server::ResourceTransfer>>()
        .0;
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].amount, 1);
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(operator)
            .unwrap()
            .0
            .network()
            .consumers[&umbilical_system_id()]
            .energy,
        0.0
    );
    assert!(
        app.world()
            .get::<TransferUmbilical>(operator)
            .unwrap()
            .running
    );
}

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
            allowed_resources: vec![],
            link: Default::default(),
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
            moves_body: true,
            range: 100.0,
            engage_distance: 100.0,
            approach_speed: 1.0,
            mate_tolerance: 1.0,
            undock_clear_distance: 1.0,
            min_power_level: 1,
        },
        PowerGroupId("dock".into()),
    );
    dock.set_engaged(true);
    dock.docked = true;
    dock.set_docking_target(Some("partner-1".into()));
    let power = transfer_power();
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
fn save_state_carries_running_intent_and_fractional_progress() {
    let mut u = umbilical();
    u.running = true;
    u.carry = 0.4;
    u.last_refusal = Some(UmbilicalRefusal::Undocked);
    u.operator_level = Some(50);
    let save = u.save_state();
    assert!(save.running);
    assert_eq!(save.carry, 0.4);
}

#[test]
fn an_idle_umbilical_saves_as_default() {
    let saved = umbilical().save_state();
    assert!(!saved.running);
    assert_eq!(saved.carry, 0.0);
    assert_eq!(saved.selected_resource, "reserve_fuel");
}

#[test]
fn selected_resource_is_authorised_and_saved_without_changing_transfer_intent() {
    let mut app = App::new();
    app.add_systems(Update, handle_umbilical_commands);
    let mut flow = umbilical();
    flow.config.allowed_resources = vec!["coolant".into()];
    flow.running = true;
    flow.carry = 0.75;
    let entity = app
        .world_mut()
        .spawn((
            flow,
            crate::core::messages::AdmittedCommands(vec![correlated_umbilical_command(
                "choose-coolant",
                SystemControlPayload::SetTransferResource {
                    resource: "coolant".into(),
                },
            )]),
        ))
        .id();
    app.update();
    let flow = app.world().get::<TransferUmbilical>(entity).unwrap();
    assert_eq!(flow.selected_resource, "coolant");
    assert!(flow.running);
    assert_eq!(flow.carry, 0.0);
    let saved = flow.save_state();
    let mut restored = umbilical();
    restored.config.allowed_resources = vec!["coolant".into()];
    restored.restore(&saved);
    assert_eq!(restored.selected_resource, "coolant");
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(entity)
        .unwrap()
        .0 = vec![correlated_umbilical_command(
        "refuse-other",
        SystemControlPayload::SetTransferResource {
            resource: "antimatter".into(),
        },
    )];
    app.update();
    assert_eq!(
        app.world()
            .get::<TransferUmbilical>(entity)
            .unwrap()
            .save_state(),
        saved
    );
}

#[test]
fn restore_reseeds_running_and_clears_the_projections() {
    let mut u = umbilical();
    u.carry = 0.7;
    u.last_refusal = Some(UmbilicalRefusal::Disabled);
    u.operator_level = Some(12);
    u.restore(&UmbilicalSaveState {
        selected_direction: UmbilicalDirection::Deliver,
        selected_resource: "reserve_fuel".into(),
        running: true,
        carry: 0.4,
        ..Default::default()
    });
    assert!(u.running);
    assert_eq!(u.carry, 0.4);
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
        moves_body: true,
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
        d.set_docking_target(Some(target.into()));
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
        allowed_resources: vec![],
        link: Default::default(),
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
            transfer_power(),
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

#[test]
fn selecting_collection_moves_finite_port_parts_and_survives_continuation() {
    use crate::resources::server::{
        tick_resource_transfers, ResourceTransfer, ResourceTransferReceipt,
    };
    let (mut app, operator) = docked_world();
    app.init_resource::<EffectQueue<ResourceTransfer>>()
        .init_resource::<EffectQueue<ResourceTransferReceipt>>()
        .init_resource::<EffectQueue<crate::infrastructure::CapacityAdjustment>>()
        .init_resource::<EffectQueue<crate::infrastructure::ConditionAdjustment>>()
        .init_resource::<crate::world::server::WorldContentRuntime>()
        .add_systems(
            Update,
            (
                crate::infrastructure::tick_infrastructure_condition,
                tick_resource_transfers,
            )
                .chain()
                .after(tick_umbilical),
        );
    let partner = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == PARTNER)
        .unwrap()
        .0;
    let stock = |amount| {
        InfrastructureCondition(InfrastructureState::from_config(&InfrastructureConfig {
            capacities: vec![CapacityConfig {
                id: "machinery_parts".into(),
                amount,
                ceiling: Some(10),
                ..Default::default()
            }],
            ..Default::default()
        }))
    };
    app.world_mut().entity_mut(operator).insert(stock(0));
    app.world_mut().entity_mut(partner).insert(stock(3));
    {
        let mut flow = app
            .world_mut()
            .get_mut::<TransferUmbilical>(operator)
            .unwrap();
        flow.config.capacity = "machinery_parts".into();
        flow.selected_resource = "machinery_parts".into();
        flow.config.rate = 2.0;
    }
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(operator)
        .unwrap()
        .0 = vec![
        correlated_umbilical_command(
            "collect-parts",
            SystemControlPayload::SetTransferDirection {
                direction: UmbilicalDirection::Collect,
            },
        ),
        correlated_umbilical_command("collect-start", SystemControlPayload::StartTransfer),
    ];
    app.update();
    let amount = |app: &App, entity| {
        app.world()
            .get::<InfrastructureCondition>(entity)
            .unwrap()
            .0
            .capacity_stock("machinery_parts")
            .unwrap()
            .level
    };
    assert_eq!((amount(&app, operator), amount(&app, partner)), (2, 1));
    let saved = app
        .world()
        .get::<TransferUmbilical>(operator)
        .unwrap()
        .save_state();
    assert_eq!(saved.selected_direction, UmbilicalDirection::Collect);
    let encoded = serde_json::to_string(&saved).unwrap();
    let decoded = serde_json::from_str::<UmbilicalSaveState>(&encoded).unwrap();
    app.world_mut()
        .get_mut::<TransferUmbilical>(operator)
        .unwrap()
        .restore(&decoded);
    // A direction change cannot reverse already running work.
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(operator)
        .unwrap()
        .0 = vec![correlated_umbilical_command(
        "cannot-reverse",
        SystemControlPayload::SetTransferDirection {
            direction: UmbilicalDirection::Deliver,
        },
    )];
    app.update();
    assert_eq!((amount(&app, operator), amount(&app, partner)), (3, 0));
    assert_eq!(
        app.world()
            .get::<TransferUmbilical>(operator)
            .unwrap()
            .selected_direction,
        UmbilicalDirection::Collect
    );
    app.update();
    assert_eq!(
        (amount(&app, operator), amount(&app, partner)),
        (3, 0),
        "collection cannot create parts after the port empties"
    );
}

#[test]
fn damaged_umbilical_reduces_actual_flow_after_paying_the_same_work() {
    use phoenix_model::damage::{ComponentCondition, DamageBinding};
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut damage = crate::ship::compartment_damage::ShipDamage::from_config(&authored)
        .unwrap()
        .expect("physical Cruiser");
    let id = umbilical_system_id();
    let name = damage.definition.components[&id]
        .iter()
        .find(|(_, row)| row.binding == DamageBinding::Operation)
        .unwrap()
        .0
        .clone();
    let full = damage.clone();
    let state = damage
        .state
        .components
        .get_mut(&id)
        .unwrap()
        .get_mut(&name)
        .unwrap();
    state.condition = ComponentCondition::Damaged;
    state.last_source = Some("test-hit".into());
    let factor = damage.factor(&id, DamageBinding::Operation);
    assert!(factor > 0.0 && factor < 1.0);
    let run = |damage| {
        let (mut app, entity) = docked_world();
        app.init_resource::<EffectQueue<crate::resources::server::ResourceTransfer>>();
        app.world_mut().entity_mut(entity).insert(damage);
        admit_start(&mut app, entity);
        app.update();
        let moved = app
            .world()
            .resource::<EffectQueue<crate::resources::server::ResourceTransfer>>()
            .0[0]
            .amount;
        let remaining = app
            .world()
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .network()
            .consumers[&id]
            .energy;
        (moved, remaining)
    };
    let (normal, normal_energy) = run(full);
    let (damaged, damaged_energy) = run(damage);
    assert_eq!(damaged, (normal as f32 * factor).floor() as i64);
    assert_eq!(
        normal_energy, damaged_energy,
        "fault loss changes output, not funded work cost"
    );
}

/// Controller/admission/applier owner fixture, not a physical host journey.
fn port_ai_owner(explicit: bool, running: bool, maintaining: bool) -> (App, Entity) {
    use crate::core::messages::{
        AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
        ViewscreenBlackboard,
    };
    let mut app = App::new();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.add_systems(
        Update,
        (operate_umbilical_ai, handle_umbilical_commands).chain(),
    );
    let hull = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let scope = crate::ship_plugin::ShipConfigComponent(hull.ship_config.unwrap());
    let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
    sources.0.set(
        umbilical_system_id(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let mut flow = umbilical();
    flow.running = running;
    flow.port_maintenance = maintaining;
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            scored_objectives: if explicit {
                vec![ScoredObjective {
                    id: "transfer".into(),
                    score: 1.0,
                    directive: AiDirective::Transfer {
                        target: PARTNER.into(),
                    },
                    source: ObjectiveSource::Mission,
                    relevance: vec![SystemAffinity::Engineering],
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
                }]
            } else {
                vec![]
            },
            ..Default::default()
        }),
    );
    boards.0.insert(
        umbilical_system_id(),
        SystemBlackboard::Umbilical(UmbilicalBlackboard {
            port: Some(phoenix_model::ports_ui::PortServiceView {
                partner_uuid: PARTNER.into(),
                visit: 1,
                policy: phoenix_model::ports::PortServicePolicy::FreeAlliedReplenishment,
                eligible: true,
                offers: vec![],
                order: None,
                stores: vec![phoenix_model::logistics_ui::LogisticsStoreView {
                    id: "hold".into(),
                    resource: "reserve_fuel".into(),
                    role: "main".into(),
                    system: None,
                    level: 0,
                    available: 0,
                    held: 0,
                    ceiling: 10,
                    headroom: 10,
                    bay_free_space: 10,
                    incoming: 0,
                    target: 10,
                    reserve: 0,
                    priority: 0,
                    compartment: "support".into(),
                }],
            }),
            ..Default::default()
        }),
    );
    let operator = app
        .world_mut()
        .spawn((
            EntityUuid(OPERATOR.into()),
            scope,
            sources,
            flow,
            dock(Some(PARTNER)),
            boards,
            crate::core::messages::AdmittedCommands::default(),
        ))
        .id();
    (app, operator)
}

#[test]
fn ordinary_umbilical_ai_applies_explicit_transfer_before_opportunistic_collection() {
    let (mut app, operator) = port_ai_owner(true, false, true);
    app.update();
    let admitted = app
        .world()
        .get::<crate::core::messages::AdmittedCommands>(operator)
        .unwrap();
    let payloads: Vec<_> = admitted
        .for_target(UMBILICAL_SYSTEM_ID)
        .map(|command| &command.payload)
        .collect();
    assert_eq!(
        payloads,
        vec![
            &SystemControlPayload::SetPortMaintenance { enabled: false },
            &SystemControlPayload::StartTransfer
        ]
    );
    let flow = app.world().get::<TransferUmbilical>(operator).unwrap();
    assert!(!flow.port_maintenance);
    assert!(flow.running);
    assert_eq!(flow.selected_direction, UmbilicalDirection::Deliver);
    assert_eq!(flow.selected_resource, "reserve_fuel");
    assert!(app.world().get::<UmbilicalAiRunning>(operator).is_some());
}

#[test]
fn ordinary_idle_umbilical_ai_retains_default_deliver_resupply_but_preserves_running_human_flow() {
    let (mut idle, operator) = port_ai_owner(false, false, false);
    idle.update();
    let flow = idle.world().get::<TransferUmbilical>(operator).unwrap();
    assert!(
        flow.port_maintenance,
        "idle default-Deliver equipment still collects Main targets"
    );
    assert_eq!(flow.selected_direction, UmbilicalDirection::Deliver);
    let (mut active, operator) = port_ai_owner(false, true, false);
    active.update();
    let flow = active.world().get::<TransferUmbilical>(operator).unwrap();
    assert!(!flow.port_maintenance);
    assert!(
        flow.running,
        "delegation does not stop a flow the AI did not claim"
    );
    assert!(active
        .world()
        .get::<crate::core::messages::AdmittedCommands>(operator)
        .unwrap()
        .0
        .is_empty());
    assert!(active.world().get::<UmbilicalAiRunning>(operator).is_none());
}
