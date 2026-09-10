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
use crate::console_bridge::{GmEntityProjectionChanged, GmStationProjectionChanged};
use crate::core::messages::{
    EntitySnapshot, EntityStateSnapshot, ObjectiveSnapshot, ShipClientConfig, StationId,
    SystemBlackboard, SystemHullStatus, SystemId, WaypointSnapshot,
};
use crate::entities::config_cache::FactionRegistryResource;
use crate::entities::spawner::{
    AsteroidFieldSection, EntityId, EntityName, EntitySystemHull, EntityTagsSection, EntityUuid,
    FactionComponent, RadarAppearanceSection, RegionEffectsSection, RegionShapeSection,
    StaticPointDefence,
};
use crate::entities::tags::EntityTag;
use crate::gm_action::{GmActionKind, GmActionLog, LocalGmActionRefusals, LoggedGmAction};
use crate::gm_puppet::{StationPuppetActivityEntry, StationPuppetTarget, StationPuppets};
use crate::infrastructure::InfrastructureCondition;
use crate::lobby::WorldResource;
use crate::lockstep::FleetSlotOf;
use crate::regions::shape::RegionShape;
use crate::server_app::{AsteroidUuid, Ship};
use crate::ship::components::{
    ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources,
};
use crate::ship::state::ShipPhysics;
use crate::world::server::WorldContentRuntime;

/// Marks the explicit production rendererless browser GM peer.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct BrowserGameMaster;

/// Requests GM projections alongside a native ship without changing its boot identity.
#[derive(Resource, Default)]
pub struct NativeGmPresentation;

pub fn gm_presentation_active(
    browser: Option<Res<BrowserGameMaster>>,
    native: Option<Res<NativeGmPresentation>>,
) -> bool {
    browser.is_some() || native.is_some()
}

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
    /// A planet, moon or star: a landmark blip the crew radars already draw
    /// from its `[radar_appearance]`, which the GM map was silently dropping
    /// because nothing here classified it.
    Celestial,
}

/// Authoritative broad status carried by the local projection.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmEntityStatus {
    pub hull_percent: Option<u8>,
    pub condition_percent: Option<u8>,
    pub destroyed: bool,
    /// Absolute hull totals in milli-HP (issue #1310), present exactly when
    /// `hull_percent` is.
    ///
    /// A percentage cannot answer "would 40 points kill this", which is the one
    /// question a GM about to press Damage needs answered, and it cannot be
    /// converted into one without the maximum. Both are carried in the same
    /// unit the action itself uses so the page compares like with like rather
    /// than reconstructing hull points from a rounded percent.
    pub hull_current_milli_hp: Option<u32>,
    pub hull_max_milli_hp: Option<u32>,
    /// One row per damageable System the target's hull tracks, with the Station
    /// that authored it (issue #1311).
    ///
    /// The GM page needs this to offer a Station/System picker at all: the
    /// scope of a directed effect is a ship-local authoring key, and a browser
    /// that had to invent one would be composing an identity the simulation
    /// never published — the same rule the map selection already follows for
    /// the target itself. Carrying the live per-System totals with it is what
    /// lets the lethality and overflow preview answer the SCOPED question
    /// rather than restating the whole hull's.
    ///
    /// Empty for every entity with no hull, and for an entity whose Systems are
    /// authored under no Station (`station_id` is then `None` on each row, so a
    /// Station picker simply offers fewer options rather than lying about
    /// ownership).
    ///
    /// Omitted from the wire when empty, so the row for a beacon, a region or
    /// an asteroid field is byte-identical to its pre-#1311 shape and the
    /// per-frame cost of the breakdown falls only on the entities that have
    /// one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub systems: Vec<GmSystemHullStatus>,
}

