use super::*;
use crate::console::weapons::PhaserCombatConfigResource;
use crate::entities::{ai_declaration_manifest::AiDeclarationMode, config::EntityConfig, spawner};
use crate::ship_plugin::{ImpulseConfigResource, ShipConfigComponent, ShipPhysicsConfigResource};

fn config(text: &str) -> EntityConfig {
    EntityConfig::from_toml_in_mode(text, AiDeclarationMode::Lenient).unwrap()
}

fn topology(config: &EntityConfig) -> crate::ship::config::ShipConfig {
    config
        .ship_config
        .clone()
        .unwrap_or_else(|| crate::ship::config::ShipConfig {
            stations: Vec::new(),
            systems: Vec::new(),
            power_groups: Default::default(),
            coordination_lag_secs: 0.0,
        })
}

fn spawn_pair(config: &EntityConfig) -> (App, Entity, Entity) {
    let mut app = App::new();
    let mut npc_config = config.clone();
    npc_config.behaviour = self::config("[behaviour]").behaviour;
    let player_topology = topology(config);
    let (resolver, ratings) = crate::ship::rating::seed_boot_ratings(&player_topology, |_| {
        crate::ship::rating::BACKFILL_RATING.into()
    });
    let (npc, player) = {
        let mut commands = app.world_mut().commands();
        let npc = spawner::spawn_entity(
            &mut commands,
            &npc_config,
            Vec3::new(10.0, 0.0, 20.0),
            "npc".into(),
            None,
        );
        let player = spawner::spawn_entity_with_ship_seed(
            &mut commands,
            config,
            Vec3::new(30.0, 0.0, 40.0),
            "player".into(),
            None,
            Some(crate::entities::ship_spawn::ShipSpawnSeed {
                ship_config: ShipConfigComponent(player_topology),
                control_sources: crate::ship_plugin::ShipSystemControlSources(resolver),
                active_ratings: crate::ship_plugin::ActiveStationRatings(ratings),
                initial_yaw: 1.2,
            }),
        );
        publish_compatibility_ship_resources(&mut commands, player);
        (npc, player)
    };
    app.world_mut().flush();
    (app, npc, player)
}

#[test]
fn shared_spawn_absent_and_policy_only_equipment_stays_absent() {
    for text in [
        "",
        "[repair]\nrepair_team_count = 0\n[shields_console.ai]\ndamage_window_secs = 7.0",
    ] {
        let authored = config(text);
        assert_eq!(
            crate::lobby::server::project_ship_client_config(&authored).repair_team_count,
            0
        );
        let (app, npc, player) = spawn_pair(&authored);
        for entity in [npc, player] {
            let e = app.world().entity(entity);
            assert!(e.contains::<Ship>());
            assert!(!e.contains::<ShipShields>());
            assert!(!e.contains::<ShipRepairTeams>());
            assert!(!e.contains::<PhaserCombatConfigResource>());
            assert!(!e.contains::<TorpedoSystemResource>());
            assert!(e.contains::<crate::console::repair::server::RepairRequestQueue>());
            assert!(e.contains::<crate::ship_plugin::ShipIntentNarration>());
            assert_eq!(
                e.get::<ImpulseConfigResource>()
                    .unwrap()
                    .steering_multiplier,
                0.1
            );
            assert!(
                !e.get::<crate::ship_plugin::BoostConfigResource>()
                    .unwrap()
                    .enabled
            );
        }
        let physics = app.world().get::<ShipPhysicsComponent>(player).unwrap();
        assert_eq!((physics.x, physics.z, physics.yaw), (30.0, 40.0, 1.2));
        assert_eq!(app.world().get::<EntityUuid>(npc).unwrap().0, "npc");
        assert!(!app
            .world()
            .contains_resource::<PhaserCombatConfigResource>());
        assert!(!app.world().contains_resource::<TorpedoSystemResource>());
    }
}

