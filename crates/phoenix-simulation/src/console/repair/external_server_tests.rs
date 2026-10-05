use super::*;

fn correlated_external_command(
    correlation: &str,
    payload: SystemControlPayload,
) -> crate::core::messages::AdmittedCommand {
    crate::core::messages::AdmittedCommand {
        target: repair_system_id(),
        payload,
        response_token: Some("repair-holder".into()),
        feedback_correlation: Some(
            crate::core::messages::ActionCorrelationId::new(correlation)
                .expect("valid test correlation"),
        ),
    }
}

#[test]
fn external_repair_feedback_finishes_at_dispatch_and_recall_verdicts() {
    let mut app = App::new();
    app.add_message::<crate::lobby::OutboundMessage>()
        .add_systems(Update, handle_external_repair_commands);
    let operator = app
        .world_mut()
        .spawn((
            AdmittedCommands(vec![correlated_external_command(
                "external-applied",
                SystemControlPayload::DispatchExternalRepair,
            )]),
            record(),
            TacticalRadarSelection(Some("ally-1".into())),
            Transform::default(),
            ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(1)),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid("ally-1".into()),
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
                    ) if token == "repair-holder"
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
            "external-applied",
            crate::core::messages::ActionFeedbackOutcome::Applied,
        ),
        1
    );
    assert_eq!(
        app.world()
            .get::<ExternalRepairDispatch>(operator)
            .and_then(|dispatch| dispatch.dispatched_target.as_deref()),
        Some("ally-1"),
    );

    app.world_mut()
        .entity_mut(operator)
        .insert(AdmittedCommands(vec![correlated_external_command(
            "external-recall",
            SystemControlPayload::RecallExternalRepair,
        )]));
    app.update();
    let messages = app
        .world()
        .resource::<Messages<crate::lobby::OutboundMessage>>();
    let second: Vec<_> = cursor.read(messages).cloned().collect();
    assert_eq!(
        feedback_count(
            &second,
            "external-recall",
            crate::core::messages::ActionFeedbackOutcome::Applied,
        ),
        1
    );
    assert!(app
        .world()
        .get::<ExternalRepairDispatch>(operator)
        .is_some_and(|dispatch| dispatch.dispatched_target.is_none()));

    app.world_mut().entity_mut(operator).insert((
        AdmittedCommands(vec![correlated_external_command(
            "external-refused",
            SystemControlPayload::DispatchExternalRepair,
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
            "external-refused",
            crate::core::messages::ActionFeedbackOutcome::Refused,
        ),
        1
    );
}

#[test]
fn external_repair_feedback_refuses_once_when_capability_is_absent() {
    let mut app = App::new();
    app.add_message::<crate::lobby::OutboundMessage>()
        .add_systems(Update, handle_external_repair_commands);
    let operator = app
        .world_mut()
        .spawn((AdmittedCommands::default(), Transform::default()))
        .id();
    let mut cursor = app
        .world()
        .resource::<Messages<crate::lobby::OutboundMessage>>()
        .get_cursor();

    for (correlation, payload) in [
        (
            "external-absent-dispatch",
            SystemControlPayload::DispatchExternalRepair,
        ),
        (
            "external-absent-recall",
            SystemControlPayload::RecallExternalRepair,
        ),
    ] {
        app.world_mut()
            .entity_mut(operator)
            .insert(AdmittedCommands(vec![correlated_external_command(
                correlation,
                payload,
            )]));
        app.update();
        let messages = app
            .world()
            .resource::<Messages<crate::lobby::OutboundMessage>>();
        let feedback: Vec<_> = cursor
            .read(messages)
            .filter(|message| {
                matches!(
                    (&message.target, &message.msg),
                    (
                        crate::lobby::Target::Token(token),
                        crate::core::messages::ServerMessage::ActionFeedback {
                            correlation: actual,
                            outcome: crate::core::messages::ActionFeedbackOutcome::Refused,
                        }
                    ) if token == "repair-holder" && actual.as_str() == correlation
                )
            })
            .collect();
        assert_eq!(feedback.len(), 1, "{correlation} must terminate once");
    }
}

fn record() -> ExternalRepairDispatch {
    ExternalRepairDispatch::new(ExternalRepairConfig {
        range: 600.0,
        repair_rate: 8.0,
    })
}

/// The claim NAMES its team (issue #1386) and only while it is live: an
/// idle record holds nobody abroad, because `claim` and `release` set and
/// clear the target and the index together.
#[test]
fn a_live_claim_names_its_team_and_an_idle_record_holds_nobody() {
    let mut r = record();
    assert_eq!(r.abroad_team(), None);

    r.claim(2, Some("ally-1".into()));
    assert_eq!(r.abroad_team(), Some(2));
    assert_eq!(r.dispatched_target.as_deref(), Some("ally-1"));

    r.release(None);
    assert_eq!(r.abroad_team(), None);
}

/// A recalled (or drifted-out) claim leaves the record in the ONE idle
/// shape, index included — otherwise `capture_external_repair` compares a
/// `{ None, N }` record against the idle default, finds them unequal and
/// writes a snapshot row for a hull holding nobody abroad.
#[test]
fn a_released_claim_returns_the_record_to_the_idle_default() {
    let mut r = record();
    r.claim(2, Some("ally-1".into()));
    r.release(None);
    assert_eq!(r.save_state(), ExternalRepairSaveState::default());
    assert_eq!(r.team_idx, 0);
}

fn critical_entry() -> crate::console::repair::server::RepairQueueEntry {
    crate::console::repair::server::RepairQueueEntry {
        station_id: "engineering".into(),
        station_label: "engineering".into(),
        tier: DamageTier::Disabled,
        deficit: 0.9,
    }
}

/// The "without starving critical repairs" reserve (issue #1162): one team is
/// held back per outstanding CRITICAL (Disabled/Destroyed) local repair, and
/// a merely-Damaged one reserves nothing.
#[test]
fn critical_local_repairs_reserves_one_team_each_and_damaged_reserves_none() {
    use crate::console::repair::server::RepairRequestQueue;
    assert_eq!(critical_local_repairs(None), 0);
    assert_eq!(
        critical_local_repairs(Some(&RepairRequestQueue { entries: vec![] })),
        0
    );
    // A merely-Damaged (non-critical) request reserves nothing.
    let damaged = crate::console::repair::server::RepairQueueEntry {
        tier: DamageTier::Damaged,
        ..critical_entry()
    };
    assert_eq!(
        critical_local_repairs(Some(&RepairRequestQueue {
            entries: vec![damaged],
        })),
        0
    );
    // Two Disabled requests reserve two teams.
    assert_eq!(
        critical_local_repairs(Some(&RepairRequestQueue {
            entries: vec![critical_entry(), critical_entry()],
        })),
        2
    );
}

/// The reserve folds into the SAME availability answer: a one-team hull with
/// a critical local repair outstanding has NO team free to dispatch, but
/// frees it the moment the local critical repair clears.
#[test]
fn a_one_team_hull_reserves_its_last_team_for_a_critical_local_repair() {
    use crate::modifiers::repair_teams::RepairTeams;
    let teams = RepairTeams::new(1);

    // A critical local repair reserves the one team → none free to dispatch.
    assert!(
        teams.free_team_indices_reserving(None, 1).is_empty(),
        "the last team must be reserved for a critical local repair"
    );

    // With no critical local repair, the one team is dispatchable.
    assert!(
        !teams.free_team_indices_reserving(None, 0).is_empty(),
        "with the local sweep clear the free team is available to help the ally"
    );

    // And the team already abroad is excluded BY NAME, not by count (issue
    // #1386): a two-team hull with team 0 out there offers team 1 and only
    // team 1, whichever end of the list the reserve would have eaten.
    let two = RepairTeams::new(2);
    assert_eq!(two.free_team_indices(Some(0)), vec![1]);
    assert_eq!(two.free_team_indices(Some(1)), vec![0]);
}

#[test]
fn save_state_carries_the_dispatched_target_and_the_team_working_it() {
    let mut r = record();
    r.claim(3, Some("ally-1".into()));
    r.last_refusal = Some(ExternalRepairRefusal::OutOfRange);
    let save = r.save_state();
    assert_eq!(save.dispatched_target.as_deref(), Some("ally-1"));
    assert_eq!(save.team_idx, 3);
}

#[test]
fn an_idle_record_saves_as_default() {
    assert_eq!(record().save_state(), ExternalRepairSaveState::default());
}

#[test]
fn restore_reseeds_the_target_and_its_team_and_clears_any_stale_refusal() {
    let mut r = record();
    r.last_refusal = Some(ExternalRepairRefusal::NoFreeTeam);
    r.restore(&ExternalRepairSaveState {
        dispatched_target: Some("ally-2".into()),
        team_idx: 1,
    });
    assert_eq!(r.dispatched_target.as_deref(), Some("ally-2"));
    assert_eq!(r.abroad_team(), Some(1));
    assert!(r.last_refusal.is_none());
}

// ── The task lifecycle (issue #1345) ─────────────────────────────────────

use crate::modifiers::repair_teams::RepairTeams;
use crate::server_app::ShipSystemBlackboards;

const OPERATOR: &str = "operator-1";
const ALLY: &str = "ally-1";

fn app_with(target_at: Option<Vec3>, free_teams: usize) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    app.add_message::<crate::lobby::OutboundMessage>();
    app.add_systems(
        Update,
        (
            handle_external_repair_commands,
            // The named recall lives with the internal one (issue #1386):
            // ONE verb answers `RecallRepairTeam` whatever the team is
            // doing, so the field claim's release is exercised through the
            // system that actually owns it rather than a stand-in.
            super::super::dispatch::handle_recall_repair_team,
            tick_external_repair,
        )
            .chain(),
    );
    let operator = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(OPERATOR.into()),
            Transform::from_translation(Vec3::ZERO),
            TacticalRadarSelection(Some(ALLY.into())),
            ShipRepairTeams(RepairTeams::new(free_teams)),
            ExternalRepairDispatch::new(ExternalRepairConfig {
                range: 600.0,
                repair_rate: 8.0,
            }),
            AdmittedCommands::default(),
            ShipSystemBlackboards::default(),
        ))
        .id();
    if let Some(position) = target_at {
        app.world_mut().spawn((
            EntityUuid(ALLY.into()),
            Transform::from_translation(position),
        ));
    }
    (app, operator)
}

