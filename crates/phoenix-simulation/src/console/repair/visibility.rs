//! Host-authoritative damage **visibility projection** (issue #737).
//!
//! This module is the single file named by the PASM entity
//! `repair-visibility-publisher`. It exists as its own host-only module so the
//! projection is a real, observed code edge — the publisher's dependency on the
//! repair-team state machine is the line-start
//! `use crate::modifiers::repair_teams::RepairTeams;` below, and its dependency
//! on the authoritative damage store is `use crate::ship::damage::SystemHull;`.
//!
//! Exact health detail is projected on the host. A Console sees its owned
//! equipment; the Repair Console also sees true Core and equipment with a team
//! on site. An explicit Repair feed carries that producing Console's projected
//! detail. A declared System whose Console has no home Station is still owned.
//!
//! Live and reconnect `SystemHullUpdate` use `projection_for_host`: the union
//! of the recipient's currently hosted Console views. Each mounted client tab
//! then narrows that union to its own view. Aggregate and destroyed fractions
//! remain public whole-ship scalars. Hosting never changes the underlying hull.
//!
//! The per-recipient cache only suppresses repeated presentation; it is not
//! authoritative simulation state. Repair blackboards use the same ownership
//! and on-site rules through `project_repair_for_console`.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::core::messages::{
    QueueEntryPreview, RepairBlackboard, RepairGroup, ServerMessage, StationId, SystemBlackboard,
    SystemHullStatus, SystemId,
};
use crate::lobby::handler::Target;
use crate::lobby::Sessions;
use crate::modifiers::repair_teams::RepairTeams;
use crate::ship::config::ShipConfig;
use crate::ship::damage::DamageTier;
use crate::ship::damage::SystemHull;
use crate::ship::system_registry::repair_system_id;

/// The bucket id used for hull entries that no station owns.
///
/// "Core" is an ownerless bucket rather than a declared station — `ShipConfig`
/// validation actively forbids a station with this id. It is named here so the
/// tier-crossing enqueue in `ship_plugin` and the queue-entry projection below
/// agree on one spelling.
pub const CORE_BUCKET_ID: &str = "core";

/// Stable lifecycle key for the recipient-projected Hull snapshot.
pub(crate) const HULL_REPLICATION_KEY: &str = "hull";

// ── Cached per-recipient projections ──────────────────────────────────────────

/// One recipient's view of the ship's damage state.
///
/// Cached per session token so the broadcaster re-sends when *that recipient's*
/// visible detail changes — which covers hull HP changes, a repair team
/// arriving or leaving, and a player moving to a different station.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HullProjection {
    pub entries: Vec<SystemHullStatus>,
    pub aggregate_fraction: Option<f32>,
    pub destroyed_fraction: Option<f32>,
}

/// Last live-broadcast Hull projection per session token.
///
/// The cache lives with the projection and publisher that own its meaning. A
/// recipient's entry can change without hull HP changing (for example when a
/// repair team arrives on site or the recipient changes Station), so this is
/// intentionally token-keyed and replaced wholesale on each live publication.
#[derive(Resource, Default)]
pub struct LastBroadcastHull(pub HashMap<String, HullProjection>);

/// Register Hull-owned replication lifecycle behavior.
///
/// [`super::server::RepairPlugin`] calls this beside the repair visibility
/// publisher. The generic lifecycle runner sees only a stable key and function
/// pointers; it does not know the cache resource or `SystemHullUpdate` shape.
pub(crate) fn register_hull_replication_lifecycle(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::core::broadcast::{RegisterReplicationLifecycle, ReplicationLifecycleAdapter};

    app.init_resource::<LastBroadcastHull>()
        .declare_state::<LastBroadcastHull>(StateClass::Cache, "digest-exclusion-classes")
        .register_replication_lifecycle(
            ReplicationLifecycleAdapter::new(HULL_REPLICATION_KEY)
                .with_reset(reset_hull_replication),
        );
    crate::core::broadcast::register_reconnect_projection::<
        HullReconnect,
        HullReconnectParams<'static, 'static>,
    >(app, HULL_REPLICATION_KEY, |params| {
        let (requests, inputs) = params
            .downcast::<HullReconnectParams>()
            .expect("registered owner parameter type");
        reconnect_hull_projection(requests, inputs)
    });
}

struct HullReconnect;
type HullReconnectParams<'w, 's> = (
    Res<'w, crate::core::broadcast::ReconnectRequests>,
    HullProjectionInputs<'w, 's>,
);

fn reset_hull_replication(world: &mut World) {
    *world.resource_mut::<LastBroadcastHull>() = LastBroadcastHull::default();
}

