use super::*;
use crate::core::messages::{
    ActionCorrelationId, AdmittedCommand, ClientMessage, DeliveryClass, ServerMessage, SystemId,
};
use crate::lobby::{server::OutboundMessage, Target};
use crate::ship::control_source::ControlSource;
use crate::ship::test_support::*;

fn correlated_command(
    target: &str,
    payload: SystemControlPayload,
    correlation: &str,
) -> AdmittedCommand {
    AdmittedCommand {
        target: SystemId(target.to_string()),
        payload,
        response_token: Some("sensors".to_string()),
        feedback_correlation: Some(
            ActionCorrelationId::new(correlation).expect("valid test correlation"),
        ),
    }
}

fn cancel_command(correlation: &str) -> AdmittedCommand {
    correlated_command(
        crate::ship::system_registry::HELM_IMPULSE_SYSTEM_ID,
        SystemControlPayload::CancelImpulse,
        correlation,
    )
}

fn has_feedback(
    messages: &[OutboundMessage],
    correlation: &str,
    outcome: ActionFeedbackOutcome,
) -> bool {
    messages.iter().any(|message| {
        message.target == Target::Token("sensors".to_string())
            && message.delivery == DeliveryClass::Reliable
            && matches!(
                &message.msg,
                ServerMessage::ActionFeedback {
                    correlation: actual,
                    outcome: actual_outcome,
                } if actual.as_str() == correlation && *actual_outcome == outcome
            )
    })
}

fn feedback_count(messages: &[OutboundMessage], correlation: &str) -> usize {
    messages
        .iter()
        .filter(|message| {
            matches!(
                &message.msg,
                ServerMessage::ActionFeedback {
                    correlation: actual,
                    ..
                } if actual.as_str() == correlation
            )
        })
        .count()
}

fn cancel_feedback_app(
    impulse: Option<crate::ship::helm::ImpulseCommand>,
    correlation: &str,
) -> (App, Entity) {
    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .add_systems(Update, process_helm_inputs);
    let entity = app
        .world_mut()
        .spawn(AdmittedCommands(vec![cancel_command(correlation)]))
        .id();
    if let Some(impulse) = impulse {
        app.world_mut().entity_mut(entity).insert(impulse);
    }
    (app, entity)
}

#[test]
fn cancel_impulse_reports_applied_only_after_the_owner_cancels_it() {
    let (mut app, entity) = cancel_feedback_app(
        Some(crate::ship::helm::ImpulseCommand(
            crate::ship::impulse::ImpulsePhase::Charging,
        )),
        "cancel-applied",
    );
    let mut cursor = app
        .world()
        .resource::<Messages<OutboundMessage>>()
        .get_cursor();

    app.update();

    assert_eq!(
        app.world()
            .get::<crate::ship::helm::ImpulseCommand>(entity)
            .expect("the impulse owner remains present")
            .0,
        crate::ship::impulse::ImpulsePhase::Idle,
    );
    let feedback: Vec<_> = cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .cloned()
        .collect();
    assert!(has_feedback(
        &feedback,
        "cancel-applied",
        ActionFeedbackOutcome::Applied,
    ));
}

#[test]
fn cancel_impulse_reports_refused_when_the_owner_component_is_absent() {
    let (mut app, _) = cancel_feedback_app(None, "cancel-refused");
    let mut cursor = app
        .world()
        .resource::<Messages<OutboundMessage>>()
        .get_cursor();

    app.update();

    let feedback: Vec<_> = cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .cloned()
        .collect();
    assert!(has_feedback(
        &feedback,
        "cancel-refused",
        ActionFeedbackOutcome::Refused,
    ));
}

#[test]
fn start_impulse_reports_one_terminal_outcome_for_present_and_missing_owners() {
    for (correlation, impulse, expected) in [
        (
            "start-applied",
            Some(crate::ship::helm::ImpulseCommand::default()),
            ActionFeedbackOutcome::Applied,
        ),
        ("start-refused", None, ActionFeedbackOutcome::Refused),
    ] {
        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .add_systems(Update, process_helm_inputs);
        let command = correlated_command(
            crate::ship::system_registry::HELM_IMPULSE_SYSTEM_ID,
            SystemControlPayload::StartImpulseCharge,
            correlation,
        );
        let entity = app.world_mut().spawn(AdmittedCommands(vec![command])).id();
        if let Some(impulse) = impulse {
            app.world_mut().entity_mut(entity).insert(impulse);
        }
        let mut cursor = app
            .world()
            .resource::<Messages<OutboundMessage>>()
            .get_cursor();

        app.update();

        let feedback: Vec<_> = cursor
            .read(app.world().resource::<Messages<OutboundMessage>>())
            .cloned()
            .collect();
        assert!(has_feedback(&feedback, correlation, expected));
        assert_eq!(feedback_count(&feedback, correlation), 1);
    }
}

