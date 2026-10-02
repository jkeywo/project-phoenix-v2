use super::*;

fn correlated_tractor_command(
    correlation: &str,
    payload: SystemControlPayload,
) -> crate::core::messages::AdmittedCommand {
    crate::core::messages::AdmittedCommand {
        target: tractor_system_id(),
        payload,
        response_token: Some("engineering-holder".into()),
        feedback_correlation: Some(
            crate::core::messages::ActionCorrelationId::new(correlation)
                .expect("valid test correlation"),
        ),
    }
}

fn beam() -> TractorBeam {
    TractorBeam::new(
        TractorConfig {
            range: 500.0,
            coupling_offset: [0.0, 0.0, -120.0],
            min_power_level: 2,
            tow_load: crate::tractor::TowLoadCurve {
                half_penalty_mass: 10_000.0,
                max_penalty: 0.8,
            },
        },
        PowerGroupId("tractor".into()),
    )
}

#[test]
fn tractor_feedback_waits_for_the_authoritative_hold_verdict() {
    let mut app = App::new();
    app.add_message::<PendingTractorActionFeedback>()
        .add_message::<crate::lobby::OutboundMessage>()
        .add_systems(
            Update,
            (
                handle_tractor_commands,
                tick_tractor,
                finish_tractor_action_feedback,
            )
                .chain(),
        );
    let mut test_beam = beam();
    test_beam.config.min_power_level = 0;
    let operator = app
        .world_mut()
        .spawn((
            crate::core::messages::AdmittedCommands(vec![correlated_tractor_command(
                "tractor-applied",
                SystemControlPayload::EngageTractor,
            )]),
            test_beam,
            TacticalRadarSelection(Some("target-1".into())),
            Transform::default(),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid("target-1".into()),
        Transform::from_xyz(10.0, 0.0, 0.0),
    ));
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
            "tractor-applied",
            crate::core::messages::ActionFeedbackOutcome::Applied,
        ),
        1
    );
    assert!(app
        .world()
        .get::<TractorBeam>(operator)
        .is_some_and(|beam| beam.coupled_target.as_deref() == Some("target-1")));

    app.world_mut()
        .entity_mut(operator)
        .insert(crate::core::messages::AdmittedCommands(vec![
            correlated_tractor_command("tractor-release", SystemControlPayload::ReleaseTractor),
        ]));
    app.update();
    let messages = app
        .world()
        .resource::<Messages<crate::lobby::OutboundMessage>>();
    let second: Vec<_> = cursor.read(messages).cloned().collect();
    assert_eq!(
        feedback_count(
            &second,
            "tractor-release",
            crate::core::messages::ActionFeedbackOutcome::Applied,
        ),
        1
    );

    app.world_mut().entity_mut(operator).insert((
        crate::core::messages::AdmittedCommands(vec![correlated_tractor_command(
            "tractor-refused",
            SystemControlPayload::EngageTractor,
        )]),
        TacticalRadarSelection(None),
    ));
    app.update();
    let messages = app
        .world()
        .resource::<Messages<crate::lobby::OutboundMessage>>();
    let third: Vec<_> = cursor.read(messages).cloned().collect();
    assert_eq!(
        feedback_count(
            &third,
            "tractor-refused",
            crate::core::messages::ActionFeedbackOutcome::Refused,
        ),
        1
    );
}

#[test]
fn save_state_carries_engage_and_target_only() {
    let mut b = beam();
    b.engaged = true;
    b.coupled_target = Some("derelict-1".into());
    b.last_refusal = Some(TractorRefusal::OutOfRange);
    let save = b.save_state();
    assert!(save.engaged);
    assert_eq!(save.coupled_target.as_deref(), Some("derelict-1"));
}

#[test]
fn an_idle_beam_saves_as_default() {
    assert_eq!(beam().save_state(), TractorSaveState::default());
}

// ── The activation follows the coupling, not the intent (issue #1341) ────

/// A beam that needs no power, so the verdict turns on the lock and the
/// separation alone.
fn unpowered_beam() -> TractorBeam {
    let mut b = beam();
    b.config.min_power_level = 0;
    b
}

