//! Ordinary joined local treatment, finite medication/energy and fresh complete
//! JSON continuation. Quiet one-host proof; not network or human-device acceptance.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    job::{JobOutcome, JobPhase},
    personnel::PersonnelCondition,
};
use project_phoenix::{
    console::repair::server::ShipRepairTeams,
    core::messages::{ClientMessage, StationId, SystemControlPayload as Control, SystemId},
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    server_app::LocalShip,
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const ENGINEER: &str = "local-medical-engineer";
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
        payment.amount = power.0.emitted_work(&id("repair"));
    }
}
fn paid(app: &App, actor: Entity) -> bool {
    let payment = app.world().resource::<Payment>();
    payment.actor == Some(actor)
        && payment.tick.and_then(|tick| tick.checked_add(1))
            == Some(app.world().resource::<SimTick>().0)
        && payment.amount > 0.0
}
fn teams(app: &App, actor: Entity) -> &ShipRepairTeams {
    app.world().get::<ShipRepairTeams>(actor).unwrap()
}
fn condition(app: &App, actor: Entity) -> PersonnelCondition {
    let uuid = &app
        .world()
        .get::<project_phoenix::entities::spawner::EntityUuid>(actor)
        .unwrap()
        .0;
    capture(app.world()).personnel.origins[uuid].state.officers["repair-chief"].condition
}
fn medication(app: &App, actor: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(actor)
        .unwrap()
        .0
        .capacity_stock("medical-supplies")
        .unwrap()
        .level
}
fn host() -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/local-medical-world.toml".into(),
        ship_path: "assets/entities/medical_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156221),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<Payment>();
    app.add_systems(
        FixedUpdate,
        observe_payment
            .in_set(project_phoenix::sim_sets::SimSet::Publish)
            .after_ignore_deferred(project_phoenix::sim_sets::FixedStep::TickRepairTeams),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    app
}
fn boot() -> (App, Entity) {
    let mut app = host();
    claim(&mut app, "engineering", ENGINEER);
    let actor = local(&mut app);
    for team_idx in 0..2 {
        control(
            &mut app,
            ENGINEER,
            "engineering",
            "repair",
            Control::RecallRepairTeam { team_idx },
        );
    }
    for _ in 0..1200 {
        if teams(&app, actor).0.free_team_indices(None).len() == 2 {
            break;
        }
        app.update();
    }
    assert_eq!(
        teams(&app, actor).0.free_team_indices(None).len(),
        2,
        "human takeover must recover initial ordinary Backfill commitment"
    );
    assert_eq!(
        condition(&app, actor),
        PersonnelCondition::Minor,
        "initial AI must not finish treatment before takeover"
    );
    assert_eq!(
        medication(&app, actor),
        2,
        "initial AI work must still be in transit before takeover"
    );
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
fn joined_treatment_spends_once_and_json360_preserves_job_patient_and_ordinary_backfill() {
    let (mut live, actor) = boot();
    // Correct Human authority cannot turn an incompatible mechanical team into medics.
    let before_jobs = teams(&live, actor).0.save_state();
    control(
        &mut live,
        ENGINEER,
        "engineering",
        "repair",
        Control::TreatDutyOfficer {
            officer: "repair-chief".into(),
            team_idx: 0,
        },
    );
    assert_eq!(teams(&live, actor).0.save_state(), before_jobs);
    assert_eq!(medication(&live, actor), 2);
    control(
        &mut live,
        ENGINEER,
        "engineering",
        "repair",
        Control::TreatDutyOfficer {
            officer: "repair-chief".into(),
            team_idx: 1,
        },
    );
    let original = teams(&live, actor)
        .0
        .work_list()
        .into_iter()
        .find(|job| job.team_idx == 1 && job.outcome.is_none())
        .expect("normal Human care command must commit its eligible actual team")
        .id;
    let mut actual_work = false;
    for _ in 0..1200 {
        let work = teams(&live, actor)
            .0
            .work_list()
            .into_iter()
            .find(|job| job.id == original)
            .unwrap();
        if work.phase == JobPhase::Working && paid(&live, actor) {
            actual_work = true;
            break;
        }
        if work.phase == JobPhase::Travelling {
            assert_eq!(
                medication(&live, actor),
                2,
                "travel cannot consume medication"
            );
        }
        live.update();
    }
    assert!(
        actual_work,
        "actual arrival and funded service work must precede care progress"
    );
    assert_eq!(medication(&live, actor), 1);
    assert_eq!(condition(&live, actor), PersonnelCondition::Minor);
    let saved = decode(&live);
    let actor_uuid = live
        .world()
        .get::<project_phoenix::entities::spawner::EntityUuid>(actor)
        .unwrap()
        .0
        .clone();
    let (mut resumed, other) = boot();
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..7 {
        let mut bad = saved.clone();
        bad.tick += 10;
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == actor_uuid)
            .unwrap();
        if fault == 0 {
            bad.personnel
                .origins
                .get_mut(&actor_uuid)
                .unwrap()
                .state
                .officers
                .get_mut("repair-chief")
                .unwrap()
                .condition = PersonnelCondition::Fit;
        } else if fault == 1 {
            bad.personnel.origins.remove(&actor_uuid);
        } else {
            let jobs = row.repair.as_mut().unwrap().jobs.as_mut().unwrap();
            if fault == 6 {
                let mut duplicate = jobs.teams[1].clone().unwrap();
                duplicate.id.0 += 1;
                jobs.next_id = duplicate.id.0 + 1;
                jobs.teams[0] = Some(duplicate);
            } else {
                let project_phoenix::jobs::repair::RepairActivity::Treatment {
                    phase, work, ..
                } = &mut jobs.teams[1].as_mut().unwrap().activity
                else {
                    panic!("actual Treatment frontier")
                };
                match fault {
                    2 => work.work_secs = 0.001,
                    3 => work.patient = "absent".into(),
                    4 => work.medication_used = false,
                    _ => {
                        work.completed = true;
                        *phase = project_phoenix::core::messages::TeamSlot::Returning {
                            remaining: 0.0,
                            system_id: None,
                            display_name: None,
                            queued_system_id: None,
                            queued_display_name: None,
                        };
                    }
                }
            }
        }
        let report = restore(resumed.world_mut(), &bad);
        assert!(
            !report.is_complete(),
            "actual care forgery {fault} must refuse: {report:?}"
        );
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    restore_actual(&mut resumed, &saved);
    let mut further_payment = false;
    for step in 0..360 {
        same(&live, &resumed, step);
        if step == 120 {
            for app in [&mut live, &mut resumed] {
                send(app, ENGINEER, ClientMessage::SetAfk { afk: true });
                steps(app, 4);
            }
            assert_eq!(
                teams(&live, actor)
                    .0
                    .work_list()
                    .into_iter()
                    .find(|job| job.team_idx == 1)
                    .unwrap()
                    .id,
                original
            );
        }
        live.update();
        resumed.update();
        further_payment |= paid(&live, actor);
    }
    same(&live, &resumed, 360);
    assert!(further_payment);
    for step in 0..1800 {
        same(&live, &resumed, step + 360);
        let item = teams(&live, actor)
            .0
            .work_list()
            .into_iter()
            .find(|job| job.id == original)
            .unwrap();
        if item.outcome.is_some() {
            break;
        }
        live.update();
        resumed.update();
    }
    let work = teams(&live, actor)
        .0
        .work_list()
        .into_iter()
        .find(|job| job.id == original)
        .unwrap();
    assert_eq!(work.outcome, Some(JobOutcome::Completed));
    assert_eq!(medication(&live, actor), 1);
    assert_eq!(condition(&live, actor), PersonnelCondition::Fit);
    assert_eq!(teams(&live, actor).0.free_team_indices(None).len(), 2);
    assert_eq!(
        live.world()
            .get::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .source_for(&id("repair")),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
    for app in [&mut live, &mut resumed] {
        send(app, ENGINEER, ClientMessage::SetAfk { afk: false });
        steps(app, 4);
    }
    same(&live, &resumed, 2200);
    assert_eq!(
        resumed
            .world()
            .get::<ShipSystemControlSources>(other)
            .unwrap()
            .0
            .source_for(&id("repair")),
        project_phoenix::ship::control_source::ControlSource::Human
    );
}

#[test]
fn ordinary_backfill_assigns_and_completes_actual_care_from_its_permitted_roster() {
    let mut app = host();
    let actor = local(&mut app);
    let mut actual_paid = false;
    let mut actual_job = None;
    for _ in 0..2400 {
        if let Some(job) = teams(&app, actor)
            .0
            .work_list()
            .into_iter()
            .find(|job| job.team_idx == 1)
        {
            actual_job = Some(job.id);
            actual_paid |= job.phase == JobPhase::Working && paid(&app, actor);
            if job.outcome == Some(JobOutcome::Completed) {
                break;
            }
        }
        app.update();
    }
    assert!(
        actual_job.is_some(),
        "ordinary Repair AI must select the actual available local patient"
    );
    assert!(
        actual_paid,
        "AI care must reach its physical service and buy real work"
    );
    assert_eq!(condition(&app, actor), PersonnelCondition::Fit);
    assert_eq!(medication(&app, actor), 1);
    assert_eq!(teams(&app, actor).0.free_team_indices(None).len(), 2);
    assert_eq!(
        teams(&app, actor)
            .0
            .work_list()
            .into_iter()
            .find(|job| Some(job.id) == actual_job)
            .unwrap()
            .outcome,
        Some(JobOutcome::Completed)
    );
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .source_for(&id("repair")),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
}
