//! Peer-local Game Master map projection (issues #1291, #1295 and #1296).
//!
//! This is a deliberately narrow read model over the authoritative ECS. It is
//! emitted only through the browser Host Channel; it is not a `ServerMessage`,
//! lockstep frame, or simulation outbox entry. Every browser GM builds it from
//! the deterministic world already running in that peer.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::console::weapons::TacticalRadarSelection;
use crate::console_bridge::GmEntityProjectionChanged;
use crate::entities::config_cache::FactionRegistryResource;
use crate::entities::spawner::{
    AsteroidFieldSection, EntityId, EntityName, EntitySystemHull, EntityTagsSection, EntityUuid,
    FactionComponent, RadarAppearanceSection, RegionEffectsSection, RegionShapeSection,
    StaticPointDefence,
};
use crate::entities::tags::EntityTag;
use crate::infrastructure::InfrastructureCondition;
use crate::lobby::WorldResource;
use crate::lockstep::FleetSlotOf;
use crate::regions::shape::RegionShape;
use crate::server_app::Ship;
use crate::ship::state::ShipPhysics;
use crate::world::server::WorldContentRuntime;

/// Marks the explicit production rendererless browser GM peer.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct BrowserGameMaster;

/// Semantic map classification projected without exposing ECS marker names.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GmEntityKind {
    PlayerShip,
    NpcShip,
    Structure,
    Hazard,
    Region,
    AsteroidField,
    AuthoredAsteroid,
}

/// Authoritative broad status carried by the local projection.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmEntityStatus {
    pub hull_percent: Option<u8>,
    pub condition_percent: Option<u8>,
    pub destroyed: bool,
}

/// Authored radar presentation, narrowed to the public map vocabulary.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GmRadarAppearance {
    pub icon: Option<String>,
    pub colour: Option<[f32; 3]>,
    pub size: Option<f32>,
    pub region_colour: Option<[f32; 3]>,
}

/// Stable reference used for faction and current-target links.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmEntityReference {
    pub entity_id: String,
    pub name: String,
}

/// Stable identity plus the small M1 map/inspector surface for one world object.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GmEntityProjection {
    pub entity_id: String,
    pub name: String,
    pub kind: GmEntityKind,
    pub position: [f32; 3],
    pub faction: Option<GmEntityReference>,
    pub status: GmEntityStatus,
    pub current_target: Option<GmEntityReference>,
    pub geometry: Option<RegionShape>,
    pub radar: GmRadarAppearance,
}

/// Absolute local map projection. An empty list explicitly clears stale page
/// state when every selectable ship has left the world.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GmEntityProjectionPayload {
    pub entities: Vec<GmEntityProjection>,
}

pub struct GmProjectionPlugin;

impl Plugin for GmProjectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<GmEntityProjectionChanged>().add_systems(
            FixedLast,
            publish_local_projection.run_if(resource_exists::<BrowserGameMaster>),
        );
    }
}

type GmShipProjectionQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        Option<&'static EntityName>,
        &'static EntitySystemHull,
        &'static ShipPhysics,
        Option<&'static FactionComponent>,
        Option<&'static FleetSlotOf>,
        Option<&'static TacticalRadarSelection>,
        Option<&'static StaticPointDefence>,
        Option<&'static InfrastructureCondition>,
        Option<&'static RadarAppearanceSection>,
    ),
    With<Ship>,
>;

type GmWorldProjectionQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        Option<&'static EntityName>,
        Option<&'static EntityId>,
        &'static Transform,
        Option<&'static EntityTagsSection>,
        Option<&'static EntitySystemHull>,
        Option<&'static FactionComponent>,
        Option<&'static RegionShapeSection>,
        Option<&'static RegionEffectsSection>,
        Option<&'static AsteroidFieldSection>,
        Option<&'static InfrastructureCondition>,
        Option<&'static RadarAppearanceSection>,
    ),
    Without<Ship>,
>;

fn percent(current: f32, maximum: f32) -> u8 {
    if maximum > 0.0 {
        ((current / maximum) * 100.0).clamp(0.0, 100.0).round() as u8
    } else {
        0
    }
}

fn broad_status(
    hull: Option<&EntitySystemHull>,
    infrastructure: Option<&InfrastructureCondition>,
) -> GmEntityStatus {
    let hull_percent = hull.map(|hull| percent(hull.0.total_current(), hull.0.total_max()));
    let condition_percent = infrastructure
        .map(|condition| percent(condition.0.condition(), condition.0.condition_max()));
    GmEntityStatus {
        hull_percent,
        condition_percent,
        destroyed: hull.is_some_and(|hull| hull.0.total_current() <= 0.0),
    }
}

