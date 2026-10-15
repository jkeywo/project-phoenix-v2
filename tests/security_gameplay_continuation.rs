//! Real local Security controls, finite payment, ordinary Backfill and complete
//! JSON continuation. This is a quiet one-host tracer, not human/network acceptance.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{ClientMessage, StationId, SystemControlPayload as Control, SystemId},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    security::{SecurityTeamState, ShipSecurityTeams},
    server_app::LocalShip,
    ship::{access::InteriorAccess, power::ShipPowerSystem},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, server::WorldContentRuntime},
};
const TACTICAL: &str = "local-security-officer";
const ENGINEER: &str = "local-security-engineer";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn claim(app: &mut App, station: &str, token: &str) {
    let actor = local(app);
    let rating = app
        .world()
        .get::<ShipConfigComponent>(actor)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|row| row.id.0 == station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    for message in [
        ClientMessage::Identify {
            token: token.into(),
            name: token.into(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ] {
        send(app, token, message);
        steps(app, 4);
    }
}
fn control(app: &mut App, token: &str, station: &str, system: &str, payload: Control) {
    let actor = local(app);
    let authorised = |app: &App| {
        let sessions = app.world().resource::<Sessions>();
        sessions.0.holder_for_station(&StationId(station.into())) == Some(token)
            && project_phoenix::command_admission::policy::is_command_authorized(
                token,
                &id(system),
                &payload,
                app.world().get::<ShipSystemControlSources>(actor).unwrap(),
                sessions,
                &app.world().get::<ShipConfigComponent>(actor).unwrap().0,
                app.world().get::<HumanSeekingHosts>(actor),
            )
    };
    for _ in 0..120 {
        if authorised(app) {
            break;
        }
        app.update();
    }
    assert!(authorised(app), "actual {station} holder must own {system}");
    send(
        app,
        token,
        ClientMessage::ControlSystem {
            target: id(system),
            payload,
        },
    );
    steps(app, 4);
}
#[derive(Resource, Default)]
struct Payment {
    actor: Option<Entity>,
    tick: Option<u64>,
    amount: f64,
}
fn observe_payment(
    tick: Res<SimTick>,
    ships: Query<(Entity, &ShipPowerSystem), With<LocalShip>>,
    mut payment: ResMut<Payment>,
) {
    *payment = Payment::default();
    if let Ok((actor, power)) = ships.single() {
        payment.actor = Some(actor);
        payment.tick = Some(tick.0);
        payment.amount = power.0.emitted_work(&id("security"));
    }
}
fn paid(app: &App, actor: Entity) -> bool {
    let payment = app.world().resource::<Payment>();
    payment.actor == Some(actor)
        && payment.tick.and_then(|tick| tick.checked_add(1))
            == Some(app.world().resource::<SimTick>().0)
        && payment.amount > 0.0
}
fn teams(app: &App, actor: Entity) -> &ShipSecurityTeams {
    app.world().get::<ShipSecurityTeams>(actor).unwrap()
}
fn boot() -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/local-security-world.toml".into(),
        ship_path: "assets/entities/security_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156220),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<Payment>();
    app.add_systems(
        FixedUpdate,
        observe_payment
            .in_set(project_phoenix::sim_sets::SimSet::Publish)
            .after_ignore_deferred(project_phoenix::sim_sets::FixedStep::TickSecurityTeams),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    claim(&mut app, "tactical", TACTICAL);
    claim(&mut app, "engineering", ENGINEER);
    let actor = local(&mut app);
    for team_idx in 0..2 {
        control(
            &mut app,
            TACTICAL,
            "tactical",
            "security",
            Control::RecallSecurityTeam { team_idx },
        );
    }
    for _ in 0..1200 {
        if teams(&app, actor)
            .teams
            .iter()
            .all(|team| team.is_available())
        {
            break;
        }
        app.update();
    }
    assert!(
        teams(&app, actor)
            .teams
            .iter()
            .all(|team| team.is_available()),
        "human takeover must recover ordinary initial Backfill work"
    );
    control(
        &mut app,
        ENGINEER,
        "engineering",
        "weapons-access",
        Control::SetAccessSeal { sealed: true },
    );
    for _ in 0..300 {
        if app
            .world()
            .get::<InteriorAccess>(actor)
            .unwrap()
            .statuses()
            .iter()
            .any(|status| {
                status.system == id("weapons-access")
                    && status.phase == phoenix_model::interior::BoundaryPhase::Sealed
            })
        {
            break;
        }
        app.update();
    }
    assert!(app
        .world()
        .get::<InteriorAccess>(actor)
        .unwrap()
        .statuses()
        .iter()
        .any(|status| status.system == id("weapons-access")
            && status.phase == phoenix_model::interior::BoundaryPhase::Sealed));
    (app, actor)
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => a
            .keys()
            .chain(b.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .find_map(|key| {
                first_difference(
                    a.get(key).unwrap_or(&serde_json::Value::Null),
                    b.get(key).unwrap_or(&serde_json::Value::Null),
                    &format!("{path}.{key}"),
                )
            }),
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(index, (a, b))| first_difference(a, b, &format!("{path}[{index}]"))),
        _ => (a != b).then(|| format!("{path}: {a} vs {b}")),
    }
}
fn same(left: &App, right: &App, step: usize) {
    let a = serde_json::to_value(capture(left.world())).unwrap();
    let b = serde_json::to_value(capture(right.world())).unwrap();
    assert!(
        a == b,
        "full capture at {step}: {:?}",
        first_difference(&a, &b, "$")
    );
    assert_eq!(
        world_digest(left.world()),
        world_digest(right.world()),
        "full digest at {step}"
    );
}
fn decode(app: &App) -> PhoenixSnapshot {
    serde_json::from_slice(&serde_json::to_vec(&capture(app.world())).unwrap()).unwrap()
}
fn restore_actual(app: &mut App, state: &PhoenixSnapshot) {
    for _ in 0..240 {
        if ready_to_restore(app.world(), state) {
            break;
        }
        app.update();
    }
    assert!(ready_to_restore(app.world(), state));
    let report = restore(app.world_mut(), state);
    assert!(report.is_complete(), "{report:?}");
}

#[test]
fn joined_human_blocked_and_working_json360_preserve_one_paid_job_through_ordinary_backfill() {
    let (mut live, actor) = boot();
    control(
        &mut live,
        TACTICAL,
        "tactical",
        "security",
        Control::DispatchSecurityTeam {
            team_idx: 0,
            target: "compartment:weapons".into(),
            action: "secure_contain".into(),
        },
    );
    for _ in 0..600 {
        if teams(&live, actor).teams.navigation.as_ref().unwrap()[0].blocked {
            break;
        }
        live.update();
    }
    assert!(teams(&live, actor).teams.navigation.as_ref().unwrap()[0].blocked);
    assert_eq!(
        teams(&live, actor).teams[0].state,
        SecurityTeamState::Deploying
    );
    let original_job = teams(&live, actor).teams.work_list()[0].id;
    let blocked = decode(&live);
    let (mut resumed, _) = boot();
    assert!(ready_to_restore(resumed.world(), &blocked));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..7 {
        let mut bad = blocked.clone();
        bad.tick += 10;
        let state = bad
            .entities
            .iter_mut()
            .find(|row| row.security.is_some())
            .unwrap()
            .security
            .as_mut()
            .unwrap();
        match fault {
            0 => state.jobs.next_id = 0,
            1 => state.navigation = None,
            2 => {
                state.navigation.as_mut().unwrap().pop();
            }
            3 => state.get_mut(0).unwrap().begin_work(6.0),
            4 => state.get_mut(0).unwrap().target = Some("compartment:absent".into()),
            5 => state.jobs.teams[1] = state.jobs.teams[0].clone(),
            _ => {
                state.navigation.as_mut().unwrap()[0].location =
                    phoenix_model::interior::RouteLocation::External {
                        target: "nonexistent-carrier".into(),
                    }
            }
        }
        let report = restore(resumed.world_mut(), &bad);
        assert!(!report.is_complete());
        assert!(
            format!("{:?}", report.gaps).contains("Security"),
            "actual Security guard must refuse: {report:?}"
        );
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    restore_actual(&mut resumed, &blocked);
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
        assert!(
            !paid(&live, actor),
            "blocked route cannot buy Security work"
        );
    }
    same(&live, &resumed, 360);
    assert_eq!(teams(&live, actor).teams.work_list()[0].id, original_job);
    for app in [&mut live, &mut resumed] {
        control(
            app,
            ENGINEER,
            "engineering",
            "weapons-access",
            Control::SetAccessSeal { sealed: false },
        );
    }
    let mut working = false;
    for step in 0..1200 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
        if teams(&live, actor).teams[0].state == SecurityTeamState::Working && paid(&live, actor) {
            working = true;
            break;
        }
    }
    assert!(
        working,
        "actual access opening, route arrival and paid Security work must occur"
    );
    let work = decode(&live);
    drop(resumed);
    let (mut continued, continuation_actor) = boot();
    restore_actual(&mut continued, &work);
    let mut continued_payment = false;
    for step in 0..360 {
        same(&live, &continued, step);
        if step == 120 {
            for app in [&mut live, &mut continued] {
                send(app, TACTICAL, ClientMessage::SetAfk { afk: true });
                steps(app, 4);
            }
            assert_eq!(
                teams(&live, actor).teams.work_list()[0].id,
                original_job,
                "ordinary Backfill must retain the actual underway Job"
            );
        }
        live.update();
        continued.update();
        continued_payment |= paid(&live, actor);
    }
    same(&live, &continued, 360);
    assert!(continued_payment);
    for step in 0..3600 {
        same(&live, &continued, step);
        if teams(&live, actor)
            .teams
            .iter()
            .all(|team| team.is_available())
            && [
                "local_weapons_secured",
                "local_engineering_secured",
                "local_bridge_assisted",
            ]
            .iter()
            .all(|flag| {
                live.world()
                    .resource::<WorldContentRuntime>()
                    .flags
                    .flag(flag)
            })
        {
            break;
        }
        live.update();
        continued.update();
    }
    assert!(teams(&live, actor)
        .teams
        .iter()
        .all(|team| team.is_available()));
    for flag in [
        "local_weapons_secured",
        "local_engineering_secured",
        "local_bridge_assisted",
    ] {
        assert!(live
            .world()
            .resource::<WorldContentRuntime>()
            .flags
            .flag(flag));
    }
    assert_eq!(
        live.world()
            .get::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .source_for(&id("security")),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
    for app in [&mut live, &mut continued] {
        send(app, TACTICAL, ClientMessage::SetAfk { afk: false });
        steps(app, 4);
    }
    same(&live, &continued, 4000);
    assert_eq!(
        continued
            .world()
            .get::<ShipSystemControlSources>(continuation_actor)
            .unwrap()
            .0
            .source_for(&id("security")),
        project_phoenix::ship::control_source::ControlSource::Human
    );
}
