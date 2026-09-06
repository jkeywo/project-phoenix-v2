//! Adapter tests for the Security System (issue #1346).
//!
//! The pure half — the verdict, the state machine, the priority ranking and the
//! capacity reservation — is tested in [`crate::security::teams`]. What is tested
//! here is everything only the world can answer: that the admitted command
//! reaches the right team, that a team walks deploy → work → withdraw on the
//! authored clock, that a completed action raises its authored flag on the world
//! store AND queues the event a scenario trigger reacts to, that a target which
//! leaves the world or drifts out of reach interrupts the work, and that both
//! halves of every assignment reach the shared #1341 task-lifecycle telemetry.

use super::*;
use crate::core::messages::AdmittedCommand;
use crate::core::task_lifecycle::TaskLifecycleRequest;
use crate::security::teams::{SecurityActionConfig, SecurityPriority, SecurityTeamState};

const OPERATOR: &str = "destroyer-1";
const TARGET: &str = "compartment-1";
const CONTAINED: &str = "compartment_contained";

fn config() -> SecurityConfig {
    SecurityConfig {
        team_count: 2,
        deploy_duration_secs: 2.0,
        withdraw_duration_secs: 2.0,
        range: 400.0,
    }
}

fn target_config() -> SecurityTargetConfig {
    SecurityTargetConfig {
        actions: vec![
            SecurityActionConfig {
                action: SecurityAction::SecureContain,
                duration_secs: 4.0,
                risk: 0.6,
                priority: SecurityPriority::ThreatContainment,
                outcome_flag: Some(CONTAINED.to_string()),
                outcome_value: 1,
                warning: Some("security.warning.test".to_string()),
            },
            SecurityActionConfig {
                action: SecurityAction::AssistEvacuation,
                duration_secs: 3.0,
                risk: 0.3,
                priority: SecurityPriority::LifeSafety,
                outcome_flag: None,
                outcome_value: 1,
                warning: None,
            },
        ],
    }
}

/// A bare app carrying the command handler, the team clock and the publisher,
/// ticked by hand with a fixed one-second delta so the authored durations are
/// exact whole ticks. No `TimePlugin`: the clock is inserted here so the tests
/// own the delta rather than a frame rate.
fn app_with(target_at: Option<Vec3>) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<WorldContentRuntime>();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs(1));
    app.insert_resource(time);
    app.add_systems(
        Update,
        (
            handle_security_commands,
            tick_security_teams,
            publish_security_blackboard,
            // Production refills `AdmittedCommands` from scratch every tick
            // (`admit_system_commands`), so a command is consumed exactly once.
            // The fixture has to do the same or a single dispatch would re-fire
            // on every update.
            clear_admitted,
        )
            .chain(),
    );
    let operator = app
        .world_mut()
        .spawn((
            EntityUuid(OPERATOR.to_string()),
            Transform::from_translation(Vec3::ZERO),
            AdmittedCommands::default(),
            ShipSecurityTeams::new(config()),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();
    if let Some(position) = target_at {
        app.world_mut().spawn((
            EntityUuid(TARGET.to_string()),
            EntityName("world.test.compartment.name".to_string()),
            Transform::from_translation(position),
            SecurityTargetActions(target_config()),
        ));
    }
    (app, operator)
}

fn clear_admitted(mut inboxes: Query<&mut AdmittedCommands>) {
    for mut inbox in inboxes.iter_mut() {
        if !inbox.0.is_empty() {
            inbox.0.clear();
        }
    }
}

fn admit(app: &mut App, operator: Entity, payload: SystemControlPayload) {
    app.world_mut()
        .entity_mut(operator)
        .get_mut::<AdmittedCommands>()
        .expect("the operator carries an admitted-command inbox")
        .0
        .push(AdmittedCommand {
            target: security_system_id(),
            payload,
            response_token: None,
            feedback_correlation: None,
        });
}

fn dispatch(app: &mut App, operator: Entity, team_idx: u8, target: &str, action: &str) {
    admit(
        app,
        operator,
        SystemControlPayload::DispatchSecurityTeam {
            team_idx,
            target: target.to_string(),
            action: action.to_string(),
        },
    );
}

fn teams(app: &App, operator: Entity) -> ShipSecurityTeams {
    app.world()
        .entity(operator)
        .get::<ShipSecurityTeams>()
        .expect("the operator musters Security teams")
        .clone()
}

fn drain_lifecycle(app: &mut App) -> Vec<TaskLifecycleRequest> {
    std::mem::take(
        &mut app
            .world_mut()
            .resource_mut::<EffectQueue<TaskLifecycleRequest>>()
            .0,
    )
}

fn blackboard(app: &App, operator: Entity) -> SecurityBlackboard {
    match app
        .world()
        .entity(operator)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .expect("the operator publishes blackboards")
        .0
        .get(&security_system_id())
    {
        Some(SystemBlackboard::Security(bb)) => bb.clone(),
        other => panic!("expected a Security blackboard, got {other:?}"),
    }
}

// ── The two teams are independent (AC1) ──────────────────────────────────────

#[test]
fn a_hull_musters_its_authored_teams_and_each_takes_its_own_assignment() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    assert_eq!(teams(&app, operator).teams.len(), 2);
    assert_eq!(teams(&app, operator).free_teams(), 2);

    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    dispatch(&mut app, operator, 1, TARGET, "assist_evacuation");
    app.update();

    let mustered = teams(&app, operator);
    assert_eq!(
        mustered.teams[0].action,
        Some(SecurityAction::SecureContain)
    );
    assert_eq!(
        mustered.teams[1].action,
        Some(SecurityAction::AssistEvacuation),
        "the second team holds its OWN assignment — two teams, two jobs"
    );
    assert_eq!(mustered.free_teams(), 0);
}

#[test]
fn one_team_cannot_hold_two_assignments() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    dispatch(&mut app, operator, 0, TARGET, "assist_evacuation");
    app.update();

    let mustered = teams(&app, operator);
    assert_eq!(
        mustered.teams[0].action,
        Some(SecurityAction::SecureContain),
        "the second order must not overwrite the job the team is already on"
    );
    assert_eq!(mustered.last_refusal, Some(SecurityRefusal::TeamBusy));
}

// ── The complete success path, with its consequence (AC3) ────────────────────

