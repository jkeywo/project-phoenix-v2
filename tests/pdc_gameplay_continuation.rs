//! Real-host PDC runtime probe. Reuses the production headless BootPlan and
//! ordinary crew/admission path; no injected observation, projectile or AI state.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
#[path = "pdc_gameplay_continuation/graph_diagnostics.rs"]
mod graph_diagnostics;
use bevy::prelude::*;
use project_phoenix::{
    console::weapons::{
        assignments::ShipWeaponTargets, BlasterSystemResource, TacticalRadarSelection,
        TorpedoSystemResource,
    },
    core::messages::{
        AdmittedCommands, ClientMessage, ServerMessage, StationId, SystemControlPayload as Control,
        SystemId,
    },
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    science::{broad::BroadTracks, contact_routing},
    server_app::{LocalShip, ShipSystemBlackboards, SimOutbox},
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_sets::FixedStep,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_rebuild, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
use std::collections::{BTreeMap, BTreeSet};

const HULL: &str = "assets/entities/pdc_cruiser.toml";
const SINGLE: &str = "tests/fixtures/pdc-gameplay-world.toml";
const SALVO: &str = "tests/fixtures/pdc-salvo-world.toml";
const BANK: &str = "blaster-side";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}

/// Read-only observer of the real owner interval. This diagnostic resource is
/// never consumed by simulation or serialized as gameplay state. Observe before
/// terminal guided settlement, so an expired/detonated round is not mislabelled
/// as PDC interception. The ordinary publisher/broadcaster stays untouched.
#[derive(Resource, Default)]
struct Trace {
    launcher: String,
    launched: BTreeSet<String>,
    bolts: BTreeSet<String>,
    qualified: BTreeSet<String>,
    interceptions: BTreeSet<String>,
    hits: BTreeSet<(String, String)>,
    ai_fire: bool,
    ai_automatic: bool,
    automatic_with_ship_primary: bool,
    paid: BTreeMap<u64, f64>,
    initial_rounds: i64,
    initial_energy: f32,
    before_payload_hp: Option<f64>,
    payload_damaged_own_hull: bool,
    producer_initial: Option<serde_json::Value>,
    launcher_readiness: [u64; 3],
    first_aiming_opportunity: Option<(u64, [f32; 3])>,
    first_protection_reading: Option<(u64, [f32; 3])>,
    missile_track_ids: BTreeSet<String>,
    missile_refresh_max: f64,
    missile_supported_max: f64,
    max_current_missiles: usize,
}

