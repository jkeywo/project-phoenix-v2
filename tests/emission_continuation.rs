//! Actual admitted profile, ordinary one-System policy and unrestricted Human
//! machinery. Common-status/device presentation has its separate acceptance.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::emission_profile::EmissionIntent;
use project_phoenix::{
    console::command::emission::ShipEmissionIntent,
    core::messages::{ClientMessage, SystemBlackboard, SystemControlPayload as Control, SystemId},
    entities::spawner::{EntityName, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    science::identity::{IdentityEmissions, ShipIdentityEmitters},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const TRANSPONDER: &str = "identity-emitter";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
#[derive(Resource, Default, Debug)]
struct Witness {
    tick: Option<u64>,
    work: f64,
    claim: bool,
    reactor: f64,
    signature: f32,
}
fn observe(
    tick: Res<project_phoenix::sim_tick::SimTick>,
    fixed: Res<Time<Fixed>>,
    ships: Query<(&ShipPowerSystem, &ShipConfigComponent, &IdentityEmissions), With<LocalShip>>,
    mut witness: ResMut<Witness>,
) {
    *witness = Witness {
        tick: Some(tick.0),
        ..Default::default()
    };
    for (power, config, emissions) in &ships {
        witness.work = power.0.emitted_work(&id(TRANSPONDER));
        witness.claim = emissions.claim(tick.0).is_some();
        witness.reactor = if power.0.network().generation_enabled {
            f64::from(power.0.network().output_actual_per_sec)
        } else {
            0.0
        };
        witness.signature = project_phoenix::science::signatures::observed_signature(
            witness.reactor,
            config
                .0
                .systems
                .iter()
                .map(|system| (system.kind.as_str(), power.0.emitted_work(&system.id))),
            fixed.timestep().as_secs_f32(),
        )
        .map_or(0.0, |value| value.0);
    }
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
fn claim(app: &mut App, station: &str, rating: &str) {
    let token = format!("emission-{station}");
    for msg in [
        ClientMessage::Identify {
            token: token.clone(),
            name: token.clone(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating.into(),
        },
    ] {
        send(app, &token, msg);
        steps(app, 4);
    }
}
fn boot(comms_human: bool) -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/emission-world.toml".into(),
        ship_path: "tests/fixtures/emission-policy-cruiser.toml".into(),
        deterministic: true,
        seed: Some(15621727),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<Witness>();
    app.add_systems(
        FixedUpdate,
        observe.in_set(project_phoenix::sim_sets::SimSet::Publish),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let stations = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .clone();
    for station in stations {
        if comms_human || station.id.0 != "comms" {
            claim(&mut app, &station.id.0, &station.ratings[0].name);
        }
    }
    (app, entity)
}
fn control(app: &mut App, entity: Entity, owner: &str, payload: Control) {
    let sessions = app.world().resource::<Sessions>();
    let sources = app.world().get::<ShipSystemControlSources>(entity).unwrap();
    let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    let token = sessions
        .0
        .players()
        .iter()
        .find(|player| {
            project_phoenix::command_admission::policy::is_command_authorized(
                &player.token,
                &id(owner),
                &payload,
                sources,
                sessions,
                config,
                app.world().get::<HumanSeekingHosts>(entity),
            )
        })
        .unwrap_or_else(|| panic!("actual Human authority for{owner}"))
        .token
        .clone();
    send(
        app,
        &token,
        ClientMessage::ControlSystem {
            target: id(owner),
            payload,
        },
    );
    steps(app, 4);
}
fn quiet(app: &App, entity: Entity) -> bool {
    let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    !app.world()
        .get::<ShipIdentityEmitters>(entity)
        .unwrap()
        .enabled(config, &id(TRANSPONDER))
        && app.world().resource::<Witness>().work == 0.0
        && !app.world().resource::<Witness>().claim
}
fn paid(app: &App) -> bool {
    let witness = app.world().resource::<Witness>();
    witness.tick.and_then(|tick| tick.checked_add(1))
        == Some(
            app.world()
                .resource::<project_phoenix::sim_tick::SimTick>()
                .0,
        )
        && witness.work > 0.0
        && witness.claim
}
fn profile(app: &App, entity: Entity) -> EmissionIntent {
    let Some(SystemBlackboard::Command(board)) = app
        .world()
        .get::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .get(&id("command"))
    else {
        panic!("actual Command publication")
    };
    let view = board.emission.as_ref().expect("own selected intent");
    assert_eq!(view.owner, id("command"));
    view.intent
}
#[test]
fn actual_permitted_profile_and_alert_drive_only_the_ordinary_backfill_transponder() {
    let (mut app, entity) = boot(false);
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(entity)
            .unwrap()
            .0
            .source_for(&id(TRANSPONDER)),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
    assert!(paid(&app));
    control(
        &mut app,
        entity,
        "command",
        Control::SetEmissionIntent {
            intent: EmissionIntent::LowEmission,
        },
    );
    for _ in 0..240 {
        if quiet(&app, entity) {
            break;
        }
        app.update();
    }
    assert!(quiet(&app, entity));
    assert_eq!(profile(&app, entity), EmissionIntent::LowEmission);
    control(
        &mut app,
        entity,
        "red-alert",
        Control::SetRedAlert { active: true },
    );
    for _ in 0..240 {
        if paid(&app) {
            break;
        }
        app.update();
    }
    assert!(
        paid(&app),
        "the same ordinary tree may emit under genuinely permitted urgent context"
    );
    assert_eq!(profile(&app, entity), EmissionIntent::LowEmission);
    control(
        &mut app,
        entity,
        "red-alert",
        Control::SetRedAlert { active: false },
    );
    for _ in 0..240 {
        if quiet(&app, entity) {
            break;
        }
        app.update();
    }
    assert!(quiet(&app, entity));
}
fn same(a: &App, b: &App, step: usize) {
    assert_eq!(
        capture(a.world()),
        capture(b.world()),
        "full captured continuation at{step}"
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest at{step}"
    );
}
#[test]
fn human_high_emission_remains_usable_and_intent_survives_whole_json360_and_nowrite_faults() {
    let (mut live, actor) = boot(true);
    control(
        &mut live,
        actor,
        "command",
        Control::SetEmissionIntent {
            intent: EmissionIntent::LowEmission,
        },
    );
    control(
        &mut live,
        actor,
        TRANSPONDER,
        Control::SetIdentityEmitterEnabled { enabled: true },
    );
    assert!(
        paid(&live),
        "LowEmission never rejects, clamps or silently turns off Human machinery"
    );
    assert_eq!(profile(&live, actor), EmissionIntent::LowEmission);
    let hardware = live
        .world()
        .get::<ShipPowerSystem>(actor)
        .unwrap()
        .0
        .config()
        .reactor
        .clone();
    let lower = hardware.sustainable_output_per_sec * 0.5;
    control(
        &mut live,
        actor,
        "power-reactor",
        Control::SetEnergyReactor {
            output_per_sec: lower,
            cooling_per_sec: hardware.default_cooling_per_sec,
        },
    );
    for _ in 0..240 {
        if live.world().resource::<Witness>().reactor <= f64::from(lower) {
            break;
        }
        live.update();
    }
    let before = live.world().resource::<Witness>().reactor;
    assert!(
        before <= f64::from(lower),
        "ordinary Human reactor demand establishes the actual lower output first"
    );
    control(
        &mut live,
        actor,
        "power-reactor",
        Control::SetEnergyReactor {
            output_per_sec: hardware.max_output_per_sec,
            cooling_per_sec: hardware.max_cooling_per_sec,
        },
    );
    for _ in 0..180 {
        if live.world().resource::<Witness>().reactor > before {
            break;
        }
        live.update();
    }
    assert!(live.world().resource::<Witness>().reactor > before);
    assert!(
        f64::from(live.world().resource::<Witness>().signature)
            >= live.world().resource::<Witness>().reactor
    );
    // Return to the actual sustainable request before the continuation window.
    control(
        &mut live,
        actor,
        "power-reactor",
        Control::SetEnergyReactor {
            output_per_sec: hardware.sustainable_output_per_sec,
            cooling_per_sec: hardware.default_cooling_per_sec,
        },
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let uuid = live.world().get::<EntityUuid>(actor).unwrap().0.clone();
    let observer = live
        .world_mut()
        .query::<(&EntityUuid, &EntityName)>()
        .iter(live.world())
        .find(|(_, name)| name.0 == "ordinary-observer")
        .unwrap()
        .0
         .0
        .clone();
    let (mut resumed, restored) = boot(true);
    assert!(ready_to_restore(resumed.world(), &saved));
    let original = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut value = serde_json::to_value(&saved).unwrap();
        let rows = value["entities"].as_array_mut().unwrap();
        let source = rows.iter().position(|row| row["uuid"] == uuid).unwrap();
        match fault {
            0 => rows[source]["emission_intent"] = serde_json::Value::Null,
            1 => rows[source]["emission_intent"]["owner"] = "foreign-command".into(),
            _ => {
                let state = rows[source]["emission_intent"].clone();
                let target = rows.iter().position(|row| row["uuid"] == observer).unwrap();
                rows[target]["emission_intent"] = state;
            }
        }
        let bad: PhoenixSnapshot = serde_json::from_value(value).unwrap();
        assert!(
            !restore(resumed.world_mut(), &bad).is_complete(),
            "malformed exact actual owner{fault}"
        );
        assert_eq!(capture(resumed.world()), original);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{result:?}");
    assert_eq!(
        resumed
            .world()
            .get::<ShipEmissionIntent>(restored)
            .unwrap()
            .0
            .intent,
        EmissionIntent::LowEmission
    );
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
        assert!(paid(&live) && paid(&resumed));
        assert_eq!(profile(&live, actor), EmissionIntent::LowEmission);
        assert_eq!(profile(&resumed, restored), EmissionIntent::LowEmission);
    }
    same(&live, &resumed, 360);
}