fn hull_status(hull: &EntitySystemHull) -> GmEntityStatus {
    let total_max = hull.0.total_max();
    let total_current = hull.0.total_current();
    GmEntityStatus {
        hull_percent: Some(percent(total_current, total_max)),
        condition_percent: None,
        destroyed: total_current <= 0.0,
    }
}

fn rgb(value: Option<&Vec<f32>>) -> Option<[f32; 3]> {
    let value = value?;
    (value.len() == 3).then(|| [value[0], value[1], value[2]])
}

fn radar_appearance(radar: Option<&RadarAppearanceSection>) -> GmRadarAppearance {
    radar.map_or_else(GmRadarAppearance::default, |radar| GmRadarAppearance {
        icon: radar.0.icon.clone(),
        colour: rgb(radar.0.colour.as_ref()),
        size: radar.0.size,
        region_colour: rgb(radar.0.region_colour.as_ref()),
    })
}

fn faction_reference(
    faction: Option<&FactionComponent>,
    factions: Option<&FactionRegistryResource>,
) -> Option<GmEntityReference> {
    faction.map(|faction| {
        let name = factions
            .and_then(|registry| registry.get(&faction.0))
            .and_then(|config| config.display_name.clone())
            .unwrap_or_else(|| faction.0.to_string());
        GmEntityReference {
            entity_id: faction.0.to_string(),
            name,
        }
    })
}

fn has_tag(tags: Option<&EntityTagsSection>, tag: EntityTag) -> bool {
    tags.is_some_and(|tags| tags.0.iter().any(|candidate| candidate == tag.as_str()))
}

fn world_kind(
    tags: Option<&EntityTagsSection>,
    shape: Option<&RegionShapeSection>,
    effects: Option<&RegionEffectsSection>,
    field: Option<&AsteroidFieldSection>,
    infrastructure: Option<&InfrastructureCondition>,
    named_asteroid: bool,
) -> Option<GmEntityKind> {
    if field.is_some() {
        Some(GmEntityKind::AsteroidField)
    } else if shape.is_some() && effects.is_some_and(|effects| !effects.0.is_empty()) {
        Some(GmEntityKind::Hazard)
    } else if shape.is_some() && has_tag(tags, EntityTag::Region) {
        Some(GmEntityKind::Region)
    } else if infrastructure.is_some()
        || has_tag(tags, EntityTag::Structure)
        || has_tag(tags, EntityTag::Station)
    {
        Some(GmEntityKind::Structure)
    } else if named_asteroid && has_tag(tags, EntityTag::Asteroid) {
        Some(GmEntityKind::AuthoredAsteroid)
    } else {
        None
    }
}

fn display_name(
    uuid: &EntityUuid,
    name: Option<&EntityName>,
    id: Option<&EntityId>,
    authored_names: &BTreeMap<&str, &str>,
) -> String {
    authored_names
        .get(uuid.0.as_str())
        .map(|name| (*name).to_owned())
        .or_else(|| name.map(|name| name.0.clone()))
        .or_else(|| id.map(|id| id.0.clone()))
        .unwrap_or_else(|| uuid.0.clone())
}

