use super::*;

#[test]
fn presentation_configuration_reuses_templates_and_invalidates_on_content_changes() {
    let mut cache = PresentationConfigCache::default();
    cache.refresh_revision((1, 2));
    cache
        .ships
        .insert("hull".into(), ShipClientConfig::default());
    cache.refresh_revision((1, 2));
    assert_eq!(cache.ships.len(), 1);
    cache.refresh_revision((2, 2));
    assert!(cache.ships.is_empty());
    cache
        .ships
        .insert("hull".into(), ShipClientConfig::default());
    cache.refresh_revision((2, 3));
    assert!(cache.ships.is_empty());
}

#[test]
fn inspector_interest_is_absolute_bounded_and_retains_world_nodes() {
    let mut app = app();
    let mut config = crate::world::config::WorldConfig::default();
    config.global.title = Some("world.test".into());
    app.insert_resource(config);
    app.world_mut().resource_mut::<GmInspectorInterest>().0 = Some(Default::default());
    app.world_mut().run_schedule(PostUpdate);
    let closed = take(&mut app).pop().unwrap();
    assert!(closed.entity_inspector.fields.is_empty());
    assert!(closed.world_inspector.fields.is_empty());
    assert_eq!(closed.world_inspector.readings["root"].label, "world.test");
    app.world_mut().resource_mut::<GmInspectorInterest>().0 =
        Some([GmInspectorKind::WorldFields].into());
    app.world_mut().run_schedule(PostUpdate);
    let opened = take(&mut app).pop().unwrap();
    assert!(!opened.world_inspector.fields.is_empty());
    assert_eq!(opened.world_inspector.readings["root"].label, "world.test");
    assert!(crate::core::codec::decode_gm_inspector_interest("[\"not-a-panel\"]").is_none());
}

#[test]
fn catch_up_ticks_do_not_build_presentation_and_paused_frames_still_publish() {
    let mut app = app();
    app.init_schedule(FixedLast);
    for _ in 0..4 {
        app.world_mut().run_schedule(FixedLast);
    }
    assert!(take(&mut app).is_empty());
    assert!(take_stations(&mut app).is_empty());
    app.world_mut().run_schedule(PostUpdate);
    assert_eq!(take(&mut app).len(), 1);
    assert_eq!(take_stations(&mut app).len(), 1);
    app.insert_resource(crate::gm_action::SimulationPaused(true));
    app.world_mut().spawn((
        Ship,
        EntityUuid("new-ship".into()),
        hull(100.0),
        ShipPhysics::default(),
    ));
    app.world_mut().run_schedule(PostUpdate);
    assert_eq!(take(&mut app).len(), 1);
}
use crate::ai::faction::{FactionConfig, FactionRegistry};
use crate::command_admission::HostSlot;
use crate::core::messages::SystemId;
use crate::entities::config::{AsteroidFieldConfig, RadarAppearanceConfig};
use crate::infrastructure::{InfrastructureConfig, InfrastructureState};
use crate::regions::effects::RegionEffectKind;
use crate::server_app::{Asteroid, AsteroidUuid};
use crate::ship::damage::SystemHull;
use crate::world::server::EntityOriginLayer;
use uuid::Uuid;

const PLAYER_ID: &str = "00000000-0000-4000-8000-000000000001";
const NPC_ID: &str = "00000000-0000-4000-8000-000000000002";
const FACTION_ID: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";

#[test]
fn ship_inspector_uses_spawned_template_identity_when_topologies_match() {
    let same = crate::entities::config::EntityConfig::default();
    let cache = crate::entities::config_cache::ConfigCache::from([
        ("assets/entities/a.toml".to_string(), same.clone()),
        ("assets/entities/b.toml".to_string(), same),
    ]);
    let identity = EntityTemplatePath::new("assets/entities/./b.toml");
    let (path, _) = ship_inspector_authored_config(Some(&identity), &cache).unwrap();
    assert_eq!(path, "assets/entities/b.toml");
    assert!(ship_inspector_authored_config(None, &cache).is_none());
}

fn hull(percent: f32) -> EntitySystemHull {
    let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);
    hull.apply_damage(100.0 - percent, &mut crate::sim_rng::unseeded_test_rng());
    EntitySystemHull(hull)
}

