//! Actual admitted machinery results, Scenario effects and complete host resume.
//! The initial incident uses the existing physical Damage owner directly; this
//! probe does not claim weapon-hit or GM admission coverage.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    activity::ActivityOutcome,
    damage::{DamageHit, DamageProfile, HitSample, RepairWorkTarget},
};
use project_phoenix::{
    console::{repair::server::ShipRepairTeams, weapons::torpedo::TorpedoSystemResource},
    core::messages::{
        ClientMessage, RepairTarget, StationId, SystemControlPayload as Control, SystemId,
    },
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    resources::logistics_server::{CompiledShipLogistics, ShipLogistics},
    server_app::LocalShip,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::{with_event_generator, SimRng},
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, server::WorldContentRuntime},
};
use std::collections::BTreeSet;
const WORLD: &str = "tests/fixtures/activity-foundation-world.toml";
const HULL: &str = "assets/entities/alliance_cruiser.toml";
const TOKEN: &str = "activity-engineer";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
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
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn claim_messages(app: &mut App, station: &str, token: &str) -> Vec<ClientMessage> {
    let entity = local(app);
    // The authored ladder is most detailed first. Selecting complexity uses
    // the public command, including on fleet peers with a different default.
    let rating = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|entry| entry.id.0 == station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    vec![
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
    ]
}
fn claim(app: &mut App, station: &str, token: &str) {
    for message in claim_messages(app, station, token) {
        send(app, token, message);
        steps(app, 4);
    }
}
fn authorised(
    app: &mut App,
    token: &str,
    station: &str,
    target: &SystemId,
    payload: &Control,
) -> bool {
    let entity = local(app);
    let sessions = app.world().resource::<Sessions>();
    let Some(player) = sessions
        .0
        .players()
        .iter()
        .find(|player| player.token == token)
    else {
        return false;
    };
    player.connected
        && player.ready
        && player.station.as_ref() == Some(&StationId(station.into()))
        && sessions.0.holder_for_station(&StationId(station.into())) == Some(token)
        && project_phoenix::command_admission::policy::is_command_authorized(
            token,
            target,
            payload,
            app.world().get::<ShipSystemControlSources>(entity).unwrap(),
            sessions,
            &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
            app.world().get::<HumanSeekingHosts>(entity),
        )
}
fn control(app: &mut App, token: &str, station: &str, target: &str, payload: Control) {
    for _ in 0..120 {
        if authorised(app, token, station, &id(target), &payload) {
            break;
        }
        app.update();
    }
    assert!(
        authorised(app, token, station, &id(target), &payload),
        "actual {station} holder must own {target}"
    );
    send(
        app,
        token,
        ClientMessage::ControlSystem {
            target: id(target),
            payload,
        },
    );
    steps(app, 4);
}

/// Test-only observer of committed per-System payment on every actual fixed
/// interval, including intervals advanced inside ordinary command helpers.
/// Read-only to machinery/World; this witness is neither captured nor folded.
#[derive(Resource, Default, Debug)]
struct ActivityPaymentWitness {
    repair: Option<(u64, f64)>,
    delivery: Option<(u64, f64)>,
}
fn observe_activity_payment(
    tick: Res<project_phoenix::sim_tick::SimTick>,
    runtime: Res<WorldContentRuntime>,
    ships: Query<
        (
            &project_phoenix::entities::spawner::EntityUuid,
            &ShipPowerSystem,
        ),
        With<LocalShip>,
    >,
    mut witness: ResMut<ActivityPaymentWitness>,
) {
    let Ok((uuid, power)) = ships.single() else {
        return;
    };
    let witness = &mut *witness;
    for (owner, slot) in [
        ("repair", &mut witness.repair),
        ("logistics", &mut witness.delivery),
    ] {
        let amount = power.0.emitted_work(&id(owner));
        let bound = runtime.situations.instances.values().any(|row| {
            row.accepted
                .as_ref()
                .is_some_and(|accepted| accepted.actor == uuid.0 && accepted.owner == id(owner))
        });
        if bound && amount > 0.0 {
            *slot = Some((tick.0, amount));
        }
    }
}

