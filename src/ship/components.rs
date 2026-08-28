use bevy::prelude::*;
use std::collections::HashMap;

use crate::core::messages::{CoordinationPayload, StationId, SystemId};
use crate::ship::config::ShipConfig;
use crate::ship::control_source::ControlSource;
use crate::ship::control_source::ControlSourceResolver;
use crate::ship::coordination::CoordinationLagQueue;
use crate::ship::damage::DamageTier;

// ── Resources ──────────────────────────────────────────────────────────────

// `HelmInputTimer` (the 30 Hz human-input sampling timer) was deleted by
// #824: `process_helm_inputs` is now the per-entity applier of admitted helm
// commands — including the per-axis AI's same-tick emissions — and a
// sampling gate here would silently drop AI commands emitted on frames the
// timer skipped (AdmittedCommands is cleared every tick at admission).

pub(crate) const HELM_AI_MAX_DT_SECS: f32 = 1.0 / 30.0;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct LastHelmInput {
    pub thrust: f32,
    pub steering: f32,
    pub lateral: f32,
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ShipSystemControlSources(pub ControlSourceResolver);

/// The parsed `ShipConfig` defining stations, systems, and per-station rating
/// tables. Populated once at startup from the embedded ship TOML.
#[derive(Component, Clone)]
pub struct ShipConfigComponent(pub ShipConfig);

/// Tracks the currently active rating name for each station.
/// Updated when a player sends `SetStationRating`.
#[derive(Component, Clone, Debug, Default)]
pub struct ActiveStationRatings(pub HashMap<StationId, String>);

/// Scenario-authored minimum-detail requirements after they have been resolved
/// to this hull's System ids. The scenario runtime owns this input; Station
/// placement only reads it when choosing a complete visiting Station's
/// effective authored rating.
///
/// A `BTreeSet` keeps the snapshot deterministic for saves and lockstep. An
/// empty set is the ordinary pre-Band-B/simple-scenario value.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenarioDetailFloor(pub std::collections::BTreeSet<SystemId>);

/// Where each sought system on this ship is currently hosted: legacy
/// `human_seeking` systems (#984) plus every system owned by a complete
/// human-seeking Station (#1097). The station whose holder is authoritative
/// RIGHT NOW need not be the station the system authors.
///
/// Rewritten every tick by `ship::coordination_systems::resolve_human_seeking_hosts`
/// from the pure [`crate::ship::coordination::seek_human_host`], and read by
/// `command_admission::station_for_system` — so admission, the `CommsState`
/// address, and the comms rejection channel all resolve the same seat from the
/// same map instead of each re-deriving one.
///
/// A `BTreeMap` rather than a `HashMap` because the map is a per-ship snapshot
/// that a save (#864) and a lockstep peer both have to agree on byte for byte;
/// iterating it must not depend on hash seeding.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct HumanSeekingHosts(pub std::collections::BTreeMap<SystemId, StationId>);

impl HumanSeekingHosts {
    /// The station currently hosting `system`, or `None` when it is not a
    /// human-seeking system or the seek found no human anywhere on the ship
    /// (in which case the system is AI-operated and has no human host).
    pub fn host_for(&self, system: &SystemId) -> Option<&StationId> {
        self.0.get(system)
    }
}

/// Live placements for complete human-seeking Stations (issue #1097; Comms
/// joined Navigation on this path in #1098). Kept separate from the legacy
/// fine-system map — which several other shipped hulls (battleship, cruiser,
/// courier) still use for their own per-system seeks — so the two authority
/// models cannot accidentally reinterpret keys.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct VisitingStationHosts(pub Vec<crate::ship::coordination::VisitingStationAssignment>);

impl VisitingStationHosts {
    pub fn assignment_for(
        &self,
        station: &StationId,
    ) -> Option<&crate::ship::coordination::VisitingStationAssignment> {
        self.0
            .iter()
            .find(|assignment| &assignment.station == station)
    }
}

/// Channel-3 coordination lag queue. Holds pending coordination messages
/// until their due time, at which point they are routed by the delivery-time
/// matrix (issue #494).
#[derive(Component, Clone, Debug, Default)]
pub struct CoordinationQueue(pub CoordinationLagQueue);

/// Pending Weapons->Helm arc-bearing request, delivered via the channel-3
/// coordination bus (issues #677, #767). Set by Helm's
/// [`crate::console::helm::server::receive_helm_coordination`] after the generic
/// lag router delivers a `CoordinationPayload::ArcBearingRequest` to an
/// AI-controlled Helm; read by `ai_helm_steering` to bias steering toward the
/// requested bearing.
///
/// `arcs` carries the emitting weapon family's usable ONLINE emitter arcs
/// (facing/arc/effective-range), copied verbatim from the request payload
/// (issue #767). `ai_helm_steering` self-clears `target` — via
/// `apply_arc_bearing_request` — the moment the target leaves the merged view,
/// leaves the range of every carried arc, or enters some carried arc, so the
/// bias never outlives the condition that created it and stays consistent with
/// the emitter that raised it.
/// (`operate_helm_ai` was the other reader until #704 deleted it; it stood down
/// from the whole arc-bearing step whenever helm-steering was AI, so the fold
/// into steering is now unconditional rather than a fallback.)
#[derive(Component, Clone, Debug, Default)]
pub struct PendingArcBearingRequest {
    /// The target the Helm is biasing to bring a weapon arc onto, or `None`.
    pub target: Option<uuid::Uuid>,
    /// The emitting family's usable ONLINE emitter arcs. Empty when no request
    /// is pending; drives both the steering bias and the geometric self-clear.
    pub arcs: Vec<crate::core::messages::WeaponEmitterArc>,
}

/// Pending Sensors→Tactical shield-frequency advisory, delivered via the
/// channel-3 coordination bus (issue #873).
///
/// Before #873 Tactical was represented by a Station-shaped
/// `SystemId("tactical")`, which had no registered fine System and therefore
/// resolved to the default Human policy. A backfilled Tactical was invisible
/// to the router and never consumed the hint.
///
/// Issue #1254 addresses the authored Tactical Station explicitly. After the
/// shared lag router resolves an AI delivery,
/// [`crate::console::weapons::receive_tactical_coordination`] owns the typed
/// `FrequencyHint` behavior and lands its unchanged value here.
/// [`crate::console::weapons::apply_tactical_frequency_hint`] reads it the
/// following tick and sets the ship's phaser frequency.
///
/// `None` = nothing pending. The applier `take()`s it, so a hint is applied
/// exactly once.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct PendingTacticalFrequencyHint(pub Option<f32>);

impl PendingTacticalFrequencyHint {
    /// Read the exact one-tick Tactical inbox continuation.
    pub(crate) fn continuation(&self) -> Option<f32> {
        self.0
    }

    /// Replace the bootstrap inbox with the restored continuation.
    pub(crate) fn replace_continuation(&mut self, frequency: Option<f32>) {
        self.0 = frequency;
    }
}

/// The per-ship channel-3 BUS SLOTS, named once (issue #873).
///
/// Every member is the same shape: the post-lag receiving path lands a consumed
/// Coordination value in it, and a System one tick later folds that value into
/// its state. Helm, Tactical, and Shields are written by their owning receivers
/// from `DeliveredCoordination`; the generic lag router does not know their
/// private pending-state representations.
/// Each is inserted by hand at
/// [`PER_SHIP_BUS_SPAWN_SITES`], and a ship that misses one silently cannot
/// RECEIVE that advisory at all — the consumer writes into a component that is
/// not there, and nothing anywhere warns.
///
/// That is the failure mode `#785`, `#786`, `#882` and `#885` each shipped: a
/// per-ship component wired into `entities::spawner::spawn_entity` and forgotten
/// in `server_app::spawn_game_start_entities`, which is the path the PLAYER ship
/// takes. `tests::every_per_ship_bus_component_is_attached_at_every_spawn_site`
/// re-derives the attachment from the crate's own source, so the omission fails
/// a test instead of shipping.
///
/// # Why not [`crate::ai::server::ai_high_fidelity_components`]
///
/// That set is the obvious-looking home and is the WRONG one: it is removed
/// wholesale on LOD demotion. These slots must survive demotion — a ship dropped
/// to low fidelity still runs the immediate emitters and still has a backfilled
/// Helm/Tactical/Shields to feed — so they belong to the ship, not to its
/// fidelity. Adding one there would make advisories silently stop landing the
/// moment a ship left the player's neighbourhood.
///
/// Names, not types, because the guard is a source scan over two hand-rolled
/// insert sites; a constructor would let the scan see only the constructor's
/// name and stop checking the members.
pub const PER_SHIP_BUS_COMPONENTS: &[&str] = &[
    "PendingArcBearingRequest",
    "PendingShieldsThreatBearing",
    "PendingTacticalFrequencyHint",
];

/// The two functions that attach [`PER_SHIP_BUS_COMPONENTS`]. The player ship
/// never goes through `spawn_entity`, so both must be checked.
pub const PER_SHIP_BUS_SPAWN_SITES: &[(&str, &str)] = &[
    ("src/entities/spawner.rs", "spawn_entity"),
    ("src/server_app/world_setup.rs", "spawn_game_start_entities"),
];

/// A distinct docking intent (issue #742): the UUID of the dock the Helm AI is
/// closing on, or `None` when not docking.
///
/// Unlike [`PendingArcBearingRequest`] — which biases *facing only* so weapons
/// can bear — a docking intent is the sanctioned request for controlled
/// *translation*: the [`helm-motion-planner`](crate::ship::helm_planner) reads
/// it and, once the dock is within the hull's authored
/// `docking_engage_distance`, folds a low-speed reverse / lateral close
/// manoeuvre ([`crate::ai::docking_close_manoeuvre`]) into the ship's
/// desired-motion contract — the reverse and lateral drift arc-bearing must
/// never command.
///
/// Expires the same way arc-bearing does: the planner clears it to `None` the
/// moment its dock target is no longer visible in the ship's merged view
/// (despawned, out of radar range). `None` = not docking.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct DockingMotionIntent(pub Option<uuid::Uuid>);

/// Which generation of this ship's [`NavigationWaypoint`] the AI Helm is
/// cleared to follow (issue #702).
///
/// The Channel-3 Navigation→Helm lag, reduced to one integer. `Navigation` sets
/// the waypoint and enqueues `CoordinationPayload::NavigateTo` carrying its
/// `generation`; the message spends the delivery lag in the queue; after the
/// generic router emits `DeliveredCoordination`, Helm's
/// [`crate::console::helm::server::receive_helm_coordination`] latches the
/// generation here. The AI Helm travels to the waypoint only while `clearance
/// == waypoint.generation()`.
///
/// Because the waypoint bumps its generation whenever it names somewhere new,
/// *every* new waypoint re-incurs the lag. There is only ever one waypoint and
/// no copy of the previous one, so during the lag the Helm does not keep flying
/// the old bearing: [`cleared_nav_waypoint`] yields `None` and the Helm falls
/// through to its own local objectives, or idles if it has none. A bare `bool`
/// ("Navigation has spoken") would only delay the first order and then wave
/// every subsequent waypoint through instantly.
///
/// `None` = never cleared for anything.
///
/// [`NavigationWaypoint`]: crate::console::navigation::NavigationWaypoint
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct HelmWaypointClearance(pub Option<u64>);

/// Tracks the last-seen `DamageTier` and HP per system per ship for detecting
/// tier crossings (issue #682) and, out of red alert, any fresh damage at
/// all. Initialised during ship spawn; each tick of the tier-crossing
/// detector updates entries for every system.
#[derive(Component, Clone, Debug, Default)]
pub struct LastSystemTiers {
    pub tiers: std::collections::HashMap<SystemId, DamageTier>,
    pub hp: std::collections::HashMap<SystemId, f32>,
}

impl LastSystemTiers {
    /// Read the authoritative damage-edge memory for continuation projection.
    pub(crate) fn continuation(
        &self,
    ) -> (
        &std::collections::HashMap<SystemId, DamageTier>,
        &std::collections::HashMap<SystemId, f32>,
    ) {
        (&self.tiers, &self.hp)
    }

    /// Replace bootstrap memory with a restored continuation.
    pub(crate) fn replace_continuation(
        &mut self,
        tiers: std::collections::HashMap<SystemId, DamageTier>,
        hp: std::collections::HashMap<SystemId, f32>,
    ) {
        self.tiers = tiers;
        self.hp = hp;
    }
}

/// Tracks which stations have already been flagged for human repair popups
/// (issue #682). Key: station_id. Value: the worst tier already alerted for.
/// Prevents re-popup every tick for Operational->Damaged crossings.
#[derive(Component, Clone, Debug, Default)]
pub struct RepairHumanAlerted(pub std::collections::HashMap<String, DamageTier>);

/// Newtype resource used by the WASM bridge to pass a custom `ShipConfig`
/// before the ship entity is spawned.  Consumed during ship spawn and then
/// removed from the world.
#[derive(Resource)]
pub struct PendingShipConfig(pub ShipConfig);

/// Server-side enqueue event for channel-3 coordination messages.
/// AI controllers fire this to send delayed advisories to human operators.
///
/// `source_entity` identifies the ship the coordination belongs to. At
/// delivery time, the message will be enqueued into that ship's own
/// `CoordinationQueue` component and routed against that ship's
/// `ShipSystemControlSources` + `ShipConfigComponent`. NPC ships (no
/// `LocalShip` marker) drain silently — popups are only emitted for the
/// LocalShip because that's the only ship with a human console holder.
#[derive(Message, Clone, Debug)]
pub struct CoordinationEnqueue {
    pub source_entity: Entity,
    pub sender_origin: ControlSource,
    pub address: crate::core::messages::CoordinationAddress,
    pub payload: CoordinationPayload,
    /// Required producer-owned display envelope. It travels beside the typed
    /// payload unchanged through the lag queue and every delivery surface.
    pub presentation: crate::core::messages::CoordinationPresentation,
    /// Fallback origin label, used only when `sender_system` does not resolve to
    /// a station (a human sender's name, or an already-resolved `station.*.name`
    /// id). For an AI system the resolved station overrides it at enqueue.
    pub sender_label: String,
    /// The fine system this message speaks FOR. Channel-3 addresses crew by
    /// STATION, so the enqueue handler resolves this to the owning station's
    /// display id (or Core) via [`crate::ship::coordination::system_addressee_label`].
    /// An empty id opts out — the pre-resolved `sender_label` is kept as-is
    /// (the intent-narration path, which already stamps its own station id).
    pub sender_system: crate::core::messages::SystemId,
}

/// World-visible unread position for the sole production
/// [`CoordinationEnqueue`] consumer.
///
/// A `MessageReader` hides this cursor in system-local state, which makes the
/// unread suffix impossible to snapshot faithfully: the `Messages` buffers
/// alone cannot distinguish an already-consumed entry from a late entry still
/// waiting for the next Input tick. Keeping the cursor as a Resource exposes
/// that exact boundary without changing when the handler drains it.
#[derive(Resource, Debug, Default)]
pub struct CoordinationEnqueueCursor(pub bevy::ecs::message::MessageCursor<CoordinationEnqueue>);

/// Outcome already resolved by the generic Coordination lag router.
///
/// `Suppress` and offline `Consume` outcomes do not cross this seam. Domain
/// receivers must still match the outcome they own: an AI-only receiver must
/// never accept a human-popup delivery merely because its address and payload
/// happen to look valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoordinationDelivery {
    Ai,
    HumanPopup {
        token: String,
        sender_label: String,
        /// Position among every same-tick popup selected by the lag router.
        /// Repair keeps this while applying its escalation rule, then hands it
        /// back to the shared ordered flush.
        order: u64,
    },
}