fn take(app: &mut App) -> Vec<GmEntityProjectionPayload> {
    app.world_mut()
        .resource_mut::<Messages<GmEntityProjectionChanged>>()
        .drain()
        .map(|event| event.payload)
        .collect()
}

fn app() -> App {
    let faction_uuid = Uuid::parse_str(FACTION_ID).unwrap();
    let mut factions = FactionRegistry::new();
    factions.insert(FactionConfig {
        uuid: faction_uuid,
        name: "Alliance".into(),
        display_name: Some("faction.alliance.display_name".into()),
        enemies: Vec::new(),
        compliance: None,
    });
    let mut app = App::new();
    app.insert_resource(BrowserGameMaster)
        .insert_resource(FactionRegistryResource(factions))
        .insert_resource(WorldContentRuntime::default())
        .insert_resource(WorldResource::default())
        .add_plugins(GmProjectionPlugin);
    app
}

fn take_stations(app: &mut App) -> Vec<GmStationProjectionPayload> {
    app.world_mut()
        .resource_mut::<Messages<GmStationProjectionChanged>>()
        .drain()
        .map(|event| event.payload)
        .collect()
}

#[test]
fn projects_player_and_npc_in_stable_order_with_position_faction_health_and_target() {
    let mut app = app();
    let faction = FactionComponent(Uuid::parse_str(FACTION_ID).unwrap());
    app.world_mut().spawn((
        Ship,
        EntityUuid(NPC_ID.into()),
        EntityName("Raider".into()),
        hull(40.0),
        ShipPhysics {
            x: 80.0,
            y: 2.0,
            z: -30.0,
            ..Default::default()
        },
        faction.clone(),
        TacticalRadarSelection(Some(PLAYER_ID.into())),
    ));
    app.world_mut().spawn((
        Ship,
        EntityUuid(PLAYER_ID.into()),
        EntityName("Cruiser".into()),
        hull(100.0),
        ShipPhysics {
            x: -12.0,
            z: 7.0,
            ..Default::default()
        },
        faction,
        FleetSlotOf(HostSlot::SOLO),
        TacticalRadarSelection(Some(NPC_ID.into())),
    ));

    app.world_mut().run_schedule(PostUpdate);
    let payload = take(&mut app).pop().expect("first absolute projection");
    assert_eq!(payload.entities.len(), 2);
    assert_eq!(payload.entities[0].entity_id, PLAYER_ID);
    assert_eq!(payload.entities[0].kind, GmEntityKind::PlayerShip);
    assert_eq!(payload.entities[0].position, [-12.0, 0.0, 7.0]);
    assert_eq!(payload.entities[0].status.hull_percent, Some(100));
    assert_eq!(payload.entities[0].status.condition_percent, None);
    assert_eq!(
        payload.entities[0].faction.as_ref().unwrap().name,
        "faction.alliance.display_name"
    );
    assert_eq!(
        payload.entities[0].current_target,
        Some(GmEntityReference {
            entity_id: NPC_ID.into(),
            name: "Raider".into(),
        })
    );
    assert_eq!(payload.entities[1].kind, GmEntityKind::NpcShip);
    assert_eq!(payload.entities[1].position, [80.0, 2.0, -30.0]);
    assert_eq!(payload.entities[1].status.hull_percent, Some(40));
    assert!(!payload.entities[1].status.destroyed);
}

#[test]
fn republishes_updates_and_removal_but_ignores_non_ship_entities() {
    let mut app = app();
    let npc = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid(NPC_ID.into()),
            EntityName("Raider".into()),
            hull(40.0),
            ShipPhysics::default(),
            TacticalRadarSelection::default(),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid("00000000-0000-4000-8000-000000000099".into()),
        EntityName("Not a ship".into()),
        hull(100.0),
        ShipPhysics::default(),
    ));

    app.world_mut().run_schedule(PostUpdate);
    assert_eq!(take(&mut app)[0].entities.len(), 1);
    app.world_mut().run_schedule(PostUpdate);
    assert!(
        take(&mut app).is_empty(),
        "unchanged absolute state is deduped"
    );

    app.world_mut()
        .entity_mut(npc)
        .get_mut::<ShipPhysics>()
        .unwrap()
        .x = 5.0;
    app.world_mut()
        .entity_mut(npc)
        .get_mut::<TacticalRadarSelection>()
        .unwrap()
        .0 = Some("removed-target".into());
    app.world_mut().run_schedule(PostUpdate);
    let updated = take(&mut app).pop().unwrap();
    assert_eq!(updated.entities[0].position[0], 5.0);
    assert_eq!(
        updated.entities[0].current_target.as_ref().unwrap().name,
        "removed-target",
        "a stale target remains a stable link without exposing ECS state"
    );

    app.world_mut().despawn(npc);
    app.world_mut().run_schedule(PostUpdate);
    assert!(take(&mut app).pop().unwrap().entities.is_empty());
}