fn boot() -> (App, Entity) {
    boot_world(WORLD)
}
fn boot_hull(hull: &str) -> (App, Entity) {
    boot_definition(WORLD, hull)
}
fn boot_world(path: &str) -> (App, Entity) {
    boot_definition(path, HULL)
}
fn boot_definition(path: &str, hull: &str) -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: path.into(),
        ship_path: hull.into(),
        deterministic: true,
        seed: Some(156222),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<ActivityPaymentWitness>();
    app.add_systems(
        FixedUpdate,
        observe_activity_payment
            .in_set(project_phoenix::sim_sets::SimSet::Physics)
            .after_ignore_deferred(project_phoenix::sim_sets::FixedStep::TickRepairTeams),
    );
    app.add_systems(
        FixedUpdate,
        observe_activity_payment.in_set(project_phoenix::sim_sets::SimSet::Publish),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    claim(&mut app, "engineering", TOKEN);
    let entity = local(&mut app);
    (app, entity)
}
fn runtime(app: &App) -> &WorldContentRuntime {
    app.world().resource::<WorldContentRuntime>()
}
fn hp(app: &App, actor: Entity) -> f64 {
    hp_at(app, actor, "engineering")
}
fn hp_at(app: &App, actor: Entity, room: &str) -> f64 {
    app.world()
        .get::<ShipDamage>(actor)
        .unwrap()
        .state
        .compartments[room]
        .hp
}
fn parts(app: &App, actor: Entity) -> i64 {
    let stock = &app.world().get::<InfrastructureCondition>(actor).unwrap().0;
    let profile = &app.world().get::<CompiledShipLogistics>(actor).unwrap().0;
    let freight = app
        .world()
        .get::<ShipLogistics>(actor)
        .unwrap()
        .0
        .batches
        .values()
        .filter(|batch| profile.store(&batch.source).unwrap().resource == "structural_parts")
        .map(|batch| batch.amount)
        .sum::<i64>();
    stock.capacity_stock("structural_parts").unwrap().level
        + stock.capacity_stock("hold-structural-parts").unwrap().level
        + freight
}
fn rounds(app: &App, actor: Entity) -> i64 {
    let torpedoes = &app.world().get::<TorpedoSystemResource>(actor).unwrap().0;
    let stock = &app.world().get::<InfrastructureCondition>(actor).unwrap().0;
    let profile = &app.world().get::<CompiledShipLogistics>(actor).unwrap().0;
    let freight = app
        .world()
        .get::<ShipLogistics>(actor)
        .unwrap()
        .0
        .batches
        .values()
        .filter(|batch| profile.store(&batch.source).unwrap().resource == "torpedoes")
        .map(|batch| batch.amount)
        .sum::<i64>();
    stock.capacity_stock("hold-ammunition").unwrap().level
        + torpedoes.magazine.level
        + freight
        + torpedoes
            .tubes
            .iter()
            .map(|tube| i64::from(tube.loaded_count))
            .sum::<i64>()
        + torpedoes
            .burst_states
            .iter()
            .map(|burst| i64::from(burst.pending))
            .sum::<i64>()
        + torpedoes.in_flight.len() as i64
}
fn incident(app: &mut App, actor: Entity) -> f64 {
    incident_at(app, actor, "engineering")
}
fn incident_at(app: &mut App, actor: Entity, room: &str) -> f64 {
    let room_name = room;
    let before = hp_at(app, actor, room);
    // An explicit initial owner-level incident, not forged HP/job/evidence.
    app.world_mut().resource_scope(|world, rng: Mut<SimRng>| {
        let mut damage = world.get_mut::<ShipDamage>(actor).unwrap();
        let deck = damage
            .definition
            .interior
            .compartments
            .iter()
            .find(|room| room.id == room_name)
            .unwrap()
            .deck
            .clone();
        let outcome = with_event_generator(Some(&rng), "activity22-probe-incident", 0, |rng| {
            damage.apply_hit(
                0.0,
                DamageHit {
                    incoming: [1.0, 0.0],
                    profile: DamageProfile::default(),
                    payload: 1.0,
                    aim: Some(room.into()),
                    source: "activity22-probe-incident".into(),
                    sample: Some(HitSample {
                        deck,
                        profile_fraction: 0.5,
                    }),
                },
                rng,
            )
        })
        .unwrap();
        assert!(outcome
            .deposits
            .iter()
            .any(|row| row.compartment == room && row.structural_loss > 0.0));
    });
    assert!(hp_at(app, actor, room) < before);
    before
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    use serde_json::Value;
    if a == b {
        return None;
    }
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                if let Some(diff) = first_difference(
                    a.get(key).unwrap_or(&Value::Null),
                    b.get(key).unwrap_or(&Value::Null),
                    &format!("{path}.{key}"),
                ) {
                    return Some(diff);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                if let Some(diff) = first_difference(a, b, &format!("{path}[{index}]")) {
                    return Some(diff);
                }
            }
        }
        _ => {}
    }
    let bounded = |value: &Value| value.to_string().chars().take(240).collect::<String>();
    Some(format!(
        "{path}: live={} restored={}",
        bounded(a),
        bounded(b)
    ))
}
fn assert_same(live: &App, resumed: &App, step: usize) {
    let a = serde_json::to_value(capture(live.world())).unwrap();
    let b = serde_json::to_value(capture(resumed.world())).unwrap();
    assert!(
        a == b,
        "complete capture at{step}: {}",
        first_difference(&a, &b, "$").unwrap_or_default()
    );
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "complete digest at{step}"
    );
}

