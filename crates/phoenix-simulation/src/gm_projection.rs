//! Peer-local Game Master map projection (issues #1291, #1295 and #1296).
//!
//! This is a deliberately narrow read model over the authoritative ECS. It is
//! emitted only through the browser Host Channel; it is not a `ServerMessage`,
//! lockstep frame, or simulation outbox entry. Every browser GM builds it from
//! the deterministic world already running in that peer.

use std::collections::BTreeMap;

use bevy::{ecs::system::SystemParam, prelude::*};
use serde::{Deserialize, Serialize};

use crate::console::weapons::TacticalRadarSelection;
use crate::console_bridge::{GmEntityProjectionChanged, GmStationProjectionChanged};
use crate::core::messages::{
    EntitySnapshot, EntityStateSnapshot, ObjectiveSnapshot, ShipClientConfig, StationId,
    SystemBlackboard, SystemHullStatus, SystemId, WaypointSnapshot,
};
use crate::entities::config_cache::FactionRegistryResource;
use crate::entities::spawner::{
    AsteroidFieldSection, EntityId, EntityName, EntitySystemHull, EntityTagsSection,
    EntityTemplatePath, EntityUuid, FactionComponent, RadarAppearanceSection, RegionEffectsSection,
    RegionShapeSection, StaticPointDefence,
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

#[derive(SystemParam)]
struct GmWorldInspectorSources<'w> {
    recipient_diagnostics: Option<Res<'w, crate::recipients::RecipientDiagnostics>>,
    runtime: Option<Res<'w, WorldContentRuntime>>,
    config: Option<Res<'w, crate::world::config::WorldConfig>>,
    objectives: Option<Res<'w, crate::world::server::ObjectiveManagerRes>>,
    layers: Option<Res<'w, crate::world::server::WorldLayerMap>>,
    paused: Option<Res<'w, crate::gm_action::SimulationPaused>>,
    interest: Option<Res<'w, GmInspectorInterest>>,
}

/// Local presentation demand, never participant input or command authority.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum GmInspectorKind {
    EntityFields,
    HullFields,
    RegionFields,
    PresentationFields,
    WorldFields,
}

#[derive(Resource, Default)]
pub struct GmInspectorInterest(pub Option<std::collections::BTreeSet<GmInspectorKind>>);

impl GmInspectorInterest {
    fn wants(&self, panel: GmInspectorKind) -> bool {
        // An unmounted adapter retains the full projection contract. Once the
        // desk declares its visible tools, closed inspectors do no schema work.
        self.0.as_ref().is_none_or(|panels| panels.contains(&panel))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum GmConsoleConsumer {
    Console,
    Comparison,
}

/// Private presentation demand. Station ownership is deliberately absent.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GmConsoleInterest {
    pub consumer: GmConsoleConsumer,
    pub ship: String,
    pub station: String,
    pub visible: bool,
    pub mount_generation: u64,
    pub world_generation: u64,
}

impl GmConsoleInterest {
    pub fn valid(&self) -> bool {
        self.ship.len() <= 128
            && self.station.len() <= 64
            && (!self.visible
                || (!self.ship.is_empty()
                    && (self.consumer == GmConsoleConsumer::Comparison
                        || !self.station.is_empty())))
    }
}

#[derive(Resource, Default)]
pub struct GmConsoleSubscriptions {
    pub requests: BTreeMap<GmConsoleConsumer, GmConsoleInterest>,
    pub generation: u64,
    lifecycle: Option<u64>,
}

impl GmConsoleSubscriptions {
    fn wants(&self, ship: &str) -> bool {
        self.requests.is_empty()
            || self.requests.values().any(|request| {
                request.visible
                    && request.world_generation == self.generation
                    && request.ship == ship
            })
    }
}

fn prepare_presentation_generation(
    mut subscriptions: ResMut<GmConsoleSubscriptions>,
    config: Option<Res<crate::world::config::WorldConfig>>,
    lifecycle: Option<Res<crate::audio_lifecycle::RoomAudioLifecycle>>,
) {
    let generation = lifecycle.map(|owner| owner.state.generation);
    if config.is_some_and(|config| config.is_changed()) || generation != subscriptions.lifecycle {
        subscriptions.generation = subscriptions.generation.wrapping_add(1);
        subscriptions.lifecycle = generation;
    }
}

#[derive(SystemParam)]
struct GmStationPresentation<'w> {
    world_data: Option<Res<'w, crate::lobby::server::WorldResource>>,
    world_config: Option<Res<'w, crate::world::config::WorldConfig>>,
    subscriptions: Res<'w, GmConsoleSubscriptions>,
}