fn admit_dispatch(app: &mut App, operator: Entity) {
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<AdmittedCommands>()
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: repair_system_id(),
            payload: SystemControlPayload::DispatchExternalRepair,
            response_token: None,
            feedback_correlation: None,
        });
}

fn admit_recall(app: &mut App, operator: Entity) {
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<AdmittedCommands>()
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: repair_system_id(),
            payload: SystemControlPayload::RecallExternalRepair,
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

fn dispatch_slot_for_test() -> TaskSlot {
    TaskSlot::new(OPERATOR, REPAIR_SYSTEM_ID, TASK_VERB_EXTERNAL_REPAIR)
}

/// A dispatch that commits opens exactly one activation, named for the
/// designated ally.
#[test]
fn a_committed_dispatch_opens_one_activation() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 1);
    admit_dispatch(&mut app, operator);
    app.update();

    assert_eq!(
        app.world()
            .entity(operator)
            .get::<ExternalRepairDispatch>()
            .unwrap()
            .dispatched_target
            .as_deref(),
        Some(ALLY)
    );
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::Start {
            slot: dispatch_slot_for_test(),
            target: Some(ALLY.into()),
        }]
    );
}

/// A dispatch refused at commit time (no free team) opens nothing — the
/// same "a refusal at dispatch time opens nothing" rule Security's own
/// dispatch keeps, since no team was ever actually claimed.
#[test]
fn a_dispatch_refused_at_commit_time_opens_nothing() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 0);
    admit_dispatch(&mut app, operator);
    app.update();

    assert!(app
        .world()
        .entity(operator)
        .get::<ExternalRepairDispatch>()
        .unwrap()
        .dispatched_target
        .is_none());
    assert!(
        drain_lifecycle(&mut app).is_empty(),
        "nothing was ever committed for a refused dispatch to end"
    );
}