#[test]
fn actual_repair_and_delivery_credit_once_across_partial_work_json_and_360_host_steps() {
    let (mut live, actor) = boot();
    let healthy = incident(&mut live, actor);
    let original_parts = parts(&live, actor);
    let original_rounds = rounds(&live, actor);
    control(
        &mut live,
        TOKEN,
        "engineering",
        "repair",
        Control::DispatchRepairTeam {
            team_idx: 0,
            target: RepairTarget::Work(RepairWorkTarget::Compartment {
                compartment: "engineering".into(),
            }),
        },
    );
    // Existing stock-maintenance AI may already have committed the declared
    // four-round batch before the player claimed Engineering. Otherwise the
    // same Human operator makes that ordinary request; demand is authored6.
    if runtime(&live).situations.instances["four-rounds"]
        .accepted
        .is_none()
    {
        control(
            &mut live,
            TOKEN,
            "engineering",
            "logistics",
            Control::DispatchDelivery {
                source: "hold-ammunition".into(),
                destination: "magazine".into(),
                amount: 4,
            },
        );
    }
    let mut paid_repair = false;
    let mut paid_delivery = false;
    for _ in 0..120 {
        let power = &live.world().get::<ShipPowerSystem>(actor).unwrap().0;
        paid_repair |= power.emitted_work(&id("repair")) > 0.0;
        paid_delivery |= power.emitted_work(&id("logistics")) > 0.0;
        let partial = live
            .world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .physical_work
            .values()
            .any(|work| work.reserved && work.progress > 0.0 && work.progress < 2.0);
        let work = &runtime(&live).situations.instances;
        let supply = work["four-rounds"].accepted.as_ref();
        if partial
            && supply.is_some_and(|scope| {
                live.world()
                    .get::<ShipLogistics>(actor)
                    .unwrap()
                    .0
                    .batches
                    .contains_key(&scope.physical_job().unwrap())
            })
        {
            break;
        }
        live.update();
    }
    let work = &runtime(&live).situations.instances;
    let repair = work["repair-room"]
        .accepted
        .clone()
        .expect("actual accepted repair JobId");
    let delivery = work["four-rounds"]
        .accepted
        .clone()
        .expect("actual committed delivery JobId");
    assert_eq!(
        live.world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .teams[0]
            .as_ref()
            .unwrap()
            .id,
        repair.physical_job().expect("accepted physical repair")
    );
    assert!(live
        .world()
        .get::<ShipLogistics>(actor)
        .unwrap()
        .0
        .batches
        .contains_key(&delivery.physical_job().expect("accepted physical delivery")));
    assert!(
        live.world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .physical_work
            .values()
            .any(|work| work.reserved && work.progress > 0.0 && work.progress < 2.0),
        "save during actual paid partial work"
    );
    assert_eq!(runtime(&live).flags.counter("repair_credit"), 0);
    assert_eq!(runtime(&live).flags.counter("delivery_credit"), 0);
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = serde_json::to_value(capture(resumed.world())).unwrap();
    for fault in 0..2 {
        let mut forged = saved.clone();
        let instances = &mut forged.scenario.as_mut().unwrap().situations.instances;
        if fault == 0 {
            instances.remove("repair-room");
        } else {
            instances.get_mut("repair-room").unwrap().accepted_tick = Some(saved.tick + 1);
        }
        assert!(
            !restore(resumed.world_mut(), &forged).is_complete(),
            "malformed activity frontier{fault}"
        );
        let after = serde_json::to_value(capture(resumed.world())).unwrap();
        assert!(
            before == after,
            "candidate refusal must not write: {:?}",
            first_difference(&before, &after, "$")
        );
    }
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    let mut recalled = false;
    for step in 0..360 {
        assert_same(&live, &resumed, step);
        assert_eq!(
            rounds(&live, actor),
            original_rounds,
            "actual cargo/round conservation at{step}"
        );
        let power = &live.world().get::<ShipPowerSystem>(actor).unwrap().0;
        paid_repair |= power.emitted_work(&id("repair")) > 0.0;
        paid_delivery |= power.emitted_work(&id("logistics")) > 0.0;
        if !recalled && runtime(&live).flags.counter("repair_credit") == 1 {
            // Useful work has already committed, but the assignment is not
            // settled until this actual recall returns the same team.
            assert!(runtime(&live).situations.instances["repair-room"]
                .settled
                .is_none());
            for app in [&mut live, &mut resumed] {
                assert!(authorised(
                    app,
                    TOKEN,
                    "engineering",
                    &id("repair"),
                    &Control::RecallRepairTeam { team_idx: 0 }
                ));
                send(
                    app,
                    TOKEN,
                    ClientMessage::ControlSystem {
                        target: id("repair"),
                        payload: Control::RecallRepairTeam { team_idx: 0 },
                    },
                );
            }
            recalled = true;
        }
        live.update();
        resumed.update();
    }
    assert_same(&live, &resumed, 360);

    assert!(
        paid_repair && paid_delivery,
        "both ordinary consumers must actually spend finite energy"
    );
    assert!(recalled);
    assert_eq!(hp(&live, actor), healthy);
    assert_eq!(hp(&resumed, restored), healthy);
    assert_eq!(
        parts(&live, actor),
        original_parts - 1,
        "completed remedy consumes its real one part"
    );
    let world = runtime(&live);
    assert_eq!(world.flags.counter("repair_credit"), 1);
    assert_eq!(world.flags.counter("return_credit"), 1);
    assert_eq!(world.flags.counter("delivery_credit"), 1);
    assert_eq!(world.flags.counter("forbidden_credit"), 0);
    let repair = &world.situations.instances["repair-room"];
    assert!(matches!(
        repair.settled.as_ref().unwrap().outcome,
        ActivityOutcome::RepairSettled {
            outcome: phoenix_model::job::JobOutcome::Cancelled
        }
    ));
    assert_eq!(
        serde_json::to_value(repair.activations["useful"]).unwrap(),
        serde_json::json!("applied")
    );
    assert_eq!(
        serde_json::to_value(repair.activations["returned"]).unwrap(),
        serde_json::json!("applied")
    );
    let delivery = &world.situations.instances["four-rounds"];
    assert!(matches!(
        delivery.settled.as_ref().unwrap().outcome,
        ActivityOutcome::Delivery {
            returned: false,
            amount: 4,
            ..
        }
    ));
    assert_eq!(
        serde_json::to_value(delivery.activations["received"]).unwrap(),
        serde_json::json!("applied")
    );
    assert_eq!(
        serde_json::to_value(delivery.activations["refused"]).unwrap(),
        serde_json::json!("refused")
    );
    assert!(repair.state.is_none() && delivery.state.is_none());
}