/// One damageable System on a projected entity, with its authored owner.
///
/// Ordered by the HULL's own declaration order, which is the order the weighted
/// distribution consumes — not a map order, and not a name sort, so the picker
/// reads in the order the ship was authored in.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmSystemHullStatus {
    pub system_id: SystemId,
    /// The `[[station]]` that authored this System as its own, or `None` when
    /// the hull tracks a System its ship config does not assign (the courier's
    /// ownerless `core` bucket) or carries no ship config at all.
    pub station_id: Option<StationId>,
    /// That Station's authored display name: the exact value
    /// `GmStationInterfaceProjection::name` already publishes for the same
    /// Stations on the same ship, carried here so the two GM surfaces cannot
    /// disagree about what a Station is called. It passes through the browser's
    /// `wireText` seam like every other name on this projection, so an authored
    /// String Table id resolves and the shipped hulls' literal `[[station]]
    /// name` reads as authored.
    ///
    /// Without it the picker would have to render the raw `station_id` beside a
    /// localised System name, so one `<select>` would mix an authoring key with
    /// a translated label and the Station half alone would be untranslatable.
    ///
    /// `None` when the System has no owner, or when this entity carries no ship
    /// config to name one — omitted from the wire then, so a hull projected
    /// without a `ShipConfigComponent` keeps the row shape it already had.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station_name: Option<String>,
    /// The authored display name — a String Table id on every shipped hull,
    /// localised at render time exactly as the rest of this projection is.
    pub name: String,
    pub current_milli_hp: u32,
    pub max_milli_hp: u32,
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
    /// Apply-boundary policy preview; authoritative admission revalidates it.
    #[serde(default)]
    pub removable: bool,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub system_controls: BTreeMap<String, Vec<GmSystemControlStatus>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_results: Vec<LoggedGmAction>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub npc_doctrines: BTreeMap<String, crate::gm_npc::NpcDoctrineStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_doctrine_results: Vec<crate::gm_action::LoggedGmAction>,
    pub entities: Vec<GmEntityProjection>,
    /// Bounded attributed results of the directed world-effect family (issue
    /// #1310), carried on the entity surface rather than a channel of its own.
    ///
    /// The panel that submits these effects is the entity inspector: it selects
    /// its target from this exact payload, so the answer belongs beside the
    /// thing it was aimed at. `publish_local_projection` compares the whole
    /// payload, so a new result republishes it the same way a moved ship does.
    #[serde(default)]
    pub results: Vec<crate::gm_action::LoggedGmAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub despawn_results: Vec<crate::gm_action::LoggedGmAction>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub contact_overrides: crate::gm_contact::ContactOverrides,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contact_results: Vec<crate::gm_action::LoggedGmAction>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmSystemControlStatus {
    pub system_id: SystemId,
    pub name: String,
    pub gm_disabled: bool,
    pub available: bool,
}

/// One authored Station interface on a fleet/player ship. `console` is copied
/// from `StationConfig.console`; the GM shell mounts that exact URL instead of
/// cloning or approximating the interface.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmStationInterfaceProjection {
    pub station_id: StationId,
    pub name: String,
    pub console: String,
    pub rating: String,
    pub operators: Vec<String>,
}

/// Absolute ship pose at the rendererless-GM projection boundary.
///
/// Ordinary clients receive this same truth through the Helm blackboard.  It is
/// repeated explicitly here because a GM may open Helm before that aggregate
/// blackboard has ever been published; an authentic console must not silently
/// fall back to the origin in that interval.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GmShipPoseProjection {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub forward_speed: f32,
}