/// Typed handoff emitted after a delayed Station-addressed Coordination
/// message resolves to a live recipient.
///
/// The lag router owns time, live routing, and the Popup/Consume/Suppress
/// decision. For a human recipient the payload has also passed the router's
/// visibility projection before it reaches this seam. System modules read the
/// message to own the resulting behavior; the router does not reach into their
/// private pending-state components.
#[derive(Message, Clone, Debug)]
pub struct DeliveredCoordination {
    pub source_entity: Entity,
    pub address: crate::core::messages::CoordinationAddress,
    pub payload: CoordinationPayload,
    pub presentation: crate::core::messages::CoordinationPresentation,
    pub delivery: CoordinationDelivery,
}

/// One human Coordination popup approved for the shared outbox, carrying the
/// lag router's original same-tick enqueue position.
///
/// Generic Station and Ship popups reach this seam directly from the router.
/// Repair's human outcome reaches it only after Repair has applied its own
/// escalation latch. The final flush sorts both sources together, so moving a
/// decision behind an owning-module receiver cannot reorder the visible bus.
#[derive(Message, Clone, Debug)]
pub(crate) struct OrderedCoordinationPopup {
    pub order: u64,
    pub token: String,
    pub message: crate::core::messages::ServerMessage,
}

/// Load `ShipConfigComponent` from `assets/entities/alliance_battleship.toml`.
///
/// Through the include resolver rather than `include_str!` (issue #876). That
/// hull is COMPOSED, so the bytes on disk are only half of it, and a baked site
/// can never see resolution: `include_str!` runs at compile time and the
/// resolver needs a fragment source at run time.
/// [`crate::entities::include_resolve::HostFragmentSource`] is the one source that
/// compiles on both targets — on native it falls through to the filesystem, on
/// WASM it reads the raw templates the host has already delivered.
///
/// This is a FALLBACK: every real boot inserts `PendingShipConfig` ahead of it
/// (`server::bridge::wasm_init` from `wasm_validate_stations`, `headless::app`
/// from `--ship`), so it is reached only by `ShipConfigComponent::default()` and
/// by a lobby that was handed no selection at all.
///
/// Panics if the file fails to compose or validate — the server cannot start
/// without a valid ship configuration, which is the contract this always had.
pub(crate) fn load_ship_config_from_disk() -> ShipConfigComponent {
    const HULL: &str = "assets/entities/alliance_battleship.toml";
    let resolved = crate::entities::include_resolve::resolve_template(
        HULL,
        &crate::entities::include_resolve::HostFragmentSource,
    )
    .unwrap_or_else(|e| panic!("ship_config: {HULL} failed to compose: {e}"));
    let registry = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
        .expect("core system registry must be valid");
    let kinds: Vec<&str> = registry.kinds().collect();
    match crate::ship::config::parse_and_validate(&resolved.toml, &kinds) {
        Ok(config) => {
            bevy::log::info!(
                "ship_config: loaded {} stations, {} systems",
                config.stations.len(),
                config.systems.len()
            );
            ShipConfigComponent(config)
        }
        Err(e) => panic!("ship_config: failed validation: {e}"),
    }
}