/// Deploy for the authored crossing, work for the authored duration, raise the
/// authored flag, come home. The consequence reaches BOTH the flag store a
/// predicate reads and the world-event stream an `on_flag_set` trigger fires on.
#[test]
fn a_completed_action_raises_its_authored_flag_and_the_team_comes_home() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    assert_eq!(
        teams(&app, operator).teams[0].state,
        SecurityTeamState::Deploying
    );
    assert_eq!(
        teams(&app, operator).teams[0].risk,
        0.6,
        "the authored risk rides the assignment"
    );

    // Crossing: 2s at 1s a tick, and the dispatch tick is the first of them, so
    // the SECOND tick is the one that lands the team and starts the work.
    app.update();
    assert_eq!(
        teams(&app, operator).teams[0].state,
        SecurityTeamState::Working
    );
    assert!(
        !app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .flag(CONTAINED),
        "nothing is banked until the work finishes"
    );

    // Work: 4s.
    for _ in 0..4 {
        app.update();
    }
    assert!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .flag(CONTAINED),
        "the authored outcome flag is up in the world store, where a scenario predicate reads it"
    );
    let events = std::mem::take(
        &mut app
            .world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_world_events,
    );
    assert!(
        events.contains(&WorldEvent::FlagSet {
            name: CONTAINED.to_string(),
            origin_layer: None,
        }),
        "…and the edge reaches the world-event stream an on_flag_set trigger fires on, got {events:?}"
    );
    assert_eq!(
        teams(&app, operator).teams[0].state,
        SecurityTeamState::Withdrawing
    );

    // Return: 2s.
    app.update();
    assert_eq!(
        teams(&app, operator).teams[0].state,
        SecurityTeamState::Withdrawing
    );
    app.update();
    let home = teams(&app, operator);
    assert_eq!(home.teams[0].state, SecurityTeamState::Available);
    assert_eq!(home.teams[0].target, None);
    assert_eq!(home.free_teams(), 2);
}

// ── Interruption (AC3) ───────────────────────────────────────────────────────

#[test]
fn a_target_that_leaves_the_world_interrupts_the_work_and_sends_the_team_home() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    app.update();
    app.update();
    assert_eq!(
        teams(&app, operator).teams[0].state,
        SecurityTeamState::Working
    );
    drain_lifecycle(&mut app);

    let target_entity = app
        .world_mut()
        .query::<(Entity, &SecurityTargetActions)>()
        .iter(app.world())
        .map(|(entity, _)| entity)
        .next()
        .expect("the target exists");
    app.world_mut().entity_mut(target_entity).despawn();
    app.update();

    let interrupted = teams(&app, operator);
    assert_eq!(interrupted.teams[0].state, SecurityTeamState::Withdrawing);
    assert_eq!(
        interrupted.last_refusal,
        Some(SecurityRefusal::NoSuchTarget),
        "the crew are told why the job ended"
    );
    assert!(
        !app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .flag(CONTAINED),
        "an interrupted action banks nothing"
    );
    let reported = drain_lifecycle(&mut app);
    assert!(
        reported.iter().any(|r| matches!(
            r,
            TaskLifecycleRequest::End {
                reason: TaskTerminalReason::TargetLost,
                ..
            }
        )),
        "the interruption reaches the shared task lifecycle, got {reported:?}"
    );
}

#[test]
fn a_target_that_drifts_past_the_authored_reach_interrupts_the_work() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();

    let target_entity = app
        .world_mut()
        .query::<(Entity, &SecurityTargetActions)>()
        .iter(app.world())
        .map(|(entity, _)| entity)
        .next()
        .expect("the target exists");
    app.world_mut()
        .entity_mut(target_entity)
        .insert(Transform::from_translation(Vec3::new(4000.0, 0.0, 0.0)));
    app.update();

    let interrupted = teams(&app, operator);
    assert_eq!(interrupted.teams[0].state, SecurityTeamState::Withdrawing);
    assert_eq!(interrupted.last_refusal, Some(SecurityRefusal::OutOfRange));
}

/// A recall ends the assignment where it stands and starts the team home; the
/// team is not available again until it has actually arrived.
#[test]
fn a_recall_ends_the_assignment_and_the_team_walks_home() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    drain_lifecycle(&mut app);

    admit(
        &mut app,
        operator,
        SystemControlPayload::RecallSecurityTeam { team_idx: 0 },
    );
    app.update();
    assert_eq!(
        teams(&app, operator).teams[0].state,
        SecurityTeamState::Withdrawing
    );
    let reported = drain_lifecycle(&mut app);
    assert!(
        reported.iter().any(|r| matches!(
            r,
            TaskLifecycleRequest::End {
                reason: TaskTerminalReason::Released,
                ..
            }
        )),
        "a recall is a cancellation, not a failure, got {reported:?}"
    );

    app.update();
    assert!(teams(&app, operator).teams[0].is_available());
}

#[test]
fn recalling_a_team_that_is_not_out_is_a_no_op() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    admit(
        &mut app,
        operator,
        SystemControlPayload::RecallSecurityTeam { team_idx: 1 },
    );
    app.update();
    assert!(teams(&app, operator).teams[1].is_available());
    assert_eq!(teams(&app, operator).last_refusal, None);
}

// ── Invalid targets (AC3) ────────────────────────────────────────────────────

#[test]
fn an_unknown_target_an_unoffered_action_and_an_unknown_verb_all_refuse_without_sending_anyone() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));

    dispatch(&mut app, operator, 0, "nothing-here", "secure_contain");
    app.update();
    assert!(teams(&app, operator).teams[0].is_available());
    assert_eq!(
        teams(&app, operator).last_refusal,
        Some(SecurityRefusal::NoSuchTarget)
    );

    dispatch(&mut app, operator, 0, TARGET, "place_charges");
    app.update();
    assert!(teams(&app, operator).teams[0].is_available());
    assert_eq!(
        teams(&app, operator).last_refusal,
        Some(SecurityRefusal::ActionUnavailable),
        "the compartment authors no demolition work"
    );

    dispatch(&mut app, operator, 0, TARGET, "vent_the_deck");
    app.update();
    assert!(teams(&app, operator).teams[0].is_available());
    assert_eq!(
        teams(&app, operator).last_refusal,
        Some(SecurityRefusal::ActionUnavailable),
        "a verb the engine has never heard of is refused, not guessed at"
    );

    dispatch(&mut app, operator, 7, TARGET, "secure_contain");
    app.update();
    assert_eq!(
        teams(&app, operator).last_refusal,
        Some(SecurityRefusal::NoSuchTeam)
    );
}