/// Build the reconnecting session's current permitted Hull projection.
///
/// This deliberately reads no delta cache and writes no projection cache, so
/// another session's reconnect cannot perturb any connected client's next
/// live delta.
fn reconnect_hull_projection(
    requests: Res<crate::core::broadcast::ReconnectRequests>,
    inputs: HullProjectionInputs,
) -> crate::core::broadcast::ReconnectBatch {
    if requests.0.is_empty() {
        return Vec::new();
    }
    requests
        .0
        .iter()
        .map(|token| {
            let station = inputs
                .sessions
                .as_ref()
                .and_then(|s| s.0.station_for_token(token));
            let Some(projection) = inputs.projection_for(station) else {
                return Vec::new();
            };
            vec![ServerMessage::SystemHullUpdate {
                entries: projection.entries,
                aggregate_fraction: projection.aggregate_fraction,
                destroyed_fraction: projection.destroyed_fraction,
            }]
        })
        .collect()
}

/// Shared read-only inputs for live and reconnect visibility, including the
/// historical first matching complete LocalShip tuple and optional team data.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct HullProjectionInputs<'w, 's> {
    pub(crate) sessions: Option<Res<'w, Sessions>>,
    hulls: Query<
        'w,
        's,
        (
            Option<&'static crate::entities::spawner::EntitySystemHull>,
            &'static crate::ship_plugin::ShipConfigComponent,
            Option<&'static super::server::ShipRepairTeams>,
            Option<&'static crate::ship::components::ConsoleHosts>,
            Option<&'static crate::ship::compartment_damage::ShipDamage>,
        ),
        With<crate::server_app::LocalShip>,
    >,
}
impl HullProjectionInputs<'_, '_> {
    pub(crate) fn visibility(&self) -> Option<HullVisibility> {
        let (hull, config, teams, _, physical) = self.hulls.iter().next()?;
        physical
            .map(|damage| HullVisibility::from_physical(damage, &config.0, teams.map(|t| &t.0)))
            .or_else(|| hull.map(|hull| ship_hull_visibility(&hull.0, &config.0, teams)))
    }
    pub(crate) fn projection_for(&self, station: Option<&StationId>) -> Option<HullProjection> {
        let (_, config, _, hosts, _) = self.hulls.iter().next()?;
        let vis = self.visibility()?;
        Some(vis.projection_for_host(&config.0, hosts.unwrap_or(&Default::default()), station))
    }
}

// ── The projection itself (pure) ───────────────────────────────────────────────

/// A resolved snapshot of everything needed to decide who may see which system.
///
/// Built once per broadcast tick, then queried per recipient.
#[derive(Clone, Debug)]
pub struct HullVisibility {
    structural_fraction: Option<f32>,
    unavailable_structure: Option<f32>,
    /// Every damageable system, exact detail. Never sent as-is.
    entries: Vec<SystemHullStatus>,
    /// Owning station per system id; `None` means ownerless — the Core bucket.
    owner_of: HashMap<SystemId, Option<StationId>>,
    /// Declared equipment remains owned even when its Console has no home seat.
    console_owned: std::collections::HashSet<SystemId>,
    group_of: HashMap<SystemId, RepairGroup>,
    /// The station that holds the `repair` system — "Engineering" in the
    /// role sense. Resolved from ship config, never assumed by name (it is
    /// `repair` on the battleship and `engineering` on cruiser/destroyer).
    engineering_station: Option<StationId>,
    /// Systems with a repair team physically present and working.
    on_site: Vec<SystemId>,
}