impl std::fmt::Debug for Trace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trace")
            .field("launcher", &self.launcher)
            .field("launched", &self.launched)
            .field("bolts", &self.bolts)
            .field("qualified", &self.qualified)
            .field("interceptions", &self.interceptions)
            .field("hits", &self.hits)
            .field("ai_fire", &self.ai_fire)
            .field("ai_automatic", &self.ai_automatic)
            .field(
                "automatic_with_ship_primary",
                &self.automatic_with_ship_primary,
            )
            .field("observed_payment_ticks", &self.paid.len())
            .field("paid_work", &self.paid.values().sum::<f64>())
            .field("initial_rounds", &self.initial_rounds)
            .field("initial_energy", &self.initial_energy)
            .field("before_payload_hp", &self.before_payload_hp)
            .field("payload_damaged_own_hull", &self.payload_damaged_own_hull)
            .field("launcher_readiness", &self.launcher_readiness)
            .field("first_aiming_opportunity", &self.first_aiming_opportunity)
            .field("first_protection_reading", &self.first_protection_reading)
            .field("missile_track_ids", &self.missile_track_ids.len())
            .field("missile_refresh_max", &self.missile_refresh_max)
            .field("missile_supported_max", &self.missile_supported_max)
            .field("max_current_missiles", &self.max_current_missiles)
            .finish()
    }
}
fn observe_launcher_readiness(
    tick: Res<SimTick>,
    aims: project_phoenix::console::weapons::assignments::WeaponAimInputs,
    ships: Query<(
        Entity,
        &EntityUuid,
        &TorpedoSystemResource,
        &project_phoenix::ship::state::ShipPhysics,
    )>,
    mut trace: ResMut<Trace>,
) {
    for (entity, uuid, torpedoes, physics) in &ships {
        if uuid.0 != trace.launcher {
            continue;
        }
        for tube in &torpedoes.0.tubes {
            if !tube.is_loaded() {
                continue;
            }
            let owner =
                project_phoenix::ship::system_registry::torpedo_tube_system_id(&tube.id).unwrap();
            let Some(aim) = aims
                .resolve(entity, &owner, |_| None)
                .filter(|aim| aim.allows(tube.targets.as_deref()))
            else {
                continue;
            };
            let geometry = aims.geometry(
                entity,
                &owner,
                &project_phoenix::weapons::mount::MountConfig {
                    facing_deg: tube.facing_deg,
                    fire_arc_deg: tube.fire_arc_deg,
                    range: torpedoes.0.config.speed * torpedoes.0.config.lifespan,
                },
                &aim,
                physics,
            );
            let cover = aims.observed_shield_cover(entity, &owner, &aim, [physics.x, physics.z]);
            if cover.is_some() {
                trace
                    .first_protection_reading
                    .get_or_insert((tick.0, [physics.x, physics.z, physics.yaw]));
            }
            if geometry.in_arc && geometry.in_range {
                trace
                    .first_aiming_opportunity
                    .get_or_insert((tick.0, [physics.x, physics.z, physics.yaw]));
                trace.launcher_readiness[0] += 1;
                if let Some(cover) = cover {
                    trace.launcher_readiness[1] += 1;
                    if cover == 0 {
                        trace.launcher_readiness[2] += 1;
                    }
                }
            }
        }
    }
}
#[allow(clippy::type_complexity)]
fn observe_pdc(
    tick: Res<SimTick>,
    outbox: Res<SimOutbox>,
    mut trace: ResMut<Trace>,
    own: Query<
        (
            &EntityUuid,
            &ShipConfigComponent,
            &ShipSystemBlackboards,
            &BroadTracks,
            &ShipWeaponTargets,
            &ShipPowerSystem,
            &AdmittedCommands,
            &TacticalRadarSelection,
            &ShipDamage,
        ),
        With<LocalShip>,
    >,
) {
    let Ok((uuid, config, boards, tracks, assignments, power, admitted, primary, damage)) =
        own.single()
    else {
        return;
    };
    trace.before_payload_hp = Some(damage.structural_totals().0);
    let owner = id(BANK);
    if let Some(radar) = tracks.0.get(&id("tactical-radar")) {
        let current: Vec<_> = radar
            .current
            .iter()
            .filter(|(source, _)| trace.launched.contains(*source))
            .filter_map(|(_, contact)| radar.tracks.get(contact))
            .collect();
        trace.max_current_missiles = trace.max_current_missiles.max(current.len());
        for track in current {
            trace.missile_track_ids.insert(track.contact_id.clone());
            trace.missile_refresh_max = trace.missile_refresh_max.max(track.refresh_secs);
            trace.missile_supported_max = trace.missile_supported_max.max(track.supported_secs);
        }
    }
    let view = config.0.observations_for_system(&owner, &boards.0);
    for record in boards
        .0
        .keys()
        .filter_map(|key| view.get(key))
        .filter_map(contact_routing::observations)
        .flatten()
    {
        if record.selectable
            && !record.predicted
            && record.classification.as_deref() == Some("missile")
            && record.elevation.is_some()
            && record.velocity.is_some()
        {
            if let Some(route) = contact_routing::resolve(
                &config.0,
                boards,
                Some(tracks),
                &owner,
                &record.contact_id,
            ) {
                assert_ne!(
                    record.contact_id, route,
                    "paid knowledge stays a public handle"
                );
                trace.qualified.insert(route);
            }
        }
    }
    trace.ai_fire |= admitted
        .for_target(BANK)
        .any(|command| matches!(command.payload, Control::ChargeBlasterStart));
    trace.ai_automatic |= admitted.for_target(BANK).any(|command| {
        matches!(
            &command.payload,
            Control::SetAutomaticWeaponTarget { uuid: Some(_) }
        )
    });
    let automatic_with_ship_primary = primary.0.as_deref() == Some(trace.launcher.as_str())
        && assignments
            .0
             .0
            .get(&owner)
            .is_some_and(|row| row.automatic.is_some() && !row.has_explicit_order());
    trace.automatic_with_ship_primary |= automatic_with_ship_primary;
    trace.paid.insert(tick.0, power.0.emitted_work(&owner));
    for (_, message) in outbox.iter() {
        match message {
            ServerMessage::TorpedoLaunched { uuid, .. } => {
                trace.launched.insert(uuid.clone());
            }
            ServerMessage::BlasterFired {
                bank,
                source_uuid,
                projectile_id,
                ..
            } if bank == "side" && source_uuid == &uuid.0 => {
                trace.bolts.insert(projectile_id.clone());
            }
            ServerMessage::BlasterHit {
                projectile_id,
                target_uuid,
                ..
            } if trace.bolts.contains(projectile_id) => {
                trace
                    .hits
                    .insert((projectile_id.clone(), target_uuid.clone()));
            }
            ServerMessage::TorpedoDestroyed { uuid }
                if trace.hits.iter().any(|(_, target)| target == uuid) =>
            {
                trace.interceptions.insert(uuid.clone());
            }
            _ => {}
        }
    }
}
fn observe_terminal_damage(own: Query<&ShipDamage, With<LocalShip>>, mut trace: ResMut<Trace>) {
    if let (Ok(damage), Some(before)) = (own.single(), trace.before_payload_hp) {
        trace.payload_damaged_own_hull |= damage.structural_totals().0 < before;
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
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn claim(app: &mut App, station: &str) {
    let entity = local(app);
    let rating = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|row| row.id.0 == station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    for message in [
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
        send(app, station, message);
        steps(app, 4);
    }
}
fn authorised(
    app: &App,
    entity: Entity,
    station: &str,
    owner: &SystemId,
    payload: &Control,
) -> bool {
    let sessions = app.world().resource::<Sessions>();
    sessions.0.holder_for_station(&StationId(station.into())) == Some(station)
        && sessions
            .0
            .players()
            .iter()
            .any(|player| player.token == station && player.connected && player.ready)
        && project_phoenix::command_admission::policy::is_command_authorized(
            station,
            owner,
            payload,
            app.world().get::<ShipSystemControlSources>(entity).unwrap(),
            sessions,
            &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
            app.world().get::<HumanSeekingHosts>(entity),
        )
}
fn control(app: &mut App, station: &str, owner: &str, payload: Control) {
    let entity = local(app);
    for _ in 0..120 {
        if authorised(app, entity, station, &id(owner), &payload) {
            break;
        }
        app.update();
    }
    assert!(
        authorised(app, entity, station, &id(owner), &payload),
        "normal {station} claim must authorize {owner}"
    );
    send(
        app,
        station,
        ClientMessage::ControlSystem {
            target: id(owner),
            payload,
        },
    );
    steps(app, 4);
}
fn boot(world: &str) -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: world.into(),
        ship_path: HULL.into(),
        deterministic: true,
        seed: Some(156_215),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.init_resource::<Trace>().add_systems(
        FixedUpdate,
        // Read actual resource writes without requesting a new Commands barrier.
        observe_pdc
            .after_ignore_deferred(FixedStep::HandleBlasterHits)
            .before_ignore_deferred(FixedStep::TickTorpedoLifecycle),
    );
    app.add_systems(
        FixedUpdate,
        observe_terminal_damage
            .after_ignore_deferred(FixedStep::TickTorpedoLifecycle)
            .before_ignore_deferred(FixedStep::HandleCollisions),
    );
    graph_diagnostics::install(&mut app);
    app.add_systems(
        FixedUpdate,
        observe_launcher_readiness
            .after_ignore_deferred(FixedStep::AiTorpedoAutoFire)
            .before_ignore_deferred(FixedStep::HandleFireTorpedo),
    );
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    let own = local(&mut app);
    // All changes below are the same public commands a joined crew sends.
    for station in ["helm", "science", "engineering", "tactical", "captain"] {
        claim(&mut app, station);
    }
    control(
        &mut app,
        "helm",
        "helm-thrust",
        Control::SetThrust { value: 0.0 },
    );
    control(
        &mut app,
        "helm",
        "helm-steering",
        Control::SetSteering { value: 0.0 },
    );
    for arc in ["fore", "port", "aft", "starboard"] {
        control(
            &mut app,
            "science",
            &format!("shield-arc-{arc}"),
            Control::SetShieldRaised { raised: false },
        );
    }
    for bank in [
        "phaser-fore",
        "phaser-aft",
        "torpedo-tube-fore-port",
        "torpedo-tube-fore-starboard",
        "torpedo-tube-aft",
    ] {
        control(
            &mut app,
            "tactical",
            bank,
            Control::SetWeaponTarget { uuid: None },
        );
    }
    // Isolate the real finite PDC reserve debit from refill, using its real
    // allocation command. No capacity, generation or reserve is increased.
    control(
        &mut app,
        "engineering",
        "power-reactor",
        Control::SetEnergyMode {
            mode: phoenix_model::energy::EnergyAllocationMode::Manual,
            cooling_usage: phoenix_model::energy::CoolingUsage::Low,
        },
    );
    control(
        &mut app,
        "engineering",
        "power-reactor",
        Control::SetEnergySystemFlow {
            system: id(BANK),
            flow_per_sec: Some(0.0),
        },
    );
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(own)
            .unwrap()
            .0
            .network()
            .fine_flows
            .get(&id(BANK)),
        Some(&0.0),
        "normal actual reactor consumer must apply the zero-intake request"
    );
    control(
        &mut app,
        "captain",
        "red-alert",
        Control::SetRedAlert { active: true },
    );
    let initial_rounds = rounds(&app, own);
    let initial_energy = reserve(&app, own);
    let mut trace = app.world_mut().resource_mut::<Trace>();
    assert_eq!(initial_rounds, 22, "actual finite trial battery stock");
    trace.initial_rounds = initial_rounds;
    trace.initial_energy = initial_energy;
    trace.paid.clear();
    assert!(
        trace.bolts.is_empty(),
        "bank is still manually held during setup"
    );
    (app, own)
}
fn ship_contact(app: &App, own: Entity) -> Option<(String, String)> {
    let config = &app.world().get::<ShipConfigComponent>(own)?.0;
    let boards = app.world().get::<ShipSystemBlackboards>(own)?;
    let tracks = app.world().get::<BroadTracks>(own)?;
    let view = config.observations_for_system(&id("tactical-radar"), &boards.0);
    // Select from the addressed instrument's actual publication, so the
    // explicit human input never depends on HashMap order among other feeds.
    contact_routing::observations(view.get(&id("tactical-radar"))?)?
        .iter()
        .filter(|row| row.selectable && !row.predicted)
        .find_map(|row| {
            (row.identity.as_ref()?.name == "PDC probe launcher").then_some(())?;
            let route = contact_routing::resolve(
                config,
                boards,
                Some(tracks),
                &id("tactical-radar"),
                &row.contact_id,
            )?;
            Some((row.contact_id.clone(), route))
        })
}
fn prepare(world: &str) -> (App, Entity, String) {
    let (mut app, own) = boot(world);
    for _ in 0..1200 {
        if ship_contact(&app, own).is_some() {
            break;
        }
        app.update();
    }
    let (contact, route) = ship_contact(&app, own).expect("actual paid identified hostile ship");
    let producer_initial = launcher_diagnostics(&mut app, &route);
    app.world_mut().resource_mut::<Trace>().producer_initial = Some(producer_initial);
    assert_ne!(contact, route);
    app.world_mut().resource_mut::<Trace>().launcher = route.clone();
    control(
        &mut app,
        "tactical",
        "tactical-radar",
        Control::SetTarget { uuid: contact },
    );
    send(
        &mut app,
        "tactical",
        ClientMessage::SetStationRating {
            rating_name: "Simplified".into(),
        },
    );
    steps(&mut app, 8);
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(own)
            .unwrap()
            .0
            .source_for(&id(BANK)),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
    assert!(app
        .world()
        .get::<ShipWeaponTargets>(own)
        .unwrap()
        .0
         .0
        .get(&id(BANK))
        .is_none_or(|row| !row.has_explicit_order()));
    (app, own, route)
}
fn rounds(app: &App, own: Entity) -> i64 {
    let stock = &app.world().get::<InfrastructureCondition>(own).unwrap().0;
    let profile = &app
        .world()
        .get::<project_phoenix::resources::logistics_server::CompiledShipLogistics>(own)
        .unwrap()
        .0;
    let moving = &app
        .world()
        .get::<project_phoenix::resources::logistics_server::ShipLogistics>(own)
        .unwrap()
        .0;
    // Dispatch removes real goods from a store before arrival. Count the actual
    // freight owner too; reservations and receiving-space claims are not goods.
    let freight: i64 = moving
        .batches
        .values()
        .filter(|batch| profile.store(&batch.source).unwrap().resource == "kinetic_rounds")
        .map(|batch| batch.amount)
        .sum();
    stock.capacity("side-kinetic-rounds").unwrap()
        + stock.capacity("hold-kinetic-rounds").unwrap()
        + freight
}
fn reserve(app: &App, own: Entity) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(own)
        .unwrap()
        .0
        .network()
        .consumers[&id(BANK)]
        .energy
}
fn flights(app: &mut App) -> BTreeMap<String, [f32; 3]> {
    app.world_mut()
        .query::<&TorpedoSystemResource>()
        .iter(app.world())
        .flat_map(|provider| {
            provider
                .0
                .in_flight
                .iter()
                .map(|round| (round.uuid.clone(), [round.x, round.y, round.z]))
        })
        .collect()
}
fn launcher_entity(app: &mut App, uuid: &str) -> Entity {
    app.world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == uuid)
        .map(|(entity, _)| entity)
        .unwrap()
}