#[test]
fn a_target_outside_the_authored_reach_is_refused_at_dispatch() {
    let (mut app, operator) = app_with(Some(Vec3::new(4000.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    assert!(teams(&app, operator).teams[0].is_available());
    assert_eq!(
        teams(&app, operator).last_refusal,
        Some(SecurityRefusal::OutOfRange)
    );
}

// ── The task lifecycle (AC5) ─────────────────────────────────────────────────

/// Each team gets its OWN lifecycle slot, so two simultaneous assignments are two
/// simultaneous activations rather than one restarting the other.
#[test]
fn two_simultaneous_assignments_occupy_two_distinct_lifecycle_slots() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    dispatch(&mut app, operator, 1, TARGET, "assist_evacuation");
    app.update();

    let reported = drain_lifecycle(&mut app);
    let slots: Vec<String> = reported
        .iter()
        .filter_map(|r| match r {
            TaskLifecycleRequest::Start { slot, .. } => Some(slot.verb.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        slots,
        vec!["security_team_0".to_string(), "security_team_1".to_string()],
        "one slot per team, so neither activation restarts the other"
    );
    for request in &reported {
        if let TaskLifecycleRequest::Start { slot, target } = request {
            assert_eq!(slot.operator, OPERATOR);
            assert_eq!(slot.system, SECURITY_SYSTEM_ID);
            assert_eq!(target.as_deref(), Some(TARGET));
        }
    }
}

#[test]
fn a_completed_assignment_reports_exactly_one_start_and_one_completion() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "assist_evacuation");
    let mut starts = 0;
    let mut completions = 0;
    for _ in 0..12 {
        app.update();
        for request in drain_lifecycle(&mut app) {
            match request {
                TaskLifecycleRequest::Start { .. } => starts += 1,
                TaskLifecycleRequest::End {
                    reason: TaskTerminalReason::Completed,
                    ..
                } => completions += 1,
                TaskLifecycleRequest::End { .. } => {}
            }
        }
    }
    assert_eq!((starts, completions), (1, 1));
    assert!(teams(&app, operator).teams[0].is_available());
}

// ── Damage takes the teams off the board ─────────────────────────────────────

#[test]
fn a_disabled_security_system_makes_every_team_unavailable_and_ends_the_work() {
    use crate::ship::damage::SystemHull;

    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    drain_lifecycle(&mut app);

    // A Security System with no hull left: `tier_for` reads a zero `current` as
    // Destroyed, which is the "damaged out" the teams go off the board for.
    let hull = SystemHull::from_config(&[(security_system_id(), 0.0)]);
    app.world_mut()
        .entity_mut(operator)
        .insert(EntitySystemHull(hull));
    app.update();

    let knocked_out = teams(&app, operator);
    assert!(
        knocked_out
            .teams
            .iter()
            .all(|t| t.state == SecurityTeamState::Unavailable),
        "a knocked-out Security System takes every team off the board"
    );
    assert_eq!(knocked_out.last_refusal, Some(SecurityRefusal::Disabled));
    let reported = drain_lifecycle(&mut app);
    assert!(
        reported.iter().any(|r| matches!(
            r,
            TaskLifecycleRequest::End {
                reason: TaskTerminalReason::Disabled,
                ..
            }
        )),
        "the interrupted assignment gets its one terminal, got {reported:?}"
    );

    // A dispatch into a knocked-out system is refused before anything else.
    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    assert_eq!(
        teams(&app, operator).last_refusal,
        Some(SecurityRefusal::Disabled)
    );
}

// ── The console readout (AC2) ────────────────────────────────────────────────

/// Everything the repair-team-style interface needs is on the wire: the team
/// list with state/assignment/progress/risk, the eligible targets with their
/// authored actions, durations, risks, priorities and warnings, and the refusal.
#[test]
fn the_blackboard_carries_the_team_list_the_eligible_targets_and_the_actions() {
    let (mut app, operator) = app_with(Some(Vec3::new(100.0, 0.0, 0.0)));
    app.update();

    let bb = blackboard(&app, operator);
    assert_eq!(bb.range, 400.0);
    assert_eq!(bb.teams.len(), 2);
    assert_eq!(bb.teams[0].state, "available");
    assert_eq!(bb.teams[0].target, None);
    assert_eq!(bb.targets.len(), 1);
    let target = &bb.targets[0];
    assert_eq!(target.uuid, TARGET);
    assert_eq!(target.name.as_deref(), Some("world.test.compartment.name"));
    assert_eq!(target.separation, 100.0);
    assert!(target.in_range);
    let ids: Vec<&str> = target.actions.iter().map(|a| a.action.as_str()).collect();
    assert_eq!(ids, vec!["secure_contain", "assist_evacuation"]);
    assert_eq!(target.actions[0].duration_secs, 4.0);
    assert_eq!(target.actions[0].risk, 0.6);
    assert_eq!(target.actions[0].priority, "threat_containment");
    assert_eq!(
        target.actions[0].warning.as_deref(),
        Some("security.warning.test"),
        "the warning is a strings.csv id, never English"
    );
    assert_eq!(target.actions[1].priority, "life_safety");

    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    let bb = blackboard(&app, operator);
    assert_eq!(bb.teams[0].state, "deploying");
    assert_eq!(bb.teams[0].target.as_deref(), Some(TARGET));
    assert_eq!(bb.teams[0].action.as_deref(), Some("secure_contain"));
    assert_eq!(bb.teams[0].risk, 0.6);
    assert!(bb.teams[0].progress > 0.0 && bb.teams[0].progress < 1.0);
    assert_eq!(bb.refusal, None);

    dispatch(&mut app, operator, 0, TARGET, "secure_contain");
    app.update();
    assert_eq!(
        blackboard(&app, operator).refusal.as_deref(),
        Some("security.dispatch.refused.team_busy"),
        "the refusal crosses as a strings.csv id"
    );
}

#[test]
fn a_target_beyond_the_reach_is_listed_but_marked_out_of_range() {
    let (mut app, operator) = app_with(Some(Vec3::new(4000.0, 0.0, 0.0)));
    app.update();
    let bb = blackboard(&app, operator);
    assert_eq!(bb.targets.len(), 1);
    assert!(
        !bb.targets[0].in_range,
        "the console shows what exists and whether it can be reached; the range rule stays \
         server-side"
    );
}

// ── Save / restore ───────────────────────────────────────────────────────────

#[test]
fn an_idle_muster_saves_as_default_and_a_committed_one_round_trips() {
    let mut security = ShipSecurityTeams::new(config());
    assert_eq!(security.save_state(), SecuritySaveState::default());

    security.teams[1].deploy(TARGET.into(), SecurityAction::Board, 0.8, 2.0);
    security.teams[1].elapsed = 0.5;
    security.last_refusal = Some(SecurityRefusal::OutOfRange);
    let save = security.save_state();
    assert_eq!(save.teams.len(), 1, "only committed teams travel");
    assert_eq!(save.teams[0].team_idx, 1);

    let mut restored = ShipSecurityTeams::new(config());
    restored.restore(&save);
    assert_eq!(restored.teams[1].state, SecurityTeamState::Deploying);
    assert_eq!(restored.teams[1].target.as_deref(), Some(TARGET));
    assert_eq!(restored.teams[1].action, Some(SecurityAction::Board));
    assert_eq!(restored.teams[1].elapsed, 0.5);
    assert!(restored.teams[0].is_available());
    assert_eq!(
        restored.last_refusal, None,
        "a refusal is a projection the next tick re-derives, never a restored fact"
    );
}

// ── The backfill host, through the world (AC4, AC5) ──────────────────────────

/// The life-safety job in the AI fixtures — the one that outranks everything
/// else, carrying an outcome flag so its completion is observable.
const EVACUATED: &str = "head_evacuation_assisted";
/// The lower-priority containment job the host's trades are made against.
const OTHER: &str = "gallery-1";
const GALLERY_CONTAINED: &str = "gallery_contained";

fn head_target_config() -> SecurityTargetConfig {
    SecurityTargetConfig {
        actions: vec![SecurityActionConfig {
            action: SecurityAction::AssistEvacuation,
            duration_secs: 4.0,
            risk: 0.6,
            priority: SecurityPriority::LifeSafety,
            outcome_flag: Some(EVACUATED.to_string()),
            outcome_value: 1,
            warning: None,
        }],
    }
}

fn gallery_target_config() -> SecurityTargetConfig {
    SecurityTargetConfig {
        actions: vec![SecurityActionConfig {
            action: SecurityAction::SecureContain,
            duration_secs: 6.0,
            risk: 0.4,
            priority: SecurityPriority::ThreatContainment,
            outcome_flag: Some(GALLERY_CONTAINED.to_string()),
            outcome_value: 1,
            warning: None,
        }],
    }
}

fn optional_target_config(action: SecurityAction) -> SecurityTargetConfig {
    SecurityTargetConfig {
        actions: vec![SecurityActionConfig {
            action,
            duration_secs: 5.0,
            risk: 0.2,
            priority: SecurityPriority::Optional,
            outcome_flag: None,
            outcome_value: 1,
            warning: None,
        }],
    }
}

/// Spawn the AI-operated Security ship and the given targets into `app`.
///
/// The Security System's control source is `Ai`, which is the whole of what makes
/// this the backfill seat: admission then takes the host's `ai:<uuid>` token down
/// exactly the path a console's token walks.
fn spawn_ai_operator(app: &mut App, targets: &[(&str, Vec3, SecurityTargetConfig)]) -> Entity {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    let mut sources = ControlSourceResolver::new();
    sources.set(security_system_id(), ControlSource::Ai);

    let operator = app
        .world_mut()
        .spawn((
            EntityUuid(OPERATOR.to_string()),
            Transform::from_translation(Vec3::ZERO),
            AdmittedCommands::default(),
            ShipSecurityTeams::new(config()),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipSystemControlSources(sources),
        ))
        .id();
    for (uuid, position, target_config) in targets {
        app.world_mut().spawn((
            EntityUuid((*uuid).to_string()),
            EntityName(format!("world.test.{uuid}.name")),
            Transform::from_translation(*position),
            SecurityTargetActions(target_config.clone()),
        ));
    }
    operator
}

/// The host and the resources admission needs, and NOTHING else — so a test can
/// read the command the host emitted before any applier consumes it.
fn host_only_app(targets: &[(&str, Vec3, SecurityTargetConfig)]) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<WorldContentRuntime>();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.add_systems(Update, operate_security_ai);
    let operator = spawn_ai_operator(&mut app, targets);
    (app, operator)
}

/// The host wired to the SAME applier a console's message reaches, so one
/// decision travels the whole path in a single `update()`.
fn ai_app(targets: &[(&str, Vec3, SecurityTargetConfig)]) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<WorldContentRuntime>();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs(1));
    app.insert_resource(time);
    app.add_systems(
        Update,
        (
            operate_security_ai,
            handle_security_commands,
            tick_security_teams,
            publish_security_blackboard,
            clear_admitted,
        )
            .chain(),
    );
    let operator = spawn_ai_operator(&mut app, targets);
    (app, operator)
}

