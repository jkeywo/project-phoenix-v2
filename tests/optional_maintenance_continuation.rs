//! Actual bounded maintenance discovery, normal offered Comms and paid owners.
//! Owner-level incident/stock consumption establish the initial condition only.
//! A Human issues released machinery work, then ordinary AI continues it; this
//! does not claim autonomous future-intent coordination or full increment 24.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    activity::ActivityOutcome,
    damage::{DamageHit, DamageProfile, HitSample},
};
use project_phoenix::{
    console::repair::server::ShipRepairTeams,
    core::messages::{
        ClientMessage, StationId, SystemBlackboard, SystemControlPayload as Control, SystemId,
    },
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    resources::logistics_server::{CompiledShipLogistics, ShipLogistics},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::{with_event_generator, SimRng},
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, opportunities::OpportunityState, server::WorldContentRuntime},
};
use std::collections::BTreeSet;
const WORLD: &str = "tests/fixtures/optional-maintenance-world.toml";
const HULL: &str = "assets/entities/alliance_cruiser.toml";
const TOKEN: &str = "optional-maintenance-engineer";
const COMMS: &str = "optional-maintenance-comms";
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
    let (mut app, entity) = boot_definition(WORLD, HULL);
    claim(&mut app, "engineering", TOKEN);
    claim(&mut app, "comms", COMMS);
    (app, entity)
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
                    profile: DamageProfile {
                        emp: Some(phoenix_model::emp::EmpPayload { duration_secs: 1.0 }),
                        ..Default::default()
                    },
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