#[test]
fn static_point_defence_is_a_structure_even_when_it_carries_the_ship_substrate() {
    let mut app = app();
    app.world_mut().spawn((
        Ship,
        StaticPointDefence,
        EntityUuid("00000000-0000-4000-8000-000000000003".into()),
        EntityName("Axiom station".into()),
        hull(100.0),
        ShipPhysics::default(),
        TacticalRadarSelection::default(),
    ));

    app.world_mut().run_schedule(PostUpdate);
    let payload = take(&mut app)
        .pop()
        .expect("first absolute projection must clear stale browser state");
    assert_eq!(payload.entities.len(), 1);
    assert_eq!(payload.entities[0].kind, GmEntityKind::Structure);
    assert_eq!(payload.entities[0].name, "Axiom station");
}

#[test]
fn canonical_structure_tag_projects_authored_structure_without_infrastructure() {
    const STRUCTURE_ID: &str = "00000000-0000-4000-8000-000000000004";
    let mut app = app();
    app.world_mut().spawn((
        EntityUuid(STRUCTURE_ID.into()),
        EntityName("entity.dock_berth.display_name".into()),
        Transform::from_xyz(30.0, 2.0, -12.0),
        EntityTagsSection(vec![EntityTag::Structure.as_str().into()]),
        RadarAppearanceSection(RadarAppearanceConfig {
            icon: Some("ship".into()),
            colour: Some(vec![0.9, 0.7, 0.2]),
            size: Some(5.0),
            region_colour: None,
        }),
    ));

    app.world_mut().run_schedule(PostUpdate);
    let payload = take(&mut app).pop().expect("absolute structure projection");
    assert_eq!(payload.entities.len(), 1);
    assert_eq!(payload.entities[0].entity_id, STRUCTURE_ID);
    assert_eq!(payload.entities[0].kind, GmEntityKind::Structure);
    assert_eq!(payload.entities[0].position, [30.0, 2.0, -12.0]);
    assert_eq!(payload.entities[0].status.hull_percent, None);
    assert_eq!(payload.entities[0].status.condition_percent, None);
    assert_eq!(payload.entities[0].radar.icon.as_deref(), Some("ship"));
}

/// A planet carries no region shape, no infrastructure and no structure
/// tag, so before this it fell through `world_kind` to `None` and never
/// reached the GM map at all (GM console feedback: "the planet in
/// combat_test didn't show up in the radar").
#[test]
fn planet_tag_projects_a_celestial_blip_with_its_radar_appearance() {
    const PLANET_ID: &str = "00000000-0000-4000-8000-000000000007";
    let mut app = app();
    app.world_mut().spawn((
        EntityUuid(PLANET_ID.into()),
        EntityName("entity.planet_ecumenopolis.name".into()),
        Transform::from_xyz(500.0, 0.0, -120.0),
        EntityTagsSection(vec![EntityTag::Planet.as_str().into(), "habitable".into()]),
        RadarAppearanceSection(RadarAppearanceConfig {
            icon: Some("planet".into()),
            colour: Some(vec![1.0, 0.8, 0.4]),
            size: Some(33.0),
            region_colour: None,
        }),
    ));

    app.world_mut().run_schedule(PostUpdate);
    let payload = take(&mut app).pop().expect("absolute celestial projection");
    assert_eq!(payload.entities.len(), 1);
    assert_eq!(payload.entities[0].entity_id, PLANET_ID);
    assert_eq!(payload.entities[0].kind, GmEntityKind::Celestial);
    assert_eq!(payload.entities[0].position, [500.0, 0.0, -120.0]);
    assert!(payload.entities[0].geometry.is_none());
    assert_eq!(payload.entities[0].radar.icon.as_deref(), Some("planet"));
    assert_eq!(payload.entities[0].radar.size, Some(33.0));
}