/// Marks an explicit production GM peer. Browser and native peers share it.
///
/// Rendererless always; shipless only sometimes. A GM that JOINED a fleet owns
/// no local ship — it selected no hull and looks at somebody else's session. A
/// GM booted STANDALONE from the landing's Host as GM route is the session: it
/// picked a World and a hull on the way in, so it owns a `LocalShip` like any
/// host, with every station on AI backfill and no viewscreen drawn for it.
/// Anything reading this marker to mean "there is no local ship" is reading it
/// wrong; ask for the ship.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct GameMasterPeer;

/// Compatibility name retained for browser call sites; the marker is no
/// longer browser-only now native GM-only fleet members use the same rules.
pub use GameMasterPeer as BrowserGameMaster;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipient_diagnostics: Vec<crate::recipients::RecipientDiagnostic>,
    /// Derived live world membership for the GM tree; not snapshot authority.
    #[serde(default)]
    pub world_membership: BTreeMap<String, String>,
    #[serde(
        default,
        skip_serializing_if = "crate::gm_world_inspector::WorldInspectorProjection::is_empty"
    )]
    pub world_inspector: crate::gm_world_inspector::WorldInspectorProjection,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub system_controls: BTreeMap<String, Vec<GmSystemControlStatus>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_results: Vec<LoggedGmAction>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub npc_doctrines: BTreeMap<String, crate::gm_npc::NpcDoctrineStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_doctrine_results: Vec<crate::gm_action::LoggedGmAction>,
    /// The entities/AI domain of the M6 Live Inspector (issue #1489): one
    /// descriptor table for the known schema plus one reading per live entity.
    ///
    /// It rides this payload rather than opening a channel of its own because
    /// it is about the same entities the rest of the payload describes, and
    /// `publish_local_projection` already republishes the whole thing whenever
    /// any of it changes — which is exactly the bounded cadence a reading wants.
    #[serde(default)]
    pub entity_inspector: crate::gm_entity_inspector::EntityInspectorProjection,
    /// Active hull/Station/System schema and runtime readings (issue #1491).
    #[serde(
        default,
        skip_serializing_if = "crate::gm_ship_inspector::ShipInspectorProjection::is_empty"
    )]
    pub ship_inspector: crate::gm_ship_inspector::ShipInspectorProjection,
    /// Active Region schema, public occupancy and effective consequences (#1492).
    #[serde(
        default,
        skip_serializing_if = "crate::gm_region_inspector::RegionInspectorProjection::is_empty"
    )]
    pub region_inspector: crate::gm_region_inspector::RegionInspectorProjection,
    /// Loaded Viewscreen presentation and authored sound definitions (#1493).
    #[serde(
        default,
        skip_serializing_if = "crate::gm_presentation_inspector::PresentationInspectorProjection::is_empty"
    )]
    pub presentation_inspector: crate::gm_presentation_inspector::PresentationInspectorProjection,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub contact_classifications: crate::gm_contact::ContactClassifications,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub presentation: crate::gm_presentation::PresentationState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presentation_results: Vec<crate::gm_action::LoggedGmAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presentation_messages: Vec<crate::gm_presentation::PresentationMessageChoice>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub presentation_cameras: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presentation_sounds: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "crate::gm_information::ContactInformation::is_empty"
    )]
    pub contact_information: crate::gm_information::ContactInformation,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contact_classification_palette: Vec<crate::gm_contact::ReportedClassification>,
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
    /// Recipient-scoped objective markers over the shared world registry.
    pub objective_targets: Vec<String>,
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
    /// Structural world data is shared by all visible consumers, not repeated
    /// in every ship. The revision allows client replicas to retain its rows.
    pub entities: Vec<EntitySnapshot>,
    pub world_revision: u64,
    pub entity_states: Vec<EntityStateSnapshot>,
    #[serde(default)]
    pub presentation_generation: u64,
    #[serde(default)]
    pub console_interest: Option<GmConsoleInterest>,
    /// None is the legacy/unmounted adapter; Some names complete console rows.
    #[serde(default)]
    pub detail_ships: Option<Vec<String>>,
    pub ships: Vec<GmPuppetShipProjection>,
    pub activity: Vec<StationPuppetActivityEntry>,
    /// Canonical terminal results for authentic Station commands. Correlation
    /// remains the exact opaque identity minted by the originating iframe, so
    /// the shell can settle that iframe's ordinary feedback lifecycle.
    pub results: Vec<LoggedGmAction>,
}

pub struct GmProjectionPlugin;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeldGmProjection;

