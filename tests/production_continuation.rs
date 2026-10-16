//! Real finite fabrication: ordinary crew setup, delegated second batch,
//! paid WIP and exact whole-host JSON continuation. No machinery state is seeded.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    console::weapons::torpedo::TorpedoSystemResource,
    core::messages::{
        ClientMessage, GamePhase, StationId, SystemControlPayload as Control, SystemId,
    },
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    resources::{
        logistics_server::{CompiledShipLogistics, ShipLogistics},
        production_server::{CompiledShipProduction, ShipProduction},
    },
    server_app::LocalShip,
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};

const TOKEN: &str = "production-engineer";
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
fn send(app: &mut App, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: TOKEN.into(),
        msg,
    });
}
fn control(app: &mut App, entity: Entity, system: &str, payload: Control) {
    let sessions = app.world().resource::<Sessions>();
    let player = sessions
        .0
        .players()
        .iter()
        .find(|player| player.token == TOKEN)
        .unwrap();
    assert!(player.connected && player.ready);
    assert_eq!(
        player.station.as_ref(),
        Some(&StationId("engineering".into()))
    );
    assert_eq!(
        sessions
            .0
            .holder_for_station(&StationId("engineering".into())),
        Some(TOKEN)
    );
    assert!(
        project_phoenix::command_admission::policy::is_command_authorized(
            TOKEN,
            &id(system),
            &payload,
            app.world().get::<ShipSystemControlSources>(entity).unwrap(),
            sessions,
            &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
            app.world().get::<HumanSeekingHosts>(entity),
        )
    );
    send(
        app,
        ClientMessage::ControlSystem {
            target: id(system),
            payload,
        },
    );
    steps(app, 4);
}
fn stock(app: &App, entity: Entity, capacity: &str) -> project_phoenix::resources::Stock {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock(capacity)
        .unwrap()
        .clone()
}
fn state(app: &App, entity: Entity) -> &project_phoenix::resources::production::ProductionState {
    &app.world().get::<ShipProduction>(entity).unwrap().0
}
fn work_secs(app: &App, entity: Entity) -> f32 {
    app.world()
        .get::<CompiledShipProduction>(entity)
        .unwrap()
        .0
        .recipe("torpedoes")
        .unwrap()
        .work_secs
}
fn period(app: &App) -> f32 {
    sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz).as_secs_f32()
}
fn rounds(app: &App, entity: Entity) -> i64 {
    let weapons = &app.world().get::<TorpedoSystemResource>(entity).unwrap().0;
    assert!(
        weapons.in_flight.is_empty(),
        "quiet actual-hull fixture cannot expend ammunition"
    );
    i64::from(weapons.rounds_aboard())
}
/// Reservations remain in stock.level; physical batches have their own amount.
fn totals(app: &App, entity: Entity) -> [i64; 3] {
    let mut amount = [
        stock(app, entity, "hold-ammunition").level + rounds(app, entity),
        stock(app, entity, "alloy").level,
        stock(app, entity, "hold-machinery-parts").level
            + stock(app, entity, "machinery_parts").level,
    ];
    let profile = &app.world().get::<CompiledShipLogistics>(entity).unwrap().0;
    for batch in app
        .world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .values()
    {
        match profile.store(&batch.source).unwrap().resource.as_str() {
            "torpedoes" => amount[0] += batch.amount,
            "machinery_parts" => amount[2] += batch.amount,
            _ => {}
        }
    }
    amount
}
fn conserved(app: &App, entity: Entity, initial: [i64; 3]) {
    let production = state(app, entity);
    assert!(production
        .last_outcome
        .as_ref()
        .is_none_or(|outcome| !outcome.discarded));
    let started = i64::try_from(production.next_id - 1).unwrap();
    let completed = started - i64::from(production.active.is_some());
    assert_eq!(
        totals(app, entity),
        [
            initial[0] + completed,
            initial[1] - 2 * started,
            initial[2] - started
        ]
    );
    let incoming = stock(app, entity, "hold-ammunition").incoming;
    if let Some(batch) = &production.active {
        assert_eq!(
            incoming.get(&production.claim(batch.id, "hold-ammunition")),
            Some(&1)
        );
        assert_eq!(
            batch.inputs,
            app.world()
                .get::<CompiledShipProduction>(entity)
                .unwrap()
                .0
                .recipe("torpedoes")
                .unwrap()
                .inputs
        );
    } else {
        assert!(!incoming
            .keys()
            .any(|claim| claim.starts_with("production/")));
    }
}
fn prepared() -> (App, Entity, [i64; 3], i64) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/physical-route-world.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156_123),
        ..Default::default()
    })
    .unwrap();
    let dt = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(dt));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );
    let entity = local(&mut app);
    let rating = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|station| station.id.0 == "engineering")
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
        ClientMessage::SelectStation {
            station: "engineering".into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ] {
        send(&mut app, msg);
        steps(&mut app, 4);
    }
    assert!(
        app.world()
            .get::<ShipSystemControlSources>(entity)
            .unwrap()
            .0
            .policy_for(&id("fabrication"))
            .accept_human_input
    );
    // Let only already-admitted real deliveries settle after taking this seat.
    // The bound follows authored loading/unloading and all compartment edges.
    let profile = &app.world().get::<CompiledShipLogistics>(entity).unwrap().0;
    let route: f32 = profile
        .graph()
        .config
        .connections
        .iter()
        .map(|edge| edge.travel_secs)
        .sum();
    let seconds =
        2.0 * profile.handling().max_batch as f32 / profile.handling().units_per_sec + route;
    let limit = (seconds / period(&app)).ceil() as usize + 120;
    for _ in 0..limit {
        if app
            .world()
            .get::<ShipLogistics>(entity)
            .unwrap()
            .0
            .batches
            .is_empty()
        {
            break;
        }
        app.update();
    }
    assert!(
        app.world()
            .get::<ShipLogistics>(entity)
            .unwrap()
            .0
            .batches
            .is_empty(),
        "real previously admitted delivery must settle"
    );
    // Keep ordinary Logistics enabled; explicitly retain the actual magazine
    // stock as its normal desired level so new products remain in the hold.
    let aboard = rounds(&app, entity);
    control(
        &mut app,
        entity,
        "logistics",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: aboard,
        },
    );
    assert_eq!(
        app.world().get::<ShipLogistics>(entity).unwrap().0.goals["magazine"],
        aboard
    );
    assert!(app.world().get::<ShipLogistics>(entity).unwrap().0.enabled);
    assert!(state(&app, entity).active.is_none());
    assert_eq!(state(&app, entity).next_id, 1);
    assert_eq!(state(&app, entity).target, 0);
    let initial = totals(&app, entity);
    assert_eq!(initial[0], 6);
    let target = stock(&app, entity, "hold-ammunition").level + 2;
    control(
        &mut app,
        entity,
        "fabrication",
        Control::SetProductionTarget { amount: target },
    );
    let first = state(&app, entity)
        .active
        .as_ref()
        .expect("ordinary Human standing goal must start actual paid work");
    assert_eq!(first.id.0, 1);
    assert!(first.progress > 0.0 && first.progress < work_secs(&app, entity));
    conserved(&app, entity, initial);
    send(&mut app, ClientMessage::ReleaseStation);
    steps(&mut app, 4);
    let source = app
        .world()
        .get::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .policy_for(&id("fabrication"));
    assert!(source.operate_ai && !source.accept_human_input);
    let first = state(&app, entity).active.as_ref().unwrap();
    assert_eq!(
        first.id.0, 1,
        "only the first batch existed at Human handoff"
    );
    assert!(first.progress > 0.0 && first.progress < work_secs(&app, entity));
    assert!(app
        .world()
        .resource::<Sessions>()
        .0
        .holder_for_station(&StationId("engineering".into()))
        .is_none());
    (app, entity, initial, target)
}
fn checkpoint() -> (App, Entity, [i64; 3], i64) {
    let (mut app, entity, initial, target) = prepared();
    let work = work_secs(&app, entity);
    let limit = (2.0 * work / period(&app)).ceil() as usize + 120;
    for _ in 0..limit {
        app.update();
        conserved(&app, entity, initial);
        let production = state(&app, entity);
        if production.active.as_ref().is_some_and(|batch| {
            batch.id.0 == 2 && batch.progress > work * 0.5 && batch.progress < work
        }) {
            assert_eq!(production.last_outcome.as_ref().unwrap().id.0, 1);
            assert!(
                app.world()
                    .get::<ShipPowerSystem>(entity)
                    .unwrap()
                    .0
                    .emitted_work(&id("fabrication"))
                    > 0.0
            );
            assert!(
                app.world()
                    .get::<ShipSystemControlSources>(entity)
                    .unwrap()
                    .0
                    .policy_for(&id("fabrication"))
                    .operate_ai
            );
            return (app, entity, initial, target);
        }
    }
    panic!("ordinary installed fabrication AI must start and pay into a second real batch");
}
#[test]
fn paid_second_fabrication_batch_and_exact_claims_continue_through_full_json360() {
    let (mut live, entity, initial, target) = checkpoint();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored, restored_initial, restored_target) = prepared();
    assert_eq!((restored_initial, restored_target), (initial, target));
    assert_ne!(capture(live.world()), capture(resumed.world()));
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..=360 {
        assert_eq!(
            capture(live.world()),
            capture(resumed.world()),
            "complete capture at {step}"
        );
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "digest at {step}"
        );
        conserved(&live, entity, initial);
        conserved(&resumed, restored, initial);
        if step < 360 {
            let tick = live.world().resource::<SimTick>().0;
            live.update();
            resumed.update();
            assert_eq!(live.world().resource::<SimTick>().0, tick + 1);
            assert_eq!(resumed.world().resource::<SimTick>().0, tick + 1);
        }
    }
    for (app, entity) in [(&live, entity), (&resumed, restored)] {
        let production = state(app, entity);
        assert!(production.active.is_none());
        assert_eq!(production.target, target);
        assert_eq!(
            production.next_id, 3,
            "exactly two batches; no incoming/output double credit"
        );
        let outcome = production.last_outcome.as_ref().unwrap();
        assert_eq!(outcome.id.0, 2);
        assert!(!outcome.discarded);
        assert_eq!(stock(app, entity, "hold-ammunition").level, target);
        assert_eq!(totals(app, entity), [8, initial[1] - 4, initial[2] - 2]);
    }
}
#[test]
fn malformed_production_owner_wip_and_receiving_claim_refuse_before_live_writes() {
    let (mut app, entity, initial, _) = checkpoint();
    let uuid = app.world().get::<EntityUuid>(entity).unwrap().0.clone();
    let mut valid = capture(app.world());
    let row = valid
        .entities
        .iter_mut()
        .find(|row| row.uuid == uuid)
        .unwrap();
    row.red_alert = Some(!row.red_alert.unwrap_or(false));
    let (mut positive, _, _, _) = prepared();
    let report = restore(positive.world_mut(), &valid);
    assert!(
        report.is_complete(),
        "unchanged WIP plus unrelated body change is admissible: {:?}",
        report.gaps
    );
    for fault in 0..7 {
        let mut bad = valid.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == uuid)
            .unwrap();
        match fault {
            0 => row.production = None,
            1 => row.production.as_mut().unwrap().owner = id("logistics"),
            2 => {
                row.production
                    .as_mut()
                    .unwrap()
                    .active
                    .as_mut()
                    .unwrap()
                    .inputs[0]
                    .amount += 1
            }
            3 => {
                row.production
                    .as_mut()
                    .unwrap()
                    .active
                    .as_mut()
                    .unwrap()
                    .progress = 0.0
            }
            4 => {
                row.production
                    .as_mut()
                    .unwrap()
                    .active
                    .as_mut()
                    .unwrap()
                    .born_tick = bad.tick + 1
            }
            5 => row.production.as_mut().unwrap().next_id = 2,
            _ => {
                let infra = row.infrastructure.as_mut().unwrap();
                let mut stock = infra.capacity_stock("hold-ammunition").unwrap().clone();
                let (claim, amount) = stock.incoming.pop_first().unwrap();
                assert!(claim.starts_with("production/"));
                stock
                    .incoming
                    .insert("foreign-production/999".into(), amount);
                stock.validate().unwrap();
                infra
                    .replace_capacity_stock("hold-ammunition", stock)
                    .unwrap();
            }
        }
        let bad: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&bad).unwrap()).unwrap();
        let before = capture(app.world());
        let digest = world_digest(app.world());
        let count = app.world().entities().len();
        let report = restore(app.world_mut(), &bad);
        assert!(!report.is_complete(), "fault {fault}");
        assert_eq!(
            (
                report.entities_restored,
                report.entities_spawned,
                report.despawned
            ),
            (0, 0, 0)
        );
        assert_eq!(app.world().entities().len(), count);
        assert_eq!(capture(app.world()), before);
        assert_eq!(world_digest(app.world()), digest);
        conserved(&app, entity, initial);
    }
}