/// One operator carrying an `EngageTractor`, one subject at the origin, and
/// the lifecycle queue the two systems push onto.
fn engage_world(lock: &str) -> (bevy::prelude::World, bevy::prelude::Entity) {
    use bevy::prelude::*;
    let mut world = World::new();
    world.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    let operator = world
        .spawn((
            EntityUuid("uuid-tender".into()),
            unpowered_beam(),
            TacticalRadarSelection(Some(lock.into())),
            Transform::default(),
            crate::core::messages::AdmittedCommands(vec![crate::core::messages::AdmittedCommand {
                target: tractor_system_id(),
                payload: SystemControlPayload::EngageTractor,
                response_token: None,
                feedback_correlation: None,
            }]),
        ))
        .id();
    for uuid in ["uuid-hulk", "uuid-depot"] {
        world.spawn((EntityUuid(uuid.into()), Transform::default()));
    }
    (world, operator)
}

/// Every lifecycle request queued so far, drained.
fn queued(world: &mut bevy::prelude::World) -> Vec<TaskLifecycleRequest> {
    std::mem::take(&mut world.resource_mut::<EffectQueue<TaskLifecycleRequest>>().0)
}

/// The defect this pins: `handle_tractor_commands` and the ship's ONE lock
/// applier are both in `SimSet::Input` with no ordering between them, and
/// scripted appliers move the lock from other sets entirely — so the lock an
/// engage is read against is not necessarily the lock `tick_tractor` couples
/// to in `SimSet::Modifiers`.
///
/// The activation must therefore be opened by the coupling that ACTUALLY
/// formed. If it were opened by the intent, the timeline would name a hull
/// the beam is not holding for the whole activation — permanently, because
/// from the next tick the coupling and the lock agree again and nothing
/// would ever correct it.
#[test]
fn a_first_coupling_names_the_hull_the_beam_actually_gripped() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut world, operator) = engage_world("uuid-hulk");

    world
        .run_system_once(handle_tractor_commands)
        .expect("the engage applies");
    assert!(
        queued(&mut world).is_empty(),
        "the intent opens nothing: it does not yet know its own subject"
    );

    // Tactical re-designates in the same tick, after the engage was handled.
    world
        .entity_mut(operator)
        .insert(TacticalRadarSelection(Some("uuid-depot".into())));
    world
        .run_system_once(tick_tractor)
        .expect("the verdict applies");

    assert_eq!(
        queued(&mut world),
        vec![TaskLifecycleRequest::Start {
            slot: TaskSlot::new("uuid-tender", TRACTOR_SYSTEM_ID, TASK_VERB_TRACTOR_HOLD),
            target: Some("uuid-depot".into()),
        }],
        "the hold names the hull the beam gripped, not the one the intent \
             happened to read"
    );
    assert_eq!(
        world
            .get::<TractorBeam>(operator)
            .expect("the operator keeps its beam")
            .coupled_target
            .as_deref(),
        Some("uuid-depot"),
    );

    // A hold that simply continues reports nothing more.
    world
        .run_system_once(tick_tractor)
        .expect("the verdict applies");
    assert!(queued(&mut world).is_empty(), "an unchanged hold is silent");
}

/// An engage the very same tick refuses still gets a whole activation — a
/// start and its own terminal — so a refused engage is a beat rather than a
/// silence, and the pair stays adjacent on one slot for the emitter's
/// coalescing rule to recognise.
#[test]
fn an_engage_refused_before_it_couples_still_opens_and_closes_one_activation() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut world, operator) = engage_world("uuid-hulk");
    // Beyond the authored 500m reach.
    world
        .entity_mut(operator)
        .insert(Transform::from_xyz(5_000.0, 0.0, 0.0));

    world
        .run_system_once(handle_tractor_commands)
        .expect("the engage applies");
    world
        .run_system_once(tick_tractor)
        .expect("the verdict applies");

    let slot = TaskSlot::new("uuid-tender", TRACTOR_SYSTEM_ID, TASK_VERB_TRACTOR_HOLD);
    assert_eq!(
        queued(&mut world),
        vec![
            TaskLifecycleRequest::Start {
                slot: slot.clone(),
                target: Some("uuid-hulk".into()),
            },
            TaskLifecycleRequest::End {
                slot,
                reason: TaskTerminalReason::OutOfRange,
            },
        ],
    );
    assert!(
        !world
            .get::<TractorBeam>(operator)
            .expect("the operator keeps its beam")
            .engaged,
        "a refused engage drops the intent with the coupling"
    );
}