impl Plugin for GmProjectionPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<NativeGmPresentation>(
            StateClass::Presentation,
            "native-local-gm-workspace",
        )
        .declare_state::<GmInspectorInterest>(StateClass::Presentation, "native-local-gm-workspace")
        .init_resource::<GmInspectorInterest>()
        .declare_state::<GmConsoleSubscriptions>(
            StateClass::Presentation,
            "native-local-gm-workspace",
        )
        .init_resource::<GmConsoleSubscriptions>()
        .init_resource::<StationPuppets>()
        .init_resource::<crate::gm_puppet::StationPuppetActivity>()
        .init_resource::<GmActionLog>()
        .init_resource::<LocalGmActionRefusals>()
        .add_message::<GmEntityProjectionChanged>()
        .add_message::<GmStationProjectionChanged>()
        .add_systems(
            PostUpdate,
            prepare_presentation_generation
                .before(HeldGmProjection)
                .after(crate::audio_lifecycle::AudioLifecyclePublished),
        )
        .add_systems(
            PostUpdate,
            // One absolute presentation build after all completed fixed ticks.
            // Also runs in Lobby and while paused; native and browser delivery
            // explicitly follow this set. Authoritative schedules are unchanged.
            (publish_local_projection, publish_station_projection)
                .in_set(HeldGmProjection)
                .run_if(gm_presentation_active),
        );
    }
}

#[derive(Default)]
struct PresentationConfigCache {
    revision: Option<(u64, u64)>,
    authored: Option<crate::entities::config_cache::ConfigCache>,
    ships: BTreeMap<String, ShipClientConfig>,
}

impl PresentationConfigCache {
    fn refresh(&mut self) {
        let revision = crate::entities::config_cache::config_cache_revision();
        self.refresh_revision(revision);
    }

    fn refresh_revision(&mut self, revision: (u64, u64)) {
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.authored = None;
            self.ships.clear();
        }
    }

    fn authored(&mut self) -> &crate::entities::config_cache::ConfigCache {
        self.refresh();
        self.authored
            .get_or_insert_with(crate::entities::config_cache::get_config_cache)
    }

    fn ship(&mut self, path: &str) -> ShipClientConfig {
        self.refresh();
        self.ships
            .entry(path.to_owned())
            .or_insert_with(|| {
                crate::entities::config_cache::get_cached_entity_config(path)
                    .as_ref()
                    .map(crate::lobby::server::project_ship_client_config)
                    .unwrap_or_default()
            })
            .clone()
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

/// Everything the entities/AI Live Inspector reads, on any entity that has it.
///
/// Separate from `GmShipProjectionQuery` and `GmWorldProjectionQuery` because
/// it spans both: a Region and a hull are equally inspectable here for their
/// identity, placement, faction and tags, and only some of them carry the AI
/// sections. Optional everywhere, because absence is the reading.
pub type GmInspectorSourceQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        Option<&'static EntityName>,
        Option<&'static EntityId>,
        Option<&'static crate::entities::spawner::EntityMass>,
        &'static Transform,
        Option<&'static EntityTagsSection>,
        Option<&'static FactionComponent>,
        Option<&'static crate::entities::spawner::BehaviourSection>,
        Option<&'static crate::ai::server::AiProfile>,
        Option<&'static crate::ai::server::LodBubble>,
        Option<&'static crate::entities::spawner::EntityTarget>,
        Option<&'static crate::ship::components::ShipSystemControlSources>,
        Option<&'static crate::modifiers::ShipModifiers>,
        // `power_rating` is an AI ranking input read by authored selectors as
        // `self_fact(power_rating)`. The runtime keeps it on whichever selector
        // components the hull authored rather than on a component of its own,
        // so the reading takes it from the first one present. Nested so the
        // whole query stays inside Bevy's tuple arity.
        (
            Option<&'static crate::ship::sensors::SensorsTargetSelector>,
            Option<&'static crate::console::weapons::beam::TacticalTargetSelector>,
            Option<&'static crate::console::navigation::server::NavigationTargetSelector>,
            Option<&'static crate::console::repair::server::RepairTargetSelector>,
        ),
    ),
>;

#[derive(SystemParam)]
struct GmRegionInspectorSources<'w, 's> {
    regions: Query<
        'w,
        's,
        (
            Entity,
            &'static EntityUuid,
            Option<&'static EntityName>,
            Option<&'static EntityId>,
            Option<&'static EntityTemplatePath>,
            Option<&'static crate::world::server::EntityOriginLayer>,
            Option<&'static EntityTagsSection>,
            &'static Transform,
            &'static RegionShapeSection,
            Option<&'static RegionEffectsSection>,
            Option<&'static RadarAppearanceSection>,
        ),
    >,
    ships: Query<
        'w,
        's,
        (
            Entity,
            &'static EntityUuid,
            Option<&'static EntityName>,
            Option<&'static EntityId>,
        ),
        With<Ship>,
    >,
    membership: Option<Res<'w, crate::regions::server::RegionMembership>>,
}