/// A recall reports the cancel before clearing the claim, and a recall of
/// an idle dispatch (a stale-UI double tap) reports nothing at all.
#[test]
fn a_recall_reports_released_exactly_once_and_an_idle_recall_reports_nothing() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 1);
    admit_dispatch(&mut app, operator);
    app.update();
    drain_lifecycle(&mut app);

    admit_recall(&mut app, operator);
    app.update();
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::End {
            slot: dispatch_slot_for_test(),
            reason: TaskTerminalReason::Released,
        }]
    );

    // A second recall of the now-idle dispatch is a no-op.
    admit_recall(&mut app, operator);
    app.update();
    assert!(drain_lifecycle(&mut app).is_empty());
}

/// A repeat `DispatchExternalRepair` onto the SAME locked target is a sim
/// no-op (the commit re-assigns the identical value) and must drain an
/// empty lifecycle queue rather than reporting a phantom restart. A
/// dispatch onto a DIFFERENT target is a genuine change of claim and
/// still pushes a fresh `Start` (issue #1345).
#[test]
fn a_repeat_dispatch_onto_the_same_target_reports_nothing_but_a_new_target_still_starts() {
    const ALLY_TWO: &str = "ally-2";

    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 1);
    admit_dispatch(&mut app, operator);
    app.update();
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::Start {
            slot: dispatch_slot_for_test(),
            target: Some(ALLY.into()),
        }]
    );

    // A stale-UI double tap on the same lock: nothing changed in the sim,
    // so nothing should be reported.
    admit_dispatch(&mut app, operator);
    app.update();
    assert!(
        drain_lifecycle(&mut app).is_empty(),
        "a repeat dispatch onto the same target must not report a phantom restart"
    );
    assert_eq!(
        app.world()
            .entity(operator)
            .get::<ExternalRepairDispatch>()
            .unwrap()
            .dispatched_target
            .as_deref(),
        Some(ALLY)
    );

    // Re-lock onto a second ally: a genuinely different target still
    // starts a fresh activation (the old one is closed as `Restarted` by
    // the narrative emitter's own registry, downstream of this queue).
    app.world_mut().spawn((
        EntityUuid(ALLY_TWO.into()),
        Transform::from_translation(Vec3::new(100.0, 0.0, 0.0)),
    ));
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<TacticalRadarSelection>()
        .unwrap()
        .0 = Some(ALLY_TWO.into());
    admit_dispatch(&mut app, operator);
    app.update();
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::Start {
            slot: dispatch_slot_for_test(),
            target: Some(ALLY_TWO.into()),
        }]
    );
    assert_eq!(
        app.world()
            .entity(operator)
            .get::<ExternalRepairDispatch>()
            .unwrap()
            .dispatched_target
            .as_deref(),
        Some(ALLY_TWO)
    );
}

