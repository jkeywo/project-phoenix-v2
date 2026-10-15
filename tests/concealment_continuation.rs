//! Real field work, independent sensing and admitted reactor operation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{ClientMessage, SystemControlPayload as Control, SystemId},
    entities::spawner::{EntityName, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::server::InboundMessage,
    science::{
        broad::BroadTracks,
        concealment::{ConcealmentFields, ShipConcealmentEmitters},
        passive::PassiveState,
        server::ShipScanRecord,
    },
    server_app::LocalShip,
    ship::{power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const OWNER: &str = "concealment";
const SCIENCE: &str = "cloak-scientist";
const ENGINEERING: &str = "cloak-engineer";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
#[derive(Resource, Default, Debug)]
struct PaidWitness {
    tick: Option<u64>,
    work: f64,
    emission: f32,
    factor: f32,
    quiet_factor: f32,
}
fn observe(
    tick: Res<project_phoenix::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    owners: Query<(&ShipPowerSystem, &ShipConfigComponent, &ConcealmentFields), With<LocalShip>>,
    mut witness: ResMut<PaidWitness>,
) {
    *witness = PaidWitness {
        tick: Some(tick.0),
        ..Default::default()
    };
    for (power, config, fields) in &owners {
        let output = if power.0.network().generation_enabled {
            f64::from(power.0.network().output_actual_per_sec)
        } else {
            0.0
        };
        let emission = project_phoenix::science::signatures::observed_signature(
            output,
            config
                .0
                .systems
                .iter()
                .map(|system| (system.kind.as_str(), power.0.emitted_work(&system.id))),
            fixed.timestep().as_secs_f32(),
        )
        .map_or(0.0, |signature| signature.0);
        witness.work = power.0.emitted_work(&id(OWNER));
        witness.emission = emission;
        witness.factor = fields.perceptibility(tick.0, emission);
        witness.quiet_factor = fields.perceptibility(tick.0, 0.0);
    }
}
fn boot() -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/concealment-world.toml".into(),
        ship_path: "assets/entities/concealment_cruiser.toml".into(),
        deterministic: true,
        seed: Some(15621726),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<PaidWitness>();
    app.add_systems(
        FixedUpdate,
        observe.in_set(project_phoenix::sim_sets::SimSet::Publish),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    let actor = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    (app, actor)
}
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn claim(app: &mut App, actor: Entity, station: &str, token: &str) {
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
        .iter()
        .find(|rating|station!="science" || rating.name=="Std")
        .unwrap()
        .name
        .clone();
    for msg in [
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
        send(app, token, msg);
        steps(app, 4);
    }
}
fn control(app: &mut App, token: &str, target: &str, payload: Control) {
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
fn field(app: &App, actor: Entity) -> bool {
    app.world()
        .get::<ShipConcealmentEmitters>(actor)
        .unwrap()
        .enabled(
            &app.world().get::<ShipConfigComponent>(actor).unwrap().0,
            &id(OWNER),
        )
}
fn find_observer(app: &mut App) -> Entity {
    app.world_mut()
        .query::<(Entity, &EntityName)>()
        .iter(app.world())
        .find_map(|(entity, name)| (name.0 == "Observer").then_some(entity))
        .unwrap()
}
fn current_alias(app: &App, observer: Entity, owner: &str, source: &str) -> Option<String> {
    let state = app
        .world()
        .get::<BroadTracks>(observer)?
        .0
        .get(&id(owner))?;
    state
        .tracks
        .keys()
        .find(|alias| state.resolve_current(alias) == Some(source))
        .cloned()
}
fn record(app: &App, observer: Entity) -> &project_phoenix::science::server::FocusedScanRecord {
    &app.world().get::<ShipScanRecord>(observer).unwrap().0[&id("sensors")]
}
fn passive(
    app: &App,
    observer: Entity,
) -> &project_phoenix::science::passive::PassiveReceiverState {
    &app.world().get::<PassiveState>(observer).unwrap().0[&id("passive-receiver")]
}
#[track_caller]
fn assert_paid(app: &App, actor: Entity) {
    let witness = app.world().resource::<PaidWitness>();
    let tick = app
        .world()
        .resource::<project_phoenix::sim_tick::SimTick>()
        .0;
    assert_eq!(
        witness.tick.and_then(|tick| tick.checked_add(1)),
        Some(tick)
    );
    assert!(
        field(app, actor) && witness.work > 0.0 && witness.factor > 0.0 && witness.factor < 1.0,
        "actual enabled paid field: {witness:?}"
    );
    assert!(
        (witness.quiet_factor - 0.15).abs() < 0.00001,
        "reference field is fully funded: {witness:?}"
    );
}
fn same(a: &App, b: &App, step: usize) {
    assert_eq!(
        capture(a.world()),
        capture(b.world()),
        "complete capture at{step}"
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "complete digest at{step}"
    );
}
fn quiet(app: &mut App, actor: Entity, observer: Entity, source: &str) -> bool {
    let witness = app.world().resource::<PaidWitness>();
    field(app, actor)
        && witness.work > 0.0
        && witness.factor < 0.3
        && current_alias(app, observer, "helm-radar", source).is_none()
        && current_alias(app, observer, "sensor-radar", source).is_some()
        && record(app, observer).refusal
            == Some(project_phoenix::science::scan::ScanRefusal::OutOfRange)
        && passive(app, observer).signals.is_empty()
}
#[test]
fn actual_ai_human_field_loses_and_recovers_real_sensing_with_intent_on_and_json_360() {
    let (mut live, actor) = boot();
    let observer = find_observer(&mut live);
    let source = live.world().get::<EntityUuid>(actor).unwrap().0.clone();
    // The existing ordinary System tree starts from authored disabled intent.
    for _ in 0..240 {
        if field(&live, actor) && live.world().resource::<PaidWitness>().work > 0.0 {
            break;
        }
        live.update();
    }
    assert!(
        live.world()
            .get::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .policy_for(&id(OWNER))
            .operate_ai
    );
    assert_paid(&live, actor);
    claim(&mut live, actor, "science", SCIENCE);
    claim(&mut live, actor, "engineering", ENGINEERING);
    control(
        &mut live,
        SCIENCE,
        OWNER,
        Control::SetConcealmentEnabled { enabled: false },
    );
    assert!(!field(&live, actor));
    let mut alias = None;
    for _ in 0..600 {
        if let Some(earned) = current_alias(&live, observer, "helm-radar", &source) {
            if record(&live, observer)
                .focused
                .as_ref()
                .is_some_and(|work| work.progress > 0.0)
                && !passive(&live, observer).signals.is_empty()
            {
                alias = Some(earned);
                break;
            }
        }
        live.update();
    }
    let alias =
        alias.expect("real broad, focused and passive work must first acquire the actual ship");
    let original_focus = record(&live, observer).focused.as_ref().unwrap().clone();
    assert!(original_focus.progress > 0.0 && original_focus.progress < original_focus.work_secs);
    control(
        &mut live,
        SCIENCE,
        OWNER,
        Control::SetConcealmentEnabled { enabled: true },
    );
    control(
        &mut live,
        ENGINEERING,
        "power-reactor",
        Control::SetEnergyGeneration { enabled: false },
    );
    for _ in 0..180 {
        if quiet(&mut live, actor, observer, &source) {
            break;
        }
        live.update();
    }
    assert!(
        quiet(&mut live, actor, observer, &source),
        "actual quiet acquisition: {:?}",
        live.world().resource::<PaidWitness>()
    );
    assert_paid(&live, actor);
    assert!(
        live.world().get::<BroadTracks>(observer).unwrap().0[&id("helm-radar")]
            .tracks
            .contains_key(&alias),
        "lost acquisition retains the earned alias/history rather than deleting it"
    );
    let held = record(&live, observer).focused.as_ref().unwrap().clone();
    assert_eq!(held.presentation_id, original_focus.presentation_id);
    assert_eq!(held.subject, original_focus.subject);
    let held_progress = held.progress;
    let lost_sample = passive(&live, observer).last_sample_tick.unwrap();
    assert!(
        record(&live, observer).history.is_empty(),
        "refusal is not a fabricated completed reading"
    );
    let start = live.world().get::<ShipPhysics>(actor).unwrap();
    let other = live.world().get::<ShipPhysics>(observer).unwrap();
    assert!(
        ((start.x - other.x).hypot(start.z - other.z) - 50.0).abs() < 0.1,
        "real fixture separation excludes movement as acquisition cause"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = boot();
    let restored_observer = find_observer(&mut resumed);
    for _ in 0..240 {
        if field(&resumed, restored) && resumed.world().resource::<PaidWitness>().work > 0.0 {
            break;
        }
        resumed.update();
    }
    assert_paid(&resumed, restored);
    claim(&mut resumed, restored, "science", SCIENCE);
    claim(&mut resumed, restored, "engineering", ENGINEERING);
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == source)
            .unwrap();
        if fault == 0 {
            row.concealment_emitters = None;
        } else {
            let state = row.concealment_emitters.as_mut().unwrap();
            if fault == 1 {
                state.0.get_mut(&id(OWNER)).unwrap().0.insert(
                    "enabled".into(),
                    project_phoenix::ship::composition::CommandChannelValue::EntityTarget(None),
                );
            } else {
                let channels = state.0.remove(&id(OWNER)).unwrap();
                state.0.insert(id("foreign-owner"), channels);
            }
        }
        assert!(!restore(resumed.world_mut(), &bad).is_complete());
        assert_eq!(
            capture(resumed.world()),
            before,
            "candidate refuses before writes{fault}"
        );
        assert_eq!(world_digest(resumed.world()), digest);
    }
    let bootstrap_tick = resumed.world().resource::<PaidWitness>().tick.unwrap();
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{result:?}");
    let fields = resumed.world().get::<ConcealmentFields>(restored).unwrap();
    assert_eq!(
        fields.perceptibility(bootstrap_tick, 0.0),
        1.0,
        "restore clears bootstrap field work"
    );
    assert_eq!(
        fields.perceptibility(saved.tick.saturating_sub(1), 0.0),
        1.0,
        "restore grants no saved prepaid field"
    );
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
        assert_paid(&live, actor);
        assert_paid(&resumed, restored);
    }
    same(&live, &resumed, 360);
    assert!(quiet(&mut live, actor, observer, &source));
    assert!(quiet(&mut resumed, restored, restored_observer, &source));
    assert_eq!(
        record(&live, observer).focused.as_ref().unwrap().progress,
        held_progress,
        "unavailable focused acquisition earns no work during quiet continuation"
    );
    assert!(
        passive(&live, observer).last_sample_tick.unwrap() > lost_sample,
        "an empty current paid passive sample replaces earlier detections"
    );
    // Actual Engineering operation raises real output while Human cloak intent
    // stays enabled. Neither scan work nor detection is granted by the command.
    for (app, ship) in [(&mut live, actor), (&mut resumed, restored)] {
        let reactor = &app
            .world()
            .get::<ShipPowerSystem>(ship)
            .unwrap()
            .0
            .config()
            .reactor;
        let cooling = reactor.max_cooling_per_sec;
        let output = reactor.max_output_per_sec;
        control(
            app,
            ENGINEERING,
            "power-reactor",
            Control::SetEnergyGeneration { enabled: true },
        );
        control(
            app,
            ENGINEERING,
            "power-reactor",
            Control::SetEnergyReactor {
                output_per_sec: output,
                cooling_per_sec: cooling,
            },
        );
    }
    let mut recovered = false;
    for step in 0..720 {
        same(&live, &resumed, 360 + step);
        let physical = live.world().resource::<PaidWitness>();
        recovered = physical.factor > 0.54
            && current_alias(&live, observer, "helm-radar", &source).as_deref()
                == Some(alias.as_str())
            && record(&live, observer)
                .focused
                .as_ref()
                .is_some_and(|work| work.progress > held_progress)
            && !passive(&live, observer).signals.is_empty();
        if recovered {
            break;
        }
        live.update();
        resumed.update();
        assert_paid(&live, actor);
        assert_paid(&resumed, restored);
    }
    assert!(
        recovered,
        "actual paid emissions must recover acquisition with field still enabled: {:?}",
        live.world().resource::<PaidWitness>()
    );
    assert!(field(&live, actor) && field(&resumed, restored));
    // Merely sharing the same ship and receiving ordinary status grants the
    // Engineering player no authority over Science's independently owned field.
    control(
        &mut live,
        ENGINEERING,
        OWNER,
        Control::SetConcealmentEnabled { enabled: false },
    );
    control(
        &mut resumed,
        ENGINEERING,
        OWNER,
        Control::SetConcealmentEnabled { enabled: false },
    );
    assert!(field(&live, actor) && field(&resumed, restored));
    same(&live, &resumed, 2000);
    assert_eq!(
        live.world().get::<EntityName>(actor).unwrap().0,
        "Veiled ship"
    );
    assert_eq!(
        current_alias(&resumed, restored_observer, "helm-radar", &source).as_deref(),
        Some(alias.as_str())
    );
    assert_eq!(
        record(&resumed, restored_observer)
            .focused
            .as_ref()
            .unwrap()
            .presentation_id,
        held.presentation_id
    );
}