#[test]
fn shared_spawn_authored_values_and_legacy_blocks_are_identical() {
    let authored = config(
        r#"
[power]
capacity = 137.0
rates = [8.0, 7.0, 6.0, 5.0, -3.0, -9.0]
emergency_threshold = 17.0
[power_groups.ops]
label = "Ops"
default_level = 3
min_level = 0
max_level = 4
[helm_console]
max_speed = 87.0
max_bank_deg = 23.0
power_multipliers = [-0.2, 0.1, 0.3, 0.7]
[repair]
repair_team_count = 3
travel_duration_secs = 7.0
repair_rate_hp_per_sec = 1.5
[shields_console]
frequency = 0.7
[shields_console.base]
num_facings = 3
max_hp = 67
regen_per_sec = 3.0
offline_duration = 8.0
[weapons_console]
[torpedoes]
count = 13
"#,
    );
    let (app, npc, player) = spawn_pair(&authored);
    for entity in [npc, player] {
        let e = app.world().entity(entity);
        assert_eq!(e.get::<PowerConfigResource>().unwrap().0.capacity, 137.0);
        assert_eq!(
            e.get::<ShipPowerSystem>()
                .unwrap()
                .0
                .level_for(&crate::core::messages::PowerGroupId("ops".into())),
            3
        );
        assert_eq!(
            e.get::<ShipPhysicsConfigResource>().unwrap().0.max_speed,
            87.0
        );
        assert_eq!(
            e.get::<ImpulseConfigResource>()
                .unwrap()
                .steering_multiplier,
            0.1
        );
        assert_eq!(
            e.get::<crate::ship_plugin::BankConfigResource>()
                .unwrap()
                .max_bank_deg,
            23.0
        );
        let repair = &e.get::<ShipRepairTeams>().unwrap().0;
        assert_eq!(repair.slots().len(), 3);
        assert_eq!(repair.timings().travel_duration, 7.0);
        let shields = e.get::<ShipShields>().unwrap();
        assert_eq!(shields.0.facings.len(), 3);
        assert_eq!(shields.1, 0.7);
        assert!(e
            .get::<PhaserCombatConfigResource>()
            .unwrap()
            .0
            .banks
            .is_empty());
        assert_eq!(
            e.get::<TorpedoSystemResource>()
                .unwrap()
                .0
                .torpedoes_remaining,
            13
        );
    }
    assert_eq!(
        app.world().resource::<PowerConfigResource>().0.capacity,
        137.0
    );
    assert_eq!(
        app.world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining,
        13
    );
}