#[test]
fn set_boost_reports_one_terminal_outcome_for_enabled_and_missing_owners() {
    for (correlation, with_owner, expected) in [
        ("boost-applied", true, ActionFeedbackOutcome::Applied),
        ("boost-refused", false, ActionFeedbackOutcome::Refused),
    ] {
        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .add_systems(Update, process_helm_inputs);
        let command = correlated_command(
            crate::ship::system_registry::HELM_BOOST_SYSTEM_ID,
            SystemControlPayload::SetBoost { active: true },
            correlation,
        );
        let entity = app.world_mut().spawn(AdmittedCommands(vec![command])).id();
        if with_owner {
            app.world_mut().entity_mut(entity).insert((
                crate::ship::components::BoostConfigResource {
                    enabled: true,
                    ..Default::default()
                },
                ShipBoost::default(),
                BoostCommand::default(),
            ));
        }
        let mut cursor = app
            .world()
            .resource::<Messages<OutboundMessage>>()
            .get_cursor();

        app.update();

        let feedback: Vec<_> = cursor
            .read(app.world().resource::<Messages<OutboundMessage>>())
            .cloned()
            .collect();
        assert!(has_feedback(&feedback, correlation, expected));
        assert_eq!(feedback_count(&feedback, correlation), 1);
        if with_owner {
            assert!(app.world().get::<BoostCommand>(entity).unwrap().0);
        }
    }
}

#[test]
fn impulse_charge_clears_actuator_latches_on_local_and_remote_ships() {
    for local in [false, true] {
        for later_steering in [None, Some(0.25)] {
            let mut app = App::new();
            app.add_systems(Update, process_helm_inputs);
            let mut commands = vec![correlated_command(
                crate::ship::system_registry::HELM_IMPULSE_SYSTEM_ID,
                SystemControlPayload::StartImpulseCharge,
                "charge",
            )];
            if let Some(value) = later_steering {
                commands.push(correlated_command(
                    crate::ship::system_registry::HELM_STEERING_SYSTEM_ID,
                    SystemControlPayload::SetSteering { value },
                    "steer",
                ));
            }
            let entity = app
                .world_mut()
                .spawn((
                    AdmittedCommands(commands),
                    ThrustInput(0.8),
                    SteeringInput(-0.6),
                    ImpulseCommand::default(),
                ))
                .id();
            if local {
                app.world_mut().entity_mut(entity).insert(LocalShip);
            }
            app.update();
            assert_eq!(
                app.world().get::<ThrustInput>(entity).unwrap().0,
                0.0,
                "charging clears thrust regardless of ownership (local={local})"
            );
            assert_eq!(
                app.world().get::<SteeringInput>(entity).unwrap().0,
                later_steering.unwrap_or(0.0),
                "charging clears steering; a later command still wins (local={local})"
            );
        }
    }
}

#[test]
fn control_system_helm_input_updates_last_input_and_moves_ship() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);

    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_thrust_system_id(),
            payload: SystemControlPayload::SetThrust { value: 1.0 },
        },
    );
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_steering_system_id(),
            payload: SystemControlPayload::SetSteering { value: 0.25 },
        },
    );
    tick_twice(&mut app);

    assert_eq!(
        get_last_helm_input(&mut app),
        LastHelmInput {
            thrust: 1.0,
            steering: 0.25,
            lateral: 0.0,
        }
    );
    assert!(get_ship_physics(&mut app).forward_speed > 0.0);
}

#[test]
fn ai_helm_operates_without_human_holder() {
    let mut app = test_app();
    set_helm_control_source(&mut app, ControlSource::Ai);

    tick_twice(&mut app);

    assert_eq!(
        get_last_helm_input(&mut app),
        LastHelmInput {
            thrust: 0.0,
            steering: 0.0,
            lateral: 0.0,
        }
    );
    assert_eq!(get_ship_physics(&mut app).forward_speed, 0.0);
}