// ── The NAMED dispatch and the one recall (issue #1386) ─────────────────

fn admit(app: &mut App, operator: Entity, payload: SystemControlPayload) {
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<AdmittedCommands>()
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: repair_system_id(),
            payload,
            response_token: None,
            feedback_correlation: None,
        });
}

fn admit_named_dispatch(app: &mut App, operator: Entity, team_idx: u8) {
    admit(
        app,
        operator,
        SystemControlPayload::DispatchRepairTeam {
            team_idx,
            target: crate::core::messages::RepairTarget::External,
        },
    );
}

fn claim(app: &App, operator: Entity) -> (Option<String>, Option<usize>) {
    let dispatch = app
        .world()
        .entity(operator)
        .get::<ExternalRepairDispatch>()
        .unwrap();
    (dispatch.dispatched_target.clone(), dispatch.abroad_team())
}

fn refusal(app: &App, operator: Entity) -> Option<ExternalRepairRefusal> {
    app.world()
        .entity(operator)
        .get::<ExternalRepairDispatch>()
        .unwrap()
        .last_refusal
}

/// The whole point of the slice: the seat picks WHICH team crosses over, and
/// the claim records that team rather than a count somebody has to guess
/// from.
#[test]
fn a_named_dispatch_sends_the_team_the_seat_chose() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 3);
    admit_named_dispatch(&mut app, operator, 2);
    app.update();

    assert_eq!(claim(&app, operator), (Some(ALLY.into()), Some(2)));
    // …and that team, not the top of the idle list, is the one the hull's
    // own sweep may no longer have.
    let teams = app
        .world()
        .entity(operator)
        .get::<ShipRepairTeams>()
        .unwrap();
    assert_eq!(teams.0.free_team_indices(Some(2)), vec![0, 1]);
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::Start {
            slot: dispatch_slot_for_test(),
            target: Some(ALLY.into()),
        }]
    );
}

/// The fieldless verb still means "send somebody", and the rule it always
/// followed implicitly — the lowest free team — is now written down on the
/// claim for the console to read.
#[test]
fn the_fieldless_verb_records_the_lowest_free_team_it_picked() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 3);
    // Team 0 is out on an internal job, so the lowest FREE team is 1.
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<ShipRepairTeams>()
        .unwrap()
        .0
        .dispatch(
            0,
            crate::core::messages::SystemId("helm".into()),
            "H".into(),
        );
    admit_dispatch(&mut app, operator);
    app.update();

    assert_eq!(claim(&app, operator), (Some(ALLY.into()), Some(1)));
}