impl HullVisibility {
    pub fn from_physical(
        damage: &crate::ship::compartment_damage::ShipDamage,
        config: &ShipConfig,
        teams: Option<&RepairTeams>,
    ) -> Self {
        let entries: Vec<_> = config
            .systems
            .iter()
            .filter_map(|system| physical_system_status(damage, &system.id, config))
            .collect();
        let owner_of = entries
            .iter()
            .map(|entry| {
                (
                    entry.system_id.clone(),
                    config
                        .system(&entry.system_id)
                        .and_then(|system| config.system_home_station(system))
                        .cloned(),
                )
            })
            .collect();
        let engineering_station = config
            .system(&repair_system_id())
            .and_then(|system| config.system_home_station(system))
            .cloned();
        let rooms: std::collections::BTreeSet<_> = teams
            .into_iter()
            .flat_map(|teams| {
                teams
                    .slots()
                    .iter()
                    .zip(teams.navigation().unwrap_or_default())
            })
            .filter_map(|(slot, team)| match (slot, &team.location) {
                (
                    crate::core::messages::TeamSlot::Repairing { .. },
                    phoenix_model::interior::TeamLocation::Compartment { compartment },
                ) => Some(compartment),
                _ => None,
            })
            .collect();
        let on_site = config
            .systems
            .iter()
            .filter(|system| {
                damage
                    .definition
                    .interior
                    .placements
                    .get(&system.id)
                    .is_some_and(|room| rooms.contains(room))
            })
            .map(|system| system.id.clone())
            .collect();
        let mut visibility = Self::new(entries, owner_of, engineering_station, on_site);
        visibility.group_of = config
            .systems
            .iter()
            .map(|system| {
                (
                    system.id.clone(),
                    crate::modifiers::repair_teams::repair_group(&system.id, config),
                )
            })
            .collect();
        visibility.console_owned = config
            .systems
            .iter()
            .filter(|system| system.console.is_some())
            .map(|system| system.id.clone())
            .collect();
        visibility.structural_fraction = Some(damage.structural_fraction());
        let total: f32 = damage
            .definition
            .interior
            .compartments
            .iter()
            .map(|room| room.max_hp)
            .sum();
        let disabled: f32 = damage
            .definition
            .interior
            .compartments
            .iter()
            .filter(|room| {
                damage
                    .state
                    .compartments
                    .get(&room.id)
                    .is_some_and(|room| room.disabled)
            })
            .map(|room| room.max_hp)
            .sum();
        visibility.unavailable_structure = Some(if total > 0.0 { disabled / total } else { 0.0 });
        visibility
    }
    /// Build from already-resolved parts. Pure — used directly by tests.
    pub fn new(
        entries: Vec<SystemHullStatus>,
        owner_of: HashMap<SystemId, Option<StationId>>,
        engineering_station: Option<StationId>,
        on_site: Vec<SystemId>,
    ) -> Self {
        let group_of = owner_of
            .iter()
            .map(|(id, owner)| {
                (
                    id.clone(),
                    owner
                        .clone()
                        .map_or(RepairGroup::Core, RepairGroup::Station),
                )
            })
            .collect();
        Self {
            structural_fraction: None,
            unavailable_structure: None,
            entries,
            owner_of,
            group_of,
            console_owned: Default::default(),
            engineering_station,
            on_site,
        }
    }