/// Every Security payload the host put in this ship's inbox, in emission order.
fn host_payloads(app: &App, operator: Entity) -> Vec<SystemControlPayload> {
    app.world()
        .entity(operator)
        .get::<AdmittedCommands>()
        .expect("the operator carries an admitted-command inbox")
        .0
        .iter()
        .filter(|c| c.target == security_system_id())
        .map(|c| c.payload.clone())
        .collect()
}

/// AC5 at the seam: what the host emits is not merely *like* a console's command,
/// it IS one — the same payload variant with the same fields, landing in the same
/// `AdmittedCommands` inbox, so `handle_security_commands` cannot tell who spoke.
#[test]
fn the_host_emits_the_command_a_console_sends_and_the_applier_cannot_tell_them_apart() {
    let (mut app, operator) =
        host_only_app(&[(TARGET, Vec3::new(100.0, 0.0, 0.0), head_target_config())]);
    app.update();

    assert_eq!(
        host_payloads(&app, operator),
        vec![SystemControlPayload::DispatchSecurityTeam {
            team_idx: 0,
            target: TARGET.to_string(),
            action: "assist_evacuation".to_string(),
        }],
        "the host's decision is the console's message, field for field"
    );

    // And the applier takes it: same inbox, same handler, same outcome.
    let (mut app, operator) = ai_app(&[(TARGET, Vec3::new(100.0, 0.0, 0.0), head_target_config())]);
    app.update();
    let muster = teams(&app, operator);
    assert_eq!(muster.teams[0].state, SecurityTeamState::Deploying);
    assert_eq!(muster.teams[0].target.as_deref(), Some(TARGET));
    assert_eq!(
        muster.teams[0].action,
        Some(SecurityAction::AssistEvacuation)
    );
    assert_eq!(
        muster.last_refusal, None,
        "the host never proposes something the applier refuses"
    );
}