impl Default for ShipConfigComponent {
    fn default() -> Self {
        load_ship_config_from_disk()
    }
}

/// Runtime ship physics config, loaded from `[helm_console]` in the entity TOML.
/// When absent, `ShipPhysicsConfig::new()` defaults are used.
/// Dual-derives `Resource` (for tests + global fallback) and `Component`
/// (per-entity component on each ship — PR 4 migration, see PRD #597).
#[derive(Resource, Component, Clone)]
pub struct ShipPhysicsConfigResource(pub crate::ship::physics::ShipPhysicsConfig);

/// Runtime impulse drive config, loaded from `[helm_console]` in the entity TOML.
/// Charge duration and speed multiplier can be overridden per ship.
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback).
#[derive(Component, Clone)]
pub struct ImpulseConfigResource {
    pub charge_duration: f32,
    pub speed_multiplier: f32,
    pub acceleration_multiplier: f32,
    pub engage_distance: f32,
    pub cancel_distance: f32,
    /// Steering multiplier applied while impulse is active.
    /// 0.0 = no steering, 0.1 = harsh but possible, 1.0 = full steering.
    pub steering_multiplier: f32,
}

impl Default for ImpulseConfigResource {
    fn default() -> Self {
        Self {
            charge_duration: crate::ship::impulse::IMPULSE_CHARGE_DURATION,
            speed_multiplier: crate::ship::impulse::IMPULSE_SPEED_MULTIPLIER,
            acceleration_multiplier: crate::ship::impulse::IMPULSE_ACCELERATION_MULTIPLIER,
            engage_distance: 200.0,
            cancel_distance: 40.0,
            steering_multiplier: 0.1,
        }
    }
}