#[derive(SystemParam)]
struct GmPresentationInspectorSources<'w, 's> {
    ships: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            Option<&'static EntityName>,
            Option<&'static EntityId>,
            Option<&'static EntityTemplatePath>,
            Option<&'static crate::world::server::EntityOriginLayer>,
            Option<&'static crate::entities::spawner::MeshSection>,
            Option<&'static crate::entities::model_rig::ModelMarkers>,
        ),
        (With<Ship>, With<crate::lockstep::FleetSlotOf>),
    >,
    catalog: Option<Res<'w, crate::gm_presentation::sound::LiveSoundCatalog>>,
    tick: Option<Res<'w, crate::sim_tick::SimTick>>,
}

fn presentation_inspector_projection(
    sources: &GmPresentationInspectorSources,
    authored_names: &BTreeMap<&str, &str>,
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    messages: &[crate::gm_presentation::PresentationMessageChoice],
    inbox: Option<&crate::comms::server::CommsInboxRes>,
) -> crate::gm_presentation_inspector::PresentationInspectorProjection {
    let tick = sources.tick.as_deref().map_or(0, |tick| tick.0);
    let mut readings = BTreeMap::new();
    for (uuid, name, id, template, layer, mesh, markers) in &sources.ships {
        let state = runtime.and_then(|runtime| runtime.presentation.get(&uuid.0));
        let label = authored_names
            .get(uuid.0.as_str())
            .copied()
            .or_else(|| name.map(|name| name.0.as_str()))
            .or_else(|| id.map(|id| id.0.as_str()))
            .unwrap_or(uuid.0.as_str());
        readings.insert(
            format!("ship:{}", uuid.0),
            crate::gm_presentation_inspector::ship_reading(
                crate::gm_presentation_inspector::ShipPresentationInputs {
                    ship_id: &uuid.0,
                    label,
                    template: template.map(|template| template.0.as_str()),
                    layer: layer.map(|layer| layer.0.as_str()),
                    model: mesh.and_then(|mesh| mesh.0.model.as_deref()),
                    variant: mesh.and_then(|mesh| mesh.0.variant.as_deref()),
                    markers,
                    state,
                    tick,
                    messages,
                    card: crate::gm_presentation::card_wire(state, tick, &uuid.0, inbox),
                },
            ),
        );
    }
    if let Some(catalog) = sources.catalog.as_deref() {
        readings.extend(crate::gm_presentation_inspector::catalog_readings(
            &catalog.0,
        ));
    }
    crate::gm_presentation_inspector::PresentationInspectorProjection {
        fields: crate::gm_presentation_inspector::fields(),
        readings,
    }
}

fn region_inspector_projection(
    sources: &GmRegionInspectorSources,
    authored_display_names: &BTreeMap<&str, &str>,
    public_names: &BTreeMap<String, String>,
    config_cache: &crate::entities::config_cache::ConfigCache,
) -> crate::gm_region_inspector::RegionInspectorProjection {
    let mut readings = BTreeMap::new();
    for (entity, uuid, name, id, template, layer, tags, transform, shape, effects, radar) in
        sources.regions.iter()
    {
        let mut occupants = sources
            .ships
            .iter()
            .filter(|(ship, _, _, _)| {
                sources.membership.as_deref().is_some_and(|membership| {
                    membership
                        .inside
                        .get(ship)
                        .is_some_and(|regions| regions.contains(&entity))
                })
            })
            .map(|(_, uuid, name, id)| {
                let label = public_names
                    .get(&uuid.0)
                    .cloned()
                    .or_else(|| name.map(|name| name.0.clone()))
                    .or_else(|| id.map(|id| id.0.clone()))
                    .unwrap_or_else(|| uuid.0.clone());
                (uuid, label)
            })
            .collect::<Vec<_>>();
        occupants.sort_by(|(left_uuid, _), (right_uuid, _)| left_uuid.0.cmp(&right_uuid.0));
        readings.insert(
            uuid.0.clone(),
            crate::gm_region_inspector::reading(crate::gm_region_inspector::RegionReadingInputs {
                uuid,
                name,
                display_name: authored_display_names
                    .get(uuid.0.as_str())
                    .copied()
                    .or_else(|| {
                        template
                            .and_then(|template| config_cache.get(&template.0))
                            .and_then(|config| config.display_name.as_deref())
                    }),
                id,
                template,
                layer,
                tags,
                transform,
                shape,
                effects,
                radar,
                occupants,
            }),
        );
    }
    crate::gm_region_inspector::RegionInspectorProjection {
        fields: crate::gm_region_inspector::fields(),
        readings,
    }
}