/// AC4's ordering half: two jobs in reach and two teams free, so both teams go
/// out — and the life-safety job is taken first.
#[test]
fn both_free_teams_are_spent_on_the_two_distinct_jobs_in_priority_order() {
    let (mut app, operator) = host_only_app(&[
        (TARGET, Vec3::new(100.0, 0.0, 0.0), head_target_config()),
        (OTHER, Vec3::new(120.0, 0.0, 0.0), gallery_target_config()),
    ]);
    app.update();

    assert_eq!(
        host_payloads(&app, operator),
        vec![
            SystemControlPayload::DispatchSecurityTeam {
                team_idx: 0,
                target: TARGET.to_string(),
                action: "assist_evacuation".to_string(),
            },
            SystemControlPayload::DispatchSecurityTeam {
                team_idx: 1,
                target: OTHER.to_string(),
                action: "secure_contain".to_string(),
            },
        ],
        "life safety takes the first team, containment the second"
    );
}

/// AC4's availability half, and the regression the pool filter exists for: with
/// one team ALREADY on the higher-priority job, the remaining free team must be
/// spent on the next job down. Selecting from an unfiltered pool returned the job
/// already under way — one pick for one free team — which the emit loop could only
/// drop, leaving the second team at home while the gallery burned.
#[test]
fn the_second_team_takes_the_lesser_job_while_the_first_works_the_urgent_one() {
    let (mut app, operator) = ai_app(&[
        (TARGET, Vec3::new(100.0, 0.0, 0.0), head_target_config()),
        (OTHER, Vec3::new(120.0, 0.0, 0.0), gallery_target_config()),
    ]);
    {
        let mut ship = app.world_mut().entity_mut(operator);
        let mut muster = ship
            .get_mut::<ShipSecurityTeams>()
            .expect("the operator musters Security teams");
        muster.teams[0].deploy(
            TARGET.to_string(),
            SecurityAction::AssistEvacuation,
            0.6,
            2.0,
        );
    }
    app.update();

    let muster = teams(&app, operator);
    assert_eq!(
        muster.teams[1].target.as_deref(),
        Some(OTHER),
        "the free team goes to the containment job rather than idling on a duplicate"
    );
    assert_eq!(muster.teams[1].action, Some(SecurityAction::SecureContain));
    assert_eq!(
        muster.teams[0].target.as_deref(),
        Some(TARGET),
        "and the team already working is left alone"
    );
}

/// Finished work leaves the pool. Once an action's authored `outcome_flag` is up
/// the host stops proposing it — otherwise every tick after the team walked home
/// would re-dispatch it forever, spamming the shared #1341 lifecycle with
/// Start/Completed pairs for work already done.
#[test]
fn the_host_stops_proposing_work_whose_outcome_flag_has_already_come_up() {
    let (mut app, operator) =
        host_only_app(&[(TARGET, Vec3::new(100.0, 0.0, 0.0), head_target_config())]);
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .flags
        .set_flag(EVACUATED);
    app.update();
    assert!(
        host_payloads(&app, operator).is_empty(),
        "the consequence has already landed; there is nothing left to send a team to"
    );
}

/// The same rule on the clock, end to end: the host dispatches, the team works,
/// the flag comes up, the team walks home — and the host does not send it out
/// again on the tick after that, or on any tick after that.
#[test]
fn a_completed_job_is_never_re_dispatched_on_the_following_ticks() {
    let (mut app, operator) = ai_app(&[(TARGET, Vec3::new(100.0, 0.0, 0.0), head_target_config())]);
    // deploy 2s + work 4s + withdraw 2s at a second a tick, and then some.
    for _ in 0..16 {
        app.update();
    }
    assert!(
        app.world()
            .resource::<WorldContentRuntime>()
            .flags
            .flag(EVACUATED),
        "the work completed and raised its authored consequence"
    );
    let muster = teams(&app, operator);
    assert!(
        muster.teams.iter().all(|t| t.is_available()),
        "both teams are home and stay home: the job is done, so the host proposes nothing"
    );
}

/// The claim marker means "the host is driving this COMMITTED team", and nothing
/// else: a team out on work the host would not have chosen — work that is not in
/// its candidate pool — is never adopted and never recalled by it.
#[test]
fn the_host_never_recalls_a_team_on_work_outside_its_pool() {
    // No targets at all, so anything committed has no job left in the pool —
    // exactly the condition the recall arm fires on.
    let (mut app, operator) = host_only_app(&[]);
    {
        let mut ship = app.world_mut().entity_mut(operator);
        let mut muster = ship
            .get_mut::<ShipSecurityTeams>()
            .expect("the operator musters Security teams");
        muster.teams[1].deploy(TARGET.to_string(), SecurityAction::Board, 0.5, 2.0);
    }
    app.update();
    assert!(
        host_payloads(&app, operator).is_empty(),
        "team 1 is on work outside the pool; the host adopts no claim on it and leaves it alone"
    );
    assert!(
        app.world()
            .entity(operator)
            .get::<SecurityAiDispatched>()
            .is_none_or(|claim| !claim.holds(1)),
        "and the adoption pass does not invent a claim on it either"
    );

    // The same team, but claimed by the host: now it comes home.
    app.world_mut()
        .entity_mut(operator)
        .insert(SecurityAiDispatched(1 << 1));
    app.update();
    assert_eq!(
        host_payloads(&app, operator),
        vec![SystemControlPayload::RecallSecurityTeam { team_idx: 1 }],
        "a team the host committed is recalled when its job leaves the pool"
    );
}