#[test]
fn projects_structures_hazards_regions_and_fields_with_broad_status_and_geometry() {
    const STRUCTURE_ID: &str = "00000000-0000-4000-8000-000000000013";
    const HAZARD_ID: &str = "00000000-0000-4000-8000-000000000011";
    const REGION_ID: &str = "00000000-0000-4000-8000-000000000012";
    const FIELD_ID: &str = "00000000-0000-4000-8000-000000000010";

    let mut app = app();
    let infrastructure = InfrastructureConfig {
        condition_max: 100.0,
        condition: Some(64.0),
        ..Default::default()
    };
    app.world_mut().spawn((
        EntityUuid(STRUCTURE_ID.into()),
        EntityId("entity.station.axiom.display_name".into()),
        Transform::from_xyz(25.0, 0.0, -10.0),
        EntityTagsSection(vec![EntityTag::Station.as_str().into()]),
        hull(80.0),
        InfrastructureCondition(InfrastructureState::from_config(&infrastructure)),
        RadarAppearanceSection(RadarAppearanceConfig {
            icon: Some("station".into()),
            colour: Some(vec![0.2, 0.4, 0.8]),
            size: Some(12.0),
            region_colour: None,
        }),
        EntityOriginLayer("assets/worlds/layer-station.toml".into()),
    ));
    let hazard_entity = app
        .world_mut()
        .spawn((
            EntityUuid(HAZARD_ID.into()),
            EntityName("region.storm.display_name".into()),
            Transform::from_xyz(100.0, 2.0, 50.0),
            EntityTagsSection(vec![EntityTag::Region.as_str().into()]),
            RegionShapeSection(RegionShape::Sphere { radius: 40.0 }),
            RegionEffectsSection(vec![RegionEffectKind::BlocksImpulse]),
            RadarAppearanceSection(RadarAppearanceConfig {
                icon: None,
                colour: None,
                size: None,
                region_colour: Some(vec![0.9, 0.2, 0.1]),
            }),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid(REGION_ID.into()),
        EntityName("region.safe_harbour.display_name".into()),
        Transform::from_xyz(-60.0, 0.0, 20.0),
        EntityTagsSection(vec![EntityTag::Region.as_str().into()]),
        RegionShapeSection(RegionShape::Box {
            half_extents: [10.0, 5.0, 30.0],
            yaw: 0.25,
        }),
        RadarAppearanceSection(RadarAppearanceConfig {
            icon: None,
            colour: None,
            size: None,
            region_colour: Some(vec![0.1, 0.7, 0.5]),
        }),
    ));
    let occupant = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid("00000000-0000-4000-8000-000000000014".into()),
            EntityName("Scout".into()),
        ))
        .id();
    let mut membership = crate::regions::server::RegionMembership::default();
    membership
        .inside
        .insert(occupant, [hazard_entity].into_iter().collect());
    app.insert_resource(membership);
    app.insert_resource(WorldResource(crate::core::messages::WorldData {
        entities: vec![crate::core::messages::EntitySnapshot {
            uuid: HAZARD_ID.into(),
            name: Some("Storm front".into()),
            ..Default::default()
        }],
        ..Default::default()
    }));
    app.world_mut().spawn((
        EntityUuid(FIELD_ID.into()),
        EntityId("entity.belt.delta.display_name".into()),
        Transform::from_xyz(999.0, 999.0, 999.0),
        EntityTagsSection(vec![EntityTag::AsteroidField.as_str().into()]),
        AsteroidFieldSection(AsteroidFieldConfig {
            inner_radius: 25.0,
            outer_radius: 125.0,
            anchor_offset: [300.0, 0.0, -50.0],
            ..Default::default()
        }),
        RadarAppearanceSection(RadarAppearanceConfig {
            icon: None,
            colour: None,
            size: None,
            region_colour: Some(vec![0.5, 0.5, 0.5]),
        }),
    ));

    app.world_mut().run_schedule(PostUpdate);
    let payload = take(&mut app).pop().unwrap();
    assert_eq!(
        payload
            .entities
            .iter()
            .map(|entity| entity.entity_id.as_str())
            .collect::<Vec<_>>(),
        vec![FIELD_ID, HAZARD_ID, REGION_ID, STRUCTURE_ID]
    );

    let field = &payload.entities[0];
    assert_eq!(field.kind, GmEntityKind::AsteroidField);
    assert_eq!(field.position, [300.0, 0.0, -50.0]);
    assert_eq!(
        field.geometry,
        Some(RegionShape::Torus {
            inner_radius: 25.0,
            outer_radius: 125.0,
        })
    );
    assert_eq!(field.radar.region_colour, Some([0.5, 0.5, 0.5]));

    let hazard = &payload.entities[1];
    assert_eq!(hazard.kind, GmEntityKind::Hazard);
    assert_eq!(hazard.geometry, Some(RegionShape::Sphere { radius: 40.0 }));
    assert_eq!(hazard.status.hull_percent, None);
    assert_eq!(hazard.status.condition_percent, None);

    let region = &payload.entities[2];
    assert_eq!(region.kind, GmEntityKind::Region);
    assert_eq!(
        region.geometry,
        Some(RegionShape::Box {
            half_extents: [10.0, 5.0, 30.0],
            yaw: 0.25,
        })
    );

    let structure = &payload.entities[3];
    assert_eq!(structure.kind, GmEntityKind::Structure);
    assert_eq!(structure.status.hull_percent, Some(80));
    assert_eq!(structure.status.condition_percent, Some(64));
    assert_eq!(structure.radar.icon.as_deref(), Some("station"));
    assert_eq!(structure.radar.colour, Some([0.2, 0.4, 0.8]));
    assert_eq!(structure.radar.size, Some(12.0));

    assert_eq!(payload.region_inspector.readings.len(), 2);
    let hazard_reading = &payload.region_inspector.readings[HAZARD_ID];
    assert_eq!(hazard_reading.values["identity.kind"], "hazard");
    assert_eq!(
        hazard_reading.values["identity.display_name"],
        "Storm front"
    );
    assert_eq!(hazard_reading.label, "Storm front");
    assert_eq!(
        hazard_reading.values["effects.blocks_impulse.present"],
        "true"
    );
    assert_eq!(hazard_reading.occupants.len(), 1);
    assert_eq!(hazard_reading.occupants[0].label, "Scout");
    assert_eq!(
        hazard_reading.occupants[0].consequences[0].kind,
        "blocks-impulse"
    );
    let region_reading = &payload.region_inspector.readings[REGION_ID];
    assert_eq!(region_reading.values["identity.kind"], "region");
    assert_eq!(region_reading.values["shape.box.yaw"], "0.25");
    assert_eq!(
        region_reading.values["presentation.radar.region_colour.g"],
        "0.7"
    );
}

