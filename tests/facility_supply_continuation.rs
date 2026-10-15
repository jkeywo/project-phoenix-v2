//! Actual two-facility supply via ordinary per-System AI and admitted crew work.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{
        AdmittedCommands, ClientMessage, StationId, SystemControlPayload as Control, SystemId,
    },
    dock::DockControl,
    entities::spawner::{EntityId, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    resources::logistics_server::{CompiledShipLogistics, ShipLogistics},
    server_app::LocalShip,
    ship::{power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_sets::{FixedStep, SimSet},
    sim_tick::sim_tick_period,
    snapshot::{capture, restore, PhoenixSnapshot},
    umbilical::{TransferUmbilical, UmbilicalDirection},
    world::config::WorldConfig,
};
use std::collections::BTreeSet;
const WORLD: &str = "tests/fixtures/facility-supply-world.toml";
const FULL: &str = "tests/fixtures/facility-supply-full-world.toml";
const HULL: &str = "assets/entities/facility_supply_cruiser.toml";
fn sid(value: &str) -> SystemId {
    SystemId(value.into())
}
#[derive(Resource, Default, Debug)]
struct WorkEvidence {
    providers: BTreeSet<String>,
    umbilical: bool,
    ordinary_start: bool,
    loaded_helm_work: bool,
}
fn observe_work(
    mut evidence: ResMut<WorkEvidence>,
    providers: Query<(&EntityUuid, &ShipPowerSystem), Without<LocalShip>>,
    own: Query<
        (
            &ShipPowerSystem,
            &AdmittedCommands,
            &DockControl,
            &ShipPhysics,
            &InfrastructureCondition,
        ),
        With<LocalShip>,
    >,
) {
    for (uuid, power) in &providers {
        if power.0.emitted_work(&sid("dock")) > 0.0 {
            evidence.providers.insert(uuid.0.clone());
        }
    }
    if let Ok((power, commands, dock, physics, stock)) = own.single() {
        evidence.loaded_helm_work |= stock.0.capacity("alloy").is_some_and(|amount| amount > 0)
            && dock.docked_partner().is_none()
            && dock.undock_target.is_none()
            && (physics.forward_speed != 0.0 || physics.lateral_speed != 0.0)
            && power.0.emitted_work(&sid("helm-thrust")) > 0.0;
        evidence.umbilical |= power.0.emitted_work(&sid("umbilical")) > 0.0;
        evidence.ordinary_start |= commands.for_target("umbilical").any(|command| {
            project_phoenix::command_admission::ai_emit::is_ai_token(
                command.response_token.as_deref(),
            ) && matches!(command.payload, Control::StartTransfer)
        });
    }
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
fn actor(app: &mut App, name: &str) -> (Entity, String) {
    app.world_mut()
        .query::<(Entity, &EntityId, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, id, _)| id.0 == name)
        .map(|(entity, _, uuid)| (entity, uuid.0.clone()))
        .unwrap()
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn claim(app: &mut App, station: &str) {
    let own = local(app);
    let rating = app
        .world()
        .get::<ShipConfigComponent>(own)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|row| row.id.0 == station)
        .unwrap()
        .ratings[0]
        .name
        .clone();
    for msg in [
        ClientMessage::Identify {
            token: station.into(),
            name: station.into(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ] {
        send(app, station, msg);
        steps(app, 4);
    }
}
fn control(app: &mut App, station: &str, owner: &str, payload: Control) {
    let own = local(app);
    for _ in 0..120 {
        let sessions = app.world().resource::<Sessions>();
        if sessions.0.holder_for_station(&StationId(station.into())) == Some(station)
            && project_phoenix::command_admission::policy::is_command_authorized(
                station,
                &sid(owner),
                &payload,
                app.world().get::<ShipSystemControlSources>(own).unwrap(),
                sessions,
                &app.world().get::<ShipConfigComponent>(own).unwrap().0,
                app.world().get::<HumanSeekingHosts>(own),
            )
        {
            break;
        }
        app.update();
    }
    let sessions = app.world().resource::<Sessions>();
    assert_eq!(
        sessions.0.holder_for_station(&StationId(station.into())),
        Some(station)
    );
    assert!(
        project_phoenix::command_admission::policy::is_command_authorized(
            station,
            &sid(owner),
            &payload,
            app.world().get::<ShipSystemControlSources>(own).unwrap(),
            sessions,
            &app.world().get::<ShipConfigComponent>(own).unwrap().0,
            app.world().get::<HumanSeekingHosts>(own)
        )
    );
    send(
        app,
        station,
        ClientMessage::ControlSystem {
            target: sid(owner),
            payload,
        },
    );
    steps(app, 4);
}
fn boot(world: &str) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: world.into(),
        ship_path: HULL.into(),
        deterministic: true,
        seed: Some(156221),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.init_resource::<WorkEvidence>().add_systems(
        FixedUpdate,
        observe_work
            .in_set(SimSet::Publish)
            .after_ignore_deferred(FixedStep::PublishUmbilicalBlackboard),
    );
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    for station in ["engineering", "helm", "captain"] {
        claim(&mut app, station);
    }
    control(
        &mut app,
        "engineering",
        "umbilical",
        Control::SetTransferStore {
            store: Some("hold-alloy".into()),
        },
    );
    control(
        &mut app,
        "captain",
        "captain",
        Control::SetObjectivePriority {
            id: "supply-load".into(),
        },
    );
    for station in ["engineering", "helm"] {
        send(&mut app, station, ClientMessage::ReleaseStation);
        steps(&mut app, 4);
    }
    app
}
fn quantity(app: &App, entity: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity("alloy")
        .unwrap()
}
fn freight(app: &App, entity: Entity) -> i64 {
    let Some(profile) = app.world().get::<CompiledShipLogistics>(entity) else {
        return 0;
    };
    let Some(state) = app.world().get::<ShipLogistics>(entity) else {
        return 0;
    };
    state
        .0
        .batches
        .values()
        .filter(|batch| profile.0.store(&batch.source).unwrap().resource == "alloy")
        .map(|batch| batch.amount)
        .sum()
}
fn conserved(app: &App, own: Entity, source: Entity, receiver: Entity, initial: i64) {
    assert_eq!(
        quantity(app, own) + freight(app, own) + quantity(app, source) + quantity(app, receiver),
        initial,
        "actual source, vessel, receiver and departed internal goods remain finite"
    );
}
fn loaded(app: &mut App) -> (Entity, Entity, String, Entity, String) {
    let own = local(app);
    let (source, source_uuid) = actor(app, "supply-source");
    let (receiver, receiver_uuid) = actor(app, "supply-receiver");
    let initial = quantity(app, source) + quantity(app, own) + quantity(app, receiver);
    for _ in 0..12000 {
        conserved(app, own, source, receiver, initial);
        if quantity(app, source) == 0 && quantity(app, own) == 6 {
            break;
        }
        app.update();
    }
    assert_eq!(
        quantity(app, source),
        0,
        "real source must pay all six loaded units"
    );
    assert_eq!(quantity(app, own), 6);
    assert_eq!(
        app.world()
            .get::<DockControl>(own)
            .unwrap()
            .docked_partner(),
        Some(source_uuid.as_str())
    );
    let evidence = app.world().resource::<WorkEvidence>();
    assert!(
        evidence.providers.contains(&source_uuid) && evidence.umbilical && evidence.ordinary_start,
        "ordinary admitted AI and both actual paid service endpoints required: {evidence:?}"
    );
    (own, source, source_uuid, receiver, receiver_uuid)
}
fn depart(app: &mut App) {
    claim(app, "engineering");
    claim(app, "helm");
    control(app, "engineering", "umbilical", Control::StopTransfer);
    control(
        app,
        "engineering",
        "umbilical",
        Control::SetPortMaintenance { enabled: false },
    );
    control(
        app,
        "engineering",
        "umbilical",
        Control::SetTransferDirection {
            direction: UmbilicalDirection::Deliver,
        },
    );
    control(app, "helm", "dock", Control::Undock);
    control(
        app,
        "captain",
        "captain",
        Control::SetObjectivePriority {
            id: "supply-unload".into(),
        },
    );
    for station in ["engineering", "helm"] {
        send(app, station, ClientMessage::ReleaseStation);
        steps(app, 4);
    }
}
fn same(a: &App, b: &App, step: usize) {
    assert_eq!(
        capture(a.world()),
        capture(b.world()),
        "complete continuation {step}"
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest {step}"
    );
}
#[test]
fn actual_two_facility_load_carry_unload_and_json_continuation() {
    let mut live = boot(WORLD);
    let (own, source, _, receiver, receiver_uuid) = loaded(&mut live);
    let initial_pose = *live.world().get::<ShipPhysics>(own).unwrap();
    depart(&mut live);
    for _ in 0..12000 {
        conserved(&live, own, source, receiver, 6);
        let pose = live.world().get::<ShipPhysics>(own).unwrap();
        if live
            .world()
            .get::<DockControl>(own)
            .unwrap()
            .docked_partner()
            .is_none()
            && (pose.x - initial_pose.x).abs() > 10.0
            && live.world().resource::<WorkEvidence>().loaded_helm_work
        {
            break;
        }
        live.update();
    }
    assert!(live
        .world()
        .get::<DockControl>(own)
        .unwrap()
        .docked_partner()
        .is_none());
    assert!(
        (live.world().get::<ShipPhysics>(own).unwrap().x - initial_pose.x).abs() > 10.0,
        "actual Helm/clearance and physics must carry loaded goods through space"
    );
    assert!(
        live.world().resource::<WorkEvidence>().loaded_helm_work,
        "paid Helm must move loaded goods after the separate Dock clearance finishes"
    );
    assert_eq!(
        quantity(&live, receiver),
        0,
        "no remote receipt while cargo is travelling"
    );
    let checkpoint: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let mut resumed = boot(WORLD);
    let applied = restore(resumed.world_mut(), &checkpoint);
    assert!(applied.is_complete(), "{:?}", applied.gaps);
    let restored = local(&mut resumed);
    let (restored_source, _) = actor(&mut resumed, "supply-source");
    let (restored_receiver, _) = actor(&mut resumed, "supply-receiver");
    let mut done = false;
    let mut completed_step = 0;
    for step in 0..12000 {
        same(&live, &resumed, step);
        conserved(&live, own, source, receiver, 6);
        conserved(&resumed, restored, restored_source, restored_receiver, 6);
        live.update();
        resumed.update();
        if quantity(&live, receiver) == 6 && step >= 359 {
            completed_step = step + 1;
            done = true;
            break;
        }
    }
    assert!(
        done,
        "actual destination service must receive the physical cargo"
    );
    same(&live, &resumed, completed_step);
    conserved(&live, own, source, receiver, 6);
    conserved(&resumed, restored, restored_source, restored_receiver, 6);
    assert_eq!(quantity(&live, own), 0);
    assert_eq!(
        live.world()
            .get::<DockControl>(own)
            .unwrap()
            .docked_partner(),
        Some(receiver_uuid.as_str())
    );
    assert!(
        live.world()
            .resource::<WorkEvidence>()
            .providers
            .contains(&receiver_uuid),
        "destination's actual Dock service must pay the unloading work"
    );
    // Existing whole-roster preflight must refuse unsupported transfer intent
    // before any live stock, job, power or controller write.
    for forged_store in [false, true] {
        let before = capture(live.world());
        let digest = world_digest(live.world());
        let mut bad = before.clone();
        let own_uuid = &live.world().get::<EntityUuid>(own).unwrap().0;
        let tool = bad
            .entities
            .iter_mut()
            .find(|row| &row.uuid == own_uuid)
            .unwrap()
            .umbilical
            .as_mut()
            .unwrap();
        if forged_store {
            tool.selected_store = Some("missing-store".into());
        } else {
            tool.selected_resource = "unsupported-resource".into();
        }
        assert!(!restore(live.world_mut(), &bad).is_complete());
        assert_eq!(capture(live.world()), before);
        assert_eq!(world_digest(live.world()), digest);
    }
}
#[test]
fn actual_full_receiver_retains_the_carried_goods_and_does_not_refund_source() {
    let mut app = boot(FULL);
    let (own, source, _, receiver, receiver_uuid) = loaded(&mut app);
    depart(&mut app);
    for _ in 0..12000 {
        conserved(&app, own, source, receiver, 12);
        if app
            .world()
            .get::<DockControl>(own)
            .unwrap()
            .docked_partner()
            == Some(receiver_uuid.as_str())
        {
            break;
        }
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<DockControl>(own)
            .unwrap()
            .docked_partner(),
        Some(receiver_uuid.as_str())
    );
    for _ in 0..180 {
        app.update();
        conserved(&app, own, source, receiver, 12);
    }
    assert_eq!(quantity(&app, own), 6);
    assert_eq!(quantity(&app, receiver), 6);
    assert_eq!(quantity(&app, source), 0);
    assert!(
        !app.world()
            .resource::<WorkEvidence>()
            .providers
            .contains(&receiver_uuid),
        "a full receiver must not charge its physical Dock service for rejected goods"
    );
    let tool = app.world().get::<TransferUmbilical>(own).unwrap();
    assert_eq!(tool.selected_direction, UmbilicalDirection::Deliver);
    assert!(
        !tool.port_maintenance,
        "an idle or blocked explicit delivery cannot silently become collection"
    );
}
