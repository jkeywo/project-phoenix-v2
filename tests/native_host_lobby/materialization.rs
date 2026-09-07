//! Exercise the shared pass through both actual native entry points (#1415).
use super::*;
use project_phoenix::lobby::{stations_config::ShipStations, SelectedShipResource};
use project_phoenix::native_host::world_load::NativeWorldLoadSet;
use project_phoenix::sim_tick::SimTick;
use project_phoenix::world::server::{WorldContentRuntime, WorldLayerMap};
use project_phoenix::world_id::{IdNamespace, WorldId, WorldIdMint};

const ROOT: &str = "tests/fixtures/world_materialization_root.toml";
const LAYERS: [&str; 2] = [
    "tests/fixtures/world_materialization_z.toml",
    "tests/fixtures/world_materialization_a.toml",
];
const HULL: &str = "assets/entities/alliance_destroyer.toml";
const PICK: &str = "materialization";

fn deferred_config() -> NativeHostConfig {
    let mut cfg = NativeHostConfig::lobby(catalog_plus(PICK, ROOT, &[HULL.into()]));
    cfg.seed = Some(SEED);
    cfg.deterministic = true;
    cfg.surface = NativeRenderSurface::Contract;
    cfg
}

fn assert_same_materialization(boot: &mut App, deferred: &mut App) {
    assert_eq!(
        boot.world().resource::<State<GamePhase>>().get(),
        deferred.world().resource::<State<GamePhase>>().get()
    );
    assert_eq!(
        boot.world().resource::<SimTick>().0,
        deferred.world().resource::<SimTick>().0
    );
    assert_eq!(entity_identities(boot), entity_identities(deferred));
    assert_eq!(world_name_to_uuid(boot), world_name_to_uuid(deferred));
    assert_eq!(boot.world().resource::<SelectedShipResource>().0, HULL);
    assert_eq!(deferred.world().resource::<SelectedShipResource>().0, HULL);
    let roster = boot.world().resource::<ShipStations>();
    assert!(!roster.stations.is_empty());
    assert_eq!(roster, deferred.world().resource::<ShipStations>());
    let expected = project_phoenix::entities::include_resolve::load_entity_config(HULL)
        .unwrap()
        .ship_config
        .unwrap();
    assert_eq!(
        roster,
        &project_phoenix::lobby::stations_config::stations_from_ship_config(&expected)
    );

    let a = boot.world().resource::<WorldContentRuntime>();
    let b = deferred.world().resource::<WorldContentRuntime>();
    assert_eq!(a.name_to_uuid, b.name_to_uuid);
    assert_eq!(a.flags, b.flags);
    assert_eq!(a.triggers.capture(), b.triggers.capture());
    let handlers = |runtime: &WorldContentRuntime| {
        runtime
            .triggers
            .iter()
            .enumerate()
            .map(|(i, state)| {
                (
                    state.origin_layer.clone(),
                    runtime.triggers.handler(i).cloned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(handlers(a), handlers(b));
    let layers = |app: &App| {
        let mut rows = app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .iter()
            .map(|(path, layer)| {
                (
                    layer.activation_order,
                    path.clone(),
                    layer.is_active,
                    layer.loader_path.clone(),
                    layer.flags.clone(),
                    layer.script_units.clone(),
                )
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        rows
    };
    assert_eq!(layers(boot), layers(deferred));
    assert_eq!(
        project_phoenix::snapshot::capture(boot.world()).boot_identity,
        project_phoenix::snapshot::capture(deferred.world()).boot_identity
    );
}

#[test]
fn composed_world_materialization_matches_after_different_lobby_delays() {
    let preload = preload();
    let mut first_lobby_ids = None;
    for delay in [4, 75, 360] {
        let mut cfg = NativeHostConfig::new(ROOT);
        cfg.solo = false;
        cfg.seed = Some(SEED);
        cfg.ship_path = Some(HULL.into());
        cfg.deterministic = true;
        cfg.surface = NativeRenderSurface::Contract;
        let mut boot = build_native_host_app(&cfg, &preload).expect("boot fixture loads");
        let mut deferred = build_native_host_app(&deferred_config(), &preload)
            .expect("world-less fixture host assembles");
        pump(&mut boot, delay + 4);
        pump(&mut deferred, delay);
        select(&mut deferred, "phone-1", PICK, HULL);
        pump(&mut deferred, 4);
        assert_eq!(
            boot.world().resource::<State<GamePhase>>().get(),
            &GamePhase::Lobby
        );
        assert_same_materialization(&mut boot, &mut deferred);
        let ids = entity_identities(&mut deferred);
        assert_eq!(
            ids.len(),
            2,
            "anonymous star and named station materialize in Lobby"
        );
        if let Some(first) = &first_lobby_ids {
            assert_eq!(&ids, first);
        } else {
            first_lobby_ids = Some(ids);
        }
        assert!(
            deferred.world().resource::<WorldLayerMap>().0.is_empty(),
            "supporting layers remain queued until the simulation starts"
        );
        let boot_identity = project_phoenix::snapshot::capture(deferred.world())
            .boot_identity
            .unwrap();
        assert!(
            boot_identity.game_start_entity_uuids.is_empty(),
            "RuntimeWorldLoad must not manufacture R1's durable GameStart roster"
        );
        assert!(deferred
            .world()
            .resource::<WorldContentRuntime>()
            .triggers
            .iter()
            .all(|s| !s.fired));

        // The contract surface has no visual-preload handshake. Enter the same
        // real GameStart schedule at the SAME logical tick in each pair. Layer
        // and GameStart IDs retain that live tick; only initial materialization
        // is tick-zero, so comparing them across different start ticks is wrong.
        for app in [&mut boot, &mut deferred] {
            app.world_mut()
                .resource_mut::<NextState<GamePhase>>()
                .set(GamePhase::InProgress);
            pump(app, 8);
        }
        assert_same_materialization(&mut boot, &mut deferred);
        let runtime = deferred.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.flags.counter("root_arrived"), 1);
        let states = runtime.triggers.capture();
        assert_eq!(states.len(), 8);
        assert_eq!(
            states.iter().filter(|state| state.fired).count(),
            5,
            "root arrival plus each layer's arrival and same-tick flag chain fired"
        );
        let map = &deferred.world().resource::<WorldLayerMap>().0;
        let mut order = map
            .iter()
            .map(|(path, layer)| (layer.activation_order, path.as_str()))
            .collect::<Vec<_>>();
        order.sort_unstable();
        assert_eq!(
            order.iter().map(|(_, path)| *path).collect::<Vec<_>>(),
            LAYERS
        );
        for path in LAYERS {
            assert!(map[path].is_active);
            assert_eq!(map[path].flags.counter("arrived"), 1);
            assert_eq!(map[path].spawned_entities.len(), 1);
        }
        let identity = project_phoenix::snapshot::capture(deferred.world())
            .boot_identity
            .unwrap();
        assert!(
            !identity.game_start_entity_uuids.is_empty(),
            "ordinary GameStart, and only GameStart, completes the durable roster"
        );
        assert_eq!(entity_identities(&mut deferred).len(), 5);
    }
}

#[derive(Resource, Default)]
struct MintProbe {
    before: Option<WorldId>,
    after: Option<WorldId>,
}

fn mint_before_load(
    config: Option<Res<WorldConfig>>,
    mint: Res<WorldIdMint>,
    mut probe: ResMut<MintProbe>,
) {
    if config.is_none() {
        probe.before = Some(mint.mint(IdNamespace::Entity));
    }
}
fn mint_after_load(
    config: Option<Res<WorldConfig>>,
    mint: Res<WorldIdMint>,
    mut probe: ResMut<MintProbe>,
) {
    if config.is_some() && probe.after.is_none() {
        probe.after = Some(mint.mint(IdNamespace::Entity));
    }
}

#[test]
fn runtime_materialization_restores_the_live_tick_and_next_mint_sequence() {
    let mut app = build_native_host_app(&deferred_config(), &preload()).unwrap();
    app.init_resource::<MintProbe>()
        .add_systems(FixedUpdate, mint_before_load.before(NativeWorldLoadSet))
        .add_systems(FixedUpdate, mint_after_load.after(NativeWorldLoadSet));
    pump(&mut app, 75);
    select(&mut app, "phone-1", PICK, HULL);
    pump(&mut app, 4);
    let probe = app.world().resource::<MintProbe>();
    let before = probe.before.expect("a mint immediately before the load");
    let after = probe.after.expect("a mint immediately after the load");
    assert!(before.tick > 0);
    assert_eq!(after.tick, before.tick);
    assert_eq!(
        after.seq,
        before.seq + 1,
        "parking at tick zero must not consume or reset the live tick's sequence"
    );
    for (_, uuid) in entity_identities(&mut app) {
        assert_eq!(WorldId::parse(&uuid).unwrap().tick, 0);
    }
}

#[test]
fn both_schedules_register_the_same_materialization_members_once() {
    use bevy::ecs::schedule::{graph::Direction, NodeId, ScheduleLabel, SystemSet};
    use project_phoenix::world::materialization::WorldMaterialization;
    let app = loaded_lobby_host();
    let members = |label: bevy::ecs::schedule::InternedScheduleLabel| {
        let graph = app.get_schedule(label).unwrap().graph();
        let set = graph
            .system_sets
            .get_key(WorldMaterialization.intern())
            .unwrap();
        let mut pending = vec![NodeId::Set(set)];
        let mut seen = std::collections::HashSet::new();
        let mut names = Vec::new();
        while let Some(node) = pending.pop() {
            if !seen.insert(node) {
                continue;
            }
            match node {
                NodeId::System(key) => {
                    names.push(graph.systems.get(key).unwrap().system.name().to_string())
                }
                NodeId::Set(_) => pending.extend(
                    graph
                        .hierarchy()
                        .graph()
                        .neighbors_directed(node, Direction::Outgoing),
                ),
            }
        }
        names.sort();
        names
    };
    let startup = members(Startup.intern());
    let runtime = members(RuntimeWorldLoad.intern());
    assert!(
        startup.iter().any(|name| name.ends_with("::setup_world")),
        "the shared pass includes anonymous spawning and dev system names are available"
    );
    assert_eq!(
        startup, runtime,
        "compare registration membership, including multiplicity, without a second system list"
    );
    assert!(
        startup.windows(2).all(|pair| pair[0] != pair[1]),
        "a materialization system must run once in each schedule"
    );
}
