//! Opt-in runtime prerequisite for #1320, not its human acceptance gate.
//! Generate the assets from integrated #1316/#1308/#1317 authoring first:
//! `node scripts/prepare-gm-live-event.mjs`
//! Then run this named test with `--features headless -- --ignored --exact`.
//! Normal CI skips it because generated event assets are deliberately not in Git.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;

use bevy::prelude::*;
use project_phoenix::command_admission::HostSlot;
use project_phoenix::entities::spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::lockstep::{
    authored_delay, join_fleet, FleetRoster, FleetShip, FleetSlotOf, MeshFrame, MeshInbox,
    MeshOutbox,
};
use project_phoenix::server_app::{LocalShip, Ship};
use project_phoenix::sim_sets::SimSet;
use project_phoenix::sim_tick::SimTick;
use project_phoenix::world::config::{parse_world, WorldConfig, WorldEntitySpawnOn};
use project_phoenix::world::server::WorldScriptRuntime;

const WORLD: &str = "assets/worlds/prepared/gm_live_event_two_ship.toml";
const SHIP: &str = "assets/entities/alliance_cruiser.toml";
const SLOTS: [HostSlot; 2] = [HostSlot(1), HostSlot(2)];
const MESH_ROUNDS: usize = 90;

/// Read the actual spawned Transform before ordinary Input/Physics can move it.
/// This observation resource never changes or substitutes for simulation state.
#[derive(Resource, Default)]
struct SpawnObservations(BTreeMap<HostSlot, (String, Vec3)>);

fn observe_spawns(
    ships: Query<(&FleetSlotOf, &EntityUuid, &Transform), With<Ship>>,
    mut observed: ResMut<SpawnObservations>,
) {
    for (slot, uuid, transform) in &ships {
        observed
            .0
            .entry(slot.0)
            .or_insert_with(|| (uuid.0.clone(), transform.translation));
    }
}

fn host(local: HostSlot) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: SHIP.into(),
        seed: Some(1320),
        deterministic: true,
        max_ticks: 90,
        ..Default::default()
    })
    .expect("the integrated prepared event must boot through ordinary world ingestion");
    let roster = FleetRoster::new(
        SLOTS
            .into_iter()
            .map(|slot| FleetShip {
                host: slot,
                ship_path: Some(SHIP.into()),
                crew: Vec::new(),
            })
            .collect(),
        local,
    );
    let delay = authored_delay(app.world());
    assert!(delay > 0);
    assert!(join_fleet(app.world_mut(), roster, delay));
    app.init_resource::<SpawnObservations>()
        .add_systems(FixedUpdate, observe_spawns.before(SimSet::Input));
    app
}

fn ferry(hosts: &mut [App]) {
    let outgoing: Vec<Vec<MeshFrame>> = hosts
        .iter_mut()
        .map(|app| app.world_mut().resource_mut::<MeshOutbox>().drain())
        .collect();
    for (receiver, app) in hosts.iter_mut().enumerate() {
        for (sender, frames) in outgoing.iter().enumerate() {
            if receiver != sender {
                let mut inbox = app.world_mut().resource_mut::<MeshInbox>();
                for frame in frames {
                    inbox.push(frame.clone());
                }
            }
        }
    }
}

fn live_fleet(app: &mut App) -> BTreeMap<HostSlot, String> {
    let mut ships = app
        .world_mut()
        .query_filtered::<(&FleetSlotOf, &EntityUuid), With<Ship>>();
    let rows: Vec<_> = ships
        .iter(app.world())
        .map(|(slot, uuid)| (slot.0, uuid.0.clone()))
        .collect();
    assert_eq!(rows.len(), 2, "exactly two live Fleet hulls must exist");
    rows.into_iter().collect()
}