fn establish_need(app: &mut App, actor: Entity) -> (f64, i64) {
    // Consume finite existing stock, never create a resource or a delivery.
    // Read-only native diagnostic: keep the original one-update/zero-stock
    // assertion so an unapplied effect cannot be mistaken for a valid need.
    eprintln!(
        "maintenance consumption before: tick={} phase={:?} stock={:?} queued={:?} deliveries={:?}",
        app.world()
            .resource::<project_phoenix::sim_tick::SimTick>()
            .0,
        app.world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get(),
        app.world()
            .get::<InfrastructureCondition>(actor)
            .unwrap()
            .0
            .capacity_stock("structural_parts"),
        app.world()
            .resource::<project_phoenix::effect_queue::EffectQueue<
                project_phoenix::infrastructure::CapacityAdjustment,
            >>()
            .0,
        app.world().get::<ShipLogistics>(actor).unwrap().0.batches,
    );
    let level = app
        .world()
        .get::<InfrastructureCondition>(actor)
        .unwrap()
        .0
        .capacity_stock("structural_parts")
        .unwrap()
        .level;
    assert!(level > 0);
    let uuid = app
        .world()
        .get::<project_phoenix::entities::spawner::EntityUuid>(actor)
        .unwrap()
        .0
        .clone();
    app.world_mut()
        .resource_mut::<project_phoenix::effect_queue::EffectQueue<
            project_phoenix::infrastructure::CapacityAdjustment,
        >>()
        .0
        .push(project_phoenix::infrastructure::CapacityAdjustment {
            uuid,
            capacity: "structural_parts".into(),
            delta: -level,
        });
    app.update();
    eprintln!(
        "maintenance consumption after: tick={} phase={:?} stock={:?} queued={:?} deliveries={:?}",
        app.world()
            .resource::<project_phoenix::sim_tick::SimTick>()
            .0,
        app.world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get(),
        app.world()
            .get::<InfrastructureCondition>(actor)
            .unwrap()
            .0
            .capacity_stock("structural_parts"),
        app.world()
            .resource::<project_phoenix::effect_queue::EffectQueue<
                project_phoenix::infrastructure::CapacityAdjustment,
            >>()
            .0,
        app.world().get::<ShipLogistics>(actor).unwrap().0.batches,
    );
    assert_eq!(
        app.world()
            .get::<InfrastructureCondition>(actor)
            .unwrap()
            .0
            .capacity_stock("structural_parts")
            .unwrap()
            .level,
        0
    );
    let original_parts = parts(app, actor);
    let healthy = incident(app, actor);
    for _ in 0..1800 {
        app.update();
        if runtime(app)
            .opportunities
            .first()
            .is_some_and(|row| matches!(row.state, OpportunityState::Offered))
        {
            break;
        }
    }
    let row = runtime(app)
        .opportunities
        .first()
        .expect("actual finite need and ordinary own-System report produce an offer");
    assert!(row.input.known && row.input.report.is_some());
    assert_eq!(row.input.local_available, 0);
    assert!(row.input.hp < row.input.max_hp);
    assert_eq!(
        row.proposal.steps.len(),
        2,
        "scratch delivery is necessary to enable this repair"
    );
    assert!(runtime(app).situations.is_empty());
    assert!(runtime(app).commitments.is_empty());
    assert!(
        runtime(app).plan_choice.is_none(),
        "optional discovery never replaces a primary root"
    );
    (healthy, original_parts)
}
fn answer_offer(app: &mut App, index: usize) {
    let actor = local(app);
    let mut response = None;
    for _ in 0..1800 {
        let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
        let boards = &app.world().get::<ShipSystemBlackboards>(actor).unwrap().0;
        let known = config.observations_for_system(&id("comms"), boards);
        if let Some(SystemBlackboard::Comms(board)) = known.get(&id("comms")) {
            if let Some(message) = board.messages.iter().find(|message| {
                message.body == "world.optional_maintenance.offer"
                    && message.selected_response.is_none()
            }) {
                assert!(message.responses[index].available);
                let powered = app
                    .world()
                    .get::<ShipPowerSystem>(actor)
                    .is_some_and(|power| {
                        let mut candidate = power.0.clone();
                        matches!(candidate.try_work_discrete(&id("comms")), Ok(true))
                    });
                if !powered {
                    app.update();
                    continue;
                }
                response = Some((
                    message.id.clone(),
                    Control::RespondToMessage {
                        message_id: message.id.clone(),
                        response_index: index,
                    },
                ));
                break;
            }
        }
        app.update();
    }
    let (message_id, payload) =
        response.expect("actual own Comms Console publishes the immutable offered dialogue");
    control(app, COMMS, "comms", "comms", payload);
    for _ in 0..12 {
        app.update();
    }
    assert!(
        app.world()
            .resource::<project_phoenix::comms::server::CommsInboxRes>()
            .0
            .iter()
            .any(|message| message.id == message_id && message.selected_response == Some(index)),
        "normal paid on_pick executes"
    );
}
fn released_delivery(
    app: &mut App,
    actor: Entity,
) -> phoenix_model::activity::PlannedDeliveryRequest {
    for _ in 0..120 {
        let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
        let boards = &app.world().get::<ShipSystemBlackboards>(actor).unwrap().0;
        let known = config.observations_for_system(&id("logistics"), boards);
        if let Some(SystemBlackboard::Logistics(board)) = known.get(&id("logistics")) {
            if let Some(row) = board.planned_requests.first() {
                return row.clone();
            }
        }
        app.update();
    }
    panic!("accepted optional delivery is published through its own Console");
}
fn released_repair(app: &mut App, actor: Entity) -> phoenix_model::activity::PlannedRepairRequest {
    for _ in 0..1800 {
        let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
        let boards = &app.world().get::<ShipSystemBlackboards>(actor).unwrap().0;
        let known = config.observations_for_system(&id("repair"), boards);
        if let Some(SystemBlackboard::Repair(board)) = known.get(&id("repair")) {
            if let Some(row) = board.planned_requests.first() {
                return row.clone();
            }
        }
        app.update();
    }
    panic!("actual received parts release the dependent repair through its own Console");
}
#[test]
fn discovered_offer_decline_creates_no_promise_or_physical_job() {
    let (mut app, actor) = boot();
    establish_need(&mut app, actor);
    let offered = capture(app.world());
    let mut unreachable = offered.clone();
    if let Some(comms) = &mut unreachable.comms {
        comms
            .dialogues
            .retain(|row| row.script.opportunity.is_none());
        comms.pending_opens.retain(|row| row.opportunity.is_none());
    }
    let before = serde_json::to_value(capture(app.world())).unwrap();
    assert!(
        !restore(app.world_mut(), &unreachable).is_complete(),
        "an Offered record must remain answerable through its actual envelope"
    );
    assert_eq!(before, serde_json::to_value(capture(app.world())).unwrap());
    let mut invented_decision = offered.clone();
    invented_decision.scenario.as_mut().unwrap().opportunities[0].state =
        OpportunityState::Pending {
            decision: phoenix_model::comms_request::OpportunityDecision::Accept,
            tick: offered.tick,
        };
    assert!(
        !restore(app.world_mut(), &invented_decision).is_complete(),
        "an unselected originating Comms offer cannot explain a Pending acceptance"
    );
    assert_eq!(before, serde_json::to_value(capture(app.world())).unwrap());
    assert!(restore(app.world_mut(), &offered).is_complete());
    answer_offer(&mut app, 1);
    assert!(matches!(
        runtime(&app).opportunities[0].state,
        OpportunityState::Declined { .. }
    ));
    assert!(runtime(&app).commitments.is_empty());
    assert!(runtime(&app).situations.is_empty());
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(app.world())).unwrap()).unwrap();
    assert!(restore(app.world_mut(), &saved).is_complete());
}
#[test]
fn actual_scratch_delivery_enables_paid_repair_and_retains_frozen_plan_through_json_resume() {
    use project_phoenix::world::commitments::CommitmentState;
    let (mut live, actor) = boot();
    let (healthy, original_parts) = establish_need(&mut live, actor);
    answer_offer(&mut live, 0);
    let record = runtime(&live).opportunities[0].clone();
    let OpportunityState::Accepted { choice, .. } = &record.state else {
        panic!("normal response atomically admits the optional child");
    };
    assert_eq!(
        choice.released.len(),
        1,
        "unmet prerequisite does not dispatch the future repair"
    );
    let promise = choice.required_commitment.clone().unwrap();
    assert_eq!(
        runtime(&live).commitments.get(&promise).unwrap().state,
        CommitmentState::Open
    );
    let delivery = released_delivery(&mut live, actor);
    assert_eq!(delivery.amount, 1);
    control(
        &mut live,
        TOKEN,
        "engineering",
        "logistics",
        Control::DispatchDelivery {
            source: delivery.source,
            destination: delivery.destination,
            amount: delivery.amount,
        },
    );
    let repair = released_repair(&mut live, actor);
    assert!(runtime(&live).situations.instances.values().any(|row| row
        .settled
        .as_ref()
        .is_some_and(|fact| matches!(
            &fact.outcome,
            ActivityOutcome::Delivery {
                returned: false,
                ..
            }
        ))));
    control(
        &mut live,
        TOKEN,
        "engineering",
        "repair",
        Control::DispatchRepairTeam {
            team_idx: repair.team,
            target: repair.request,
        },
    );
    // Delegate only after the exact released leaf has bound actual admitted work.
    // Autonomous prevention of competing pre-prerequisite repair is follow-up work.
    assert_eq!(
        runtime(&live)
            .situations
            .instances
            .values()
            .filter(|row| row.accepted.is_some())
            .count(),
        2
    );
    for token in [TOKEN, COMMS] {
        send(&mut live, token, ClientMessage::ReleaseStation);
    }
    let mut saved = None;
    for _ in 0..1800 {
        live.update();
        if live
            .world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .physical_work
            .values()
            .any(|work| work.reserved && work.progress > 0.0 && work.progress < 2.0)
        {
            saved = Some(capture(live.world()));
            break;
        }
    }
    let saved: PhoenixSnapshot = serde_json::from_slice(
        &serde_json::to_vec(&saved.expect("save during actual paid partial repair")).unwrap(),
    )
    .unwrap();
    assert_eq!(runtime(&live).flags.counter("actual_optional_structure"), 0);
    let (mut resumed, restored) = boot_definition(WORLD, HULL);
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = serde_json::to_value(capture(resumed.world())).unwrap();
    for fault in 0..8 {
        let mut forged = saved.clone();
        if fault == 7 {
            forged.scenario = None;
        } else {
            let state = forged.scenario.as_mut().unwrap();
            match fault {
                0 => state.opportunities[0].reference.ordinal = 2,
                1 => state.opportunities[0].input.max_hp += 1.0,
                2 => {
                    state.opportunities[0]
                        .input
                        .report
                        .as_mut()
                        .unwrap()
                        .received_tick = Some(saved.tick + 1)
                }
                3 => {
                    state.opportunities.clear();
                }
                4 => {
                    let OpportunityState::Accepted { choice, .. } =
                        &mut state.opportunities[0].state
                    else {
                        unreachable!()
                    };
                    choice.chosen_tick = saved.tick + 1;
                }
                5 => {
                    state.opportunities[0].proposal.steps[0].task =
                        "maintenance::restore-room".into();
                }
                _ => {
                    let key = state.situations.instances.keys().next().unwrap().clone();
                    state.situations.instances.remove(&key);
                }
            }
        }
        assert!(
            !restore(resumed.world_mut(), &forged).is_complete(),
            "malformed frozen child{fault}"
        );
        assert_eq!(
            before,
            serde_json::to_value(capture(resumed.world())).unwrap(),
            "whole-candidate refusal writes nothing{fault}"
        );
    }
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    for step in 0..1200 {
        assert_same(&live, &resumed, step);
        live.update();
        resumed.update();
    }
    assert_same(&live, &resumed, 1200);
    assert_eq!(hp(&live, actor), healthy);
    assert_eq!(hp(&resumed, restored), healthy);
    assert_eq!(
        parts(&live, actor),
        original_parts - 1,
        "only actual completed repair consumes a part"
    );
    assert_eq!(runtime(&live).flags.counter("actual_optional_structure"), 1);
    assert_eq!(
        runtime(&live).commitments.get(&promise).unwrap().state,
        CommitmentState::Kept
    );
    let work = &runtime(&live).situations.instances;
    assert_eq!(work.len(), 2);
    let jobs = work
        .values()
        .map(|row| row.accepted.as_ref().unwrap().clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        jobs.len(),
        2,
        "distinct actual consumers retain their original jobs"
    );
    assert!(
        work.values().all(|row| row.settled.is_some()),
        "team and delivery physically settle"
    );
    // Erasing settled execution history must not leave a legitimate-looking
    // generated Kept promise. Remove its now-inactive dialogue too so this
    // specifically exercises the reserved World history guard.
    let mut erased = capture(live.world());
    let state = erased.scenario.as_mut().unwrap();
    state.opportunities.clear();
    state.situations.instances.clear();
    if let Some(comms) = &mut erased.comms {
        comms
            .dialogues
            .retain(|row| row.script.opportunity.is_none());
        comms.pending_opens.retain(|row| row.opportunity.is_none());
    }
    let completed_before = serde_json::to_value(capture(resumed.world())).unwrap();
    assert!(
        !restore(resumed.world_mut(), &erased).is_complete(),
        "settled generated promise cannot survive erased execution history"
    );
    assert_eq!(
        completed_before,
        serde_json::to_value(capture(resumed.world())).unwrap()
    );
    let witness = live.world().resource::<ActivityPaymentWitness>();
    assert!(
        witness.repair.is_some() && witness.delivery.is_some(),
        "both actual consumers paid finite machinery work"
    );
}