/// The local read model needed to drive an authentic Station iframe.
/// Blackboard values remain the exact tagged `SystemBlackboard` variants the
/// ordinary client receives, while topology is projected from the ship's
/// authored config rather than guessed from System ids.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GmPuppetShipProjection {
    pub ship_id: String,
    pub name: String,
    pub stations: Vec<GmStationInterfaceProjection>,
    /// The exact complete config ordinary players receive on `Welcome`, built
    /// by the same Rust projector. This is deliberately one wire object rather
    /// than a GM-maintained subset: authentic Station builders consume authored
    /// radar filters/ranges, arcs, tutorials, identity and assist gaps from it.
    pub ship_config: ShipClientConfig,
    pub station_ratings: BTreeMap<String, String>,
    pub control_sources: BTreeMap<SystemId, String>,
    pub blackboards: Vec<(SystemId, SystemBlackboard)>,
    /// Static/reconnect world registry from `WorldResource`. The browser folds
    /// `entity_states` over this through the ordinary `ClientSimState` reducer,
    /// exactly as `WorldSetup` followed by `SimState` does for a player.
    pub entities: Vec<EntitySnapshot>,
    /// Absolute (not delta-compressed) version of the ordinary `SimState`
    /// entity lane. Field meanings and shield derivation are identical; being
    /// absolute is what makes a newly opened GM iframe complete immediately.
    pub entity_states: Vec<EntityStateSnapshot>,
    /// Current mission objective snapshots from `ObjectiveManager`.
    pub objectives: Vec<ObjectiveSnapshot>,
    /// Explicit current pose from the selected fleet ship's `ShipPhysics`.
    pub ship_pose: GmShipPoseProjection,
    /// The ship-owned Navigation goal, using the ordinary wire shape.
    pub navigation_waypoint: Option<WaypointSnapshot>,
    /// Full authoritative hull rows. The GM is not a player-recipient, so this
    /// local projection is intentionally not passed through station privacy.
    pub console_hull: Vec<SystemHullStatus>,
}

/// Absolute rendererless-GM Station projection. Activity is emitted from the
/// attribution sidecar written at GM admission; downstream System consumers
/// continue to see only source-stripped commands.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GmStationProjectionPayload {
    pub ships: Vec<GmPuppetShipProjection>,
    pub activity: Vec<StationPuppetActivityEntry>,
    /// Canonical terminal results for authentic Station commands. Correlation
    /// remains the exact opaque identity minted by the originating iframe, so
    /// the shell can settle that iframe's ordinary feedback lifecycle.
    pub results: Vec<LoggedGmAction>,
}

pub struct GmProjectionPlugin;