/// Runtime boost drive config, loaded from `[helm_console.boost]` in the entity
/// TOML. `enabled` is false (the default) when the TOML omits the table, which
/// disables the feature entirely.
/// Per-entity `Component` on each ship (issue #606: component is the sole
/// source of truth; no Resource fallback).
#[derive(Component, Clone)]
pub struct BoostConfigResource {
    pub enabled: bool,
    pub multiplier: f32,
    pub steering_multiplier: f32,
    pub active_duration: f32,
    pub recharge_duration: f32,
}

impl Default for BoostConfigResource {
    fn default() -> Self {
        Self {
            enabled: false,
            multiplier: crate::ship::boost::BOOST_MULTIPLIER,
            steering_multiplier: crate::ship::boost::BOOST_STEERING_MULTIPLIER,
            active_duration: crate::ship::boost::BOOST_ACTIVE_DURATION,
            recharge_duration: crate::ship::boost::BOOST_RECHARGE_DURATION,
        }
    }
}

/// Runtime banking config, loaded from `[helm_console] max_bank_deg` in the entity TOML.
/// Dual-derives `Resource` (for tests + global fallback) and `Component`
/// (per-entity component on each ship — PR 4 migration, see PRD #597).
#[derive(Resource, Component, Clone)]
pub struct BankConfigResource {
    pub max_bank_deg: f32,
    pub bank_lerp_rate: f32,
}