/// A named dispatch of a team that is already out on an internal job sends
/// nobody and says why — the console only offers the field row on idle
/// cards, so this is the stale-UI case.
#[test]
fn naming_a_busy_team_is_refused_team_busy_and_sends_nobody() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 2);
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<ShipRepairTeams>()
        .unwrap()
        .0
        .dispatch(
            1,
            crate::core::messages::SystemId("helm".into()),
            "H".into(),
        );
    admit_named_dispatch(&mut app, operator, 1);
    app.update();

    assert_eq!(claim(&app, operator), (None, None));
    assert_eq!(
        refusal(&app, operator),
        Some(ExternalRepairRefusal::TeamBusy)
    );
    assert!(
        drain_lifecycle(&mut app).is_empty(),
        "a refusal at dispatch time claims nothing, so it opens nothing"
    );
}

/// The claim stays single: a second team cannot be sent while one is out
/// there. The crew recall first.
#[test]
fn naming_a_second_team_while_one_is_abroad_is_refused_already_abroad() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 3);
    admit_named_dispatch(&mut app, operator, 2);
    app.update();
    drain_lifecycle(&mut app);

    admit_named_dispatch(&mut app, operator, 0);
    app.update();

    assert_eq!(
        claim(&app, operator),
        (Some(ALLY.into()), Some(2)),
        "the team already abroad stays abroad — a refusal sends nobody and recalls nobody"
    );
    assert_eq!(
        refusal(&app, operator),
        Some(ExternalRepairRefusal::AlreadyAbroad)
    );
    assert!(drain_lifecycle(&mut app).is_empty());
}

/// `RecallRepairTeam` is the ONE recall verb (issue #1386): naming the team
/// abroad releases the claim, exactly as `RecallExternalRepair` does, and
/// reports the same terminal.
#[test]
fn the_named_recall_brings_the_abroad_team_home() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 3);
    admit_named_dispatch(&mut app, operator, 2);
    app.update();
    drain_lifecycle(&mut app);

    admit(
        &mut app,
        operator,
        SystemControlPayload::RecallRepairTeam { team_idx: 2 },
    );
    app.update();

    assert_eq!(claim(&app, operator), (None, None));
    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::End {
            slot: dispatch_slot_for_test(),
            reason: TaskTerminalReason::Released,
        }]
    );
}

/// …and naming a DIFFERENT idle team leaves the claim alone. The abroad
/// team's slot reads `Idle` like every other idle slot, so a recall that
/// matched on status rather than on the claim's own index would bring home
/// whichever team the seat tapped.
#[test]
fn the_named_recall_of_another_idle_team_leaves_the_claim_alone() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 3);
    admit_named_dispatch(&mut app, operator, 2);
    app.update();
    drain_lifecycle(&mut app);

    admit(
        &mut app,
        operator,
        SystemControlPayload::RecallRepairTeam { team_idx: 0 },
    );
    app.update();

    assert_eq!(claim(&app, operator), (Some(ALLY.into()), Some(2)));
    assert!(drain_lifecycle(&mut app).is_empty());
}

/// A target that drifts past the authored range ends a live dispatch with
/// the mapped reason — `tick_external_repair` only ever CLOSES a dispatch,
/// so this never opens a fresh activation of its own.
#[test]
fn a_target_that_drifts_out_of_range_ends_the_live_dispatch() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)), 1);
    admit_dispatch(&mut app, operator);
    app.update();
    drain_lifecycle(&mut app);

    let target = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, uuid)| uuid.0 == ALLY)
        .map(|(e, _)| e)
        .unwrap();
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_xyz(9000.0, 0.0, 0.0));
    app.update();

    assert_eq!(
        drain_lifecycle(&mut app),
        vec![TaskLifecycleRequest::End {
            slot: dispatch_slot_for_test(),
            reason: TaskTerminalReason::OutOfRange,
        }]
    );
    assert!(app
        .world()
        .entity(operator)
        .get::<ExternalRepairDispatch>()
        .unwrap()
        .dispatched_target
        .is_none());
}