/// Exercises the actual optional Personnel hull, not a fabricated officer or
/// work-multiplier component. Both ships have identical incidents and paid
/// machinery; the control ship leaves its same authored officer unposted.
#[test]
fn actual_officer_posting_improves_paid_repair_and_preserves_custody_through_json() {
    use phoenix_model::personnel::OfficerLocation;
    const PERSONNEL: &str = "assets/entities/personnel_cruiser.toml";
    let (mut live, actor) = boot_hull(PERSONNEL);
    let (mut ordinary, ordinary_actor) = boot_hull(PERSONNEL);
    for (app, posting) in [(&mut live, Some(0)), (&mut ordinary, None)] {
        control(
            app,
            TOKEN,
            "engineering",
            "repair",
            Control::SetDutyOfficerPosting {
                officer: "repair-chief".into(),
                team_idx: posting,
            },
        );
    }
    assert_eq!(capture(live.world()).tick, capture(ordinary.world()).tick);
    let healthy = incident(&mut live, actor);
    assert_eq!(incident(&mut ordinary, ordinary_actor), healthy);
    let original_parts = parts(&live, actor);
    for app in [&mut live, &mut ordinary] {
        control(
            app,
            TOKEN,
            "engineering",
            "repair",
            Control::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Work(RepairWorkTarget::Compartment {
                    compartment: "engineering".into(),
                }),
            },
        );
    }
    let progress = |app: &App, entity| {
        app.world()
            .get::<ShipRepairTeams>(entity)
            .unwrap()
            .0
            .job_state()
            .physical_work
            .values()
            .filter(|work| work.reserved)
            .map(|work| work.progress)
            .sum::<f32>()
    };
    let mut observed_bonus = false;
    for _ in 0..120 {
        let boosted = progress(&live, actor);
        let baseline = progress(&ordinary, ordinary_actor);
        if boosted > 0.0 && baseline > 0.0 {
            assert!(
                boosted > baseline,
                "real expertise must advance actual work: {boosted}/{baseline}"
            );
            assert!((boosted / baseline - 1.25).abs() < 0.01);
            assert!(
                live.world()
                    .get::<ShipPowerSystem>(actor)
                    .unwrap()
                    .0
                    .emitted_work(&id("repair"))
                    > 0.0
            );
            observed_bonus = true;
            break;
        }
        live.update();
        ordinary.update();
    }
    assert!(
        observed_bonus,
        "both ordinary teams must reach actual paid partial work"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let officer_row = saved
        .entities
        .iter()
        .position(|row| row.personnel.is_some())
        .unwrap();
    assert_eq!(
        saved.entities[officer_row]
            .personnel
            .as_ref()
            .unwrap()
            .officers["repair-chief"]
            .location,
        OfficerLocation::WithRepairTeam { team_idx: 0 }
    );
    let (mut resumed, restored) = boot_hull(PERSONNEL);
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest_before = world_digest(resumed.world());
    for fault in 0..3 {
        let mut forged = saved.clone();
        let row = &mut forged.entities[officer_row];
        match fault {
            0 => row.personnel = None,
            1 => {
                row.personnel
                    .as_mut()
                    .unwrap()
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .changed_tick = saved.tick + 1
            }
            _ => {
                row.personnel
                    .as_mut()
                    .unwrap()
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .location = OfficerLocation::WithRepairTeam { team_idx: 99 }
            }
        }
        assert!(
            !restore(resumed.world_mut(), &forged).is_complete(),
            "forged personnel{fault}"
        );
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest_before);
    }
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    let mut recalled = false;
    for step in 0..360 {
        assert_same(&live, &resumed, step);
        if !recalled && hp(&live, actor) == healthy {
            for app in [&mut live, &mut resumed] {
                send(
                    app,
                    TOKEN,
                    ClientMessage::ControlSystem {
                        target: id("repair"),
                        payload: Control::RecallRepairTeam { team_idx: 0 },
                    },
                );
            }
            recalled = true;
        }
        live.update();
        resumed.update();
    }
    assert_same(&live, &resumed, 360);
    assert!(recalled);
    assert_eq!(hp(&live, actor), healthy);
    assert_eq!(hp(&resumed, restored), healthy);
    assert_eq!(parts(&live, actor), original_parts - 1);
    // A normal release is allowed only after the actual team returns home.
    for app in [&mut live, &mut resumed] {
        control(
            app,
            TOKEN,
            "engineering",
            "repair",
            Control::SetDutyOfficerPosting {
                officer: "repair-chief".into(),
                team_idx: None,
            },
        );
    }
    assert_same(&live, &resumed, 364);
    let final_snapshot = capture(live.world());
    assert_eq!(
        final_snapshot.entities[officer_row]
            .personnel
            .as_ref()
            .unwrap()
            .officers["repair-chief"]
            .location,
        OfficerLocation::AtCompartment {
            compartment: "engineering".into()
        }
    );
}