#[test]
#[ignore = "requires integrated GM authoring and generated #1320 assets; run explicitly before the human event"]
fn prepared_event_has_two_authored_fleet_hulls_and_equal_peer_digests() {
    let source = std::fs::read_to_string(WORLD)
        .expect("run node scripts/prepare-gm-live-event.mjs on the integrated checkout first");
    let config = parse_world(&source).expect("the generated full event must parse");
    // These real parsed fields deliberately require the integrated NPC/Comms
    // implementation. A base build silently ignoring those tables is no proof.
    assert!(config
        .gm_npc_doctrine_palette
        .iter()
        .any(|entry| entry.id == "raider-regroup"));
    let hail = config
        .gm_comms_routes
        .iter()
        .find(|route| route.id == "starbase-selected")
        .and_then(|route| {
            route
                .hails
                .iter()
                .find(|hail| hail.id == "defence-briefing")
        })
        .expect("the integrated selected-ship hail must be retained");
    assert_eq!(hail.script_path, format!("{WORLD}#script.setup"));

    let authored: Vec<_> = config
        .entities
        .iter()
        .filter(|entity| entity.spawn_on == WorldEntitySpawnOn::GameStart)
        .collect();
    assert_eq!(authored.len(), 2);
    let first = config
        .player_spawn
        .as_ref()
        .and_then(|spawn| spawn.position)
        .expect("Combat Test authors its first player position");
    let second = authored[1]
        .transform
        .as_ref()
        .and_then(|transform| transform.position)
        .expect("the event fixture authors its second player position");
    let expected_positions = [Vec3::from_array(first), Vec3::from_array(second)];
    assert_ne!(expected_positions[0], expected_positions[1]);

    let mut hosts = [host(SLOTS[0]), host(SLOTS[1])];
    let initial_tick = hosts[0].world().resource::<SimTick>().0;
    for _ in 0..MESH_ROUNDS {
        ferry(&mut hosts);
        for app in &mut hosts {
            run(app, 1);
        }
        let tick = hosts[0].world().resource::<SimTick>().0;
        assert_eq!(tick, hosts[1].world().resource::<SimTick>().0);
        assert_eq!(
            world_digest(hosts[0].world()),
            world_digest(hosts[1].world()),
            "ordinary event peers disagree at tick {tick}"
        );
    }
    assert!(hosts[0].world().resource::<SimTick>().0 > initial_tick + 30);

    let expected_identities = live_fleet(&mut hosts[0]);
    assert_eq!(expected_identities.len(), 2);
    assert_ne!(
        expected_identities[&SLOTS[0]],
        expected_identities[&SLOTS[1]]
    );
    let mut peer_evidence = Vec::new();
    for (index, app) in hosts.iter_mut().enumerate() {
        assert_eq!(live_fleet(app), expected_identities);
        let mut local = app
            .world_mut()
            .query_filtered::<(&FleetSlotOf, &EntityUuid), (With<Ship>, With<LocalShip>)>();
        let projected: Vec<_> = local
            .iter(app.world())
            .map(|(slot, uuid)| (slot.0, uuid.0.clone()))
            .collect();
        assert_eq!(
            projected,
            vec![(SLOTS[index], expected_identities[&SLOTS[index]].clone())]
        );
        let observed = &app.world().resource::<SpawnObservations>().0;
        assert_eq!(observed.len(), 2);
        for (slot, position) in SLOTS.into_iter().zip(expected_positions) {
            assert_eq!(
                observed[&slot],
                (expected_identities[&slot].clone(), position)
            );
        }
        let loaded = app.world().resource::<WorldConfig>();
        assert_eq!(loaded.gm_comms_routes, config.gm_comms_routes);
        let hail_ast_loaded = app
            .world()
            .resource::<WorldScriptRuntime>()
            .asts
            .contains_key(&hail.script_path);
        assert!(hail_ast_loaded);
        let spawns: Vec<_> = observed.iter().map(|(slot, (uuid, position))| {
            serde_json::json!({ "slot": slot.0, "uuid": uuid, "position": position.to_array() })
        }).collect();
        peer_evidence.push(serde_json::json!({
            "local_slot": projected[0].0.0,
            "local_ship": projected[0].1,
            "spawns": spawns,
            "tick": app.world().resource::<SimTick>().0,
            "digest": format!("{:016x}", world_digest(app.world())),
            "hail_ast_loaded": hail_ast_loaded,
        }));
    }
    // Test output only, after every authoritative/topology assertion passed.
    // --nocapture makes the eventual native receipt retain actual observations.
    println!(
        "GM_LIVE_EVENT_PRECHECK {}",
        serde_json::json!({
            "schema": 1, "world": WORLD, "seed": 1320,
            "initial_tick": initial_tick, "compared_rounds": MESH_ROUNDS,
            "expected_positions": expected_positions.map(|position| position.to_array()),
            "peers": peer_evidence,
        })
    );
}