#[test]
fn ordinary_ai_observes_future_intent_then_executes_useful_supply_and_repair_through_json_resume() {
    let (mut live, actor) = boot();
    let (healthy, original_parts) = establish_need(&mut live, actor);
    answer_offer(&mut live, 0);
    let OpportunityState::Accepted { choice, .. } = &runtime(&live).opportunities[0].state else {
        panic!("actual offered acceptance")
    };
    let original_choice = choice.clone();
    assert_eq!(choice.released.len(), 1);
    let known = live
        .world()
        .get::<ShipConfigComponent>(actor)
        .unwrap()
        .0
        .observations_for_system(
            &id("repair"),
            &live.world().get::<ShipSystemBlackboards>(actor).unwrap().0,
        );
    let Some(SystemBlackboard::Repair(board)) = known.get(&id("repair")) else {
        panic!("actual own Repair publication")
    };
    assert!(board
        .planned_intents
        .iter()
        .any(|intent| intent.released_tick.is_none()
            && intent
                .prerequisites
                .iter()
                .any(|dependency| !dependency.satisfied)));
    // Both ordinary controllers take over the same released and future orders;
    // no Human starts either Job, and no temporary-controller policy is added.
    for token in [TOKEN, COMMS] {
        send(&mut live, token, ClientMessage::ReleaseStation);
    }
    let mut saved = None;
    for _ in 0..2400 {
        live.update();
        let work = &runtime(&live).situations.instances;
        for job in live
            .world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .teams
            .iter()
            .flatten()
        {
            assert!(
                work.values()
                    .any(|row| row
                        .accepted
                        .as_ref()
                        .is_some_and(|scope| scope.owner == id("repair")
                            && scope.physical_job() == Some(job.id))),
                "generic AI cannot consume the accepted future target first"
            );
        }
        if live
            .world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .physical_work
            .values()
            .any(|work| work.reserved && work.progress > 0.0 && work.progress < 2.0)
        {
            saved = Some(capture(live.world()));
            break;
        }
    }
    let saved: PhoenixSnapshot = serde_json::from_slice(
        &serde_json::to_vec(&saved.expect("ordinary AI reaches actual paid partial repair"))
            .unwrap(),
    )
    .unwrap();
    let OpportunityState::Accepted { choice, .. } = &runtime(&live).opportunities[0].state else {
        unreachable!()
    };
    assert_eq!(choice.chosen_tick, original_choice.chosen_tick);
    assert_eq!(choice.methods, original_choice.methods);
    let delivery = runtime(&live)
        .situations
        .instances
        .values()
        .find(|row| row.request.owner() == &id("logistics"))
        .unwrap();
    let repair = runtime(&live)
        .situations
        .instances
        .values()
        .find(|row| row.request.owner() == &id("repair"))
        .unwrap();
    assert!(repair.accepted_tick.unwrap() >= delivery.settled.as_ref().unwrap().tick);
    let (mut resumed, restored) = boot_definition(WORLD, HULL);
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    for step in 0..1200 {
        assert_same(&live, &resumed, step);
        live.update();
        resumed.update();
    }
    assert_same(&live, &resumed, 1200);
    assert_eq!(hp(&live, actor), healthy);
    assert_eq!(hp(&resumed, restored), healthy);
    assert_eq!(parts(&live, actor), original_parts - 1);
    assert_eq!(runtime(&live).flags.counter("actual_optional_structure"), 1);
    assert_eq!(runtime(&live).flags.counter("actual_optional_changed"), 0);
    assert!(live
        .world()
        .resource::<ActivityPaymentWitness>()
        .repair
        .is_some());
    assert!(live
        .world()
        .resource::<ActivityPaymentWitness>()
        .delivery
        .is_some());
}