#[test]
fn ai_helm_ignores_human_input() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);
    set_helm_control_source(&mut app, ControlSource::Ai);

    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_thrust_system_id(),
            payload: SystemControlPayload::SetThrust { value: -1.0 },
        },
    );
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_steering_system_id(),
            payload: SystemControlPayload::SetSteering { value: 1.0 },
        },
    );
    tick_twice(&mut app);

    // Human input must be ignored when policy is AI; no BehaviourSection
    // on the player ship, so LastHelmInput stays at default.
    assert_eq!(get_last_helm_input(&mut app), LastHelmInput::default());
}

/// The #701 mismatch, fixed by #801: with `helm-thrust = Ai` and
/// `helm-steering = Human`, the human's combined joystick input used to be
/// admitted or refused on the COARSE helm policy, so the whole input got
/// in and the AI's thrust write had to win by ordering. Per-axis wire
/// targets make admission itself per-axis: the human's `SetSteering` is
/// admitted, the human's `SetThrust` is refused at the gate.
#[test]
fn per_axis_admission_fixes_the_coarse_vs_per_axis_mismatch() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);
    // AI holds the throttle; the human keeps the stick.
    set_fine_control_source(
        &mut app,
        crate::ship::system_registry::helm_thrust_system_id(),
        ControlSource::Ai,
    );

    // The human joystick fans out into the two per-axis messages.
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_thrust_system_id(),
            payload: SystemControlPayload::SetThrust { value: 1.0 },
        },
    );
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_steering_system_id(),
            payload: SystemControlPayload::SetSteering { value: 0.25 },
        },
    );
    tick_twice(&mut app);

    let last = get_last_helm_input(&mut app);
    assert_eq!(
        last.steering, 0.25,
        "the human-held steering axis must admit the human's SetSteering"
    );
    assert_eq!(
        last.thrust, 0.0,
        "the AI-held thrust axis must refuse the human's SetThrust at admission"
    );
}

#[test]
fn human_helm_suppresses_ai_operate() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);

    tick(&mut app);

    assert_eq!(get_last_helm_input(&mut app), LastHelmInput::default());
    assert_eq!(get_ship_physics(&mut app).forward_speed, 0.0);
}

/// When helm is under AI control (`operate_ai = true`), `process_helm_inputs`
/// must NOT admit stale human input over the AI's decision.
///
/// Post-#695 `process_helm_inputs` no longer integrates physics at all —
/// `integrate_ship_physics` is the sole helm-path writer. What this test
/// pins is the *admission* skip: with helm AI-controlled, a stale non-zero
/// `LastHelmInput` must not reach the intent components and therefore must
/// not move the ship. (Before #695 this same setup guarded against a second
/// `compute_physics` call at a different dt, which made the player ship
/// move ~3× faster than AI-driven NPCs.)
#[test]
fn ai_controlled_helm_does_not_admit_stale_human_input() {
    let mut app = test_app();
    set_helm_control_source(&mut app, ControlSource::Ai);

    // Set a non-zero last input so that if process_helm_inputs incorrectly
    // runs compute_physics it will produce a non-trivial displacement.
    set_last_helm_input(
        &mut app,
        LastHelmInput {
            thrust: 1.0,
            steering: 0.0,
            lateral: 0.0,
        },
    );

    // Snapshot physics before the tick.
    let before = get_ship_physics(&mut app);

    tick(&mut app);

    let after = get_ship_physics(&mut app);

    // operate_helm_ai has no objectives in this test (blackboard empty), so
    // it zeros the intent components. If process_helm_inputs admitted the
    // stale thrust=1.0 anyway, integrate_ship_physics would have moved the
    // ship.
    assert_eq!(
        after.x, before.x,
        "ShipPhysics.x must not advance when helm is AI-controlled: \
             process_helm_inputs must skip admission"
    );
    assert_eq!(
        after.forward_speed, before.forward_speed,
        "forward_speed must not change when process_helm_inputs skips admission"
    );
}

// ── Ship-aware admission symmetry (issue #824) ─────────────────────────

/// Spawn a minimal NPC ship the admission gate can route to: its own
/// `AdmittedCommands`, control sources with `helm-thrust` on `source`,
/// a `ShipConfigComponent`, and a `ThrustInput` intent for
/// `process_helm_inputs` to land on. Registers `ai:<uuid>` in the
/// `AiTokenRegistry` and returns `(entity, token)`.
fn spawn_admission_npc(app: &mut App, source: ControlSource) -> (Entity, String) {
    spawn_admission_npc_with_thrust_id(
        app,
        source,
        crate::ship::system_registry::HELM_THRUST_SYSTEM_ID,
    )
}