/// Build the entities/AI inspector domain from the live world.
///
/// Derived context is passed in rather than re-derived: `intents` and `targets`
/// are already resolved for the projection this rides on, so the Inspector
/// reports the same values the rest of the desk shows instead of a second
/// opinion computed from the same components a tick later.
fn entity_inspector_projection(
    sources: &GmInspectorSourceQuery,
    factions: Option<&FactionRegistryResource>,
    intents: &BTreeMap<String, String>,
    targets: &BTreeMap<String, (String, String)>,
) -> crate::gm_entity_inspector::EntityInspectorProjection {
    use crate::gm_entity_inspector::{reading, EntityReadingInputs};
    let mut readings = BTreeMap::new();
    for (
        uuid,
        name,
        id,
        mass,
        transform,
        tags,
        faction,
        behaviour,
        ai_profile,
        lod_bubble,
        target,
        control_sources,
        modifiers,
        (sensors_selector, tactical_selector, navigation_selector, repair_selector),
    ) in sources.iter()
    {
        let faction_name = faction
            .and_then(|faction| faction_reference(Some(faction), factions))
            .map(|reference| reference.name);
        let control_source = control_sources.map(|sources| control_source_summary(&sources.0));
        let power_rating = sensors_selector
            .and_then(|selector| selector.power_rating)
            .or_else(|| tactical_selector.and_then(|selector| selector.power_rating))
            .or_else(|| navigation_selector.and_then(|selector| selector.power_rating))
            .or_else(|| repair_selector.and_then(|selector| selector.power_rating));
        let inputs = EntityReadingInputs {
            name: name.map(|name| name.0.as_str()),
            id: id.map(|id| id.0.as_str()),
            mass: mass.map(|mass| mass.0),
            power_rating,
            translation: Some(transform.translation.to_array()),
            rotation: Some(transform.rotation.to_array()),
            scale: Some(transform.scale.to_array()),
            faction: faction_name.as_deref(),
            tags: tags.map(|tags| tags.0.as_slice()),
            behaviour: behaviour.map(|behaviour| &behaviour.0),
            ai_profile,
            lod_bubble_radius: lod_bubble.map(|bubble| bubble.radius),
            target: target.map(|target| &target.0),
            intent: intents.get(&uuid.0).map(String::as_str),
            current_target: targets.get(&uuid.0).map(|(name, _)| name.as_str()),
            current_target_id: targets.get(&uuid.0).map(|(_, id)| id.as_str()),
            control_source: control_source.as_deref(),
            modifiers: modifiers.map(modifier_summary),
        };
        readings.insert(uuid.0.clone(), reading(&inputs));
    }
    crate::gm_entity_inspector::EntityInspectorProjection {
        fields: crate::gm_entity_inspector::fields(),
        readings,
    }
}

/// How many Systems each control source is holding, in a stable order.
///
/// A count rather than a per-System table: this reading exists to explain the
/// authored behaviour beside it — whether anyone is actually flying to it —
/// and the per-System breakdown already has its own projection.
fn control_source_summary(resolver: &crate::ship::control_source::ControlSourceResolver) -> String {
    use crate::ship::control_source::ControlSource;
    let mut human = 0usize;
    let mut ai = 0usize;
    let mut offline = 0usize;
    for (_, source) in resolver.entries() {
        match source {
            ControlSource::Human => human += 1,
            ControlSource::Ai | ControlSource::Simplified => ai += 1,
            ControlSource::Offline => offline += 1,
        }
    }
    format!("human {human}, ai {ai}, offline {offline}")
}

