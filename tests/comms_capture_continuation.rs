//! Actual foreign planned emission -> paid interception -> own paid decoder.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    comms_capture::{CommsCaptureConfig, CommsCaptureView, DecodedCommsClaim, DecodedRequestClaim},
    comms_request::CommsRadioContent,
};
use project_phoenix::{
    comms::{capture::ShipCommsCapture, server::CommsInboxRes},
    core::messages::{
        AdmittedCommands, ClientMessage, SystemBlackboard, SystemControlPayload as Control,
        SystemId,
    },
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    lobby::server::InboundMessage,
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::power::ShipPowerSystem,
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, server::WorldContentRuntime},
};
const WORLD: &str = "tests/fixtures/capture-world.toml";
const TOKEN: &str = "capture-operator";
fn owner() -> SystemId {
    SystemId("comms".into())
}
#[derive(Resource, Default, Debug)]
struct PaidWitness {
    tick: Option<u64>,
    work: f64,
    baseline: f64,
    ai_selection: bool,
}
fn observe(
    tick: Res<project_phoenix::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    actors: Query<
        (
            &ShipPowerSystem,
            &AdmittedCommands,
            &ShipSystemControlSources,
        ),
        With<LocalShip>,
    >,
    mut witness: ResMut<PaidWitness>,
) {
    *witness = PaidWitness {
        tick: Some(tick.0),
        ..Default::default()
    };
    for (power, commands, sources) in &actors {
        witness.work = power.0.emitted_work(&owner());
        witness.baseline = f64::from(
            power
                .0
                .config()
                .consumers
                .iter()
                .find(|row| row.system == owner())
                .unwrap()
                .continuous_cost_per_sec,
        ) * f64::from(fixed.timestep().as_secs_f32());
        witness.ai_selection = sources.0.source_for(&owner())
            == project_phoenix::ship::control_source::ControlSource::Ai
            && commands.for_target("comms").any(|command| {
                matches!(
                    command.payload,
                    Control::SelectCommsCapture { capture: Some(_) }
                )
            });
    }
}
fn boot() -> (App, Entity, CommsCaptureConfig) {
    let decoder = project_phoenix::entities::include_resolve::load_entity_config(
        "assets/entities/capture_cruiser.toml",
    )
    .unwrap()
    .comms
    .unwrap()
    .radio
    .unwrap()
    .capture
    .unwrap();
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/capture_cruiser.toml".into(),
        deterministic: true,
        seed: Some(15621667),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<PaidWitness>();
    app.add_systems(
        FixedUpdate,
        observe
            .in_set(project_phoenix::sim_sets::SimSet::Physics)
            .after_ignore_deferred(project_phoenix::sim_sets::FixedStep::TickCommsRadio),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    let actor = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    (app, actor, decoder)
}
fn views(app: &App, actor: Entity, config: &CommsCaptureConfig) -> Vec<CommsCaptureView> {
    let canonical = app
        .world()
        .get::<ShipCommsCapture>(actor)
        .unwrap()
        .0
        .views(config, &[]);
    let ship = app.world().get::<ShipConfigComponent>(actor).unwrap();
    let boards = app.world().get::<ShipSystemBlackboards>(actor).unwrap();
    let permitted = ship.0.observations_for_system(&owner(), &boards.0);
    let Some(SystemBlackboard::Comms(board)) = permitted.get(&owner()) else {
        panic!("own Comms must receive its actual Console publication");
    };
    let rows = board
        .captured
        .as_ref()
        .expect("installed decoder has a supported own publication")
        .rows
        .clone();
    for row in &rows {
        if let Some(evidence) = &row.evidence {
            assert_eq!(canonical.iter().find(|actual| actual.capture == row.capture).and_then(|actual| actual.evidence.as_ref()), Some(evidence),
                "the actual permitted Console must never publish plaintext before canonical paid completion");
        }
    }
    rows
}
fn send(app: &mut App, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: TOKEN.into(),
        msg,
    });
}
fn claim(app: &mut App, actor: Entity) {
    let rating = app
        .world()
        .get::<ShipConfigComponent>(actor)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|row| row.id.0 == "comms")
        .unwrap()
        .ratings[0]
        .name
        .clone();
    for msg in [
        ClientMessage::Identify {
            token: TOKEN.into(),
            name: TOKEN.into(),
        },
        ClientMessage::SelectStation {
            station: "comms".into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ] {
        send(app, msg);
        for _ in 0..4 {
            app.update();
        }
    }
}
fn assert_not_participating(app: &App, actor: Entity) {
    let uuid = &app.world().get::<EntityUuid>(actor).unwrap().0;
    assert!(
        app.world()
            .resource::<CommsInboxRes>()
            .0
            .iter()
            .filter(|message| message.is_for_ship(Some(uuid)))
            .all(|message| message
                .transmission
                .as_ref()
                .is_none_or(|wire| matches!(wire.content, CommsRadioContent::Text))),
        "captured foreign requests/outcomes never enter this audience's machinery/receipt inbox"
    );
}
fn same(a: &App, b: &App, step: usize) {
    assert_eq!(
        capture(a.world()),
        capture(b.world()),
        "complete canonical capture differs at {step}"
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest differs at {step}"
    );
}
fn assert_extra_paid(app: &App) {
    let witness = app.world().resource::<PaidWitness>();
    assert_eq!(
        witness.tick.and_then(|tick| tick.checked_add(1)),
        Some(
            app.world()
                .resource::<project_phoenix::sim_tick::SimTick>()
                .0
        )
    );
    assert!(
        witness.baseline > 0.0 && witness.work > witness.baseline * 1.9,
        "decode requires actual additional work: {witness:?}"
    );
}
#[test]
fn human_decodes_actual_foreign_request_without_early_plaintext_or_participation() {
    let (mut app, actor, config) = boot();
    claim(&mut app, actor);
    let mut chosen = None;
    for _ in 0..360 {
        if let Some(row) = views(&app, actor, &config)
            .into_iter()
            .find(|row| row.feasible && row.evidence.is_none())
        {
            chosen = Some(row.capture);
            break;
        }
        app.update();
    }
    let chosen = chosen.expect("ordinary foreign Comms must emit a real protected request within this actual receiver's reach");
    let before = views(&app, actor, &config);
    assert!(before.iter().all(|row| row.evidence.is_none()));
    assert_not_participating(&app, actor);
    send(
        &mut app,
        ClientMessage::ControlSystem {
            target: owner(),
            payload: Control::SelectCommsCapture {
                capture: Some(chosen),
            },
        },
    );
    let mut decoded = false;
    let mut paid_ticks = 0;
    for _ in 0..240 {
        app.update();
        let row = views(&app, actor, &config)
            .into_iter()
            .find(|row| row.capture == chosen)
            .unwrap();
        if row.progress_per_mille > 0 {
            assert_extra_paid(&app);
            paid_ticks += 1;
        }
        assert_not_participating(&app, actor);
        if let Some(evidence) = row.evidence {
            assert_eq!(
                evidence.kind,
                phoenix_model::comms_capture::CapturedContentKind::Request
            );
            assert_eq!(
                evidence.claim,
                DecodedCommsClaim::Request {
                    request: DecodedRequestClaim::ReportStatus
                }
            );
            decoded = true;
            break;
        }
    }
    assert!(decoded && paid_ticks > 1);
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .source_for(&owner()),
        project_phoenix::ship::control_source::ControlSource::Human
    );
}
#[test]
fn ordinary_ai_partially_paid_capture_resumes_json_360_complete_steps_and_bad_candidates_never_write(
) {
    let (mut live, actor, config) = boot();
    let mut ai_selected = false;
    let mut chosen = None;
    for _ in 0..360 {
        live.update();
        ai_selected |= live.world().resource::<PaidWitness>().ai_selection;
        if let Some(row) = views(&live, actor, &config)
            .into_iter()
            .find(|row| row.selected && row.progress_per_mille > 0 && row.progress_per_mille < 1000)
        {
            chosen = Some(row.capture);
            assert_extra_paid(&live);
            break;
        }
    }
    let chosen =
        chosen.expect("same ordinary per-System AI selects and funds an observed feasible capture");
    assert!(ai_selected);
    assert_not_participating(&live, actor);
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let source = live.world().get::<EntityUuid>(actor).unwrap().0.clone();
    let (mut resumed, restored, restored_config) = boot();
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..5 {
        let mut bad = serde_json::to_value(&saved).unwrap();
        let row = bad["entities"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["uuid"].as_str() == Some(source.as_str()))
            .unwrap();
        let state = &mut row["comms_capture"];
        match fault {
            0 => *state = serde_json::Value::Null,
            1 => state["owner"] = "foreign-system".into(),
            2 => state["rows"][0]["last_work_tick"] = (saved.tick + 1).into(),
            3 => state["rows"][0]["payload"]["body"] = "forged captured content".into(),
            _ => state["rows"][0]["payload"]["transmission"]["channel"] = "public".into(),
        }
        let bad: PhoenixSnapshot = serde_json::from_value(bad).unwrap();
        assert!(
            !restore(resumed.world_mut(), &bad).is_complete(),
            "fault {fault}"
        );
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    assert!(restore(resumed.world_mut(), &saved).is_complete());
    let mut decoded = false;
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
        assert_not_participating(&live, actor);
        assert_not_participating(&resumed, restored);
        let a = views(&live, actor, &config)
            .into_iter()
            .find(|row| row.capture == chosen)
            .unwrap();
        let b = views(&resumed, restored, &restored_config)
            .into_iter()
            .find(|row| row.capture == chosen)
            .unwrap();
        assert_eq!(a, b);
        if a.selected {
            assert_extra_paid(&live);
            assert_extra_paid(&resumed);
        }
        decoded |= a.evidence.is_some();
        if let Some(evidence) = &a.evidence {
            assert_eq!(
                evidence.claim,
                DecodedCommsClaim::Request {
                    request: DecodedRequestClaim::ReportStatus
                }
            );
        }
    }
    same(&live, &resumed, 360);
    assert!(
        decoded,
        "ordinary AI's actual work completes under finite real supply"
    );
    drop(resumed);
    // The shared ordinary DestroyEntity cleanup is the physical removal owner;
    // this explicitly is not a GM-admission witness. Historical inbox claims and
    // captured evidence survive, without retaining/protecting the emitter body.
    let emitted_source =
        live.world().resource::<WorldContentRuntime>().name_to_uuid["emitter"].clone();
    let removed = live
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(live.world())
        .find_map(|(entity, uuid)| (uuid.0 == emitted_source).then_some(entity))
        .unwrap();
    assert_ne!(removed, actor);
    let _ = project_phoenix::gm_despawn::remove_entity(live.world_mut(), removed);
    live.update();
    let historical = views(&live, actor, &config)
        .into_iter()
        .find(|row| row.capture == chosen)
        .unwrap();
    assert!(historical.evidence.is_some());
    let without_source: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    assert!(without_source
        .entities
        .iter()
        .all(|row| row.uuid != emitted_source));
    let (mut after_loss, loss_actor, loss_config) = boot();
    for _ in 0..240 {
        if ready_to_restore(after_loss.world(), &without_source) {
            break;
        }
        after_loss.update();
    }
    assert!(ready_to_restore(after_loss.world(), &without_source));
    // The retained public claim is derived from the same paid canonical row.
    // A plausible changed request kind must refuse before any live write.
    let mut forged = without_source.clone();
    let captured_owner = forged
        .entities
        .iter_mut()
        .find(|row| row.uuid == source)
        .unwrap();
    let SystemBlackboard::Comms(board) = &mut captured_owner
        .blackboards
        .iter_mut()
        .find(|(owner, _)| owner == "comms")
        .unwrap()
        .1
    else {
        panic!("actual saved Comms")
    };
    board
        .captured
        .as_mut()
        .unwrap()
        .rows
        .iter_mut()
        .find(|row| row.capture == chosen)
        .unwrap()
        .evidence
        .as_mut()
        .unwrap()
        .claim = DecodedCommsClaim::Request {
        request: DecodedRequestClaim::FormUp { slot: [40, -5] },
    };
    let before = capture(after_loss.world());
    let digest = world_digest(after_loss.world());
    assert!(!restore(after_loss.world_mut(), &forged).is_complete());
    assert_eq!(capture(after_loss.world()), before);
    assert_eq!(world_digest(after_loss.world()), digest);
    assert!(restore(after_loss.world_mut(), &without_source).is_complete());
    assert!(after_loss
        .world_mut()
        .query::<&EntityUuid>()
        .iter(after_loss.world())
        .all(|uuid| uuid.0 != emitted_source));
    for step in 0..360 {
        same(&live, &after_loss, 360 + step);
        assert_eq!(
            views(&live, actor, &config)
                .into_iter()
                .find(|row| row.capture == chosen),
            Some(historical.clone())
        );
        assert_eq!(
            views(&after_loss, loss_actor, &loss_config)
                .into_iter()
                .find(|row| row.capture == chosen),
            Some(historical.clone())
        );
        live.update();
        after_loss.update();
    }
    same(&live, &after_loss, 720);
}
