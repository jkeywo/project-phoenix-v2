//! Normal Comms accept/refuse gates a root and existing promise history.
//! Incident setup uses the ordinary physical Damage owner; this is not authored
//! encounter pacing, weapon admission, or a measured two-/six-player trial.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    activity::{ActivityExecutionIdentity, ActivityOutcome},
    damage::{DamageHit, DamageProfile, HitSample},
};
use project_phoenix::{
    comms::server::CommsRuntime,
    core::messages::SystemId,
    headless::{build_headless_app, HeadlessArgs},
    server_app::LocalShip,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    sim_digest::world_digest,
    sim_rng::{with_event_generator, SimRng},
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, server::WorldContentRuntime},
};
const WORLD: &str = "tests/fixtures/committed-assistance-world.toml";
fn boot() -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(1562224),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    app
}
fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}
fn hp(app: &App, actor: Entity) -> f64 {
    app.world()
        .get::<ShipDamage>(actor)
        .unwrap()
        .state
        .compartments["engineering"]
        .hp
}
fn incident(app: &mut App, actor: Entity) -> f64 {
    let healthy = hp(app, actor);
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
        let result = with_event_generator(Some(&rng), "hazard-clearance-incident", 0, |rng| {
            damage.apply_hit(
                0.0,
                DamageHit {
                    incoming: [1.0, 0.0],
                    profile: DamageProfile::default(),
                    payload: 1.0,
                    aim: Some("engineering".into()),
                    source: "hazard-clearance-incident".into(),
                    sample: Some(HitSample {
                        deck,
                        profile_fraction: 0.5,
                    }),
                },
                rng,
            )
        })
        .unwrap();
        assert!(result
            .deposits
            .iter()
            .any(|row| row.compartment == "engineering" && row.structural_loss > 0.0));
    });
    assert!(hp(app, actor) < healthy);
    healthy
}
fn same(live: &App, resumed: &App, step: usize) {
    assert!(
        capture(live.world()) == capture(resumed.world()),
        "complete capture differs at step {step}"
    );
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "digest at step {step}"
    );
}