#[test]
fn only_named_authored_asteroids_project_and_streamed_rocks_use_their_lifecycle_marker() {
    const AUTHORED_ID: &str = "00000000-0000-4000-8000-000000000021";
    const ANONYMOUS_ID: &str = "00000000-0000-4000-8000-000000000022";
    const STREAMED_ID: &str = "00000000-0000-4000-8000-000000000023";

    let mut app = app();
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .name_to_uuid
        .insert("named_rock".into(), AUTHORED_ID.into());
    app.world_mut().spawn((
        EntityUuid(AUTHORED_ID.into()),
        EntityId("entity.named_rock.display_name".into()),
        Transform::from_xyz(12.0, 0.0, 18.0),
        EntityTagsSection(vec![EntityTag::Asteroid.as_str().into()]),
        hull(75.0),
        RadarAppearanceSection(RadarAppearanceConfig {
            icon: Some("asteroid".into()),
            colour: Some(vec![0.7, 0.6, 0.4]),
            size: Some(3.0),
            region_colour: None,
        }),
    ));
    app.world_mut().spawn((
        EntityUuid(ANONYMOUS_ID.into()),
        EntityName("anonymous authored template".into()),
        Transform::default(),
        EntityTagsSection(vec![EntityTag::Asteroid.as_str().into()]),
        hull(100.0),
    ));
    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid(STREAMED_ID.into()),
        Transform::default(),
        hull(100.0),
    ));

    app.world_mut().run_schedule(PostUpdate);
    let payload = take(&mut app).pop().unwrap();
    assert_eq!(payload.entities.len(), 1);
    assert_eq!(payload.entities[0].entity_id, AUTHORED_ID);
    assert_eq!(payload.entities[0].kind, GmEntityKind::AuthoredAsteroid);
    assert_eq!(payload.entities[0].status.hull_percent, Some(75));
    assert_eq!(payload.entities[0].radar.icon.as_deref(), Some("asteroid"));
    assert!(payload
        .entities
        .iter()
        .all(|entity| entity.entity_id != STREAMED_ID));
}