/// Failure-only inspection of the actual producer. This grants no game input
/// or observation and is never consumed by a controller.
fn launcher_diagnostics(app: &mut App, uuid: &str) -> serde_json::Value {
    let entity = launcher_entity(app, uuid);
    let mut aim_state = bevy::ecs::system::SystemState::<
        project_phoenix::console::weapons::assignments::WeaponAimInputs,
    >::new(app.world_mut());
    let aim_details = {
        let aims = aim_state.get(app.world());
        let world = app.world();
        let physics = world
            .get::<project_phoenix::ship::state::ShipPhysics>(entity)
            .unwrap();
        world.get::<TorpedoSystemResource>(entity).unwrap().0.tubes.iter().map(|tube| {
            let owner = project_phoenix::ship::system_registry::torpedo_tube_system_id(&tube.id).unwrap();
            let aim = aims.resolve(entity, &owner, |_| None);
            let detail = aim.map(|aim| {
                let geometry = aims.geometry(entity, &owner, &project_phoenix::weapons::mount::MountConfig {
                    facing_deg: tube.facing_deg, fire_arc_deg: tube.fire_arc_deg,
                    range: world.get::<TorpedoSystemResource>(entity).unwrap().0.config.speed * world.get::<TorpedoSystemResource>(entity).unwrap().0.config.lifespan,
                }, &aim, physics);
                serde_json::json!({"route":aim.route,"position":aim.position,"allowed":aim.allows(tube.targets.as_deref()),"arc":geometry.in_arc,"range":geometry.in_range,"cover":aims.observed_shield_cover(entity,&owner,&aim,[physics.x,physics.z])})
            });
            serde_json::json!({"id":tube.id,"aim":detail})
        }).collect::<Vec<_>>()
    };
    let world = app.world();
    let pose = world
        .get::<project_phoenix::ship::state::ShipPhysics>(entity)
        .unwrap();
    let boards: BTreeMap<_, _> = world
        .get::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .iter()
        .filter(|(owner, _)| {
            matches!(
                owner.0.as_str(),
                "viewscreen" | "tactical-radar" | "sensor-radar" | "captain"
            )
        })
        .map(|(owner, board)| (owner.0.clone(), board))
        .collect();
    let sources: BTreeMap<_, _> = world
        .get::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .entries()
        .map(|(owner, source)| (owner.0.clone(), format!("{source:?}")))
        .collect();
    let tubes: Vec<_> = world.get::<TorpedoSystemResource>(entity).unwrap().0.tubes.iter()
        .map(|tube| serde_json::json!({"id":tube.id,"loaded":tube.loaded_count,"desired":tube.target_count,"loading":tube.load_progress(),"targets":tube.targets})).collect();
    serde_json::json!({
        "pose":[pose.x,pose.y,pose.z,pose.yaw],
        "high_fidelity":world.get::<project_phoenix::ai::server::AiHighFidelity>(entity).is_some(),
        "red_alert":world.get::<project_phoenix::ship::state::ShipRedAlert>(entity).map(|alert| alert.0),
        "ratings":world.get::<project_phoenix::ship_plugin::ActiveStationRatings>(entity).map(|ratings| &ratings.0),
        "energy":world.get::<ShipPowerSystem>(entity).map(|power| power.0.network()),
        "captain_policy":world.get::<project_phoenix::console::captain::server::CaptainAiPolicy>(entity).map(|policy| format!("{:?}", policy.0)),
        "world_entities":world.get_resource::<project_phoenix::ai::server::WorldSnapshot>().map(|snapshot| snapshot.entities.iter().map(|row| serde_json::json!({"uuid":row.uuid,"faction":row.faction,"position":row.position})).collect::<Vec<_>>()),
        "selection":world.get::<TacticalRadarSelection>(entity).map(|selection| &selection.0),
        "sources":sources,"tubes":tubes,"boards":boards,"aims":aim_details
    })
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
            keys.into_iter().find_map(|key| {
                first_difference(
                    a.get(key).unwrap_or(&serde_json::Value::Null),
                    b.get(key).unwrap_or(&serde_json::Value::Null),
                    &format!("{path}.{key}"),
                )
            })
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(index, (a, b))| first_difference(a, b, &format!("{path}[{index}]"))),
        _ => (a != b).then(|| format!("{path}: {a} != {b}")),
    }
}