const TOKEN: &str = "assistance-comms-officer";
fn send(app: &mut App, msg: project_phoenix::core::messages::ClientMessage) {
    app.world_mut()
        .write_message(project_phoenix::lobby::server::InboundMessage {
            token: TOKEN.into(),
            msg,
        });
}
fn claim_comms(app: &mut App) {
    use project_phoenix::{core::messages::ClientMessage, ship_plugin::ShipConfigComponent};
    let actor = local(app);
    let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
    let station = config
        .system_home_station(config.system(&SystemId("comms".into())).unwrap())
        .unwrap()
        .clone();
    let rating = config
        .station(&station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    for msg in [
        ClientMessage::Identify {
            token: TOKEN.into(),
            name: TOKEN.into(),
        },
        ClientMessage::SelectStation { station: station.0 },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ] {
        send(app, msg);
        app.update();
    }
}
fn answer(app: &mut App, index: usize) {
    use project_phoenix::{
        core::messages::{ClientMessage, SystemBlackboard, SystemControlPayload as Control},
        lobby::Sessions,
        server_app::ShipSystemBlackboards,
        ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    };
    let actor = local(app);
    let owner = SystemId("comms".into());
    let mut response = None;
    for _ in 0..240 {
        let config = &app.world().get::<ShipConfigComponent>(actor).unwrap().0;
        let boards = &app.world().get::<ShipSystemBlackboards>(actor).unwrap().0;
        let known = config.observations_for_system(&owner, boards);
        if let Some(SystemBlackboard::Comms(board)) = known.get(&owner) {
            if let Some(message) = board.messages.iter().find(|message| {
                message.body == "world.committed_assistance.offer"
                    && message.selected_response.is_none()
            }) {
                assert!(message.responses[index].available);
                let payload = Control::RespondToMessage {
                    message_id: message.id.clone(),
                    response_index: index,
                };
                if project_phoenix::command_admission::policy::is_command_authorized(
                    TOKEN,
                    &owner,
                    &payload,
                    app.world().get::<ShipSystemControlSources>(actor).unwrap(),
                    app.world().resource::<Sessions>(),
                    config,
                    app.world().get::<HumanSeekingHosts>(actor),
                ) && app
                    .world()
                    .get::<ShipPowerSystem>(actor)
                    .is_some_and(|power| {
                        let mut candidate = power.0.clone();
                        matches!(candidate.try_work_discrete(&owner), Ok(true))
                    })
                {
                    response = Some(payload);
                    break;
                }
            }
        }
        app.update();
    }
    let payload = response.expect("current Human Comms holder sees the actual offer and its response in their permitted Console view");
    assert!(
        app.world()
            .resource::<WorldContentRuntime>()
            .plan_choice
            .is_none(),
        "offer alone releases no root"
    );
    let Control::RespondToMessage { message_id, .. } = &payload else {
        unreachable!()
    };
    let message_id = message_id.clone();
    send(
        app,
        ClientMessage::ControlSystem {
            target: owner,
            payload,
        },
    );
    let mut admitted = false;
    let mut applied = false;
    for _ in 0..8 {
        app.update();
        admitted |= app.world().get::<project_phoenix::core::messages::AdmittedCommands>(actor).unwrap().0.iter().any(|command|
            command.response_token.as_deref() == Some(TOKEN) && matches!(&command.payload, Control::RespondToMessage { response_index, .. } if *response_index == index));
        applied |= app
            .world()
            .resource::<project_phoenix::comms::server::CommsInboxRes>()
            .0
            .iter()
            .any(|message| message.id == message_id && message.selected_response == Some(index));
    }
    assert!(
        admitted,
        "ordinary Human response traverses existing admission"
    );
    assert!(applied, "the actual range-checked and paid response executes, rather than merely entering admission");
}
#[test]
fn normal_comms_refusal_creates_no_promise_or_released_instruction() {
    let mut app = boot();
    claim_comms(&mut app);
    answer(&mut app, 1);
    for _ in 0..120 {
        app.update();
    }
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.flags.counter("assistance_declined"), 1);
    assert!(runtime.commitments.is_empty());
    assert!(runtime.plan_choice.is_none());
    assert!(runtime.situations.is_empty());
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(app.world())).unwrap()).unwrap();
    assert!(
        restore(app.world_mut(), &saved).is_complete(),
        "a genuinely declined root remains a valid dormant candidate"
    );
}
#[test]
fn normal_comms_acceptance_and_actual_work_keep_one_promise_through_full_json_resume() {
    use project_phoenix::{core::messages::ClientMessage, world::commitments::CommitmentState};
    let mut live = boot();
    claim_comms(&mut live);
    answer(&mut live, 0);
    let ledger = &live.world().resource::<WorldContentRuntime>().commitments;
    assert_eq!(ledger.records.len(), 1);
    assert_eq!(
        ledger.get("maintenance-assistance").unwrap().state,
        CommitmentState::Open
    );
    send(&mut live, ClientMessage::ReleaseStation);
    let mut checkpoint = None;
    for _ in 0..240 {
        live.update();
        if live
            .world()
            .resource::<CommsRuntime>()
            .pending_ai_responses
            .iter()
            .any(|(key, row)| {
                key.planned_request
                    && row.request_reading.is_some_and(|reading| {
                        reading.remaining_ticks > 0
                            && reading.remaining_ticks < reading.required_ticks
                    })
            })
        {
            checkpoint = Some(capture(live.world()));
            break;
        }
    }
    let saved: PhoenixSnapshot = serde_json::from_slice(
        &serde_json::to_vec(
            &checkpoint
                .expect("delegated ordinary Comms AI partially reads the accepted root request"),
        )
        .unwrap(),
    )
    .unwrap();
    let choice = saved
        .scenario
        .as_ref()
        .unwrap()
        .plan_choice
        .as_ref()
        .unwrap();
    assert_eq!(
        choice.required_commitment.as_deref(),
        Some("maintenance-assistance")
    );
    let chosen = choice.chosen_tick;
    let made = saved
        .scenario
        .as_ref()
        .unwrap()
        .commitments
        .get("maintenance-assistance")
        .unwrap()
        .made_at_tick;
    assert!(made <= chosen);
    let mut resumed = boot();
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    let actor = local(&mut live);
    let continued_actor = local(&mut resumed);
    let mut damaged = false;
    let mut healthy = 0.0;
    let mut paid_repair = false;
    for step in 0..1200 {
        same(&live, &resumed, step);
        if !damaged
            && live
                .world()
                .resource::<WorldContentRuntime>()
                .situations
                .instances
                .contains_key("htn/committed-assistance/root/repair")
        {
            healthy = incident(&mut live, actor);
            assert_eq!(incident(&mut resumed, continued_actor), healthy);
            damaged = true;
            same(&live, &resumed, step);
        }
        paid_repair |= live
            .world()
            .get::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .emitted_work(&SystemId("repair".into()))
            > 0.0;
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 1200);
    assert!(damaged && paid_repair && hp(&live, actor) >= healthy);
    let state = capture(live.world());
    let scenario = state.scenario.as_ref().unwrap();
    let choice = scenario.plan_choice.as_ref().unwrap();
    assert_eq!(choice.chosen_tick, chosen);
    let promise = scenario.commitments.get("maintenance-assistance").unwrap();
    assert_eq!(promise.state, CommitmentState::Kept);
    assert_eq!(promise.made_at_tick, made);
    assert!(promise.resolved_at_tick.unwrap() >= chosen);
    let runtime = live.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.flags.counter("actual_status_report"), 1);
    assert_eq!(runtime.flags.counter("assistance_work_achieved"), 1);
    let work = &scenario.situations.instances["htn/committed-assistance/root/repair"];
    assert!(work
        .accepted
        .as_ref()
        .is_some_and(|scope| matches!(scope.identity, ActivityExecutionIdentity::Physical { .. })));
    assert!(matches!(
        work.achieved.as_ref().unwrap().outcome,
        ActivityOutcome::RepairAchieved { .. }
    ));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..5 {
        let mut bad = state.clone();
        let scenario = bad.scenario.as_mut().unwrap();
        match fault {
            0 => scenario.commitments.records.clear(),
            1 => scenario.commitments.records[0].made_at_tick = chosen + 1,
            2 => scenario.plan_choice.as_mut().unwrap().required_commitment = None,
            3 => {
                scenario
                    .commitments
                    .records
                    .push(scenario.commitments.records[0].clone());
            }
            _ => {
                scenario.plan_choice = None;
                scenario.situations.instances.clear();
            }
        }
        assert!(
            !restore(resumed.world_mut(), &bad).is_complete(),
            "forged accepted obligation {fault}"
        );
        assert!(capture(resumed.world()) == before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
}