fn spawn_admission_npc_with_thrust_id(
    app: &mut App,
    source: ControlSource,
    thrust_id: &str,
) -> (Entity, String) {
    let mut config = crate::ship::components::ShipConfigComponent::default();
    config
        .0
        .systems
        .iter_mut()
        .find(|system| system.kind == crate::ship::system_registry::HELM_THRUST_KIND)
        .expect("the test hull has a thrust owner")
        .id = crate::core::messages::SystemId(thrust_id.into());
    let mut sources = ShipSystemControlSources::default();
    sources
        .0
        .set(crate::core::messages::SystemId(thrust_id.into()), source);
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            config,
            sources,
            crate::core::messages::AdmittedCommands::default(),
            ThrustInput::default(),
        ))
        .id();
    let uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut()
        .resource_mut::<crate::ai::server::AiTokenRegistry>()
        .register_with_entity(&uuid, npc);
    (npc, format!("ai:{uuid}"))
}

fn thrust_input_of(app: &App, entity: Entity) -> f32 {
    app.world().entity(entity).get::<ThrustInput>().unwrap().0
}

/// AC (issue #824): a registered `ai:` token's `ControlSystem` resolves
/// through `AiTokenRegistry` to the owning NPC entity and is admitted
/// into THAT entity's `AdmittedCommands` — and the admitted command is
/// applied to the NPC's own intent components, not the LocalShip's.
#[test]
fn ai_token_routes_to_owning_npc_entity_and_applies_to_its_intents() {
    let mut app = test_app();
    let (npc, token) = spawn_admission_npc(&mut app, ControlSource::Ai);

    push(
        &mut app,
        &token,
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_thrust_system_id(),
            payload: SystemControlPayload::SetThrust { value: 0.7 },
        },
    );
    tick(&mut app);

    assert_eq!(
        thrust_input_of(&app, npc),
        0.7,
        "the NPC's admitted AI command must apply to the NPC's own ThrustInput"
    );
    let local = find_ship_entity(&mut app);
    assert_eq!(
        thrust_input_of(&app, local),
        0.0,
        "the LocalShip's ThrustInput must be untouched by an NPC-routed command"
    );
}

#[test]
fn arbitrary_authored_helm_instance_routes_and_applies_by_kind() {
    let mut app = test_app();
    let thrust_id = "port-main-drive";
    let (npc, token) = spawn_admission_npc_with_thrust_id(&mut app, ControlSource::Ai, thrust_id);

    push(
        &mut app,
        &token,
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId(thrust_id.into()),
            payload: SystemControlPayload::SetThrust { value: 0.65 },
        },
    );
    tick(&mut app);

    assert_eq!(thrust_input_of(&app, npc), 0.65);
}

/// AC (issue #824): a human token still routes to the LocalShip even
/// with NPC ships present.
#[test]
fn human_token_still_routes_to_the_local_ship() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);
    let (npc, _token) = spawn_admission_npc(&mut app, ControlSource::Ai);

    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_thrust_system_id(),
            payload: SystemControlPayload::SetThrust { value: 0.4 },
        },
    );
    tick_twice(&mut app);

    let local = find_ship_entity(&mut app);
    assert_eq!(
        thrust_input_of(&app, local),
        0.4,
        "the station holder's SetThrust must land on the LocalShip's intent"
    );
    assert_eq!(
        thrust_input_of(&app, npc),
        0.0,
        "a human command must never land on an NPC's intent"
    );
}

/// AC (issue #824): mismatched authority is rejected — an `ai:` token
/// addressing a system the owning ship holds as Human is refused by that
/// ship's own `ControlSourceResolver` at the gate.
#[test]
fn mismatched_authority_ai_token_is_rejected() {
    let mut app = test_app();
    let (npc, token) = spawn_admission_npc(&mut app, ControlSource::Human);

    push(
        &mut app,
        &token,
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_thrust_system_id(),
            payload: SystemControlPayload::SetThrust { value: 0.9 },
        },
    );
    tick(&mut app);

    assert_eq!(
        thrust_input_of(&app, npc),
        0.0,
        "an ai: token must be refused when the owning ship's helm-thrust is human-held"
    );
}

// ── Boost applies on every AI ship, not only the local one (issue #881) ──