    /// Resolve ownership + the engineering station from ship config, and the
    /// on-site set from the repair-team state machine.
    pub fn from_parts(hull: &SystemHull, config: &ShipConfig, teams: Option<&RepairTeams>) -> Self {
        let entries: Vec<SystemHullStatus> = hull
            .iter()
            .map(|(sid, entry)| SystemHullStatus {
                physical: None,
                system_id: sid.clone(),
                display_name: entry.display_name.clone(),
                current: entry.current,
                max_hp: entry.max,
                tier: hull.tier_for(sid),
                debuff_magnitude: hull.debuff_magnitude_for(sid),
            })
            .collect();

        let owner_of = entries
            .iter()
            .map(|e| {
                let owner = config
                    .system(&e.system_id)
                    .and_then(|s| config.system_home_station(s).cloned());
                (e.system_id.clone(), owner)
            })
            .collect();

        let engineering_station = config
            .system(&repair_system_id())
            .and_then(|s| config.system_home_station(s).cloned());

        // `on_site_systems` is the named predicate on `RepairTeams`: only
        // `Repairing` counts. A team still `Travelling` has not arrived, and a
        // team `Returning` has left — including a team recalled before arrival,
        // which goes `Travelling -> Returning` without ever passing through
        // `Repairing` and therefore never reveals anything.
        let on_site = teams
            .map(|t| t.on_site_systems().cloned().collect())
            .unwrap_or_default();

        let mut visibility = Self::new(entries, owner_of, engineering_station, on_site);
        visibility.group_of = visibility
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.system_id.clone(),
                    crate::modifiers::repair_teams::repair_group(&entry.system_id, config),
                )
            })
            .collect();
        visibility.console_owned = config
            .systems
            .iter()
            .filter(|system| system.console.is_some())
            .map(|system| system.id.clone())
            .collect();
        visibility
    }

    /// Ship-wide hull fraction (0.0–1.0) across **every** damageable system.
    ///
    /// This is the only whole-ship figure a recipient may be given, because a
    /// projected `entries` list can no longer be summed into one. Returns
    /// `None` when the ship declares no damageable systems.
    pub fn aggregate_fraction(&self) -> Option<f32> {
        if let Some(fraction) = self.structural_fraction {
            return Some(fraction);
        }
        let max: f32 = self.entries.iter().map(|e| e.max_hp).sum();
        if max <= 0.0 {
            return None;
        }
        let current: f32 = self.entries.iter().map(|e| e.current).sum();
        Some((current / max).clamp(0.0, 1.0))
    }

    /// Share of the ship's total hull capacity (0.0–1.0) held by systems at the
    /// `Destroyed` tier — capability that is gone rather than merely damaged
    /// (issue #1014).
    ///
    /// Computed over **every** damageable system, exactly like
    /// [`Self::aggregate_fraction`] and for the same reason: the projected
    /// `entries` a recipient receives are a slice of the ship, so a destroyed
    /// system nobody may see would otherwise be invisible to every whole-ship
    /// figure. It is a single scalar reduction over the full list, which names
    /// no system and therefore leaks nothing #737 withholds — the same privacy
    /// argument that lets every recipient have the aggregate.
    ///
    /// `Destroyed` latches at exactly 0 HP (`crate::ship::damage::DamageTier`), so
    /// this share is already inside `aggregate_fraction`'s *loss*; the two
    /// scalars answer different questions — "how much hull is left" versus "how
    /// much of it is unrecoverable" — and the client paints the second as a
    /// distinct band. Returns `None` when the ship declares no damageable
    /// systems, matching [`Self::aggregate_fraction`].
    pub fn destroyed_fraction(&self) -> Option<f32> {
        if let Some(fraction) = self.unavailable_structure {
            return Some(fraction);
        }
        let max: f32 = self.entries.iter().map(|e| e.max_hp).sum();
        if max <= 0.0 {
            return None;
        }
        let destroyed: f32 = self
            .entries
            .iter()
            .filter(|e| e.tier == DamageTier::Destroyed)
            .map(|e| e.max_hp)
            .sum();
        Some((destroyed / max).clamp(0.0, 1.0))
    }

    /// Per-station hull fraction (0.0–1.0), one scalar per owning station.
    ///
    /// The station-level companion to [`Self::aggregate_fraction`], and the
    /// authoritative figure the Hero Bar publishes so no client has to sum
    /// another station's damage rows it is not entitled to hold (issue #1100).
    /// Each entry is summed-current over summed-max across exactly that
    /// station's damageable systems; ownerless systems accumulate under the
    /// [`CORE_BUCKET_ID`] bucket, matching [`Self::can_see_group`].
    ///
    /// A station whose damageable capacity sums to zero — it owns only systems
    /// the ship declares at zero max, or (as bucketed here) none at all —
    /// yields an explicit `None`: the neutral "no-damage-model" state, exactly
    /// as `aggregate_fraction` returns `None` for a ship with no damageable
    /// systems. Each value is a single scalar reduction naming no system, so it
    /// is safe to publish station-level to every recipient — the same privacy
    /// argument that lets everyone have the ship-wide aggregate.
    ///
    /// Stations appear in first-seen `entries` order so the wire byte stream is
    /// deterministic.
    pub fn station_fractions(&self) -> Vec<(StationId, Option<f32>)> {
        let mut order: Vec<StationId> = Vec::new();
        let mut sums: HashMap<StationId, (f32, f32)> = HashMap::new();
        for e in &self.entries {
            let station = self.station_bucket(&e.system_id);
            let slot = sums.entry(station.clone()).or_insert_with(|| {
                order.push(station.clone());
                (0.0, 0.0)
            });
            slot.0 += e.current;
            slot.1 += e.max_hp;
        }
        order
            .into_iter()
            .map(|station| {
                let (current, max) = sums[&station];
                let fraction = if max > 0.0 {
                    Some((current / max).clamp(0.0, 1.0))
                } else {
                    None
                };
                (station, fraction)
            })
            .collect()
    }

    /// The Station bucket a system falls into: its owning Station, or the
    /// [`CORE_BUCKET_ID`] bucket when it is ownerless or unknown.
    ///
    /// The single source of the station-bucketing [`Self::station_fractions`]
    /// reduces health over — reused by the host importance projection (issue
    /// #1101) so an objective's owning Station is attributed exactly the way its
    /// health is bucketed, never by a second, divergent rule.
    pub fn station_bucket(&self, system_id: &SystemId) -> StationId {
        match self.owner_of.get(system_id) {
            Some(Some(owner)) => owner.clone(),
            _ => StationId(CORE_BUCKET_ID.to_string()),
        }
    }

    /// The owning Station of a system, or `None` when the system is ownerless
    /// or unknown (i.e. it would fall to the Core bucket).
    ///
    /// Unlike [`Self::station_bucket`] this does not substitute the Core bucket,
    /// so importance attribution (issue #1101) can tell "owned by Station X"
    /// from "attribute to the ship-wide core bucket".
    pub fn owned_station(&self, system_id: &SystemId) -> Option<StationId> {
        match self.owner_of.get(system_id) {
            Some(Some(owner)) => Some(owner.clone()),
            _ => None,
        }
    }

    /// True when a hull entry has no owning station — the Core bucket.
    fn is_core(&self, system_id: &SystemId) -> bool {
        !self.console_owned.contains(system_id)
            && matches!(self.owner_of.get(system_id), Some(None) | None)
    }

    /// Exact health belongs to a Console, independent of where it is hosted.
    pub fn console_can_see(
        &self,
        config: &ShipConfig,
        console: &crate::core::messages::ConsoleId,
        system: &SystemId,
    ) -> bool {
        let Some(definition) = config.console(console) else {
            return false;
        };
        if config
            .console_for_system(system)
            .is_some_and(|owner| &owner.id == console)
        {
            return true;
        }
        let Some(repair) = config
            .systems
            .iter()
            .find(|candidate| candidate.kind == crate::ship::system_registry::REPAIR_KIND)
        else {
            return false;
        };
        let reads_repair = repair.console.as_ref() == Some(console)
            || definition
                .feeds
                .iter()
                .any(|feed| feed.kind == phoenix_model::console::ConsoleObservationKind::Repair);
        if !reads_repair {
            return false;
        }
        let producer_owns = repair.console.as_ref().is_some_and(|producer| {
            config
                .console_for_system(system)
                .is_some_and(|owner| &owner.id == producer)
        });
        producer_owns || self.is_core(system) || self.on_site.contains(system)
    }

    /// A transport recipient receives the union of its hosted Console views.
    /// The client applies the individual Console view again when mounting a tab.
    pub fn projection_for_host(
        &self,
        config: &ShipConfig,
        hosts: &crate::ship::components::ConsoleHosts,
        station: Option<&StationId>,
    ) -> HullProjection {
        let entries = self
            .entries
            .iter()
            .filter(|entry| {
                station.is_some()
                    && hosts.0.iter().any(|host| {
                        host.host.as_ref() == station
                            && self.console_can_see(config, &host.console, &entry.system_id)
                    })
            })
            .cloned()
            .collect();
        HullProjection {
            entries,
            aggregate_fraction: self.aggregate_fraction(),
            destroyed_fraction: self.destroyed_fraction(),
        }
    }

    /// True when `viewer` is the station that holds the `repair` system.
    fn is_engineering(&self, viewer: Option<&StationId>) -> bool {
        match (viewer, self.engineering_station.as_ref()) {
            (Some(v), Some(eng)) => v == eng,
            _ => false,
        }
    }

    /// Is `viewer` entitled to exact detail for `system_id`?
    ///
    /// The whole of #737's information boundary is these four lines.
    pub fn can_see(&self, viewer: Option<&StationId>, system_id: &SystemId) -> bool {
        // A station owner always sees its own systems.
        if let (Some(v), Some(Some(owner))) = (viewer, self.owner_of.get(system_id)) {
            if v == owner {
                return true;
            }
        }
        if !self.is_engineering(viewer) {
            return false;
        }
        // Engineering: Core always, non-Core only while a team is on site.
        self.is_core(system_id) || self.on_site.iter().any(|s| s == system_id)
    }

    /// Is `viewer` entitled to exact detail for anything in the `station_id`
    /// bucket?
    ///
    /// The repair *queue* is deduped per station, but the information boundary
    /// is per system, so entitlement for a bucket is "entitled to at least one
    /// system in it". Deliberately implemented by asking [`Self::can_see`] about
    /// each member system rather than by re-deriving the rule, so the queue
    /// preview and the hull rows cannot drift apart.
    pub fn can_see_group(&self, viewer: Option<&StationId>, group: &RepairGroup) -> bool {
        self.entries.iter().any(|entry| {
            self.group_of.get(&entry.system_id) == Some(group)
                && self.can_see(viewer, &entry.system_id)
        })
    }

    /// May `viewer` receive the repair blackboard *at all*?
    ///
    /// The repair blackboard is Engineering's console payload and nothing else
    /// renders it (`buildRepairConsoleState` is only reached from a station that
    /// owns the `repair` system). Restricting the audience here is what stops
    /// its non-hull fields — `teams`, which names each team's dispatch target,
    /// and `queue_depth` — from reaching stations that have no use for them.
    pub fn may_receive_repair_blackboard(&self, viewer: Option<&StationId>) -> bool {
        self.is_engineering(viewer)
    }

    /// The exact-detail rows `viewer` is entitled to, in authoritative order.
    pub fn entries_for(&self, viewer: Option<&StationId>) -> Vec<SystemHullStatus> {
        self.entries
            .iter()
            .filter(|e| self.can_see(viewer, &e.system_id))
            .cloned()
            .collect()
    }

    /// The full projection (visible rows + the ship-wide aggregate) for `viewer`.
    pub fn projection_for(&self, viewer: Option<&StationId>) -> HullProjection {
        HullProjection {
            entries: self.entries_for(viewer),
            aggregate_fraction: self.aggregate_fraction(),
            destroyed_fraction: self.destroyed_fraction(),
        }
    }

    /// Rewrite a repair blackboard into `viewer`'s projection.
    ///
    /// **Every field is written out explicitly, and that is deliberate.** The
    /// first cut of this used `..bb.clone()` for "the rest", which silently
    /// exempted `queue_depth` and `teams` from the boundary the function exists
    /// to enforce — `queue_depth` in particular carried the exact tier and HP
    /// deficit of every *damaged* system, which is the detail that actually
    /// matters. A struct-update spread here means the next field added to
    /// `RepairBlackboard` leaks by default; naming each field means it fails to
    /// compile until someone decides. Do not reintroduce the spread.
    ///
    /// Field by field:
    ///
    /// | Field | Treatment | Why |
    /// |---|---|---|
    /// | `system_hull` | projected via [`Self::entries_for`] | exact per-system hull |
    /// | `queue_depth` | projected via [`Self::can_see_group`] | exact tier + HP deficit, scoped to damaged systems |
    /// | `aggregate_hull_fraction` | recomputed ship-wide | the one whole-ship figure everyone may have |
    /// | `destroyed_hull_fraction` | recomputed ship-wide | a second whole-ship scalar; a reduction over every system names none of them |
    /// | `damageable_systems` | whole | system ids only, no hull detail; Engineering dispatches to systems it cannot see |
    /// | `priority_targets` | filtered by `can_see` | exact live sweep eligibility only for rows this viewer may see |
    /// | `teams` | whole | this viewer's *own* teams — where it already chose to send them, not a fact about the ship |
    /// | `travel_duration_secs` | whole | a ship constant from `[repair]` TOML |
    ///
    /// `teams` is only sound as-is because the caller restricts this payload to
    /// the Engineering holder ([`Self::may_receive_repair_blackboard`]); it
    /// names each team's destination system, which is a leak to anyone else.
    pub fn project_repair_blackboard(
        &self,
        viewer: Option<&StationId>,
        bb: &RepairBlackboard,
    ) -> RepairBlackboard {
        RepairBlackboard {
            planned_requests: bb.planned_requests.clone(),
            personnel: bb.personnel.clone(),
            boundaries: bb.boundaries.clone(),
            interior: bb.interior.clone(),
            navigation: bb.navigation.clone(),
            system_hull: self.entries_for(viewer),
            queue_depth: self.queue_entries_for(viewer, &bb.queue_depth),
            aggregate_hull_fraction: self.aggregate_fraction(),
            destroyed_hull_fraction: self.destroyed_fraction(),
            damageable_systems: bb.damageable_systems.clone(),
            repair_groups: bb.repair_groups.clone(),
            priority_targets: bb
                .priority_targets
                .iter()
                .filter(|system_id| self.can_see(viewer, system_id))
                .cloned()
                .collect(),
            teams: bb.teams.clone(),
            jobs: bb.jobs.clone(),
            travel_duration_secs: bb.travel_duration_secs,
            // External dispatch (issue #1161): whole-ship scalars/ids revealing
            // no per-system detail, so the Engineering holder this projection is
            // built for sees them unfiltered — the same passthrough
            // `travel_duration_secs` gets.
            external_dispatch_range: bb.external_dispatch_range,
            external_dispatch_target: bb.external_dispatch_target.clone(),
            external_dispatch_target_name: bb.external_dispatch_target_name.clone(),
            external_dispatch_candidate_name: bb.external_dispatch_candidate_name.clone(),
            external_dispatch_candidate_refusal: bb.external_dispatch_candidate_refusal.clone(),
            external_dispatch_refusal: bb.external_dispatch_refusal.clone(),
            // Which of this viewer's OWN teams went abroad, and how the target
            // it is working is doing (issue #1386). Passed through on the same
            // reading `teams` is: a seat that already chose where to send its
            // teams learns nothing new from being told which one it picked, and
            // the target's condition is a single whole-target scalar naming no
            // system of anyone's.
            external_dispatch_team_idx: bb.external_dispatch_team_idx,
            external_dispatch_target_condition: bb.external_dispatch_target_condition,
        }
    }

    /// Projection shared by a Repair Console and Consoles explicitly bound to
    /// its publication. Hosting changes neither team visibility nor hull detail.
    pub fn project_repair_for_console(
        &self,
        config: &crate::ship::config::ShipConfig,
        console: &phoenix_model::identity::ConsoleId,
        raw: &RepairBlackboard,
    ) -> RepairBlackboard {
        let can_see = |id: &SystemId| {
            config
                .system(id)
                .is_some_and(|system| system.console.as_ref() == Some(console))
                || self.is_core(id)
                || self.on_site.contains(id)
        };
        // Preserve the existing explicit field policy and replace its three
        // detailed collections with the equivalent Console-scoped predicate.
        let mut projected = self.project_repair_blackboard(None, raw);
        projected.system_hull = self
            .entries
            .iter()
            .filter(|entry| can_see(&entry.system_id))
            .cloned()
            .collect();
        projected.priority_targets = raw
            .priority_targets
            .iter()
            .filter(|id| can_see(id))
            .cloned()
            .collect();
        projected.queue_depth = raw
            .queue_depth
            .iter()
            .filter(|entry| {
                self.entries.iter().any(|hull| {
                    self.group_of.get(&hull.system_id) == Some(&entry.group)
                        && can_see(&hull.system_id)
                })
            })
            .map(|entry| filter_queue_observations(entry, can_see))
            .collect();
        projected
    }

    /// The queue preview rows `viewer` is entitled to, in the order given.
    pub fn queue_entries_for(
        &self,
        viewer: Option<&StationId>,
        queue: &[QueueEntryPreview],
    ) -> Vec<QueueEntryPreview> {
        queue
            .iter()
            .filter(|e| self.can_see_group(viewer, &e.group))
            .map(|entry| filter_queue_observations(entry, |id| self.can_see(viewer, id)))
            .collect()
    }
}