/// What makes the claim marker genuinely DERIVED, and therefore honest to
/// exclude from the authoritative digest and from the snapshot: the host
/// re-derives it.
///
/// A restore is the exact shape that proves it. `restore_entities` reseeds the
/// muster from `SecuritySaveState` — committed teams, their targets and their
/// clocks — and brings back NO `SecurityAiDispatched`, because nothing captures
/// one. Unless the host adopts that team back, it has forgotten the team is its
/// own and will leave it standing on finished work forever.
#[test]
fn a_restored_team_is_re_adopted_within_one_ai_tick_and_still_recalled() {
    let (mut app, operator) =
        host_only_app(&[(TARGET, Vec3::new(100.0, 0.0, 0.0), head_target_config())]);

    // The post-restore shape, built through the real save/restore seam rather
    // than by hand, so it cannot drift from what a resume actually produces.
    let save = SecuritySaveState {
        teams: vec![SecurityTeamSaveState {
            team_idx: 0,
            state: SecurityTeamState::Working,
            target: Some(TARGET.to_string()),
            action: Some(SecurityAction::AssistEvacuation),
            risk: 0.6,
            elapsed: 1.0,
            phase_duration: 4.0,
        }],
    };
    {
        let mut ship = app.world_mut().entity_mut(operator);
        ship.get_mut::<ShipSecurityTeams>()
            .expect("the operator musters Security teams")
            .restore(&save);
    }
    assert!(
        app.world()
            .entity(operator)
            .get::<SecurityAiDispatched>()
            .is_none(),
        "a restore brings the committed team back and no claim marker with it"
    );

    // One AI tick. The team's job is still in the pool, so there is nothing to
    // send and nothing to recall — but the host recognises the work as its own.
    app.update();
    assert!(
        host_payloads(&app, operator).is_empty(),
        "the restored team is already on the only job there is"
    );
    assert_eq!(
        app.world()
            .entity(operator)
            .get::<SecurityAiDispatched>()
            .copied(),
        Some(SecurityAiDispatched(1)),
        "the lost marker healed within one AI tick: team 0 is the host's again"
    );

    // The work's authored consequence lands, so the job leaves the pool. The
    // re-adopted team comes home — which is the behaviour the marker exists for
    // and the behaviour a restore used to lose.
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .flags
        .set_flag(EVACUATED);
    app.update();
    assert_eq!(
        host_payloads(&app, operator),
        vec![SystemControlPayload::RecallSecurityTeam { team_idx: 0 }],
        "the re-adopted team is recalled when its job leaves the pool"
    );
    assert!(
        app.world()
            .entity(operator)
            .get::<SecurityAiDispatched>()
            .copied()
            .is_some_and(|claim| !claim.holds(0)),
        "and the claim is let go with the recall"
    );
}

/// A team that came home is no longer the host's. If the claim bit outlived the
/// assignment, the next console dispatch of that team would be recalled by the
/// host on the following tick.
#[test]
fn the_host_lets_go_of_a_team_the_moment_it_is_home() {
    let (mut app, operator) = host_only_app(&[]);
    app.world_mut()
        .entity_mut(operator)
        .insert(SecurityAiDispatched(0b11));
    app.update();
    assert_eq!(
        app.world()
            .entity(operator)
            .get::<SecurityAiDispatched>()
            .copied(),
        Some(SecurityAiDispatched(0)),
        "both teams are at home, so the host holds neither"
    );
}

/// The mission's one contribution, through the adapter: a live `Secure` directive
/// naming a target BY NAME is resolved to the uuid the world carries, and promotes
/// that target's authored work above a job the tiebreak would otherwise serve
/// first.
#[test]
fn a_secure_directive_names_a_target_by_name_and_promotes_its_authored_work() {
    // Two equally optional jobs; on the uuid tiebreak alone `alpha` wins.
    let (mut app, operator) = host_only_app(&[
        (
            "alpha",
            Vec3::new(50.0, 0.0, 0.0),
            optional_target_config(SecurityAction::Board),
        ),
        (
            "zulu",
            Vec3::new(60.0, 0.0, 0.0),
            optional_target_config(SecurityAction::PlaceCharges),
        ),
    ]);

    // The world binds the authored NAME the directive uses to `zulu`'s uuid.
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .name_to_uuid
        .insert("world.test.derelict".to_string(), "zulu".to_string());

    // A scored Objective carrying a `Secure` directive for that name, relevant to
    // Security — the shape the phase-1b aggregator publishes.
    {
        let mut ship = app.world_mut().entity_mut(operator);
        let mut blackboards = ship
            .get_mut::<crate::server_app::ShipSystemBlackboards>()
            .expect("the operator carries a blackboard map");
        let viewscreen = crate::core::messages::ViewscreenBlackboard {
            scored_objectives: vec![crate::core::messages::ScoredObjective {
                id: "secure_the_derelict".into(),
                score: 40.0,
                directive: crate::core::messages::AiDirective::Secure {
                    target: "world.test.derelict".into(),
                },
                source: crate::core::messages::ObjectiveSource::Mission,
                relevance: vec![SystemAffinity::Security],
                snapshot: crate::core::messages::ObjectiveSnapshot {
                    id: "secure_the_derelict".into(),
                    text: "world.test.objective.secure".into(),
                    text_params: Default::default(),
                    mandatory: false,
                    status: crate::core::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::core::messages::ObjectiveSource::Mission,
                },
            }],
            ..Default::default()
        };
        blackboards.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(viewscreen),
        );
    }
    app.update();

    let payloads = host_payloads(&app, operator);
    assert!(
        matches!(
            payloads.first(),
            Some(SystemControlPayload::DispatchSecurityTeam { target, .. }) if target == "zulu"
        ),
        "the promoted target is served first, ahead of the uuid tiebreak: {payloads:?}"
    );
}

/// A Security System damaged out has no seat for the host to sit in either.
#[test]
fn a_disabled_security_system_silences_the_host() {
    use crate::ship::damage::SystemHull;

    let (mut app, operator) =
        host_only_app(&[(TARGET, Vec3::new(100.0, 0.0, 0.0), head_target_config())]);
    app.world_mut()
        .entity_mut(operator)
        .insert(EntitySystemHull(SystemHull::from_config(&[(
            security_system_id(),
            0.0,
        )])));
    app.update();
    assert!(
        host_payloads(&app, operator).is_empty(),
        "the teams are off the board; the host proposes nothing"
    );
}

// ── The shipped content (AC1, AC3) ───────────────────────────────────────────