/// AC1/AC5 (issue #881): an admitted `SetBoost` on a NON-`LocalShip`
/// `AiHighFidelity` NPC reaches `BoostCommand` and engages `ShipBoost` in
/// the same tick. Before #881 the only `SetBoost` → `BoostCommand`
/// converter was `handle_boost_messages`, filtered `With<LocalShip>` and
/// reading a single entity, so `ai_helm_boost`'s admitted `SetBoost` for a
/// non-local NPC was admitted and then silently dropped.
#[test]
fn admitted_set_boost_engages_a_non_local_npc() {
    let mut app = test_app();

    // A boost-capable NPC with the helm-boost system on AI. No
    // `ShipPhysics`, so `ai_helm_boost` itself skips this entity — the
    // only boost writer under test is the shared applier.
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(
        crate::ship::system_registry::helm_boost_system_id(),
        ControlSource::Ai,
    );
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::ai::server::AiHighFidelity,
            crate::ship::components::ShipConfigComponent::default(),
            sources,
            crate::core::messages::AdmittedCommands::default(),
            crate::ship::components::BoostConfigResource {
                enabled: true,
                ..Default::default()
            },
            ShipBoost::default(),
            BoostCommand::default(),
        ))
        .id();
    let uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut()
        .resource_mut::<crate::ai::server::AiTokenRegistry>()
        .register_with_entity(&uuid, npc);
    let token = format!("ai:{uuid}");

    assert!(!app.world().entity(npc).get::<BoostCommand>().unwrap().0);

    // Burn one tick so the spawn-tick `is_added` exclusion in
    // `apply_helm_commands` is behind us, exactly as it is for a
    // LOD-promoted NPC in production.
    tick(&mut app);

    push(
        &mut app,
        &token,
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_boost_system_id(),
            payload: SystemControlPayload::SetBoost { active: true },
        },
    );
    tick(&mut app);

    assert!(
        app.world().entity(npc).get::<BoostCommand>().unwrap().0,
        "an admitted SetBoost must reach a non-LocalShip NPC's BoostCommand"
    );
    assert!(
        app.world()
            .entity(npc)
            .get::<ShipBoost>()
            .unwrap()
            .0
            .is_active(),
        "the NPC's BoostCommand must engage ShipBoost in the same tick"
    );
}

/// AC2 (issue #881): `ToggleBoost` behaves identically for an AI origin —
/// nothing downstream of admission branches on who sent it.
#[test]
fn admitted_toggle_boost_engages_a_non_local_npc() {
    let mut app = test_app();

    let mut sources = ShipSystemControlSources::default();
    sources.0.set(
        crate::ship::system_registry::helm_boost_system_id(),
        ControlSource::Ai,
    );
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::ai::server::AiHighFidelity,
            crate::ship::components::ShipConfigComponent::default(),
            sources,
            crate::core::messages::AdmittedCommands::default(),
            crate::ship::components::BoostConfigResource {
                enabled: true,
                ..Default::default()
            },
            ShipBoost::default(),
            BoostCommand::default(),
        ))
        .id();
    let uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut()
        .resource_mut::<crate::ai::server::AiTokenRegistry>()
        .register_with_entity(&uuid, npc);
    let token = format!("ai:{uuid}");

    tick(&mut app);

    push(
        &mut app,
        &token,
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_boost_system_id(),
            payload: SystemControlPayload::ToggleBoost,
        },
    );
    tick(&mut app);

    assert!(
        app.world().entity(npc).get::<BoostCommand>().unwrap().0,
        "ToggleBoost from an AI origin must engage the NPC's boost, same as a human's"
    );
}

/// AC3 (issue #881): a hull that authors no boost is unaffected — the
/// `enabled` guard is unchanged by the applier relocation.
#[test]
fn admitted_set_boost_is_ignored_without_an_enabled_boost_config() {
    let mut app = test_app();

    let mut sources = ShipSystemControlSources::default();
    sources.0.set(
        crate::ship::system_registry::helm_boost_system_id(),
        ControlSource::Ai,
    );
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::ai::server::AiHighFidelity,
            crate::ship::components::ShipConfigComponent::default(),
            sources,
            crate::core::messages::AdmittedCommands::default(),
            // No BoostConfigResource at all: no authored boost.
            ShipBoost::default(),
            BoostCommand::default(),
        ))
        .id();
    let uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut()
        .resource_mut::<crate::ai::server::AiTokenRegistry>()
        .register_with_entity(&uuid, npc);
    let token = format!("ai:{uuid}");

    tick(&mut app);

    push(
        &mut app,
        &token,
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_boost_system_id(),
            payload: SystemControlPayload::SetBoost { active: true },
        },
    );
    tick(&mut app);

    assert!(
        !app.world().entity(npc).get::<BoostCommand>().unwrap().0,
        "a hull authoring no boost must ignore SetBoost"
    );
    assert!(
        !app.world()
            .entity(npc)
            .get::<ShipBoost>()
            .unwrap()
            .0
            .is_active(),
        "a hull authoring no boost must never engage ShipBoost"
    );
}