pub(crate) fn physical_system_status(
    damage: &crate::ship::compartment_damage::ShipDamage,
    id: &SystemId,
    _config: &ShipConfig,
) -> Option<SystemHullStatus> {
    use phoenix_model::damage::{DamageBinding, PhysicalComponentStatus, PhysicalSystemStatus};
    let compartment = damage.definition.interior.placements.get(id)?;
    let room = damage.definition.compartment(compartment)?;
    let room_state = damage.state.compartments.get(compartment)?;
    let components = damage
        .definition
        .components
        .get(id)?
        .iter()
        .map(|(name, component)| {
            Some(PhysicalComponentStatus {
                id: name.clone(),
                binding: component.binding,
                condition: damage
                    .state
                    .components
                    .get(id)?
                    .get(name)?
                    .condition
                    .clone(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let factor = damage.factor(id, DamageBinding::Operation);
    Some(SystemHullStatus {
        system_id: id.clone(),
        display_name: id.0.clone(),
        current: 0.0,
        max_hp: 0.0,
        tier: damage.operating_tier(id),
        debuff_magnitude: 1.0 - factor,
        physical: Some(PhysicalSystemStatus {
            compartment: compartment.clone(),
            structural_hp: room_state.hp as f32,
            structural_max_hp: room.max_hp,
            structurally_disabled: room_state.disabled,
            components,
        }),
    })
}

/// The repair blackboard a viewer who is not entitled to it receives.
///
/// Not "no message": a player who *was* Engineering and moved elsewhere has the
/// previous payload cached on their phone, so they are sent one empty
/// blackboard to overwrite it. Every detail-bearing field is empty; the ship
/// constant is kept so the console does not render a nonsense travel bar if it
/// is ever shown again.
fn withheld_repair_blackboard(bb: &RepairBlackboard) -> RepairBlackboard {
    RepairBlackboard {
        planned_requests: vec![],
        personnel: None,
        boundaries: vec![],
        interior: None,
        navigation: vec![],
        teams: vec![],
        jobs: vec![],
        travel_duration_secs: bb.travel_duration_secs,
        system_hull: vec![],
        damageable_systems: vec![],
        repair_groups: vec![],
        priority_targets: vec![],
        queue_depth: vec![],
        aggregate_hull_fraction: None,
        destroyed_hull_fraction: None,
        // A recipient not entitled to the repair blackboard is not the
        // Engineering holder, so it renders no dispatch control (issue #1161);
        // the dispatched target and refusal are Engineering's working state and
        // are cleared with the rest.
        external_dispatch_range: None,
        external_dispatch_target: None,
        external_dispatch_target_name: None,
        external_dispatch_candidate_name: None,
        external_dispatch_candidate_refusal: None,
        external_dispatch_refusal: None,
        external_dispatch_team_idx: None,
        external_dispatch_target_condition: None,
    }
}

// ── Bevy adapters ─────────────────────────────────────────────────────────────

/// Build a [`HullVisibility`] from one ship's already-borrowed parts.
///
/// **The single on-site resolution path.** Two places decide how much damage
/// detail a recipient may have — the broadcast/resync projection below, and the
/// `CoordinationPopup` gate in `ship_plugin` — and both go through here, so the
/// rule that resolves the on-site set cannot drift between them. That drift is
/// exactly the class of bug #737 exists to close: a second door onto the same
/// numbers, gated by a slightly different rule.
///
/// `entity_teams` is the per-entity `ShipRepairTeams` component and is
/// authoritative (the per-entity path landed in #590). `local_ship_teams` is
/// the global `ShipRepairTeams` resource, and callers must pass it **only for
/// the `LocalShip`** — it is the player ship's singleton, so an NPC carrying no
/// component must not inherit it. Same preference order as
/// `publish_repair_blackboard`.
pub fn ship_hull_visibility(
    hull: &SystemHull,
    config: &ShipConfig,
    entity_teams: Option<&super::server::ShipRepairTeams>,
) -> HullVisibility {
    HullVisibility::from_parts(hull, config, entity_teams.map(|t| &t.0))
}

/// Build a [`HullVisibility`] for the `LocalShip`, or `None` before spawn.
pub fn hull_visibility(world: &mut World) -> Option<HullVisibility> {
    let mut state = bevy::ecs::system::SystemState::<HullProjectionInputs>::new(world);
    state.get(world).visibility()
}

/// Every connected session token paired with the station it currently holds.
fn viewers(world: &World) -> Vec<(String, Option<StationId>)> {
    world
        .get_resource::<Sessions>()
        .map(|s| {
            s.0.players()
                .iter()
                .filter(|p| p.connected)
                .map(|p| (p.token.clone(), p.station.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Push a per-recipient `SystemHullUpdate` to every connected token whose
/// *visible* detail changed since the last send.
///
/// Replaces the pre-#737 single `Target::All` push. The cache is keyed by token
/// (see [`LastBroadcastHull`]) because the projection
/// can change without the hull changing — a repair team arriving, or a player
/// moving station, both alter what a given recipient may see.
pub fn push_hull_updates(world: &mut World) {
    use crate::server_app::SimOutbox;

    let viewers = viewers(world);
    let projections: Vec<_> = {
        let mut state = bevy::ecs::system::SystemState::<HullProjectionInputs>::new(world);
        let inputs = state.get(world);
        viewers
            .iter()
            .filter_map(|(token, station)| {
                inputs
                    .projection_for(station.as_ref())
                    .map(|projection| (token.clone(), projection))
            })
            .collect()
    };

    let mut pending: Vec<(Target, ServerMessage)> = Vec::new();
    let mut next: HashMap<String, HullProjection> = HashMap::new();
    {
        let last = world.resource::<LastBroadcastHull>();
        for (token, projection) in projections {
            if last.0.get(&token) != Some(&projection) {
                pending.push((
                    Target::Token(token.clone()),
                    ServerMessage::SystemHullUpdate {
                        entries: projection.entries.clone(),
                        aggregate_fraction: projection.aggregate_fraction,
                        destroyed_fraction: projection.destroyed_fraction,
                    },
                ));
            }
            next.insert(token.clone(), projection);
        }
    }

    // Replace wholesale so tokens that disconnected drop out of the cache
    // instead of accumulating forever.
    world.resource_mut::<LastBroadcastHull>().0 = next;
    if !pending.is_empty() {
        world.resource_mut::<SimOutbox>().extend_snapshot(pending);
    }
}

/// Read the connected-viewer list for the blackboard broadcaster.
pub fn connected_viewers(world: &World) -> Vec<(String, Option<StationId>)> {
    viewers(world)
}

// ── Reconnect resync — same projection, different trigger ─────────────────────

/// The `SystemHullUpdate` a reconnecting token is entitled to.
///
/// Deliberately shares [`HullVisibility::projection_for`] with the live path so
/// reconnecting cannot be used to obtain detail the live broadcast withholds.
/// Does not touch the delta cache — same rule as the other resync payloads.
pub fn hull_update_for_token(world: &mut World, token: &str) -> Option<ServerMessage> {
    let station = world
        .get_resource::<Sessions>()
        .and_then(|s| s.0.station_for_token(token).cloned());
    let mut state = bevy::ecs::system::SystemState::<HullProjectionInputs>::new(world);
    let projection = state.get(world).projection_for(station.as_ref())?;
    Some(ServerMessage::SystemHullUpdate {
        entries: projection.entries,
        aggregate_fraction: projection.aggregate_fraction,
        destroyed_fraction: projection.destroyed_fraction,
    })
}

/// Project one blackboard for a reconnecting token. Non-repair blackboards pass
/// through untouched; the repair blackboard is filtered exactly as it is live —
/// same audience test, same field projection — so reconnecting cannot be used to
/// obtain anything the live broadcast withholds.
pub fn project_blackboard_for_token(
    vis: Option<&HullVisibility>,
    station: Option<&StationId>,
    bb: &SystemBlackboard,
) -> SystemBlackboard {
    let SystemBlackboard::Repair(raw) = bb else {
        return bb.clone();
    };
    match vis {
        Some(v) if v.may_receive_repair_blackboard(station) => {
            SystemBlackboard::Repair(v.project_repair_blackboard(station, raw))
        }
        _ => SystemBlackboard::Repair(withheld_repair_blackboard(raw)),
    }
}

#[cfg(test)]
#[path = "visibility_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "console_visibility_tests.rs"]
mod console_tests;

fn filter_queue_observations(
    entry: &QueueEntryPreview,
    can_see: impl Fn(&SystemId) -> bool,
) -> QueueEntryPreview {
    let mut projected = entry.clone();
    projected.observations.retain(|observation| {
        let subject_allowed = match &observation.subject {
            phoenix_model::observation::ObservationSubject::System { id } => can_see(id),
            _ => true,
        };
        subject_allowed && observation.provenance.system.as_ref().is_none_or(&can_see)
    });
    projected
}