impl Default for BankConfigResource {
    fn default() -> Self {
        Self {
            max_bank_deg: 0.0,
            bank_lerp_rate: BANK_LERP_RATE,
        }
    }
}

/// How quickly the ship's visual roll lerps toward the target bank angle.
/// Used as the serde default for `HelmConsoleConfig::bank_lerp_rate`.
pub const BANK_LERP_RATE: f32 = 5.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::ai_declaration_manifest::source_scan::{
        read_non_test_source, spawn_site_source,
    };
    use std::collections::BTreeSet;

    /// AC: the per-ship bus slots reach the PLAYER ship too.
    ///
    /// `spawn_game_start_entities` is a hand-rolled second spawn path that
    /// `entities::spawner::spawn_entity` does not feed, and four separate issues
    /// (#785, #786, #882, #885) shipped a per-ship component attached on one and
    /// not the other. The failure is always silent — the router writes into a
    /// component that is not on the entity, so the advisory simply never lands.
    ///
    /// Same technique as
    /// `ai_declaration_manifest::tests::every_kind_is_attached_at_every_one_of_its_spawn_sites`,
    /// which covers the AI *config* components. This covers the bus slots, which
    /// that manifest's `FINE_SYSTEM_KINDS` walk cannot see at all.
    #[test]
    fn every_per_ship_bus_component_is_attached_at_every_spawn_site() {
        assert!(
            !PER_SHIP_BUS_COMPONENTS.is_empty() && !PER_SHIP_BUS_SPAWN_SITES.is_empty(),
            "the scan must have something to check"
        );
        for (file, func) in PER_SHIP_BUS_SPAWN_SITES {
            let body = spawn_site_source(file, func);
            for component in PER_SHIP_BUS_COMPONENTS {
                assert!(
                    body.contains(component),
                    "{file}::{func} never mentions `{component}`. Either the attachment \
                     moved (point PER_SHIP_BUS_SPAWN_SITES at where it went) or this path \
                     never got it — and for `spawn_game_start_entities` that means the \
                     PLAYER ship cannot RECEIVE that coordination advisory at all, \
                     silently."
                );
            }
        }
    }

    /// AC: the class cannot grow a member in silence.
    ///
    /// The test above only checks the components someone remembered to name. A
    /// new `Pending*` bus slot added to `ship::components` or `ship::shields`
    /// without joining [`PER_SHIP_BUS_COMPONENTS`] would be back to having no
    /// spawn-site guard — the same hole one layer up. So the roll call is
    /// re-derived from the source, and anything deliberately outside the class
    /// has to say so here.
    #[test]
    fn every_pending_ship_component_either_joins_the_bus_class_or_is_excused() {
        /// Not a channel-3 bus slot: a deferred whole-config apply, attached and
        /// consumed by the config-load path, not written by
        /// `process_coordination_lag`.
        const NOT_BUS_SLOTS: &[&str] = &["PendingShipConfig"];

        let mut found: BTreeSet<String> = BTreeSet::new();
        for file in ["src/ship/components.rs", "src/ship/shields.rs"] {
            for line in read_non_test_source(file).lines() {
                let Some(rest) = line.trim_start().strip_prefix("pub struct Pending") else {
                    continue;
                };
                let tail: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                found.insert(format!("Pending{tail}"));
            }
        }

        let accounted: BTreeSet<&str> = PER_SHIP_BUS_COMPONENTS
            .iter()
            .chain(NOT_BUS_SLOTS.iter())
            .copied()
            .collect();
        let unaccounted: Vec<&String> = found
            .iter()
            .filter(|name| !accounted.contains(name.as_str()))
            .collect();
        assert!(
            unaccounted.is_empty(),
            "{unaccounted:?} is a per-ship `Pending*` component that is neither in \
             PER_SHIP_BUS_COMPONENTS nor excused in NOT_BUS_SLOTS. If it is a channel-3 \
             bus slot, add it to the class so the spawn-site guard covers it; if it is \
             not, excuse it here with the reason."
        );
        let stale: Vec<&&str> = PER_SHIP_BUS_COMPONENTS
            .iter()
            .chain(NOT_BUS_SLOTS.iter())
            .filter(|c| !found.contains(**c))
            .collect();
        assert!(
            stale.is_empty(),
            "{stale:?} is named here but no longer defined in the scanned files — a \
             rename or a move would leave the spawn-site guard checking a string nothing \
             uses, which passes for the wrong reason"
        );
    }
}
