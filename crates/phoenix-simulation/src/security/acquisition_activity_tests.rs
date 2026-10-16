//! Structural Activity proof over accepted real World/domain source and actual
//! Security machinery. Incident/sensor preconditions are inherited explicitly;
//! this is not an encounter, paid-sensing, lobby or Patrol-return host proof.
use super::*;
use phoenix_model::activity::{ActivityExecutionIdentity, ActivityOutcome};

fn source(app: &mut App) {
    use crate::world::load::{load, LoadPolicy, LoadRequest, MemoryReader};
    let reader = MemoryReader::new([
        (
            "tests/fixtures/acquisition-htn-world.toml",
            include_str!("../../../../tests/fixtures/acquisition-htn-world.toml"),
        ),
        (
            "assets/worlds/domains/security-acquisition.toml",
            include_str!("../../../../assets/worlds/domains/security-acquisition.toml"),
        ),
    ]);
    let mut base = load(
        LoadRequest::new(
            "tests/fixtures/acquisition-htn-world.toml",
            &reader,
            &crate::world::script::load::NoSiblingScripts,
            LoadPolicy::Activate,
        ),
        &crate::entities::loader::FixtureTemplates,
        &crate::entities::include_resolve::FixtureFragments,
    )
    .expect("full accepted World/domain closure before machinery tracing")
    .config;
    // This reduced fixture uses the exact already-spawned stable identities.
    base.name_to_uuid
        .insert("operator".into(), "operator".into());
    base.name_to_uuid.insert("prize".into(), "prize".into());
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .gm_npc_doctrine_palette = base.gm_npc_doctrine_palette.clone();
    app.insert_resource(base);
    app.add_systems(Update, ack.after(publish_security_blackboard));
}
fn release(app: &mut App) {
    let tick = app.world().resource::<SimTick>().0;
    let base = app
        .world()
        .resource::<crate::world::config::WorldConfig>()
        .clone();
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut runtime = app
        .world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>();
    crate::world::planning::advance_for_actor(
        &mut runtime,
        &base,
        "operator",
        None,
        tick,
        |request| crate::world::planning::supports_recipe(request, &config).unwrap(),
    )
    .unwrap();
    drop(runtime);
    app.world_mut()
        .run_system_once(publish_security_blackboard)
        .unwrap();
}
fn ack(tick: Res<SimTick>, mut runtime: ResMut<crate::world::server::WorldContentRuntime>) {
    crate::world::activities::acknowledge_pending(&mut runtime, tick.0);
    // The imported source has no script calls. Use the same actual Situation
    // activation/result owner, with no manufactured physical completion.
    for activation in runtime.situations.ready(&runtime.situation_definitions) {
        assert!(activation.call.is_none());
        let definitions = runtime.situation_definitions.clone();
        runtime
            .situations
            .finish_activation(
                &activation,
                phoenix_sim_world::world::activities::ScenarioCallResult::Applied,
                tick.0,
                &definitions,
            )
            .unwrap();
    }
}
fn instance(app: &App) -> phoenix_sim_world::world::activities::SituationInstance {
    app.world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .situations
        .instances
        .values()
        .next()
        .unwrap()
        .clone()
}
fn planned_fixture() -> (App, Entity, Entity, String) {
    let (mut app, actor, target, contact) = fixture();
    source(&mut app);
    release(&mut app);
    (app, actor, target, contact)
}
#[test]
fn exact_planned_board_binds_before_work_then_acquires_and_settles_through_json1200() {
    let (mut app, actor, target, contact) = planned_fixture();
    let before = instance(&app);
    assert!(before.accepted.is_none() && before.achieved.is_none() && before.settled.is_none());
    dispatch(&mut app, actor, &contact, 1);
    step(&mut app);
    let accepted = instance(&app);
    let scope = accepted.accepted.unwrap();
    let job = app
        .world()
        .get::<ShipSecurityTeams>(actor)
        .unwrap()
        .teams
        .jobs
        .teams[1]
        .as_ref()
        .unwrap();
    assert_eq!(scope.physical_job(), Some(job.id));
    assert_eq!(job.activity.team.state, SecurityTeamState::Deploying);
    assert!(job.activity.team.elapsed < job.activity.team.phase_duration);
    assert!(
        accepted.achieved.is_none() && app.world().get::<VesselOperationalOwner>(target).is_none()
    );
    let mut partial = false;
    for _ in 0..900 {
        step(&mut app);
        let job = app
            .world()
            .get::<ShipSecurityTeams>(actor)
            .unwrap()
            .teams
            .jobs
            .teams[1]
            .as_ref()
            .unwrap();
        if job.activity.team.state == SecurityTeamState::Working
            && job.activity.team.elapsed > 0.0
            && job.activity.team.elapsed < job.activity.team.phase_duration
        {
            partial = true;
            break;
        }
    }
    assert!(partial, "actual paid partial Board job");
    let saved = crate::snapshot::capture(app.world());
    let saved: crate::snapshot::PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let (mut resumed, _, _, _) = planned_fixture();
    let restored = crate::snapshot::restore(resumed.world_mut(), &saved);
    assert!(restored.is_complete(), "{:?}", restored.gaps);
    assert_eq!(
        crate::snapshot::capture(app.world()),
        crate::snapshot::capture(resumed.world())
    );
    let mut witnessed = false;
    for _ in 0..1200 {
        step(&mut app);
        step(&mut resumed);
        let row = instance(&app);
        if row.achieved.is_some() && row.settled.is_none() {
            assert!(app.world().get::<VesselOperationalOwner>(target).is_some());
            assert_eq!(
                app.world().get::<ShipSecurityTeams>(actor).unwrap().teams[1].state,
                SecurityTeamState::Withdrawing
            );
            witnessed = true;
        }
        assert_eq!(
            crate::snapshot::capture(app.world()),
            crate::snapshot::capture(resumed.world())
        );
        assert_eq!(
            crate::sim_digest::world_digest(app.world()),
            crate::sim_digest::world_digest(resumed.world())
        );
    }
    assert!(
        witnessed,
        "ownership is useful before the same team is home"
    );
    let row = instance(&app);
    let achieved = row.achieved.as_ref().unwrap();
    let settled = row.settled.as_ref().unwrap();
    assert!(matches!(
        scope.identity,
        ActivityExecutionIdentity::Physical { .. }
    ));
    assert_eq!(achieved.scope, scope);
    assert_eq!(settled.scope, scope);
    assert!(achieved.tick < settled.tick);
    assert!(matches!(
        achieved.outcome,
        ActivityOutcome::VesselAcquired { .. }
    ));
    assert!(matches!(
        settled.outcome,
        ActivityOutcome::SecuritySettled {
            outcome: phoenix_model::job::JobOutcome::Completed,
            ..
        }
    ));
    assert!(row.state.is_none());
    let completed = crate::snapshot::capture(app.world());
    for fault in 0..4 {
        let mut bad = completed.clone();
        let row = bad
            .scenario
            .as_mut()
            .unwrap()
            .situations
            .instances
            .values_mut()
            .next()
            .unwrap();
        match fault {
            0 => row.accepted_tick = Some(row.armed_tick + 500),
            1 => {
                if let ActivityOutcome::VesselAcquired { ownership, .. } =
                    &mut row.achieved.as_mut().unwrap().outcome
                {
                    ownership.controller = "foreign".into();
                }
            }
            2 => {
                if let ActivityOutcome::SecuritySettled { vessel, .. } =
                    &mut row.settled.as_mut().unwrap().outcome
                {
                    *vessel = "foreign".into();
                }
            }
            _ => row.achieved = None,
        }
        let before = crate::snapshot::capture(app.world());
        let digest = crate::sim_digest::world_digest(app.world());
        let report = crate::snapshot::restore(app.world_mut(), &bad);
        assert!(!report.is_complete());
        assert_eq!(crate::snapshot::capture(app.world()), before);
        assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
    }
}
#[test]
fn ordinary_ai_uses_own_released_intent_for_exact_team_and_no_retroactive_credit() {
    let (mut app, actor, _target, _) = planned_fixture();
    app.init_resource::<crate::lobby::Sessions>();
    app.add_systems(
        Update,
        crate::security::server::operate_security_ai.before(handle_security_commands),
    );
    step(&mut app);
    let row = instance(&app);
    let scope = row.accepted.expect("ordinary AI actual admitted work");
    let teams = &app.world().get::<ShipSecurityTeams>(actor).unwrap().teams;
    assert_eq!(
        teams.jobs.teams[1].as_ref().unwrap().id,
        scope.physical_job().unwrap()
    );
    assert!(teams[0].is_available());
    let (mut earlier, actor, target, contact) = fixture();
    source(&mut earlier);
    dispatch(&mut earlier, actor, &contact, 0);
    for _ in 0..900 {
        step(&mut earlier);
        if earlier
            .world()
            .get::<VesselOperationalOwner>(target)
            .is_some()
            && earlier
                .world()
                .get::<ShipSecurityTeams>(actor)
                .unwrap()
                .teams[0]
                .is_available()
        {
            break;
        }
    }
    assert!(earlier
        .world()
        .get::<VesselOperationalOwner>(target)
        .is_some());
    let mint = earlier
        .world()
        .get::<ShipSecurityTeams>(actor)
        .unwrap()
        .teams
        .jobs
        .next_id;
    release(&mut earlier);
    for _ in 0..8 {
        step(&mut earlier);
    }
    let row = instance(&earlier);
    assert!(
        row.accepted.is_none()
            && row.accepted_tick.is_none()
            && row.achieved.is_none()
            && row.settled.is_none()
    );
    assert_eq!(
        earlier
            .world()
            .get::<ShipSecurityTeams>(actor)
            .unwrap()
            .teams
            .jobs
            .next_id,
        mint
    );
    let boards = &earlier
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(actor)
        .unwrap()
        .0;
    let SystemBlackboard::Security(board) = &boards[&security_system_id()] else {
        panic!("ownSecurity");
    };
    assert!(board.planned_acquisitions.is_empty());
    assert!(earlier
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .commitments
        .is_empty());
}