/// AC1, read off the shipped hull rather than asserted about it: the Alliance
/// Destroyer exposes a Security System, Tactical owns it, it musters exactly two
/// independently-assigned teams, and there is no second gate on it — no power
/// group to brown out and nothing resembling a Duty Officer anywhere in the
/// authored block.
#[test]
fn the_alliance_destroyer_exposes_two_tactical_owned_security_teams() {
    let entity = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_destroyer.toml",
    )
    .expect("the shipped destroyer parses");

    let security = entity
        .security
        .as_ref()
        .expect("the destroyer authors a [security] table");
    assert_eq!(
        security.team_count, 2,
        "Falling Skyway asks for exactly two"
    );
    security.validate().expect("the authored terms are usable");

    let ship = entity
        .ship_config
        .as_ref()
        .expect("the destroyer authors a system topology");
    let system = ship
        .systems
        .iter()
        .find(|s| s.kind == crate::ship::system_registry::SECURITY_KIND)
        .expect("the destroyer authors a kind = \"security\" [[system]]");
    assert_eq!(
        system.station.as_ref().map(|s| s.0.as_str()),
        Some("tactical"),
        "Tactical owns Security on this hull"
    );
    assert_eq!(
        system.power_group, None,
        "Security is people: no power group to brown out"
    );
    // …and it does NOT buy the hull a damage compartment. `[[hull.system_hull]]`
    // is the pool `SystemHull::apply_damage` spreads incoming hull damage across,
    // so every box a hull authors is durability its combat ladder never priced —
    // the caveat the destroyer's own hull table already records about its three
    // coupling compartments. A fourth measurably moved a shipped balance run
    // (`probe_aggressor`, seed 3: 105 extra ticks of engagement, all of them
    // inside the cruiser's bow hold), so Security is authored the way `[repair]`
    // is on this hull — teams, a reach, and no compartment of its own. Asserted
    // rather than left silent because "no entry" is a DECISION here, and the next
    // hand to add one should have to come past this line and re-measure.
    //
    // The lever stays available and data-driven: `security_disabled` reads
    // `EntitySystemHull.tier_for("security")` for whatever hull DOES author a box,
    // and `a_disabled_security_system_makes_every_team_unavailable_and_ends_the_work`
    // proves the whole knocked-out path against one.
    assert!(
        entity
            .hull
            .as_ref()
            .is_some_and(|hull| !hull.system_hull.iter().any(|e| e.system_id.0 == "security")),
        "the destroyer's Security System must author no [[hull.system_hull]] box: a muster \
         space is not armour, and adding one silently widens this hull's damage pool"
    );
}

/// Issue #1389, read off the shipped hull the same way: the Alliance Cruiser
/// musters Security too, on the same terms and under the same seat. `kind =
/// "security"` was generic from the day it landed — nothing in the engine
/// mentions a hull — so this is the proof the claim was true, taken from the
/// second hull to make it rather than from the code that would have to change if
/// it were not.
///
/// The `[security]` table and its `[[system]]` partner PAIR: `load_entity_config`
/// refuses a hull that authors one without the other (and a `kind = "security"`
/// block with no station), so reaching the assertions below at all is that
/// validation passing on the shipped cruiser.
#[test]
fn the_alliance_cruiser_exposes_two_tactical_owned_security_teams() {
    let entity = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .expect("the shipped cruiser parses — which is the [security]/[[system]] pairing check");

    let security = entity
        .security
        .as_ref()
        .expect("the cruiser authors a [security] table");
    assert_eq!(
        security.team_count, 2,
        "the same two teams the destroyer musters"
    );
    security.validate().expect("the authored terms are usable");

    let ship = entity
        .ship_config
        .as_ref()
        .expect("the cruiser authors a system topology");
    let system = ship
        .systems
        .iter()
        .find(|s| s.kind == crate::ship::system_registry::SECURITY_KIND)
        .expect("the cruiser authors a kind = \"security\" [[system]]");
    assert_eq!(
        system.station.as_ref().map(|s| s.0.as_str()),
        Some("tactical"),
        "Tactical owns Security on this hull too"
    );
    assert_eq!(
        system.power_group, None,
        "Security is people: no power group to brown out"
    );
    // No damage compartment, for the reason spelled out on the destroyer above —
    // and with more force here, because THIS is the hull whose bow hold moved a
    // shipped balance run when a fourth box was added. `npm run balance:cruiser`
    // is a blocking gate on this hull's ladder; a box authored here would move
    // the destroyed sum it measures.
    assert!(
        entity
            .hull
            .as_ref()
            .is_some_and(|hull| !hull.system_hull.iter().any(|e| e.system_id.0 == "security")),
        "the cruiser's Security System must author no [[hull.system_hull]] box: a muster \
         space is not armour, and adding one silently widens this hull's damage pool"
    );
}

/// AC3, read off the shipped world: Falling Skyway authors a complete Security
/// path — a target, an action from the required vocabulary, an authored duration,
/// risk and priority, and a `outcome_flag` the scenario script hangs its
/// consequence off. The success/interruption/invalid-target behaviours those
/// numbers drive are proved above; this asserts the content exists and is
/// coherent, so the two halves cannot drift apart.
#[test]
fn falling_skyway_authors_a_complete_security_path_with_a_scripted_consequence() {
    let source = include_str!("../../assets/worlds/falling_skyway.toml");
    let world = crate::world::config::parse_world(source).expect("the shipped world parses");

    let head = world
        .entities
        .iter()
        .find(|e| e.id.as_deref() == Some("skyhook"))
        .expect("the tether head is in the world");
    let overrides = head
        .overrides
        .as_ref()
        .expect("the head carries instance overrides");
    let authored = overrides
        .get("security_target")
        .expect("the head authors [security_target]");
    let target: SecurityTargetConfig = authored
        .clone()
        .try_into()
        .expect("the authored Security table is well formed");
    target.validate().expect("the authored actions are usable");

    let evacuation = target
        .action(SecurityAction::AssistEvacuation)
        .expect("the head offers the evacuation-assistance action");
    assert_eq!(
        evacuation.priority,
        SecurityPriority::LifeSafety,
        "people on a slipping tether are the top of the backfill's order"
    );
    assert!(evacuation.duration_secs > 0.0);
    assert!((0.0..=1.0).contains(&evacuation.risk));
    let flag = evacuation
        .outcome_flag
        .as_deref()
        .expect("the action raises a consequence flag");
    assert!(
        source.contains(&format!("on_flag_set(\"{flag}\"")),
        "the scenario script must hang a beat off the outcome flag '{flag}' — that is the whole \
         consequence path"
    );

    // The second, lower-priority job the trade is against.
    let ladder = world
        .entities
        .iter()
        .find(|e| e.id.as_deref() == Some("depot_ladder_b"))
        .and_then(|e| e.overrides.as_ref())
        .and_then(|o| o.get("security_target"))
        .expect("Ladder B authors [security_target] too");
    let ladder: SecurityTargetConfig = ladder
        .clone()
        .try_into()
        .expect("Ladder B's table is well formed");
    ladder.validate().expect("valid");
    assert_eq!(
        ladder
            .action(SecurityAction::SecureContain)
            .expect("Ladder B offers containment")
            .priority,
        SecurityPriority::ThreatContainment,
        "containment sits below life safety, so one free team is held for the head"
    );
}

