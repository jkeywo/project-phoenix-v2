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

fn boot() -> (App, Entity) {
    boot_hull(HULL)
}
fn boot_hull(hull: &str) -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: hull.into(),
        deterministic: true,
        seed: Some(156222),
        ..Default::default()
    })
    .unwrap();
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
    app.world()
        .get::<ShipDamage>(actor)
        .unwrap()
        .state
        .compartments["engineering"]
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
    let before = hp(app, actor);
    // An explicit initial owner-level incident, not forged HP/job/evidence.
    app.world_mut().resource_scope(|world, rng: Mut<SimRng>| {
        let mut damage = world.get_mut::<ShipDamage>(actor).unwrap();
        let deck = damage
            .definition
            .interior
            .compartments
            .iter()
            .find(|room| room.id == "engineering")
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
                    aim: Some("engineering".into()),
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
            .any(|row| row.compartment == "engineering" && row.structural_loss > 0.0));
    });
    assert!(hp(app, actor) < before);
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
                    .contains_key(&scope.job)
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
        repair.job
    );
    assert!(live
        .world()
        .get::<ShipLogistics>(actor)
        .unwrap()
        .0
        .batches
        .contains_key(&delivery.job));
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