impl Plugin for GmProjectionPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<NativeGmPresentation>(
            StateClass::Presentation,
            "native-local-gm-workspace",
        )
        .init_resource::<StationPuppets>()
        .init_resource::<crate::gm_puppet::StationPuppetActivity>()
        .init_resource::<GmActionLog>()
        .init_resource::<LocalGmActionRefusals>()
        .add_message::<GmEntityProjectionChanged>()
        .add_message::<GmStationProjectionChanged>()
        .add_systems(
            FixedLast,
            (publish_local_projection, publish_station_projection).run_if(gm_presentation_active),
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
        // The Station->System ownership map behind the scoped direct-effect
        // picker (issue #1311). `Option` because a `StaticPointDefence`
        // structure reaches this query too and need not be a stationed hull.
        Option<&'static ShipConfigComponent>,
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

/// The per-System breakdown a scoped direct effect is picked from (#1311).
///
/// The Station comes from the ship config rather than from the hull, because
/// ownership is authored in `[[system]]` and the hull only says what can be
/// damaged. A System the hull tracks but the config never assigned is projected
/// with no owner rather than dropped — the courier's `core` bucket is exactly
/// that, and it is a legitimate System-scope target.
fn system_statuses(
    hull: &EntitySystemHull,
    config: Option<&ShipConfigComponent>,
) -> Vec<GmSystemHullStatus> {
    hull.0
        .iter()
        .map(|(system_id, entry)| {
            let station_id = config
                .and_then(|config| config.0.system(system_id))
                .and_then(|system| system.station.clone());
            // The owner's display name comes from the same `[[station]]` block
            // `GmStationInterfaceProjection` reads, so the two GM surfaces
            // cannot disagree about what a Station is called.
            let station_name = station_id.as_ref().and_then(|station_id| {
                config
                    .and_then(|config| config.0.station(station_id))
                    .map(|station| station.name.clone())
            });
            GmSystemHullStatus {
                system_id: system_id.clone(),
                station_id,
                station_name,
                name: entry.display_name.clone(),
                current_milli_hp: crate::gm_effect::hp_to_milli(entry.current),
                max_milli_hp: crate::gm_effect::hp_to_milli(entry.max),
            }
        })
        .collect()
}

fn broad_status(
    hull: Option<&EntitySystemHull>,
    infrastructure: Option<&InfrastructureCondition>,
    config: Option<&ShipConfigComponent>,
) -> GmEntityStatus {
    let hull_percent = hull.map(|hull| percent(hull.0.total_current(), hull.0.total_max()));
    let condition_percent = infrastructure
        .map(|condition| percent(condition.0.condition(), condition.0.condition_max()));
    GmEntityStatus {
        hull_percent,
        condition_percent,
        destroyed: hull.is_some_and(|hull| hull.0.total_current() <= 0.0),
        hull_current_milli_hp: hull
            .map(|hull| crate::gm_effect::hp_to_milli(hull.0.total_current())),
        hull_max_milli_hp: hull.map(|hull| crate::gm_effect::hp_to_milli(hull.0.total_max())),
        systems: hull.map_or_else(Vec::new, |hull| system_statuses(hull, config)),
    }
}

fn hull_status(hull: &EntitySystemHull, config: Option<&ShipConfigComponent>) -> GmEntityStatus {
    let total_max = hull.0.total_max();
    let total_current = hull.0.total_current();
    GmEntityStatus {
        hull_percent: Some(percent(total_current, total_max)),
        condition_percent: None,
        destroyed: total_current <= 0.0,
        hull_current_milli_hp: Some(crate::gm_effect::hp_to_milli(total_current)),
        hull_max_milli_hp: Some(crate::gm_effect::hp_to_milli(total_max)),
        systems: system_statuses(hull, config),
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
    } else if has_tag(tags, EntityTag::Planet)
        || has_tag(tags, EntityTag::Moon)
        || has_tag(tags, EntityTag::Star)
    {
        Some(GmEntityKind::Celestial)
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
    system_sources: Query<
        (
            &EntityUuid,
            &ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
        ),
        With<Ship>,
    >,
    npc_control: crate::gm_npc::NpcDoctrineControl,
    ships: GmShipProjectionQuery,
    world_entities: GmWorldProjectionQuery,
    all_names: Query<(&EntityUuid, Option<&EntityName>, Option<&EntityId>)>,
    factions: Option<Res<FactionRegistryResource>>,
    world_content: Option<Res<WorldContentRuntime>>,
    world_setup: Option<Res<WorldResource>>,
    action_log: Res<GmActionLog>,
    local_refusals: Res<LocalGmActionRefusals>,
    mut previous: Local<Option<GmEntityProjectionPayload>>,
    mut changed: MessageWriter<GmEntityProjectionChanged>,
    removal_targets: crate::gm_despawn::RemovalQuery,
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
                ship_config,
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
                    removable: crate::gm_despawn::validate_target(&removal_targets, &uuid.0)
                        .is_ok(),
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
                        broad_status(Some(hull), infrastructure, ship_config)
                    } else {
                        hull_status(hull, ship_config)
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
                removable: crate::gm_despawn::validate_target(&removal_targets, &uuid.0).is_ok(),
                entity_id: uuid.0.clone(),
                name: display_name(uuid, name, id, &authored_names),
                kind,
                position,
                faction: faction_reference(faction, factions.as_deref()),
                // A non-`Ship` world entity carries no `ShipConfigComponent`
                // at all, so its Systems project with no Station owner — which
                // is the honest answer for a beacon, a planet or an authored
                // asteroid, and leaves System scope available on it.
                status: broad_status(hull, infrastructure, None),
                current_target: None,
                geometry,
                radar: radar_appearance(radar),
            })
        },
    ));

    projected.sort_by(|left, right| left.entity_id.cmp(&right.entity_id));
    projected.dedup_by(|left, right| left.entity_id == right.entity_id);

    let mut contact_results: Vec<_> = [
        crate::gm_action::GmActionKind::ContactReveal,
        crate::gm_action::GmActionKind::ContactConceal,
        crate::gm_action::GmActionKind::ContactNormal,
    ]
    .into_iter()
    .flat_map(|kind| crate::gm_action::projected_results(kind, &action_log, &local_refusals))
    .collect();
    contact_results.sort_by(|a, b| {
        (a.tick, a.order, &a.operator_id, a.correlation.as_str()).cmp(&(
            b.tick,
            b.order,
            &b.operator_id,
            b.correlation.as_str(),
        ))
    });
    let next = GmEntityProjectionPayload {
        system_controls: system_sources
            .iter()
            .map(|(uuid, config, sources)| {
                (
                    uuid.0.clone(),
                    config
                        .0
                        .systems
                        .iter()
                        .map(|system| GmSystemControlStatus {
                            system_id: system.id.clone(),
                            name: projected
                                .iter()
                                .find(|row| row.entity_id == uuid.0)
                                .and_then(|row| {
                                    row.status
                                        .systems
                                        .iter()
                                        .find(|row| row.system_id == system.id)
                                })
                                .map(|row| row.name.clone())
                                .unwrap_or_else(|| system.id.0.clone()),
                            gm_disabled: sources.0.is_gm_disabled(&system.id),
                            available: sources.0.policy_for(&system.id).coordinate,
                        })
                        .collect(),
                )
            })
            .collect(),
        system_results: [
            crate::gm_action::GmActionKind::SystemDisable,
            crate::gm_action::GmActionKind::SystemRestore,
        ]
        .into_iter()
        .flat_map(|kind| crate::gm_action::projected_results(kind, &action_log, &local_refusals))
        .collect(),
        contact_overrides: world_content
            .as_ref()
            .map(|runtime| runtime.contact_overrides.clone())
            .unwrap_or_default(),
        contact_results,
        npc_doctrines: world_content
            .as_deref()
            .map(|runtime| npc_control.projection(runtime))
            .unwrap_or_default(),
        npc_doctrine_results: crate::gm_action::projected_results(
            crate::gm_action::GmActionKind::NpcDoctrine,
            &action_log,
            &local_refusals,
        ),
        despawn_results: crate::gm_action::projected_results(
            crate::gm_action::GmActionKind::WorldDespawn,
            &action_log,
            &local_refusals,
        ),
        entities: projected,
        results: crate::gm_action::projected_results(
            crate::gm_action::GmActionKind::DirectEffect,
            &action_log,
            &local_refusals,
        ),
    };
    if previous.as_ref() != Some(&next) {
        changed.write(GmEntityProjectionChanged {
            payload: next.clone(),
        });
        *previous = Some(next);
    }
}