#[test]
fn joystick_publishes_state_to_engines_via_inter_system() {
    let mut app = test_app_with_engine_hull();
    // Ensure InterSystemQueue is initialised.
    app.init_resource::<InterSystemQueue>();

    // Set a known LastHelmInput before ticking.
    set_last_helm_input(
        &mut app,
        LastHelmInput {
            thrust: 0.75,
            steering: 0.25,
            lateral: 0.0,
        },
    );

    tick(&mut app);

    let queue = app.world().resource::<InterSystemQueue>();
    let port_id = crate::ship::system_registry::helm_engine_port_system_id();
    let stbd_id = crate::ship::system_registry::helm_engine_starboard_system_id();

    let port_msgs: Vec<_> = queue.for_target(port_id.0.as_str()).collect();
    let stbd_msgs: Vec<_> = queue.for_target(stbd_id.0.as_str()).collect();

    // `publish_joystick_to_engines` and `operate_helm_engine_ai` may both push.
    // At least one message must arrive for each engine.
    assert!(
        !port_msgs.is_empty(),
        "expected at least one JoystickState message for helm-engine-port"
    );
    assert!(
        !stbd_msgs.is_empty(),
        "expected at least one JoystickState message for helm-engine-starboard"
    );

    // The first message should carry the joystick values.
    let InterSystemPayload::JoystickState { thrust, steering } = &port_msgs[0].payload else {
        panic!("expected JoystickState payload for port engine");
    };
    assert!(
        (*thrust - 0.75).abs() < 0.01,
        "port engine thrust should match joystick thrust"
    );
    assert!(
        (*steering - 0.25).abs() < 0.01,
        "port engine steering should match joystick steering"
    );
}

/// Issue #968: an offline axis has its latched intent CLEARED, not merely
/// masked downstream.
///
/// `integrate_ship_physics` gates each helm axis on its system being online,
/// which stops a destroyed actuator acting. But a gate leaves the last
/// commanded fraction sitting in the component — `snapshot.rs` serialises it,
/// and the tick a repair lifts the tier back out of `Disabled` it would be
/// applied again for up to a whole AI decision period before the axis owner
/// wrote a fresh one. So the owner clears it here, and the repair edge below
/// is the half of the behaviour a masking gate cannot give.
#[test]
fn an_offline_axis_clears_its_latched_intent() {
    let mut app = test_app();
    start_game_with_helm_and_science(&mut app);

    let thrust_of = |app: &mut App| {
        app.world_mut()
            .query_filtered::<&ThrustInput, With<LocalShip>>()
            .single(app.world())
            .expect("the fixture ship carries a ThrustInput")
            .0
    };
    let set_thrust_offline = |app: &mut App, offline: bool| {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ShipSystemControlSources>()
            .expect("the fixture ship carries control sources")
            .0
            .set_offline(
                crate::ship::system_registry::helm_thrust_system_id(),
                offline,
            );
    };

    // Latch a real command through the normal admitted path.
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::helm_thrust_system_id(),
            payload: SystemControlPayload::SetThrust { value: 1.0 },
        },
    );
    tick_twice(&mut app);
    assert_eq!(
        thrust_of(&mut app),
        1.0,
        "precondition: the axis must be holding a latched command for this \
             test to be about clearing one"
    );

    // Shoot the throttle away.
    set_thrust_offline(&mut app, true);
    tick(&mut app);
    assert_eq!(
        thrust_of(&mut app),
        0.0,
        "an offline axis must have its latched intent cleared, not left in \
             the component for a downstream gate to hide"
    );

    // Repair it. Nothing must resurrect the pre-damage throttle: the axis
    // starts from rest and waits for its owner to command it again.
    set_thrust_offline(&mut app, false);
    tick(&mut app);
    assert_eq!(
        thrust_of(&mut app),
        0.0,
        "a repaired axis must not resume the command it was holding when it \
             was destroyed"
    );
}