// ── The backfill host's candidate pool (AC4) ─────────────────────────────────

fn rows() -> Vec<TargetRow> {
    vec![
        TargetRow {
            uuid: "fire".into(),
            name: None,
            position: Vec3::new(100.0, 0.0, 0.0),
            config: SecurityTargetConfig {
                actions: vec![SecurityActionConfig {
                    action: SecurityAction::AssistEvacuation,
                    duration_secs: 20.0,
                    risk: 0.5,
                    priority: SecurityPriority::LifeSafety,
                    outcome_flag: None,
                    outcome_value: 1,
                    warning: None,
                }],
            },
        },
        TargetRow {
            uuid: "salvage".into(),
            name: None,
            position: Vec3::new(200.0, 0.0, 0.0),
            config: SecurityTargetConfig {
                actions: vec![SecurityActionConfig {
                    action: SecurityAction::Board,
                    duration_secs: 10.0,
                    risk: 0.2,
                    priority: SecurityPriority::Optional,
                    outcome_flag: None,
                    outcome_value: 1,
                    warning: None,
                }],
            },
        },
    ]
}

/// Every action every target offers is a candidate, and eligibility is the
/// authored reach — nothing else. The host proposes only what the applier admits.
#[test]
fn the_candidate_pool_is_every_authored_action_gated_on_the_authored_reach() {
    let candidates = build_candidates(&rows(), Vec3::ZERO, 150.0, &[], None);
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].target, "fire");
    assert!(
        candidates[0].eligible,
        "100 units is inside a 150-unit reach"
    );
    assert!(
        !candidates[1].eligible,
        "200 units is outside it, so the host never proposes it"
    );
    assert_eq!(candidates[0].priority, SecurityPriority::LifeSafety);
    assert_eq!(candidates[1].priority, SecurityPriority::Optional);
}

/// The one thing a mission contributes: urgency. A `Secure` directive naming a
/// target promotes its authored work to `urgent_objective` — and never below what
/// it already was.
#[test]
fn an_ordered_target_is_promoted_to_urgent_but_life_safety_is_never_demoted() {
    let ordered = vec!["salvage".to_string()];
    let candidates = build_candidates(&rows(), Vec3::ZERO, 400.0, &ordered, None);
    assert_eq!(
        candidates[1].priority,
        SecurityPriority::UrgentObjective,
        "the mission naming the derelict lifts optional salvage above containment"
    );
    assert_eq!(
        candidates[0].priority,
        SecurityPriority::LifeSafety,
        "and leaves the fire where it was"
    );

    let both = vec!["fire".to_string(), "salvage".to_string()];
    let candidates = build_candidates(&rows(), Vec3::ZERO, 400.0, &both, None);
    assert_eq!(candidates[0].priority, SecurityPriority::LifeSafety);
}

/// The whole backfill policy in one pass: with both jobs in reach and two teams,
/// life safety goes first; with one team and the fire out of reach, the team is
/// held rather than spent on the salvage.
#[test]
fn the_host_works_life_safety_first_and_reserves_for_a_blocked_higher_priority_job() {
    let candidates = build_candidates(&rows(), Vec3::ZERO, 400.0, &[], None);
    let picks = select_assignments(&candidates, 2);
    assert_eq!(
        picks
            .iter()
            .map(|i| candidates[*i].target.as_str())
            .collect::<Vec<_>>(),
        vec!["fire", "salvage"]
    );

    // The fire out of reach, the salvage in it, one team left.
    let far = vec![
        TargetRow {
            position: Vec3::new(9000.0, 0.0, 0.0),
            ..rows().remove(0)
        },
        rows().remove(1),
    ];
    let candidates = build_candidates(&far, Vec3::ZERO, 400.0, &[], None);
    assert!(
        select_assignments(&candidates, 1).is_empty(),
        "the last team is kept for the fire rather than spent on optional salvage"
    );
    assert_eq!(
        select_assignments(&candidates, 2).len(),
        1,
        "with two teams one can be spent and one still held"
    );
}

/// Work whose authored consequence has already landed is not work: the raised
/// `outcome_flag` takes it out of the pool, which is what stops the host
/// re-dispatching a finished job on every tick forever. An action that authors no
/// flag has no observable completion and stays in.
#[test]
fn an_action_whose_outcome_flag_is_already_up_leaves_the_pool() {
    let mut flagged = rows();
    flagged[0].config.actions[0].outcome_flag = Some("fire_out".to_string());

    let mut flags = crate::world::flags::FlagStore::new();
    assert_eq!(
        build_candidates(&flagged, Vec3::ZERO, 400.0, &[], Some(&flags)).len(),
        2,
        "nothing raised yet, so nothing is finished"
    );

    flags.set_flag("fire_out");
    let candidates = build_candidates(&flagged, Vec3::ZERO, 400.0, &[], Some(&flags));
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].target, "salvage",
        "the finished job is gone; the flagless one is untouched"
    );
}

#[test]
fn the_host_marker_tracks_teams_independently() {
    let mut claimed = SecurityAiDispatched::default();
    assert!(!claimed.holds(0));
    claimed.claim(1);
    assert!(claimed.holds(1) && !claimed.holds(0));
    claimed.claim(0);
    claimed.release(1);
    assert!(claimed.holds(0) && !claimed.holds(1));
}