/// The imported plan does not issue physical commands. This probe deliberately
/// uses ordinary Human Repair dispatch, then ordinary Logistics AI after a real
/// Gunner demand and Engineering Backfill handoff. Initial damage uses the same
/// explicit Damage owner seam as the 22A probe above, without forged facts/jobs.
#[test]
fn imported_alternative_partial_order_runs_actual_human_and_ai_jobs_with_1200_host_resume() {
    const HTN_WORLD: &str = "tests/fixtures/maintenance-htn-host-world.toml";
    let (mut live, actor) = boot_world(HTN_WORLD);
    claim(&mut live, "tactical", "htn-gunner");
    let healthy_engineering = incident_at(&mut live, actor, "engineering");
    let healthy_bridge = incident_at(&mut live, actor, "bridge");
    let original_parts = parts(&live, actor);
    let original_rounds = rounds(&live, actor);
    steps(&mut live, 4);
    let choice = runtime(&live).plan_choice.as_ref().unwrap();
    assert_eq!(choice.methods["root"].method, "two-real-teams");
    assert_eq!(
        choice.released.len(),
        2,
        "unordered Repair leaves arm together"
    );
    assert!(choice.released.contains_key("root/engineering"));
    assert!(choice.released.contains_key("root/bridge"));
    assert!(!choice.released.contains_key("root/supply"));
    let repair_ids =
        ["engineering", "bridge"].map(|node| format!("htn/maintenance-host-proof/root/{node}"));
    assert_eq!(runtime(&live).situations.instances.len(), 2);
    assert!(repair_ids
        .iter()
        .all(|key| runtime(&live).situations.instances.contains_key(key)));
    assert_eq!(repair_ids.len(), 2);
    for (team, room) in [(0, "engineering"), (1, "bridge")] {
        control(
            &mut live,
            TOKEN,
            "engineering",
            "repair",
            Control::DispatchRepairTeam {
                team_idx: team,
                target: RepairTarget::Work(RepairWorkTarget::Compartment {
                    compartment: room.into(),
                }),
            },
        );
    }
    let mut paid_repair = false;
    for _ in 0..180 {
        paid_repair |= live
            .world()
            .get::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .emitted_work(&id("repair"))
            > 0.0;
        let partial = live
            .world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .physical_work
            .values()
            .any(|work| work.reserved && work.progress > 0.0 && work.progress < 2.0);
        if partial
            && repair_ids
                .iter()
                .all(|key| runtime(&live).situations.instances[key].accepted.is_some())
        {
            break;
        }
        live.update();
    }
    assert!(
        live.world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .physical_work
            .values()
            .any(|work| work.reserved && work.progress > 0.0 && work.progress < 2.0),
        "save during paid reserved remedy work"
    );
    assert!(repair_ids
        .iter()
        .all(|key| runtime(&live).situations.instances[key].accepted.is_some()));
    assert_eq!(runtime(&live).flags.counter("htn_repair_credit"), 0);
    assert!(
        !runtime(&live)
            .plan_choice
            .as_ref()
            .unwrap()
            .released
            .contains_key("root/supply"),
        "acceptance alone cannot satisfy Achieved dependency"
    );
    assert!(
        live.world()
            .get::<ShipLogistics>(actor)
            .unwrap()
            .0
            .batches
            .values()
            .all(|batch| batch.original_destination != "magazine"),
        "no invented demand before normal Gunner request"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let saved_choice = saved.scenario.as_ref().unwrap().plan_choice.clone();
    let (mut resumed, restored) = boot_world(HTN_WORLD);
    claim(&mut resumed, "tactical", "htn-gunner");
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    let mut recalled = BTreeSet::new();
    let mut demand_sent = false;
    let mut paid_delivery = false;
    // Bridge travel alone crosses two authored 2.5-second links before its
    // two-second remedy. The former six-second continuation ended before that
    // team could achieve its goal, so the explicit Gunner request never ran.
    // Compare twenty simulated seconds, including real return and delivery work;
    // keep every physical outcome, energy, stock and once-only credit assertion.
    const CONTINUATION_STEPS: usize = 1200;
    for step in 0..CONTINUATION_STEPS {
        assert_same(&live, &resumed, step);
        assert_eq!(
            rounds(&live, actor),
            original_rounds,
            "finite round conservation at{step}"
        );
        assert_eq!(
            runtime(&live).plan_choice.as_ref().unwrap().methods,
            saved_choice.as_ref().unwrap().methods,
            "no live alternative reselection"
        );
        let power = &live.world().get::<ShipPowerSystem>(actor).unwrap().0;
        paid_repair |= power.emitted_work(&id("repair")) > 0.0;
        paid_delivery |= power.emitted_work(&id("logistics")) > 0.0;
        for key in &repair_ids {
            let instance = &runtime(&live).situations.instances[key];
            if instance.achieved.is_some() && !recalled.contains(key) {
                // These are the same two explicit Human requests sent above;
                // the World binding must already hold their actual accepted jobs.
                let team = if key.ends_with("/engineering") { 0 } else { 1 };
                assert!(instance.settled.is_none(), "achieved is not settled");
                for app in [&mut live, &mut resumed] {
                    control(
                        app,
                        TOKEN,
                        "engineering",
                        "repair",
                        Control::RecallRepairTeam {
                            team_idx: team as u8,
                        },
                    );
                }
                recalled.insert(key.clone());
            }
        }
        let all_achieved = repair_ids
            .iter()
            .all(|key| runtime(&live).situations.instances[key].achieved.is_some());
        if all_achieved && !demand_sent {
            let runtime = runtime(&live);
            let choice = runtime.plan_choice.as_ref().unwrap();
            let released = *choice
                .released
                .get("root/supply")
                .expect("acknowledged useful work releases dependent delivery");
            let predecessor = &runtime.situations.instances[&repair_ids[0]];
            assert!(released >= predecessor.achieved.as_ref().unwrap().tick);
            for app in [&mut live, &mut resumed] {
                control(
                    app,
                    "htn-gunner",
                    "tactical",
                    "torpedo-magazine",
                    Control::SetStockTarget {
                        store: "magazine".into(),
                        amount: 6,
                    },
                );
                send(
                    app,
                    TOKEN,
                    ClientMessage::SetStationRating {
                        rating_name: "Backfill".into(),
                    },
                );
                steps(app, 4);
            }
            demand_sent = true;
        }
        live.update();
        resumed.update();
    }
    assert_same(&live, &resumed, CONTINUATION_STEPS);
    if !paid_delivery
        && live
            .world()
            .resource::<ActivityPaymentWitness>()
            .delivery
            .is_none()
    {
        // Read-only failure evidence from the ordinary owner, its exact permitted
        // publication and actual World frontiers. No dispatch or work is seeded.
        let logistics = &live.world().get::<ShipLogistics>(actor).unwrap().0;
        let owner = &logistics.owner;
        let sources = &live
            .world()
            .get::<ShipSystemControlSources>(actor)
            .unwrap()
            .0;
        let config = &live.world().get::<ShipConfigComponent>(actor).unwrap().0;
        let boards = &live
            .world()
            .get::<project_phoenix::server_app::ShipSystemBlackboards>(actor)
            .unwrap()
            .0;
        let permitted = config.observations_for_system(owner, boards);
        let snapshot = capture(live.world());
        let actor_row = snapshot
            .entities
            .iter()
            .find(|row| row.uuid == runtime(&live).plan_choice.as_ref().unwrap().actor_uuid)
            .unwrap();
        let tree = live
            .world()
            .get::<project_phoenix::ai::state_tree_runtime::ShipAiTreeRuntime>(actor)
            .and_then(|trees| trees.0.get(owner));
        let diagnostic = serde_json::json!({
            "actual_repair": actor_row.repair,
            "tick": snapshot.tick,
            "demand_sent": demand_sent,
            "owner": owner,
            "source": format!("{:?}", sources.source_for(owner)),
            "has_actual_source": sources.entries().any(|(id, _)| id == owner),
            "host": project_phoenix::command_admission::policy::station_for_system(config, live.world().get::<HumanSeekingHosts>(actor), owner),
            "ratings": live.world().get::<project_phoenix::ship_plugin::ActiveStationRatings>(actor).map(|ratings| &ratings.0),
            "enabled": logistics.enabled,
            "goals": logistics.goals,
            "replenishing": logistics.replenishing,
            "jobs": actor_row.logistics,
            "power": {
                "repair_work": live.world().get::<ShipPowerSystem>(actor).unwrap().0.emitted_work(&id("repair")),
                "delivery_work": live.world().get::<ShipPowerSystem>(actor).unwrap().0.emitted_work(owner),
            },
            "permitted_logistics": permitted.get(owner),
            "tree": tree,
            "situations": runtime(&live).situations,
            "choice": runtime(&live).plan_choice,
            "credits": [runtime(&live).flags.counter("htn_repair_credit"), runtime(&live).flags.counter("htn_return_credit"), runtime(&live).flags.counter("htn_delivery_credit")],
        });
        eprintln!(
            "HTN delivery owner diagnostic: {}",
            diagnostic
                .to_string()
                .chars()
                .take(16000)
                .collect::<String>()
        );
    }
    assert!(
        (paid_repair || live.world().resource::<ActivityPaymentWitness>().repair.is_some())
            && (paid_delivery || live.world().resource::<ActivityPaymentWitness>().delivery.is_some()),
        "both ordinary owners must spend finite real energy: sampled repair={paid_repair} delivery={paid_delivery}; actual fixed-interval witness={:?}",
        live.world().resource::<ActivityPaymentWitness>()
    );
    assert_eq!(recalled.len(), 2);
    assert!(demand_sent);
    assert_eq!(hp_at(&live, actor, "engineering"), healthy_engineering);
    assert_eq!(hp_at(&live, actor, "bridge"), healthy_bridge);
    assert_eq!(
        hp_at(&resumed, restored, "engineering"),
        healthy_engineering
    );
    assert_eq!(hp_at(&resumed, restored, "bridge"), healthy_bridge);
    assert_eq!(
        parts(&live, actor),
        original_parts - 2,
        "two actual remedies consume two finite parts"
    );
    assert_eq!(rounds(&live, actor), original_rounds);
    let world = runtime(&live);
    assert_eq!(world.flags.counter("htn_repair_credit"), 2);
    assert_eq!(world.flags.counter("htn_return_credit"), 2);
    assert_eq!(world.flags.counter("htn_delivery_credit"), 1);
    assert_eq!(world.plan_choice.as_ref().unwrap().released.len(), 3);
    assert_eq!(world.situations.instances.len(), 3);
    let delivery = &world.situations.instances["htn/maintenance-host-proof/root/supply"];
    assert!(matches!(
        delivery.settled.as_ref().unwrap().outcome,
        ActivityOutcome::Delivery {
            returned: false,
            amount: 4,
            ..
        }
    ));
    assert!(
        world
            .situations
            .instances
            .values()
            .all(|row| row.state.is_none()),
        "each scripted terminal frontier applies once"
    );
}