fn publish_local_projection(
    ships: GmShipProjectionQuery,
    world_entities: GmWorldProjectionQuery,
    all_names: Query<(&EntityUuid, Option<&EntityName>, Option<&EntityId>)>,
    factions: Option<Res<FactionRegistryResource>>,
    world_content: Option<Res<WorldContentRuntime>>,
    world_setup: Option<Res<WorldResource>>,
    mut previous: Local<Option<GmEntityProjectionPayload>>,
    mut changed: MessageWriter<GmEntityProjectionChanged>,
) {
    // Resolve names in a separate deterministic lookup so target links never
    // leak a Bevy `Entity` and remain useful after a projection refresh.
    let authored_names: BTreeMap<&str, &str> = world_setup
        .as_deref()
        .map(|world| {
            world
                .0
                .entities
                .iter()
                .filter_map(|entity| Some((entity.uuid.as_str(), entity.name.as_deref()?)))
                .collect()
        })
        .unwrap_or_default();
    let names: BTreeMap<String, String> = all_names
        .iter()
        .map(|(uuid, name, id)| {
            (
                uuid.0.clone(),
                display_name(uuid, name, id, &authored_names),
            )
        })
        .collect();
    let named_uuids: std::collections::BTreeSet<&str> = world_content
        .as_deref()
        .map(|world| world.name_to_uuid.values().map(String::as_str).collect())
        .unwrap_or_default();

    let mut projected: Vec<GmEntityProjection> = ships
        .iter()
        .map(
            |(
                uuid,
                name,
                hull,
                physics,
                faction,
                fleet_slot,
                target,
                point_defence,
                infrastructure,
                radar,
            )| {
                let current_target =
                    target
                        .and_then(|selection| selection.0.as_ref())
                        .map(|target_id| GmEntityReference {
                            entity_id: target_id.clone(),
                            name: names
                                .get(target_id)
                                .cloned()
                                .unwrap_or_else(|| target_id.clone()),
                        });
                GmEntityProjection {
                    entity_id: uuid.0.clone(),
                    name: display_name(uuid, name, None, &authored_names),
                    kind: if point_defence.is_some() {
                        GmEntityKind::Structure
                    } else if fleet_slot.is_some() {
                        GmEntityKind::PlayerShip
                    } else {
                        GmEntityKind::NpcShip
                    },
                    position: [physics.x, physics.y, physics.z],
                    faction: faction_reference(faction, factions.as_deref()),
                    status: if infrastructure.is_some() {
                        broad_status(Some(hull), infrastructure)
                    } else {
                        hull_status(hull)
                    },
                    current_target,
                    geometry: None,
                    radar: radar_appearance(radar),
                }
            },
        )
        .collect();

    projected.extend(world_entities.iter().filter_map(
        |(
            uuid,
            name,
            id,
            transform,
            tags,
            hull,
            faction,
            shape,
            effects,
            field,
            infrastructure,
            radar,
        )| {
            let kind = world_kind(
                tags,
                shape,
                effects,
                field,
                infrastructure,
                named_uuids.contains(uuid.0.as_str()),
            )?;
            let (position, geometry) = if let Some(field) = field {
                (
                    field.0.anchor_offset,
                    Some(RegionShape::Torus {
                        inner_radius: field.0.inner_radius,
                        outer_radius: field.0.outer_radius,
                    }),
                )
            } else {
                (
                    transform.translation.to_array(),
                    shape.map(|shape| shape.0.clone()),
                )
            };
            Some(GmEntityProjection {
                entity_id: uuid.0.clone(),
                name: display_name(uuid, name, id, &authored_names),
                kind,
                position,
                faction: faction_reference(faction, factions.as_deref()),
                status: broad_status(hull, infrastructure),
                current_target: None,
                geometry,
                radar: radar_appearance(radar),
            })
        },
    ));

    projected.sort_by(|left, right| left.entity_id.cmp(&right.entity_id));
    projected.dedup_by(|left, right| left.entity_id == right.entity_id);

    let next = GmEntityProjectionPayload {
        entities: projected,
    };
    if previous.as_ref() != Some(&next) {
        changed.write(GmEntityProjectionChanged {
            payload: next.clone(),
        });
        *previous = Some(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

        app.world_mut().run_schedule(FixedLast);
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

        app.world_mut().run_schedule(FixedLast);
        assert_eq!(take(&mut app)[0].entities.len(), 1);
        app.world_mut().run_schedule(FixedLast);
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
        app.world_mut().run_schedule(FixedLast);
        let updated = take(&mut app).pop().unwrap();
        assert_eq!(updated.entities[0].position[0], 5.0);
        assert_eq!(
            updated.entities[0].current_target.as_ref().unwrap().name,
            "removed-target",
            "a stale target remains a stable link without exposing ECS state"
        );

        app.world_mut().despawn(npc);
        app.world_mut().run_schedule(FixedLast);
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

        app.world_mut().run_schedule(FixedLast);
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

        app.world_mut().run_schedule(FixedLast);
        let payload = take(&mut app).pop().expect("absolute structure projection");
        assert_eq!(payload.entities.len(), 1);
        assert_eq!(payload.entities[0].entity_id, STRUCTURE_ID);
        assert_eq!(payload.entities[0].kind, GmEntityKind::Structure);
        assert_eq!(payload.entities[0].position, [30.0, 2.0, -12.0]);
        assert_eq!(payload.entities[0].status.hull_percent, None);
        assert_eq!(payload.entities[0].status.condition_percent, None);
        assert_eq!(payload.entities[0].radar.icon.as_deref(), Some("ship"));
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
        app.world_mut().spawn((
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
        ));
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

        app.world_mut().run_schedule(FixedLast);
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

        app.world_mut().run_schedule(FixedLast);
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

        app.world_mut().run_schedule(FixedLast);
        let initial = take(&mut app).pop().unwrap();
        assert_eq!(initial.entities.len(), 2, "duplicate UUIDs collapse");
        assert_eq!(initial.entities[0].entity_id, FIRST_ID);
        assert_eq!(initial.entities[1].entity_id, SECOND_ID);
        app.world_mut().run_schedule(FixedLast);
        assert!(take(&mut app).is_empty(), "unchanged state emits nothing");

        app.world_mut().despawn(second);
        app.world_mut().run_schedule(FixedLast);
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
        app.world_mut().run_schedule(FixedLast);
        let reappeared = take(&mut app).pop().unwrap();
        assert_eq!(reappeared.entities.len(), 1);
        assert_eq!(reappeared.entities[0].entity_id, FIRST_ID);
        assert_eq!(reappeared.entities[0].position, [77.0, 0.0, -9.0]);
    }
}