#[test]
fn absolute_projection_sorts_deduplicates_and_reconciles_layer_removal_by_uuid() {
    const FIRST_ID: &str = "00000000-0000-4000-8000-000000000031";
    const SECOND_ID: &str = "00000000-0000-4000-8000-000000000032";

    let mut app = app();
    let second = app
        .world_mut()
        .spawn((
            EntityUuid(SECOND_ID.into()),
            EntityName("region.second.display_name".into()),
            Transform::from_xyz(20.0, 0.0, 0.0),
            EntityTagsSection(vec![EntityTag::Region.as_str().into()]),
            RegionShapeSection(RegionShape::Sphere { radius: 5.0 }),
            EntityOriginLayer("assets/worlds/layer-two.toml".into()),
        ))
        .id();
    let first = app
        .world_mut()
        .spawn((
            EntityUuid(FIRST_ID.into()),
            EntityName("region.first.display_name".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            EntityTagsSection(vec![EntityTag::Region.as_str().into()]),
            RegionShapeSection(RegionShape::Sphere { radius: 4.0 }),
            EntityOriginLayer("assets/worlds/layer-one.toml".into()),
        ))
        .id();
    let duplicate = app
        .world_mut()
        .spawn((
            EntityUuid(FIRST_ID.into()),
            EntityName("region.first.display_name".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            EntityTagsSection(vec![EntityTag::Region.as_str().into()]),
            RegionShapeSection(RegionShape::Sphere { radius: 4.0 }),
            EntityOriginLayer("assets/worlds/layer-one.toml".into()),
        ))
        .id();

    app.world_mut().run_schedule(PostUpdate);
    let initial = take(&mut app).pop().unwrap();
    assert_eq!(initial.entities.len(), 2, "duplicate UUIDs collapse");
    assert_eq!(initial.entities[0].entity_id, FIRST_ID);
    assert_eq!(initial.entities[1].entity_id, SECOND_ID);
    app.world_mut().run_schedule(PostUpdate);
    assert!(take(&mut app).is_empty(), "unchanged state emits nothing");

    app.world_mut().despawn(second);
    app.world_mut().run_schedule(PostUpdate);
    let removed = take(&mut app).pop().unwrap();
    assert_eq!(removed.entities.len(), 1);
    assert_eq!(removed.entities[0].entity_id, FIRST_ID);

    app.world_mut().despawn(first);
    app.world_mut().despawn(duplicate);
    app.world_mut().spawn((
        EntityUuid(FIRST_ID.into()),
        EntityName("region.first.display_name".into()),
        Transform::from_xyz(77.0, 0.0, -9.0),
        EntityTagsSection(vec![EntityTag::Region.as_str().into()]),
        RegionShapeSection(RegionShape::Sphere { radius: 4.0 }),
        EntityOriginLayer("assets/worlds/reloaded-layer-one.toml".into()),
    ));
    app.world_mut().run_schedule(PostUpdate);
    let reappeared = take(&mut app).pop().unwrap();
    assert_eq!(reappeared.entities.len(), 1);
    assert_eq!(reappeared.entities[0].entity_id, FIRST_ID);
    assert_eq!(reappeared.entities[0].position, [77.0, 0.0, -9.0]);
}

#[test]
fn projects_only_fleet_ship_authored_station_interfaces_with_takeover_membership() {
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "captain"
name = "Captain"
description = ""
rank = ""
console = "gui/captain-console.html"

[[station]]
id = "hidden"
name = "Hidden"
description = ""
rank = ""

[[system]]
id = "red-alert"
kind = "captain"
station = "captain"
"#,
        &["captain"],
    )
    .unwrap();
    let uuid = "player-ship-1";
    let mut ratings = ActiveStationRatings::default();
    ratings.0.insert(
        StationId("captain".into()),
        crate::ship::rating::BACKFILL_RATING.into(),
    );
    let target = StationPuppetTarget::new(
        crate::command_admission::log::ShipKey(uuid.into()),
        StationId("captain".into()),
    );

    let mut app = App::new();
    app.insert_resource(BrowserGameMaster)
        .add_plugins(GmProjectionPlugin);
    app.world_mut()
        .resource_mut::<StationPuppets>()
        .set_operator(target, "gm-1".into(), true);
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::lockstep::FleetSlotOf(crate::command_admission::HostSlot::SOLO),
        EntityUuid(uuid.into()),
        EntityName("Resolute".into()),
        ShipConfigComponent(config),
        ratings,
        ShipSystemControlSources::default(),
        crate::server_app::ShipSystemBlackboards::default(),
    ));
    // A configured NPC is intentionally absent from this local projection.
    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid("npc-ship".into()),
        crate::ship::components::load_ship_config_from_disk(),
        ActiveStationRatings::default(),
        ShipSystemControlSources::default(),
        crate::server_app::ShipSystemBlackboards::default(),
    ));

    app.world_mut().run_schedule(PostUpdate);
    let payloads = take_stations(&mut app);
    assert_eq!(payloads.len(), 1);
    let ship = &payloads[0].ships[0];
    assert_eq!(ship.ship_id, uuid);
    assert_eq!(ship.stations.len(), 1, "no console means no interface row");
    assert_eq!(ship.stations[0].console, "gui/captain-console.html");
    assert_eq!(ship.stations[0].operators, ["gm-1"]);
    assert_eq!(
        ship.ship_config,
        ShipClientConfig::default(),
        "a fixture with no resolved entity-template path must not invent a partial GM config"
    );
}