#[test]
fn human_override_restores_future_target_without_retroactive_job_credit_and_runs_authored_consequence_once(
) {
    let (mut live, actor) = boot();
    let (healthy, original_parts) = establish_need(&mut live, actor);
    answer_offer(&mut live, 0);
    let delivery = released_delivery(&mut live, actor);
    // A different team performs a legitimate manual order before its planned
    // peer is released. Empty local stores make it wait for real delivered parts.
    control(
        &mut live,
        TOKEN,
        "engineering",
        "repair",
        Control::DispatchRepairTeam {
            team_idx: 1,
            target: project_phoenix::core::messages::RepairTarget::Work(
                phoenix_model::damage::RepairWorkTarget::Compartment {
                    compartment: "engineering".into(),
                },
            ),
        },
    );
    let override_job = live
        .world()
        .get::<ShipRepairTeams>(actor)
        .unwrap()
        .0
        .job_state()
        .teams[1]
        .as_ref()
        .expect("actual manual override assignment")
        .id;
    assert!(runtime(&live)
        .situations
        .instances
        .values()
        .all(|row| row.request.owner() != &id("repair") || row.accepted.is_none()));
    control(
        &mut live,
        TOKEN,
        "engineering",
        "logistics",
        Control::DispatchDelivery {
            source: delivery.source,
            destination: delivery.destination,
            amount: delivery.amount,
        },
    );
    let mut saved = None;
    for _ in 0..2400 {
        live.update();
        if live
            .world()
            .get::<ShipRepairTeams>(actor)
            .unwrap()
            .0
            .job_state()
            .physical_work
            .values()
            .any(|work| work.reserved && work.progress > 0.0 && work.progress < 2.0)
        {
            saved = Some(capture(live.world()));
            break;
        }
    }
    assert!(saved.is_some(), "actual override partial-work checkpoint");
    for token in [TOKEN, COMMS] {
        send(&mut live, token, ClientMessage::ReleaseStation);
    }
    steps(&mut live, 2);
    let team_state = live
        .world()
        .get::<ShipRepairTeams>(actor)
        .unwrap()
        .0
        .job_state();
    assert_eq!(
        team_state.teams[1].as_ref().unwrap().id,
        override_job,
        "handoff preserves the same actual Job"
    );
    assert!(
        team_state
            .physical_work
            .get(&override_job.0)
            .is_some_and(|work| work.reserved && work.progress > 0.0 && work.progress < 2.0),
        "fresh JSON starts from this same paid partial override after delegation"
    );

    let saved = Some(capture(live.world()));
    let saved: PhoenixSnapshot = serde_json::from_slice(
        &serde_json::to_vec(&saved.expect("unbound Human override reaches real paid partial work"))
            .unwrap(),
    )
    .unwrap();
    let (mut resumed, restored) = boot_definition(WORLD, HULL);
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    for step in 0..1200 {
        assert_same(&live, &resumed, step);
        live.update();
        resumed.update();
    }
    assert_same(&live, &resumed, 1200);
    assert_eq!(hp(&live, actor), healthy);
    assert_eq!(hp(&resumed, restored), healthy);
    assert_eq!(parts(&live, actor), original_parts - 1);
    let OpportunityState::Accepted { choice, .. } = &runtime(&live).opportunities[0].state else {
        unreachable!()
    };
    let changed = choice
        .node_dispositions
        .values()
        .next()
        .expect("actual own gauge observes restored future target");
    assert!(changed.latest.terminal());
    assert_eq!(
        serde_json::to_value(changed.consequence.as_ref().unwrap().result).unwrap(),
        serde_json::json!("applied")
    );
    assert_eq!(runtime(&live).flags.counter("actual_optional_changed"), 1);
    assert_eq!(
        runtime(&live).flags.counter("actual_optional_structure"),
        0,
        "unbound physical work is not a planned achievement"
    );
    assert_eq!(
        runtime(&live)
            .commitments
            .get(choice.required_commitment.as_ref().unwrap())
            .unwrap()
            .state,
        project_phoenix::world::commitments::CommitmentState::Broken,
        "only the actual authored consequence resolves this promise"
    );
    for row in runtime(&live)
        .situations
        .instances
        .values()
        .filter(|row| row.request.owner() == &id("repair"))
    {
        assert!(
            row.accepted.is_none() && row.achieved.is_none() && row.settled.is_none(),
            "no fabricated Repair execution or evidence"
        );
    }
    let final_state = capture(live.world());
    for fault in 0..5 {
        let mut forged = final_state.clone();
        let OpportunityState::Accepted { choice, .. } =
            &mut forged.scenario.as_mut().unwrap().opportunities[0].state
        else {
            unreachable!()
        };
        let disposition = choice.node_dispositions.values_mut().next().unwrap();
        match fault {
            0 => disposition.latest.hp -= 1.0,
            1 => disposition.latest.max_hp += 1.0,
            2 => disposition.latest.observation.provenance.system = Some(id("foreign")),
            3 => disposition.consequence.as_mut().unwrap().tick = final_state.tick + 1,
            _ => {
                disposition.latest.observation.subject =
                    phoenix_model::observation::ObservationSubject::System { id: id("helm") }
            }
        };
        let before = serde_json::to_value(capture(live.world())).unwrap();
        assert!(
            !restore(live.world_mut(), &forged).is_complete(),
            "changed-work history fault{fault}"
        );
        assert_eq!(
            before,
            serde_json::to_value(capture(live.world())).unwrap(),
            "candidate refuses before writes{fault}"
        );
    }
}
