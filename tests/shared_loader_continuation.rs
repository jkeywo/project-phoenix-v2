//! Actual finite shared handling via ordinary joined controls and Backfill.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    console::weapons::TorpedoSystemResource,
    core::messages::{ClientMessage, GamePhase, SystemControlPayload as Control, SystemId},
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    server_app::LocalShip,
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
fn id(value: &str) -> SystemId {
    SystemId(value.into())
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
    let token = format!("loader-{station}");
    for msg in [
        ClientMessage::Identify {
            token: token.clone(),
            name: token.clone(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetStationRating {
            rating_name: rating.into(),
        },
        ClientMessage::SetReady { ready: true },
    ] {
        send(app, &token, msg);
        steps(app, 4);
    }
}
fn boot(tactical_human: bool) -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/shared-loader-world.toml".into(),
        ship_path: "assets/entities/shared_loader_cruiser.toml".into(),
        deterministic: true,
        seed: Some(15621504),
        ..Default::default()
    })
    .unwrap();
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
        if tactical_human || station.id.0 != "tactical" {
            claim(&mut app, &station.id.0, &station.ratings[0].name);
        }
    }
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "ordinary bootstrap reaches the actual mission phase"
    );
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

fn ammunition(app: &App, entity: Entity) -> i64 {
    let hold = app
        .world()
        .get::<project_phoenix::infrastructure::InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock("hold-ammunition")
        .unwrap()
        .level;
    let local = i64::from(
        app.world()
            .get::<TorpedoSystemResource>(entity)
            .unwrap()
            .0
            .rounds_aboard(),
    );
    let transit: i64 = app
        .world()
        .get::<project_phoenix::resources::logistics_server::ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .values()
        .filter(|batch| {
            batch.source == "hold-ammunition" || batch.original_destination == "magazine"
        })
        .map(|batch| batch.amount)
        .sum();
    hold + local + transit
}
fn prepared(app: &App, entity: Entity) -> f32 {
    let system = &app.world().get::<TorpedoSystemResource>(entity).unwrap().0;
    ["fore_port", "fore_starboard"]
        .iter()
        .map(|id| {
            let tube = system.tube(id).unwrap();
            tube.loaded_count as f32 * tube.load_time
                + match tube.load_state {
                    project_phoenix::weapons::torpedo::TubeLoadState::Loading {
                        remaining,
                        total,
                    } => total - remaining,
                    _ => 0.0,
                }
        })
        .sum()
}
fn same(live: &App, resumed: &App, step: usize) {
    assert_eq!(
        capture(live.world()),
        capture(resumed.world()),
        "complete capture step {step}"
    );
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "digest step {step}"
    );
}
#[test]
fn joined_priority_and_partial_shared_loading_survive_whole_json360_and_nowrite_guards() {
    let (mut live, actor) = boot(true);
    for tube in ["fore_port", "fore_starboard", "aft"] {
        control(
            &mut live,
            actor,
            &format!("torpedo-tube-{}", tube.replace('_', "-")),
            Control::SetTorpedoVolleyTarget { count: 0 },
        );
    }
    for _ in 0..1800 {
        if live
            .world()
            .get::<TorpedoSystemResource>(actor)
            .unwrap()
            .0
            .tubes
            .iter()
            .all(|tube| tube.loaded_count == 0)
        {
            break;
        }
        live.update();
    }
    assert!(live
        .world()
        .get::<TorpedoSystemResource>(actor)
        .unwrap()
        .0
        .tubes
        .iter()
        .all(|tube| tube.loaded_count == 0));
    control(
        &mut live,
        actor,
        "torpedo-tube-fore-starboard",
        Control::SetTorpedoLoadPriority { priority: 1 },
    );
    for tube in ["fore_port", "fore_starboard"] {
        control(
            &mut live,
            actor,
            &format!("torpedo-tube-{}", tube.replace('_', "-")),
            Control::SetTorpedoVolleyTarget { count: 1 },
        );
    }
    steps(&mut live, 12);
    let system = &live.world().get::<TorpedoSystemResource>(actor).unwrap().0;
    assert_eq!(system.tube("fore_starboard").unwrap().load_priority, 1);
    assert!(
        system.tube("fore_starboard").unwrap().load_progress()
            > system.tube("fore_port").unwrap().load_progress(),
        "the higher priority actually receives handling"
    );
    assert!(prepared(&live, actor) > 0.0);
    let uuid = live.world().get::<EntityUuid>(actor).unwrap().0.clone();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    for fault in 0..2 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == uuid)
            .unwrap();
        let tubes = &mut row.weapons.as_mut().unwrap().tubes;
        if fault == 0 {
            tubes
                .iter_mut()
                .find(|tube| tube.id == "fore_port")
                .unwrap()
                .load_priority = None;
        } else {
            tubes
                .iter_mut()
                .find(|tube| tube.id == "aft")
                .unwrap()
                .load_priority = Some(1);
        }
        let before = capture(live.world());
        let digest = world_digest(live.world());
        assert!(!restore(live.world_mut(), &bad).gaps.is_empty());
        assert_eq!(before, capture(live.world()));
        assert_eq!(digest, world_digest(live.world()));
    }
    let (mut resumed, _) = boot(true);
    assert!(ready_to_restore(resumed.world(), &saved));
    assert!(restore(resumed.world_mut(), &saved).gaps.is_empty());
    same(&live, &resumed, 0);
    let initial = prepared(&live, actor);
    let mut work_seen = false;
    for step in 1..=360 {
        let before = prepared(&live, actor);
        live.update();
        resumed.update();
        same(&live, &resumed, step);
        let after = prepared(&live, actor);
        assert_eq!(ammunition(&live, actor), 6);
        assert!(after >= before);
        let dt = live
            .world()
            .resource::<Time<Fixed>>()
            .timestep()
            .as_secs_f32();
        assert!(
            after - before <= dt + 0.0001,
            "shared rate is aggregate physical progress"
        );
        work_seen |= live
            .world()
            .get::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .emitted_work(&id("torpedo-tube-fore-starboard"))
            > 0.0;
    }
    assert!(
        work_seen,
        "ordinary progressing hardware spends real local energy"
    );
    assert!(prepared(&live, actor) > initial);
}
#[test]
fn ordinary_backfill_prepares_grouped_salvo_from_real_finite_magazine() {
    let (mut app, actor) = boot(false);
    let sources = app.world().get::<ShipSystemControlSources>(actor).unwrap();
    assert!(
        sources
            .0
            .policy_for(&id("torpedo-tube-fore-port"))
            .operate_ai
    );
    for _ in 0..3600 {
        if ["fore_port", "fore_starboard"].iter().all(|id| {
            app.world()
                .get::<TorpedoSystemResource>(actor)
                .unwrap()
                .0
                .tube(id)
                .unwrap()
                .loaded_count
                > 0
        }) {
            break;
        }
        app.update();
    }
    let system = &app.world().get::<TorpedoSystemResource>(actor).unwrap().0;
    system.validate_magazine().unwrap();
    assert!(
        system.tube("fore_port").unwrap().loaded_count > 0
            && system.tube("fore_starboard").unwrap().loaded_count > 0
    );
    assert!(system.in_flight.is_empty());
    assert_eq!(ammunition(&app, actor), 6);
}