#[test]
fn helm_projection_carries_absolute_world_pose_objective_waypoint_and_hull_truth() {
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = ""
rank = ""
console = "gui/cruiser/helm.html"

[[system]]
id = "drive-main"
kind = "helm_thrust"
station = "helm"
"#,
        &["helm_thrust"],
    )
    .unwrap();
    let ship_uuid = "player-ship-helm";
    let contact_uuid = "moving-contact";
    let mut app = App::new();
    app.insert_resource(BrowserGameMaster)
        .insert_resource(crate::lobby::server::WorldResource(
            crate::core::messages::WorldData {
                entities: vec![EntitySnapshot {
                    uuid: contact_uuid.into(),
                    name: Some("contact.name".into()),
                    position: Some([1.0, 0.0, 2.0]),
                    tags: vec!["ship".into()],
                    radar_icon: Some("ship".into()),
                    region_colour: Some([0.1, 0.2, 0.3]),
                    radius: Some(12.0),
                    ..EntitySnapshot::default()
                }],
                scenario_title: "scenario.title".into(),
                scenario_description: "scenario.description".into(),
            },
        ))
        .insert_resource(crate::world::server::ObjectiveManagerRes::default())
        .add_plugins(GmProjectionPlugin);
    app.world_mut()
        .resource_mut::<crate::world::server::ObjectiveManagerRes>()
        .0
        .add(
            "reach-contact",
            "objective.reach_contact",
            true,
            vec!["contact.name".into()],
        );
    app.world_mut().spawn((
        EntityUuid(contact_uuid.into()),
        Transform::from_xyz(40.0, 3.0, -25.0),
        hull(50.0),
    ));
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::lockstep::FleetSlotOf(crate::command_admission::HostSlot::SOLO),
        EntityUuid(ship_uuid.into()),
        EntityName("Resolute".into()),
        ShipConfigComponent(config),
        ActiveStationRatings::default(),
        ShipSystemControlSources::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        ShipPhysics {
            x: 125.0,
            y: 4.0,
            z: -75.0,
            yaw: 0.75,
            forward_speed: 18.0,
            ..ShipPhysics::default()
        },
        crate::console::navigation::server::NavigationWaypoint::new(
            crate::console::navigation::server::WaypointMode::Free {
                x: 240.0,
                z: -160.0,
            },
        ),
        hull(80.0),
    ));

    app.world_mut().run_schedule(PostUpdate);
    let payload = take_stations(&mut app).pop().expect("projection");
    let ship = payload
        .ships
        .iter()
        .find(|ship| ship.ship_id == ship_uuid)
        .expect("fleet ship");
    assert_eq!(ship.ship_pose.x, 125.0);
    assert_eq!(ship.ship_pose.z, -75.0);
    assert_eq!(ship.ship_pose.yaw, 0.75);
    assert_eq!(ship.ship_pose.forward_speed, 18.0);
    assert_eq!(
        ship.navigation_waypoint,
        Some(WaypointSnapshot {
            x: 240.0,
            z: -160.0,
            source_uuid: None,
        })
    );
    assert_eq!(ship.objectives.len(), 1);
    assert_eq!(ship.objectives[0].id, "reach-contact");
    assert_eq!(payload.entities[0].position, Some([1.0, 0.0, 2.0]));
    let live_contact = payload
        .entity_states
        .iter()
        .find(|entity| entity.uuid == contact_uuid)
        .expect("absolute live entity state");
    assert_eq!(live_contact.position, Some([40.0, 3.0, -25.0]));
    assert_eq!(ship.console_hull[0].current, 80.0);
}