#[test]
fn ordinary_pdc_intercepts_and_survives_launcher_removal_with_exact_json_continuation() {
    let (mut live, own, launcher) = prepare(SINGLE);
    let before_rounds = live.world().resource::<Trace>().initial_rounds;
    let before_energy = live.world().resource::<Trace>().initial_energy;
    for _ in 0..2400 {
        if live.world().resource::<Trace>().automatic_with_ship_primary
            && !live.world().resource::<Trace>().bolts.is_empty()
            && !flights(&mut live).is_empty()
        {
            break;
        }
        live.update();
    }
    let producer = launcher_diagnostics(&mut live, &launcher);
    let producer_initial = live
        .world()
        .resource::<Trace>()
        .producer_initial
        .as_ref()
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    assert!(live.world().resource::<Trace>().automatic_with_ship_primary,
        "normal bank AI must actually use a measured missile while retaining the ship primary: {:?}; producer: {producer}; producer_initial: {producer_initial}",
        live.world().resource::<Trace>());
    let departing = flights(&mut live);
    assert!(
        !departing.is_empty(),
        "ordinary identified foe must launch actual finite ordnance"
    );
    let source = launcher_entity(&mut live, &launcher);
    let before_source_ammo = live
        .world()
        .get::<TorpedoSystemResource>(source)
        .unwrap()
        .0
        .rounds_aboard();
    let source_hold = live
        .world()
        .get::<InfrastructureCondition>(source)
        .unwrap()
        .0
        .capacity("hold-ammunition")
        .unwrap();
    let emitted_source = live.world().resource::<Trace>().launched.len() as i64;
    assert_eq!(
        i64::from(before_source_ammo) + source_hold + emitted_source,
        6,
        "actual source magazine/tubes/hold and emitted rounds conserve authored six"
    );
    // This is explicitly a removal-owner/hook witness, not GM admission.
    let _ = project_phoenix::gm_despawn::remove_entity(live.world_mut(), source);
    live.update();
    let carried = flights(&mut live);
    assert!(
        departing.keys().any(|id| carried.contains_key(id)),
        "actual flight must outlive launcher"
    );
    assert!(
        emitted_source > 0,
        "actual source spent finite physical ammunition"
    );
    assert!(
        departing
            .iter()
            .any(|(id, point)| carried.get(id).is_some_and(|next| next != point)),
        "same existing vector must continue moving after removal"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored, _) = prepare(SINGLE);
    assert!(ready_to_rebuild(resumed.world(), &saved));
    let applied = restore(resumed.world_mut(), &saved);
    assert!(applied.is_complete(), "{:?}", applied.gaps);
    // Trace is intentionally outside gameplay state. Its original launch and
    // knowledge witnesses stay on the live host; both hosts retain real owners.
    for step in 0..360 {
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "digest {step}"
        );
        let a = serde_json::to_value(capture(live.world())).unwrap();
        let b = serde_json::to_value(capture(resumed.world())).unwrap();
        assert_eq!(
            a,
            b,
            "complete continuation {step}: {:?}",
            first_difference(&a, &b, "snapshot")
        );
        live.update();
        resumed.update();
    }
    let trace = live.world().resource::<Trace>();
    assert!(
        !trace.interceptions.is_empty(),
        "actual paid PDC interception required: {trace:?}"
    );
    assert!(trace.interceptions.is_subset(&trace.qualified),
        "every intercepted actor must have a current paid classified/elevated moving handle: {trace:?}");
    assert!(
        trace.ai_fire && trace.ai_automatic,
        "ordinary admitted bank controller must act: {trace:?}"
    );
    // This witness is collected before removing the primary ship; losing its
    // source later must not cancel already-emitted physical ordnance.
    assert!(
        trace.automatic_with_ship_primary,
        "ship primary must not suppress defensive fallback: {trace:?}"
    );
    let emitted = trace.bolts.len() as i64;
    assert!(emitted > 0 && emitted <= before_rounds);
    assert_eq!(rounds(&live, own), before_rounds - emitted);
    let energy_paid: f64 = trace.paid.values().sum();
    assert!(energy_paid > 0.0);
    assert!(
        (f64::from(before_energy - reserve(&live, own)) - energy_paid).abs() < 1e-5,
        "no refill or unpaid shots: actual reserve and work receipts must agree"
    );
    assert_eq!(rounds(&live, own), rounds(&resumed, restored));
    assert_eq!(reserve(&live, own), reserve(&resumed, restored));
    assert!(
        flights(&mut live).is_empty(),
        "intercepted single salvo cannot leave ghost flights"
    );
}