#[test]
fn shared_spawn_game_start_without_behaviour_does_not_invent_equipment() {
    use crate::world::config::{TransformConfig, WorldConfig, WorldEntity, WorldEntitySpawnOn};
    let mut app = App::new();
    let empty = config("");
    app.insert_resource(crate::ship_plugin::PendingShipConfig(topology(&empty)));
    app.insert_resource(WorldConfig {
        entities: vec![WorldEntity {
            template_path: "assets/entities/nav_beacon.toml".into(),
            name: Some("test-unarmed-ship".into()),
            spawn_on: WorldEntitySpawnOn::GameStart,
            overrides: Some(toml::from_str("tags = ['ship']").unwrap()),
            transform: Some(TransformConfig {
                position: Some([11.0, 0.0, 22.0]),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    });
    app.add_systems(Update, spawn_game_start_entities);
    app.update();
    let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
    let entity = q.single(app.world()).unwrap();
    let e = app.world().entity(entity);
    assert!(!e.contains::<BehaviourSection>());
    assert!(!e.contains::<ShipShields>());
    assert!(!e.contains::<ShipRepairTeams>());
    assert!(!e.contains::<PhaserCombatConfigResource>());
    assert!(!e.contains::<TorpedoSystemResource>());
    assert!(e.contains::<crate::ai::server::AiHighFidelity>());
    assert!(e.contains::<crate::console::repair::server::RepairRequestQueue>());
    assert!(!app
        .world()
        .contains_resource::<crate::ship_plugin::PendingShipConfig>());
    assert_eq!(e.get::<ShipPhysicsComponent>().unwrap().x, 11.0);
}

#[test]
fn shared_spawn_shipped_hulls_keep_authored_equipment_on_game_start() {
    use crate::world::config::{WorldConfig, WorldEntity, WorldEntitySpawnOn};
    for path in [
        "assets/entities/alliance_cruiser.toml",
        "assets/entities/alliance_destroyer.toml",
    ] {
        let authored = crate::entities::include_resolve::load_entity_config(path).unwrap();
        let mut app = App::new();
        app.insert_resource(crate::ship_plugin::PendingShipConfig(topology(&authored)));
        app.insert_resource(WorldConfig {
            entities: vec![WorldEntity {
                template_path: path.into(),
                spawn_on: WorldEntitySpawnOn::GameStart,
                ..Default::default()
            }],
            ..Default::default()
        });
        app.add_systems(Update, spawn_game_start_entities);
        app.update();
        let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
        let player = q.single(app.world()).unwrap();
        let npc = spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &authored,
            Vec3::ZERO,
            "npc".into(),
            None,
        );
        app.world_mut().flush();
        for entity in [npc, player] {
            let e = app.world().entity(entity);
            assert_eq!(
                e.get::<PowerConfigResource>().unwrap().0.capacity,
                authored.power.as_ref().unwrap().capacity
            );
            assert_eq!(
                e.get::<ShipPhysicsConfigResource>().unwrap().0.max_speed,
                authored.helm_console.as_ref().unwrap().max_speed
            );
            assert_eq!(
                e.get::<PhaserCombatConfigResource>()
                    .map(|c| c.0.banks.clone()),
                authored
                    .weapons_console
                    .as_ref()
                    .map(|c| c.phaser_banks.clone())
            );
            assert_eq!(
                e.get::<ShipRepairTeams>().map(|r| r.0.slots().len()),
                authored
                    .repair
                    .as_ref()
                    .filter(|r| r.declares_teams())
                    .map(|r| r.repair_team_count as usize)
            );
            assert_eq!(
                e.get::<TorpedoSystemResource>().map(|r| r.0.config.count),
                authored.torpedoes.as_ref().map(|r| r.count)
            );
        }
        let npc_shields = &app.world().get::<ShipShields>(npc).unwrap().0;
        let player_shields = &app.world().get::<ShipShields>(player).unwrap().0;
        assert_eq!(npc_shields.snapshot(), player_shields.snapshot());
    }
}

#[test]
fn shared_spawn_resource_publication_keeps_fleet_order_without_cross_ship_state() {
    use bevy::ecs::system::RunSystemOnce;
    #[derive(Resource)]
    struct Hulls(Vec<EntityConfig>);
    let mut app = App::new();
    let mut first = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut second = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_destroyer.toml",
    )
    .unwrap();
    first.power.as_mut().unwrap().capacity = 111.0;
    second.power.as_mut().unwrap().capacity = 222.0;
    app.insert_resource(Hulls(vec![first, second]));
    app.world_mut()
        .run_system_once(|mut commands: Commands, hulls: Res<Hulls>| {
            let cache: crate::entities::config_cache::ConfigCache = [
                ("first".into(), hulls.0[0].clone()),
                ("second".into(), hulls.0[1].clone()),
            ]
            .into_iter()
            .collect();
            for (index, name) in ["first", "second"].into_iter().enumerate() {
                let crew = vec![(
                    crate::core::messages::StationId("helm".into()),
                    "Std".into(),
                )];
                let placement = FleetPlacement {
                    host: crate::command_admission::HostSlot(index as u32 + 1),
                    is_local: index == 0,
                    uses_live_sessions: false,
                    crew: &crew,
                    hull_path: Some(name),
                    authored_slot_id: None,
                };
                let seed =
                    prepare_ship_seed(&mut commands, 0.3, &mut None, &mut None, &placement, &cache);
                let entity = spawner::spawn_entity_with_ship_seed(
                    &mut commands,
                    &hulls.0[index],
                    Vec3::ZERO,
                    name.into(),
                    None,
                    Some(seed),
                );
                insert_fleet_metadata(&mut commands, entity, &hulls.0[index], &placement);
                publish_compatibility_ship_resources(&mut commands, entity);
            }
        })
        .unwrap();
    app.world_mut().flush();
    let mut q = app.world_mut().query::<(
        &EntityUuid,
        &PowerConfigResource,
        &crate::ship_plugin::ActiveStationRatings,
    )>();
    let rows: Vec<_> = q.iter(app.world()).collect();
    assert_eq!(rows.len(), 2);
    for (uuid, power, ratings) in rows {
        assert_eq!(
            power.0.capacity,
            if uuid.0 == "first" { 111.0 } else { 222.0 }
        );
        assert_eq!(
            ratings
                .0
                .get(&crate::core::messages::StationId("helm".into()))
                .map(String::as_str),
            Some("Std")
        );
    }
    let mut local = app
        .world_mut()
        .query_filtered::<&EntityUuid, With<LocalShip>>();
    assert_eq!(local.single(app.world()).unwrap().0, "first");
    assert_eq!(
        app.world().resource::<PowerConfigResource>().0.capacity,
        222.0
    );
}

#[test]
fn shared_spawn_static_defence_and_nonship_capabilities_keep_their_classification() {
    let station =
        crate::entities::include_resolve::load_entity_config("assets/entities/station_axiom.toml")
            .unwrap();
    assert!(station.is_static_point_defence());
    let nonship = config(
        r#"
tags = ["ship"]
[helm_console]
max_speed = 29.0
[weapons_console]
[torpedoes]
count = 7
[[shield_arc]]
id = "all"
label = "test.arc"
center_deg = 0.0
width_deg = 360.0
max_hp = 15
regen_per_sec = 0.0
"#,
    );
    assert!(nonship.ship_config.is_some(), "arcs synthesize topology");
    let mut app = App::new();
    let station = spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &station,
        Vec3::ZERO,
        "station".into(),
        None,
    );
    let nonship = spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &nonship,
        Vec3::ZERO,
        "capabilities".into(),
        None,
    );
    app.world_mut().flush();
    let station = app.world().entity(station);
    assert!(station.contains::<Ship>());
    assert!(station.contains::<spawner::StaticPointDefence>());
    assert!(!station.contains::<spawner::BehaviourSection>());
    assert!(station.contains::<ShipPowerSystem>());
    assert!(station.contains::<PhaserCombatConfigResource>());
    let nonship = app.world().entity(nonship);
    assert!(
        !nonship.contains::<Ship>(),
        "tags and synthesized topology do not classify a ship"
    );
    assert!(!nonship.contains::<ShipPowerSystem>());
    assert!(nonship.contains::<ShipShields>());
    assert!(nonship.contains::<PhaserCombatConfigResource>());
    assert_eq!(
        nonship
            .get::<TorpedoSystemResource>()
            .unwrap()
            .0
            .torpedoes_remaining,
        7
    );
    assert_eq!(
        nonship
            .get::<ShipPhysicsConfigResource>()
            .unwrap()
            .0
            .max_speed,
        29.0
    );
}