fn publish_station_projection(
    capabilities: crate::gm_puppet::capability::StationCapabilities,
    puppets: Res<StationPuppets>,
    activity: Res<crate::gm_puppet::StationPuppetActivity>,
    action_log: Res<GmActionLog>,
    local_refusals: Res<LocalGmActionRefusals>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    selected_ship: Option<Res<crate::lobby::SelectedShipResource>>,
    world_data: Option<Res<crate::lobby::server::WorldResource>>,
    objectives: Option<Res<crate::world::server::ObjectiveManagerRes>>,
    live_entities: Query<
        (
            Option<&EntityUuid>,
            Option<&AsteroidUuid>,
            Option<&Transform>,
            Option<&EntitySystemHull>,
            Option<&crate::ship::shields::ShipShields>,
        ),
        Or<(With<EntityUuid>, With<AsteroidUuid>)>,
    >,
    ships: Query<
        (
            &EntityUuid,
            Option<&EntityName>,
            Option<&crate::lockstep::FleetSlotOf>,
            Option<&crate::gm_puppet::capability::NpcStationConfig>,
            &ShipConfigComponent,
            &ActiveStationRatings,
            &ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            Option<&ShipPhysics>,
            Option<&crate::console::navigation::server::NavigationWaypoint>,
            Option<&EntitySystemHull>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut previous: Local<Option<GmStationProjectionPayload>>,
    mut changed: MessageWriter<GmStationProjectionChanged>,
) {
    // This is the absolute counterpart of `build_sim_state_entity_states`:
    // same wire fields, same authoritative ECS components, but no broadcaster
    // caches and therefore no delta suppression. The GM browser then feeds it
    // through the ordinary `ClientSimState` reducer over `WorldResource`, so
    // there is one raw/local projection boundary rather than a GM-only radar
    // model.
    let mut entity_states = live_entities
        .iter()
        .filter_map(|(uuid, asteroid_uuid, transform, hull, shields)| {
            let uuid = uuid
                .map(|uuid| uuid.0.clone())
                .or_else(|| asteroid_uuid.map(|uuid| uuid.0.clone()))?;
            let hull_fraction = crate::server_app::project_entity_hull_fraction(hull);
            let (shield_fraction, shields_wire, shield_freq) =
                crate::server_app::project_entity_shield_state(shields);
            let (position, yaw) = if asteroid_uuid.is_some() {
                // Asteroid transforms are immutable and already live in the
                // `WorldResource` entry, matching ordinary SimState omission.
                (None, None)
            } else {
                transform.map_or((None, None), |transform| {
                    (
                        Some([
                            transform.translation.x,
                            transform.translation.y,
                            transform.translation.z,
                        ]),
                        Some(transform.rotation.to_euler(EulerRot::YXZ).0),
                    )
                })
            };
            Some(EntityStateSnapshot {
                uuid,
                position,
                yaw,
                hull_fraction,
                shield_fraction,
                flags: Vec::new(),
                shields: shields_wire,
                shield_freq,
                warp_out_remaining_secs: None,
            })
        })
        .collect::<Vec<_>>();
    entity_states.sort_by(|left, right| left.uuid.cmp(&right.uuid));
    let entities = world_data
        .as_ref()
        .map(|world| world.0.entities.clone())
        .unwrap_or_default();

    let mut projected_ships = ships
        .iter()
        .map(
            |(
                uuid,
                name,
                slot,
                npc_config,
                config,
                ratings,
                sources,
                blackboards,
                physics,
                waypoint,
                hull,
            )| {
                let config_path = roster
                    .as_ref()
                    .and_then(|roster| {
                        roster
                            .ships()
                            .iter()
                            .find(|ship| slot.is_some_and(|slot| ship.host == slot.0))
                            .and_then(|ship| ship.ship_path.as_deref())
                    })
                    .or_else(|| selected_ship.as_ref().map(|selected| selected.0.as_str()));
                let config_cache = crate::entities::config_cache::get_config_cache();
                let ship_client_config = if slot.is_some() {
                    config_path
                        .and_then(|path| config_cache.get(path))
                        .map(crate::lobby::server::project_ship_client_config)
                        .unwrap_or_default()
                } else {
                    npc_config
                        .map(|config| config.0.clone())
                        .unwrap_or_default()
                };
                let station_ratings = config
                    .0
                    .stations
                    .iter()
                    .map(|station| {
                        (
                            station.id.0.clone(),
                            ratings.0.get(&station.id).cloned().unwrap_or_default(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let stations = config
                    .0
                    .stations
                    .iter()
                    .filter_map(|station| {
                        capabilities.check(&uuid.0, &station.id).ok()?;
                        let console = station
                            .console
                            .as_deref()
                            .filter(|console| !console.is_empty())?;
                        let target = StationPuppetTarget::new(
                            crate::command_admission::log::ShipKey(uuid.0.clone()),
                            station.id.clone(),
                        );
                        Some(GmStationInterfaceProjection {
                            station_id: station.id.clone(),
                            name: station.name.clone(),
                            console: console.to_string(),
                            rating: ratings.0.get(&station.id).cloned().unwrap_or_default(),
                            operators: puppets.operators(&target).to_vec(),
                        })
                    })
                    .collect();
                let control_sources = sources
                    .0
                    .entries()
                    .map(|(system, source)| {
                        let source = if sources.0.is_offline(system) {
                            crate::ship::control_source::ControlSource::Offline
                        } else {
                            *source
                        };
                        let label = match source {
                            crate::ship::control_source::ControlSource::Human => "Human",
                            crate::ship::control_source::ControlSource::Ai => "Ai",
                            crate::ship::control_source::ControlSource::Offline => "Offline",
                        };
                        (system.clone(), label.to_string())
                    })
                    .collect();
                let mut blackboards = blackboards
                    .0
                    .iter()
                    .map(|(system, value)| (system.clone(), value.clone()))
                    .collect::<Vec<_>>();
                blackboards.sort_by(|left, right| left.0.cmp(&right.0));
                let console_hull = hull
                    .map(|hull| {
                        hull.0
                            .iter()
                            .map(|(system_id, entry)| SystemHullStatus {
                                system_id: system_id.clone(),
                                display_name: entry.display_name.clone(),
                                current: entry.current,
                                max_hp: entry.max,
                                tier: hull.0.tier_for(system_id),
                                debuff_magnitude: hull.0.debuff_magnitude_for(system_id),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let ship_pose = physics.map_or_else(GmShipPoseProjection::default, |physics| {
                    GmShipPoseProjection {
                        x: physics.x,
                        y: physics.y,
                        z: physics.z,
                        yaw: physics.yaw,
                        forward_speed: physics.forward_speed,
                    }
                });

                let scoped_objectives = objectives
                    .as_ref()
                    .map(|manager| manager.0.snapshots_for(&uuid.0))
                    .unwrap_or_default();
                GmPuppetShipProjection {
                    ship_id: uuid.0.clone(),
                    name: name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
                    stations,
                    ship_config: ship_client_config,
                    station_ratings,
                    control_sources,
                    blackboards,
                    entities: crate::objectives::project_entity_targets(
                        &entities,
                        &scoped_objectives,
                    ),
                    entity_states: entity_states.clone(),
                    objectives: scoped_objectives,
                    ship_pose,
                    navigation_waypoint: waypoint.and_then(|waypoint| waypoint.snapshot()),
                    console_hull,
                }
            },
        )
        .filter(|ship| !ship.stations.is_empty())
        .collect::<Vec<_>>();
    projected_ships.sort_by(|left, right| left.ship_id.cmp(&right.ship_id));

    let next = GmStationProjectionPayload {
        ships: projected_ships,
        activity: activity.entries().to_vec(),
        results: crate::gm_action::projected_results(
            GmActionKind::StationCommand,
            &action_log,
            &local_refusals,
        ),
    };
    if previous.as_ref() != Some(&next) {
        changed.write(GmStationProjectionChanged {
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

        app.world_mut().run_schedule(FixedLast);
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
            ShipConfigComponent::default(),
            ActiveStationRatings::default(),
            ShipSystemControlSources::default(),
            crate::server_app::ShipSystemBlackboards::default(),
        ));

        app.world_mut().run_schedule(FixedLast);
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

        app.world_mut().run_schedule(FixedLast);
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
        assert_eq!(ship.entities[0].position, Some([1.0, 0.0, 2.0]));
        let live_contact = ship
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
            app.world_mut().run_schedule(FixedLast);
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
            assert!(recipient
                .entities
                .iter()
                .all(|entity| entity.objective_target));
            assert!(other.objectives.is_empty());
            assert!(other.entities.iter().all(|entity| !entity.objective_target));
            assert_eq!(
                app.world().resource::<crate::lobby::WorldResource>().0,
                metadata
            );
        }
        app.world_mut()
            .resource_mut::<crate::world::server::ObjectiveManagerRes>()
            .0
            .fail("private");
        app.world_mut().run_schedule(FixedLast);
        assert!(take_stations(&mut app)
            .pop()
            .unwrap()
            .ships
            .iter()
            .all(|ship| ship.entities.iter().all(|entity| !entity.objective_target)));
    }
}