#[test]
fn crew_objective_markers_are_scoped_after_real_entity_lifecycle_publication() {
    use crate::server_app::{
        LastBroadcastEntityHealth, LastBroadcastEntityPositions, SimOutbox, TrackedEntities,
    };
    use bevy::ecs::system::RunSystemOnce;
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "navigation"
name = "Navigation"
description = ""
rank = ""
console = "gui/navigation-console.html"
[[system]]
id = "nav-main"
kind = "navigation"
station = "navigation"
"#,
        &["navigation"],
    )
    .unwrap();
    let mut app = App::new();
    app.insert_resource(BrowserGameMaster)
        .init_resource::<crate::lobby::WorldResource>()
        .init_resource::<TrackedEntities>()
        .init_resource::<LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<SimOutbox>()
        .init_resource::<crate::world::server::ObjectiveManagerRes>()
        .add_plugins(GmProjectionPlugin);
    for (slot, id) in [(1, "ship-a"), (2, "ship-b")] {
        app.world_mut().spawn((
            crate::server_app::Ship,
            EntityUuid(id.into()),
            FleetSlotOf(crate::command_admission::HostSlot(slot)),
            ShipConfigComponent(config.clone()),
            ActiveStationRatings::default(),
            ShipSystemControlSources::default(),
            crate::server_app::ShipSystemBlackboards::default(),
        ));
    }
    {
        let manager = &mut app
            .world_mut()
            .resource_mut::<crate::world::server::ObjectiveManagerRes>()
            .0;
        manager.add(
            "private",
            "objective.test",
            true,
            vec!["seeded".into(), "spawned".into()],
        );
        manager.set_recipients("private", vec!["ship-a".into()]);
    }
    for id in ["seeded", "spawned"] {
        app.world_mut().spawn((
            EntityUuid(format!("uuid-{id}")),
            EntityId(id.into()),
            EntityTagsSection(vec!["objective_marker".into()]),
            Transform::from_xyz(20.0, 0.0, 10.0),
            RadarAppearanceSection(RadarAppearanceConfig {
                icon: Some("waypoint".into()),
                colour: None,
                size: None,
                region_colour: Some(vec![0.1, 0.2, 0.3]),
            }),
        ));
        app.world_mut()
            .run_system_once(crate::server_app::reconcile_runtime_entities)
            .unwrap();
        let metadata = app
            .world()
            .resource::<crate::lobby::WorldResource>()
            .0
            .clone();
        assert!(metadata
            .entities
            .iter()
            .all(|entity| entity.objective_target));
        app.world_mut().run_schedule(PostUpdate);
        let payload = take_stations(&mut app).pop().unwrap();
        let recipient = payload
            .ships
            .iter()
            .find(|ship| ship.ship_id == "ship-a")
            .unwrap();
        let other = payload
            .ships
            .iter()
            .find(|ship| ship.ship_id == "ship-b")
            .unwrap();
        assert_eq!(recipient.objectives.len(), 1);
        assert_eq!(recipient.objective_targets.len(), payload.entities.len());
        assert!(other.objectives.is_empty());
        assert!(other.objective_targets.is_empty());
        assert_eq!(
            app.world().resource::<crate::lobby::WorldResource>().0,
            metadata
        );
    }
    app.world_mut()
        .resource_mut::<crate::world::server::ObjectiveManagerRes>()
        .0
        .fail("private");
    app.world_mut().run_schedule(PostUpdate);
    assert!(take_stations(&mut app)
        .pop()
        .unwrap()
        .ships
        .iter()
        .all(|ship| ship.objective_targets.is_empty()));
}