#[test]
fn actual_larger_salvo_exceeds_one_paid_two_barrel_cycle() {
    let (mut app, own, _) = prepare(SALVO);
    let before = app.world().resource::<Trace>().initial_rounds;
    for _ in 0..2400 {
        if !app.world().resource::<Trace>().interceptions.is_empty()
            && app.world().resource::<Trace>().launched.len() >= 2
            && flights(&mut app).is_empty()
        {
            break;
        }
        app.update();
    }
    let trace = app.world().resource::<Trace>();
    assert!(
        trace.launched.len() >= 2,
        "ordinary two-tube launch: {trace:?}"
    );
    assert!(
        !trace.interceptions.is_empty(),
        "useful actual PDC baseline: {trace:?}"
    );
    assert!(
        trace.interceptions.len() < trace.launched.len(),
        "larger finite salvo must not be guaranteed defence: {trace:?}"
    );
    assert!(trace.ai_fire && trace.automatic_with_ship_primary);
    assert_eq!(rounds(&app, own), before - trace.bolts.len() as i64);
    assert!(
        trace.payload_damaged_own_hull,
        "saturation must reach actual terminal payload damage, not mere expiry: {trace:?}"
    );
    assert!(app
        .world()
        .get::<BlasterSystemResource>(own)
        .unwrap()
        .0
        .iter()
        .any(|bank| bank.config.id == "side" && bank.config.cooldown_secs > 0.0));
}