/// A count of what is actually modifying this hull, from the same structured
/// debug payload the T1 observability work already publishes.
fn modifier_summary(modifiers: &crate::modifiers::ShipModifiers) -> String {
    let payload = modifiers.debug_payload();
    format!(
        "{} float, {} int, {} flags",
        payload.float_modifiers.len(),
        payload.int_modifiers.len(),
        payload.flags.len()
    )
}

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
            Option<&EntityName>,
            Option<&EntityTemplatePath>,
            &ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
            &ActiveStationRatings,
            &EntitySystemHull,
            Option<&crate::server_app::ShipSystemBlackboards>,
        ),
        With<Ship>,
    >,
    // Doctrine control and the inspector's sources both touch BehaviourSection —
    // the control mutably, the inspector read-only — and Bevy refuses one
    // system holding both (B0001). This publisher only ever READS the control,
    // for its owned projection, so the two take turns: the control first, then
    // the sources, never both at once. A read-only doctrine view would let a
    // publisher stop holding a control at all; that is a change to gm_npc's
    // shape rather than to this system, and belongs to its own issue.
    mut doctrine_then_inspector: ParamSet<(
        crate::gm_npc::NpcDoctrineControl,
        GmInspectorSourceQuery,
    )>,
    ships: GmShipProjectionQuery,
    world_entities: GmWorldProjectionQuery,
    region_inspector_sources: GmRegionInspectorSources,
    presentation_inspector_sources: GmPresentationInspectorSources,
    all_names: Query<(
        &EntityUuid,
        Option<&EntityName>,
        Option<&EntityId>,
        Option<&crate::world::server::EntityOriginLayer>,
    )>,
    factions: Option<Res<FactionRegistryResource>>,
    world_sources: GmWorldInspectorSources,
    world_setup: Option<Res<WorldResource>>,
    action_log: Res<GmActionLog>,
    local_refusals: Res<LocalGmActionRefusals>,
    mut presentation: Local<(Option<GmEntityProjectionPayload>, PresentationConfigCache)>,
    mut changed: MessageWriter<GmEntityProjectionChanged>,
    removal_targets: crate::gm_despawn::RemovalQuery,
    presentation_control: crate::gm_presentation::PresentationControl,
) {
    let world_content = &world_sources.runtime;
    let wants = |panel| {
        world_sources
            .interest
            .as_ref()
            .is_none_or(|interest| interest.wants(panel))
    };
    let (previous, configuration) = &mut *presentation;
    if world_sources
        .config
        .as_ref()
        .is_some_and(|config| config.is_changed())
    {
        *configuration = PresentationConfigCache::default();
    }
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
        .map(|(uuid, name, id, _)| {
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
        crate::gm_action::GmActionKind::ContactMisclassify,
        crate::gm_action::GmActionKind::ContactClassificationNormal,
        crate::gm_action::GmActionKind::ContactInformation,
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
    // Derived context for the Inspector, taken from what this projection has
    // already decided rather than recomputed: the Inspector must agree with the
    // desk around it, and two derivations of the same fact from the same
    // components are two chances to disagree.
    let inspector_targets: BTreeMap<String, (String, String)> = projected
        .iter()
        .filter_map(|entity| {
            let target = entity.current_target.as_ref()?;
            Some((
                entity.entity_id.clone(),
                (target.name.clone(), target.entity_id.clone()),
            ))
        })
        .collect();
    let npc_doctrines = world_content
        .as_deref()
        .map(|runtime| doctrine_then_inspector.p0().projection(runtime))
        .unwrap_or_default();
    let inspector_intents: BTreeMap<String, String> = npc_doctrines
        .iter()
        .filter_map(|(uuid, status)| Some((uuid.clone(), status.intent.clone()?)))
        .collect();
    let inspector_sources = doctrine_then_inspector.p1();
    let entity_inspector = if wants(GmInspectorKind::EntityFields) {
        entity_inspector_projection(
            &inspector_sources,
            factions.as_deref(),
            &inspector_intents,
            &inspector_targets,
        )
    } else {
        Default::default()
    };
    let inspection_config_cache = configuration.authored();
    let ship_inspector = if wants(GmInspectorKind::HullFields) {
        crate::gm_ship_inspector::projection(system_sources.iter().map(
            |(uuid, name, template, config, controls, ratings, hull, blackboards)| {
                let authored = ship_inspector_authored_config(template, inspection_config_cache);
                crate::gm_ship_inspector::ShipInspectorInputs {
                    id: &uuid.0,
                    label: name.map(|name| name.0.as_str()).unwrap_or(&uuid.0),
                    config: &config.0,
                    authored: authored.map(|(_, config)| config),
                    authored_document: authored.map(|(path, _)| path.as_str()),
                    ratings,
                    controls: &controls.0,
                    hull: &hull.0,
                    blackboards: blackboards.map(|rows| &rows.0),
                }
            },
        ))
    } else {
        Default::default()
    };
    let region_inspector = if wants(GmInspectorKind::RegionFields) {
        region_inspector_projection(
            &region_inspector_sources,
            &authored_names,
            &names,
            inspection_config_cache,
        )
    } else {
        Default::default()
    };
    let presentation_messages = presentation_control.message_choices();
    let presentation_inspector = if wants(GmInspectorKind::PresentationFields) {
        presentation_inspector_projection(
            &presentation_inspector_sources,
            &authored_names,
            world_content.as_deref(),
            &presentation_messages,
            presentation_control.inbox.as_deref(),
        )
    } else {
        Default::default()
    };
    let root_members: std::collections::BTreeSet<&str> = named_uuids
        .iter()
        .copied()
        .chain(
            world_setup
                .iter()
                .flat_map(|world| world.0.entities.iter().map(|entity| entity.uuid.as_str())),
        )
        .chain(
            ships
                .iter()
                .filter(|ship| ship.5.is_some())
                .map(|ship| ship.0 .0.as_str()),
        )
        .collect();
    let next = GmEntityProjectionPayload {
        recipient_diagnostics: world_sources
            .recipient_diagnostics
            .as_deref()
            .map(|rows| rows.0.iter().cloned().collect())
            .unwrap_or_default(),
        world_membership: all_names
            .iter()
            .map(|(uuid, _, _, layer)| {
                (
                    uuid.0.clone(),
                    layer.map(|layer| layer.0.clone()).unwrap_or_else(|| {
                        if root_members.contains(uuid.0.as_str()) {
                            "root".into()
                        } else {
                            "unassigned".into()
                        }
                    }),
                )
            })
            .collect(),
        world_inspector: if wants(GmInspectorKind::WorldFields) {
            crate::gm_world_inspector::projection(
                world_sources.config.as_deref(),
                world_content.as_deref(),
                world_sources
                    .objectives
                    .as_deref()
                    .map(|objectives| &objectives.0),
                world_sources.layers.as_deref(),
                world_sources.paused.as_deref().map(|paused| paused.0),
            )
        } else {
            crate::gm_world_inspector::topology(
                world_sources.config.as_deref(),
                world_sources.layers.as_deref(),
            )
        },
        system_controls: system_sources
            .iter()
            .map(
                |(uuid, _name, _template, config, sources, _ratings, _hull, _blackboards)| {
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
                },
            )
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
        presentation: world_content
            .as_deref()
            .map(|r| r.presentation.clone())
            .unwrap_or_default(),
        presentation_messages,
        presentation_cameras: presentation_control.cameras(),
        presentation_sounds: presentation_control.sound_choices(),
        presentation_results: crate::gm_action::projected_results(
            crate::gm_action::GmActionKind::Presentation,
            &action_log,
            &local_refusals,
        ),
        contact_information: world_content
            .as_ref()
            .map(|runtime| runtime.contact_information.clone())
            .unwrap_or_default(),
        contact_classifications: world_content
            .as_ref()
            .map(|runtime| runtime.contact_classifications.clone())
            .unwrap_or_default(),
        contact_classification_palette: world_content
            .as_ref()
            .map(|runtime| {
                runtime
                    .gm_palette
                    .iter()
                    .map(|entry| crate::gm_contact::ReportedClassification {
                        palette: entry.id.clone(),
                        label: entry.label.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        contact_results,
        npc_doctrines,
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
        entity_inspector,
        ship_inspector,
        region_inspector,
        presentation_inspector,
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

fn ship_inspector_authored_config<'a>(
    template: Option<&'a EntityTemplatePath>,
    cache: &'a crate::entities::config_cache::ConfigCache,
) -> Option<(&'a String, &'a crate::entities::config::EntityConfig)> {
    let template = template?;
    cache.get(&template.0).map(|config| (&template.0, config))
}

fn publish_station_projection(
    capabilities: crate::gm_puppet::capability::StationCapabilities,
    puppets: Res<StationPuppets>,
    activity: Res<crate::gm_puppet::StationPuppetActivity>,
    action_log: Res<GmActionLog>,
    local_refusals: Res<LocalGmActionRefusals>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    selected_ship: Option<Res<crate::lobby::SelectedShipResource>>,
    presentation: GmStationPresentation,
    objectives: Option<Res<crate::world::server::ObjectiveManagerRes>>,
    objective_instances: Option<Res<crate::world::server::ObjectiveInstanceManagerRes>>,
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
            Option<&crate::entities::spawner::EntityTemplatePath>,
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
    mut configuration: Local<PresentationConfigCache>,
    mut changed: MessageWriter<GmStationProjectionChanged>,
) {
    // This is the absolute counterpart of `build_sim_state_entity_states`:
    // same wire fields, same authoritative ECS components, but no broadcaster
    // caches and therefore no delta suppression. The GM browser then feeds it
    // through the ordinary `ClientSimState` reducer over `WorldResource`, so
    // there is one raw/local projection boundary rather than a GM-only radar
    // model.
    if presentation
        .world_config
        .as_ref()
        .is_some_and(|config| config.is_changed())
    {
        *configuration = PresentationConfigCache::default();
    }
    let wants_detail = presentation.subscriptions.requests.is_empty()
        || presentation.subscriptions.requests.values().any(|request| {
            request.visible && request.world_generation == presentation.subscriptions.generation
        });
    let mut entity_states = live_entities
        .iter()
        .filter(|_| wants_detail)
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
    let entities = presentation
        .world_data
        .as_ref()
        .filter(|_| wants_detail)
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
                template,
                config,
                ratings,
                sources,
                blackboards,
                physics,
                waypoint,
                hull,
            )| {
                let detailed = presentation.subscriptions.wants(&uuid.0);
                // A GM-only host has no SelectedShipResource. In particular,
                // an AI-filled mission slot is not a ship in the peer roster.
                // Its instance template, not the operator's hull, owns the
                // console topology and authored controls.
                let config_path = template.map(|path| path.0.as_str()).or_else(|| {
                    roster
                        .as_ref()
                        .and_then(|roster| {
                            roster
                                .ships()
                                .iter()
                                .find(|ship| slot.is_some_and(|slot| ship.host == slot.0))
                                .and_then(|ship| ship.ship_path.as_deref())
                        })
                        .or_else(|| selected_ship.as_ref().map(|selected| selected.0.as_str()))
                });
                let ship_client_config = if !detailed {
                    ShipClientConfig::default()
                } else if slot.is_some() {
                    config_path
                        .map(|path| configuration.ship(path))
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
                    .filter(|_| detailed)
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
                    .filter(|_| detailed)
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
                            crate::ship::control_source::ControlSource::Simplified => "Simplified",
                        };
                        (system.clone(), label.to_string())
                    })
                    .collect();
                let mut blackboards = blackboards
                    .0
                    .iter()
                    .filter(|_| detailed)
                    .map(|(system, value)| (system.clone(), value.clone()))
                    .collect::<Vec<_>>();
                blackboards.sort_by(|left, right| left.0.cmp(&right.0));
                let console_hull = hull
                    .filter(|_| detailed)
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
                let ship_pose = physics.filter(|_| detailed).map_or_else(
                    GmShipPoseProjection::default,
                    |physics| GmShipPoseProjection {
                        x: physics.x,
                        y: physics.y,
                        z: physics.z,
                        yaw: physics.yaw,
                        forward_speed: physics.forward_speed,
                    },
                );

                let mut scoped_objectives = objectives
                    .as_ref()
                    .filter(|_| detailed)
                    .map(|manager| manager.0.snapshots_for(&uuid.0))
                    .unwrap_or_default();
                if let Some(instances) = objective_instances.as_ref().filter(|_| detailed) {
                    scoped_objectives = instances
                        .0
                        .project_snapshots_for_ship(&uuid.0, scoped_objectives);
                }
                GmPuppetShipProjection {
                    ship_id: uuid.0.clone(),
                    name: name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
                    stations,
                    ship_config: ship_client_config,
                    station_ratings,
                    control_sources,
                    blackboards,
                    objective_targets: if detailed {
                        crate::objectives::project_entity_targets(&entities, &scoped_objectives)
                            .into_iter()
                            .filter(|entity| entity.objective_target)
                            .map(|entity| entity.uuid)
                            .collect()
                    } else {
                        Vec::new()
                    },
                    objectives: scoped_objectives,
                    ship_pose,
                    navigation_waypoint: waypoint
                        .filter(|_| detailed)
                        .and_then(|waypoint| waypoint.snapshot()),
                    console_hull,
                }
            },
        )
        .filter(|ship| !ship.stations.is_empty())
        .collect::<Vec<_>>();
    projected_ships.sort_by(|left, right| left.ship_id.cmp(&right.ship_id));

    let world_revision = previous.as_ref().map_or(1, |previous| {
        previous.world_revision
            + u64::from(
                previous.entities != entities
                    || previous.presentation_generation != presentation.subscriptions.generation,
            )
    });
    let next = GmStationProjectionPayload {
        world_revision,
        entities,
        entity_states,
        presentation_generation: presentation.subscriptions.generation,
        console_interest: presentation
            .subscriptions
            .requests
            .get(&GmConsoleConsumer::Console)
            .cloned(),
        detail_ships: (!presentation.subscriptions.requests.is_empty()).then(|| {
            projected_ships
                .iter()
                .filter(|ship| presentation.subscriptions.wants(&ship.ship_id))
                .map(|ship| ship.ship_id.clone())
                .collect()
        }),
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
#[path = "gm_projection_tests.rs"]
mod tests;
