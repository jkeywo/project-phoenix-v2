//! Host-authoritative damage **visibility projection** (issue #737).
//!
//! This module is the single file named by the PASM entity
//! `repair-visibility-publisher`. It exists as its own host-only module so the
//! projection is a real, observed code edge — the publisher's dependency on the
//! repair-team state machine is the line-start
//! `use crate::modifiers::repair_teams::RepairTeams;` below, and its dependency
//! on the authoritative damage store is `use crate::damage::SystemHull;`.
//!
//! # Why a projection exists at all
//!
//! Before #737 every connected client received exact per-system hull detail for
//! the *whole* ship (`SystemHullUpdate { entries }` at `Target::All`, and
//! `RepairBlackboard.system_hull` inside a `Target::All` `BlackboardUpdate`).
//! Role separation was presentation-only in the client, which means the hidden
//! rows were already on every phone. The four view states below are therefore
//! decided **here, on the host**, and each recipient is sent only what it is
//! entitled to see:
//!
//! | View state | Who sees it | Rule |
//! |---|---|---|
//! | aggregate hull | everyone | ship-wide fraction over *every* damageable system |
//! | Core detail | the Engineering holder | hull entries no station owns |
//! | station-owner detail | that station's holder | hull entries its station owns |
//! | on-site detail | the Engineering holder | non-Core entries with a team *on site* |
//!
//! "Core" is an ownerless bucket, not a flag: a hull entry whose `system_id`
//! has no `[[system]]` declaration carrying a `station` belongs to Core. That
//! is the same rule the client used to apply locally, lifted to the host.
//!
//! # Two gates, not one
//!
//! Filtering *rows* is only half of it. The `RepairBlackboard` also carries
//! `queue_depth` (every damaged system's exact tier and HP deficit) and `teams`
//! (each team's destination system). Those are Engineering's working state, and
//! fanning the blackboard out to every connected token made the row filter
//! cosmetic wherever a system was damaged enough to be queued — which is
//! precisely the case that matters. So there are two gates:
//!
//! 1. **Audience** — [`HullVisibility::may_receive_repair_blackboard`] sends the
//!    repair blackboard to the Engineering holder alone. Everyone else is sent
//!    an empty one, which also clears a stale copy off the phone of a player who
//!    has just moved off Engineering.
//! 2. **Contents** — [`HullVisibility::project_repair_blackboard`] then projects
//!    the fields that carry exact detail, field by field and never with a
//!    struct-update spread.
//!
//! The same two gates apply to `SystemHullUpdate` (which has no audience gate —
//! every station needs its own rows) and to the reconnect resync.
//!
//! # One code path, two callers
//!
//! [`HullVisibility`] is pure and Bevy-free once constructed. Both the live
//! broadcast ([`push_hull_updates`], [`project_repair_blackboards`]) and the
//! reconnect resync ([`hull_update_for_token`], [`project_blackboard_for_token`])
//! build the same struct and call the same [`HullVisibility::entries_for`], so a
//! reconnecting client cannot be handed detail the live path withholds.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::damage::SystemHull;
use crate::lobby::handler::Target;
use crate::lobby::Sessions;
use crate::messages::{
    QueueEntryPreview, RepairBlackboard, ServerMessage, StationId, SystemBlackboard,
    SystemHullStatus, SystemId,
};
use crate::modifiers::repair_teams::RepairTeams;
use crate::ship::config::ShipConfig;
use crate::ship::system_registry::repair_system_id;

/// The bucket id used for hull entries that no station owns.
///
/// "Core" is an ownerless bucket rather than a declared station — `ShipConfig`
/// validation actively forbids a station with this id. It is named here so the
/// tier-crossing enqueue in `ship_plugin` and the queue-entry projection below
/// agree on one spelling.
pub const CORE_BUCKET_ID: &str = "core";

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
}

/// Last-sent repair blackboard projection per session token.
///
/// `LastBroadcastBlackboards` caches the *internal* (unprojected) blackboard
/// and answers "did anything change at all". This answers "did **this
/// recipient's** view change", which is the question that matters once the
/// wire payload differs per token.
///
/// `stations` records which station each token held when its projection was
/// computed. The internal blackboard is not the only input to a projection —
/// the recipient's station is the other one — so a player moving Engineering →
/// Helm changes what they may see without changing anything the
/// `LastBroadcastBlackboards` diff can observe. On an idle, undamaged ship the
/// internal blackboard may never change again, which would leave the stale core
/// detail on that phone indefinitely. [`Self::stations_changed`] is the trigger
/// that closes that window.
#[derive(Resource, Default)]
pub struct LastVisibleRepairBlackboard {
    pub projections: HashMap<String, RepairBlackboard>,
    pub stations: HashMap<String, Option<StationId>>,
}

impl LastVisibleRepairBlackboard {
    /// True when any connected token holds a different station than it did when
    /// its cached projection was computed (including a token seen for the first
    /// time).
    pub fn stations_changed(&self, viewers: &[(String, Option<StationId>)]) -> bool {
        viewers
            .iter()
            .any(|(token, station)| self.stations.get(token) != Some(station))
    }

    /// Record the station each connected token currently holds.
    pub fn record_stations(&mut self, viewers: &[(String, Option<StationId>)]) {
        self.stations = viewers.iter().cloned().collect();
    }

    /// Forget everything — used when the game restarts.
    pub fn clear(&mut self) {
        self.projections.clear();
        self.stations.clear();
    }
}

// ── The projection itself (pure) ───────────────────────────────────────────────

/// A resolved snapshot of everything needed to decide who may see which system.
///
/// Built once per broadcast tick, then queried per recipient.
#[derive(Clone, Debug)]
pub struct HullVisibility {
    /// Every damageable system, exact detail. Never sent as-is.
    entries: Vec<SystemHullStatus>,
    /// Owning station per system id; `None` means ownerless — the Core bucket.
    owner_of: HashMap<SystemId, Option<StationId>>,
    /// The station that holds the `repair` system — "Engineering" in the
    /// role sense. Resolved from ship config, never assumed by name (it is
    /// `repair` on the battleship and `engineering` on cruiser/destroyer).
    engineering_station: Option<StationId>,
    /// Systems with a repair team physically present and working.
    on_site: Vec<SystemId>,
}

impl HullVisibility {
    /// Build from already-resolved parts. Pure — used directly by tests.
    pub fn new(
        entries: Vec<SystemHullStatus>,
        owner_of: HashMap<SystemId, Option<StationId>>,
        engineering_station: Option<StationId>,
        on_site: Vec<SystemId>,
    ) -> Self {
        Self {
            entries,
            owner_of,
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
                let owner = config.system(&e.system_id).and_then(|s| s.station.clone());
                (e.system_id.clone(), owner)
            })
            .collect();

        let engineering_station = config
            .system(&repair_system_id())
            .and_then(|s| s.station.clone());

        // `on_site_systems` is the named predicate on `RepairTeams`: only
        // `Repairing` counts. A team still `Travelling` has not arrived, and a
        // team `Returning` has left — including a team recalled before arrival,
        // which goes `Travelling -> Returning` without ever passing through
        // `Repairing` and therefore never reveals anything.
        let on_site = teams
            .map(|t| t.on_site_systems().cloned().collect())
            .unwrap_or_default();

        Self::new(entries, owner_of, engineering_station, on_site)
    }

    /// Ship-wide hull fraction (0.0–1.0) across **every** damageable system.
    ///
    /// This is the only whole-ship figure a recipient may be given, because a
    /// projected `entries` list can no longer be summed into one. Returns
    /// `None` when the ship declares no damageable systems.
    pub fn aggregate_fraction(&self) -> Option<f32> {
        let max: f32 = self.entries.iter().map(|e| e.max_hp).sum();
        if max <= 0.0 {
            return None;
        }
        let current: f32 = self.entries.iter().map(|e| e.current).sum();
        Some((current / max).clamp(0.0, 1.0))
    }

    /// True when a hull entry has no owning station — the Core bucket.
    fn is_core(&self, system_id: &SystemId) -> bool {
        matches!(self.owner_of.get(system_id), Some(None) | None)
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
    pub fn can_see_station(&self, viewer: Option<&StationId>, station_id: &str) -> bool {
        self.entries.iter().any(|e| {
            let bucket = match self.owner_of.get(&e.system_id) {
                Some(Some(owner)) => owner.0.as_str(),
                _ => CORE_BUCKET_ID,
            };
            bucket == station_id && self.can_see(viewer, &e.system_id)
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
    /// | `queue_depth` | projected via [`Self::can_see_station`] | exact tier + HP deficit, scoped to damaged systems |
    /// | `aggregate_hull_fraction` | recomputed ship-wide | the one whole-ship figure everyone may have |
    /// | `damageable_systems` | whole | system ids only, no hull detail; Engineering dispatches to systems it cannot see |
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
            system_hull: self.entries_for(viewer),
            queue_depth: self.queue_entries_for(viewer, &bb.queue_depth),
            aggregate_hull_fraction: self.aggregate_fraction(),
            damageable_systems: bb.damageable_systems.clone(),
            teams: bb.teams.clone(),
            travel_duration_secs: bb.travel_duration_secs,
        }
    }

    /// The queue preview rows `viewer` is entitled to, in the order given.
    pub fn queue_entries_for(
        &self,
        viewer: Option<&StationId>,
        queue: &[QueueEntryPreview],
    ) -> Vec<QueueEntryPreview> {
        queue
            .iter()
            .filter(|e| self.can_see_station(viewer, &e.station_id))
            .cloned()
            .collect()
    }
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
        teams: vec![],
        travel_duration_secs: bb.travel_duration_secs,
        system_hull: vec![],
        damageable_systems: vec![],
        queue_depth: vec![],
        aggregate_hull_fraction: None,
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
    local_ship_teams: Option<&super::server::ShipRepairTeams>,
) -> HullVisibility {
    HullVisibility::from_parts(
        hull,
        config,
        entity_teams.or(local_ship_teams).map(|t| &t.0),
    )
}

/// Build a [`HullVisibility`] for the `LocalShip`, or `None` before spawn.
pub fn hull_visibility(world: &mut World) -> Option<HullVisibility> {
    use crate::entity_spawner::EntitySystemHull;
    use crate::server_app::LocalShip;
    use crate::ship_plugin::ShipConfigComponent;

    let mut q = world.query_filtered::<(
        &EntitySystemHull,
        &ShipConfigComponent,
        Option<&super::server::ShipRepairTeams>,
    ), With<LocalShip>>();
    // Clone out of the query so the `world` borrow is released before reading
    // the fallback resource.
    let (hull, config, entity_teams) = {
        let (hull, config, teams) = q.iter(world).next()?;
        (hull.0.clone(), config.0.clone(), teams.cloned())
    };

    // This *is* the LocalShip, so the resource fallback applies here.
    let resource_teams = world
        .get_resource::<super::server::ShipRepairTeams>()
        .cloned();
    Some(ship_hull_visibility(
        &hull,
        &config,
        entity_teams.as_ref(),
        resource_teams.as_ref(),
    ))
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
/// (see [`crate::core::broadcast::LastBroadcastHull`]) because the projection
/// can change without the hull changing — a repair team arriving, or a player
/// moving station, both alter what a given recipient may see.
pub fn push_hull_updates(world: &mut World) {
    use crate::core::broadcast::LastBroadcastHull;
    use crate::simulation::SimOutbox;

    let Some(vis) = hull_visibility(world) else {
        return;
    };
    let viewers = viewers(world);

    let mut pending: Vec<(Target, ServerMessage)> = Vec::new();
    let mut next: HashMap<String, HullProjection> = HashMap::new();
    {
        let last = world.resource::<LastBroadcastHull>();
        for (token, station) in &viewers {
            let projection = vis.projection_for(station.as_ref());
            if last.0.get(token) != Some(&projection) {
                pending.push((
                    Target::Token(token.clone()),
                    ServerMessage::SystemHullUpdate {
                        entries: projection.entries.clone(),
                        aggregate_fraction: projection.aggregate_fraction,
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
        world.resource_mut::<SimOutbox>().0.extend(pending);
    }
}

/// Split a batch of changed blackboards into the unprojected ones (broadcast to
/// all, unchanged behaviour) and per-recipient repair projections.
///
/// Returns the outbox entries to push. The repair blackboard is the only one
/// carrying exact hull detail, so it is the only one fanned out per token.
pub fn project_repair_blackboards(
    updates: Vec<(SystemId, SystemBlackboard)>,
    vis: Option<&HullVisibility>,
    viewers: &[(String, Option<StationId>)],
    last: &mut LastVisibleRepairBlackboard,
) -> Vec<(Target, ServerMessage)> {
    let mut out = Vec::new();

    let (repair, shared): (Vec<_>, Vec<_>) = updates
        .into_iter()
        .partition(|(_, bb)| matches!(bb, SystemBlackboard::Repair(_)));

    if !shared.is_empty() {
        out.push((
            Target::All,
            ServerMessage::BlackboardUpdate { updates: shared },
        ));
    }

    for (system_id, bb) in repair {
        let SystemBlackboard::Repair(raw) = bb else {
            continue;
        };
        for (token, station) in viewers {
            // Audience first, contents second. The repair blackboard is the
            // Engineering console's payload — no other console reads it — and
            // its `teams` / `queue_depth` fields describe dispatch targets and
            // damaged-system severity, which are exactly what #737 withholds.
            // Anyone else gets the empty blackboard so a stale copy from a
            // previous station cannot linger on their phone.
            let entitled = vis
                .map(|v| v.may_receive_repair_blackboard(station.as_ref()))
                .unwrap_or(false);
            let projected = match (vis, entitled) {
                (Some(v), true) => v.project_repair_blackboard(station.as_ref(), &raw),
                // Not Engineering, or no LocalShip resolved to decide with:
                // withhold rather than fall back to the unprojected blackboard.
                _ => withheld_repair_blackboard(&raw),
            };
            if last.projections.get(token) == Some(&projected) {
                continue;
            }
            last.projections.insert(token.clone(), projected.clone());
            out.push((
                Target::Token(token.clone()),
                ServerMessage::BlackboardUpdate {
                    updates: vec![(system_id.clone(), SystemBlackboard::Repair(projected))],
                },
            ));
        }
    }

    out
}

/// Drop cached projections for tokens that are no longer connected.
pub fn prune_repair_blackboard_cache(
    last: &mut LastVisibleRepairBlackboard,
    viewers: &[(String, Option<StationId>)],
) {
    last.projections
        .retain(|token, _| viewers.iter().any(|(t, _)| t == token));
    last.stations
        .retain(|token, _| viewers.iter().any(|(t, _)| t == token));
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
    let vis = hull_visibility(world)?;
    let station = world
        .get_resource::<Sessions>()
        .and_then(|s| s.0.station_for_token(token).cloned());
    let projection = vis.projection_for(station.as_ref());
    Some(ServerMessage::SystemHullUpdate {
        entries: projection.entries,
        aggregate_fraction: projection.aggregate_fraction,
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
mod tests {
    use super::*;
    use crate::damage::DamageTier;

    fn status(id: &str, current: f32) -> SystemHullStatus {
        SystemHullStatus {
            system_id: SystemId(id.into()),
            display_name: id.into(),
            current,
            max_hp: 100.0,
            tier: DamageTier::Operational,
            debuff_magnitude: 0.0,
        }
    }

    /// helm-radar -> helm, sensors -> science, repair -> engineering,
    /// "core" -> ownerless.
    fn vis(on_site: Vec<&str>) -> HullVisibility {
        let entries = vec![
            status("core", 40.0),
            status("helm-radar", 60.0),
            status("sensors", 100.0),
            status("repair", 100.0),
        ];
        let owner_of = [
            (SystemId("core".into()), None),
            (
                SystemId("helm-radar".into()),
                Some(StationId("helm".into())),
            ),
            (
                SystemId("sensors".into()),
                Some(StationId("science".into())),
            ),
            (
                SystemId("repair".into()),
                Some(StationId("engineering".into())),
            ),
        ]
        .into_iter()
        .collect();
        HullVisibility::new(
            entries,
            owner_of,
            Some(StationId("engineering".into())),
            on_site.into_iter().map(|s| SystemId(s.into())).collect(),
        )
    }

    fn ids(rows: &[SystemHullStatus]) -> Vec<&str> {
        rows.iter().map(|r| r.system_id.0.as_str()).collect()
    }

    #[test]
    fn engineering_sees_core_and_its_own_systems_but_no_other_detail() {
        let v = vis(vec![]);
        let eng = StationId("engineering".into());
        assert_eq!(ids(&v.entries_for(Some(&eng))), vec!["core", "repair"]);
    }

    #[test]
    fn station_owner_sees_only_its_own_systems() {
        let v = vis(vec![]);
        let helm = StationId("helm".into());
        assert_eq!(ids(&v.entries_for(Some(&helm))), vec!["helm-radar"]);
        let science = StationId("science".into());
        assert_eq!(ids(&v.entries_for(Some(&science))), vec!["sensors"]);
    }

    #[test]
    fn station_owner_never_sees_core() {
        let v = vis(vec![]);
        let helm = StationId("helm".into());
        assert!(!v.can_see(Some(&helm), &SystemId("core".into())));
    }

    #[test]
    fn on_site_team_reveals_non_core_detail_to_engineering_only() {
        let v = vis(vec!["helm-radar"]);
        let eng = StationId("engineering".into());
        assert_eq!(
            ids(&v.entries_for(Some(&eng))),
            vec!["core", "helm-radar", "repair"]
        );
        // The reveal is Engineering-scoped: Science still sees only its own.
        let science = StationId("science".into());
        assert_eq!(ids(&v.entries_for(Some(&science))), vec!["sensors"]);
    }

    #[test]
    fn unassigned_viewer_sees_no_detail_but_still_gets_the_aggregate() {
        let v = vis(vec![]);
        let p = v.projection_for(None);
        assert!(p.entries.is_empty());
        assert!(p.aggregate_fraction.is_some());
    }

    #[test]
    fn aggregate_covers_every_system_including_ones_the_viewer_cannot_see() {
        let v = vis(vec![]);
        // 40 + 60 + 100 + 100 out of 400.
        let expected = 300.0 / 400.0;
        for viewer in [
            None,
            Some(StationId("helm".into())),
            Some(StationId("engineering".into())),
        ] {
            let p = v.projection_for(viewer.as_ref());
            assert!((p.aggregate_fraction.unwrap() - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn blackboard_projection_filters_hull_but_keeps_dispatch_targets() {
        let v = vis(vec![]);
        let bb = RepairBlackboard {
            teams: vec![],
            travel_duration_secs: 5.0,
            system_hull: vec![
                status("core", 40.0),
                status("helm-radar", 60.0),
                status("sensors", 100.0),
                status("repair", 100.0),
            ],
            damageable_systems: vec![
                SystemId("core".into()),
                SystemId("helm-radar".into()),
                SystemId("sensors".into()),
                SystemId("repair".into()),
            ],
            queue_depth: vec![],
            aggregate_hull_fraction: None,
        };
        let eng = StationId("engineering".into());
        let projected = v.project_repair_blackboard(Some(&eng), &bb);
        assert_eq!(ids(&projected.system_hull), vec!["core", "repair"]);
        // Dispatch targets are ids only — no hull detail — so they stay whole.
        assert_eq!(projected.damageable_systems.len(), 4);
        assert!(projected.aggregate_hull_fraction.is_some());
        assert_eq!(projected.travel_duration_secs, 5.0);
    }

    // ── queue_depth: the other carrier of exact detail ────────────────────────
    //
    // `queue_depth` is scoped to *damaged* systems and carries each one's exact
    // tier and HP deficit, so leaving it whole made the `system_hull` filter
    // cosmetic precisely where it mattered. These pin it to the same rule.

    fn queue(entries: &[(&str, f32)]) -> Vec<QueueEntryPreview> {
        entries
            .iter()
            .map(|(station, deficit)| QueueEntryPreview {
                station_id: (*station).into(),
                station_label: (*station).into(),
                tier: DamageTier::Damaged,
                deficit: *deficit,
            })
            .collect()
    }

    fn bb_with_queue(queue: Vec<QueueEntryPreview>) -> RepairBlackboard {
        RepairBlackboard {
            teams: vec![],
            travel_duration_secs: 5.0,
            system_hull: vec![status("core", 40.0), status("helm-radar", 60.0)],
            damageable_systems: vec![SystemId("core".into()), SystemId("helm-radar".into())],
            queue_depth: queue,
            aggregate_hull_fraction: None,
        }
    }

    fn queued_stations(bb: &RepairBlackboard) -> Vec<&str> {
        bb.queue_depth
            .iter()
            .map(|e| e.station_id.as_str())
            .collect()
    }

    #[test]
    fn a_station_owner_gets_no_queue_entry_for_a_station_it_does_not_own() {
        let v = vis(vec![]);
        let bb = bb_with_queue(queue(&[("core", 60.0), ("helm", 40.0), ("science", 10.0)]));
        let helm = StationId("helm".into());
        let projected = v.project_repair_blackboard(Some(&helm), &bb);
        assert_eq!(
            queued_stations(&projected),
            vec!["helm"],
            "Helm must not learn the exact tier or HP deficit of core or science"
        );
    }

    #[test]
    fn an_unassigned_viewer_gets_no_queue_entries_at_all() {
        let v = vis(vec![]);
        let bb = bb_with_queue(queue(&[("core", 60.0), ("helm", 40.0)]));
        assert!(v
            .project_repair_blackboard(None, &bb)
            .queue_depth
            .is_empty());
    }

    #[test]
    fn engineering_gets_no_non_core_queue_entry_before_a_team_arrives() {
        let v = vis(vec![]);
        let bb = bb_with_queue(queue(&[("core", 60.0), ("helm", 40.0), ("science", 10.0)]));
        let eng = StationId("engineering".into());
        let projected = v.project_repair_blackboard(Some(&eng), &bb);
        assert_eq!(
            queued_stations(&projected),
            vec!["core"],
            "Engineering may queue-preview Core (and its own) only until a team is on site"
        );
    }

    #[test]
    fn engineering_gets_the_non_core_queue_entry_once_a_team_is_on_site() {
        // helm-radar is a helm-owned system with a team on site.
        let v = vis(vec!["helm-radar"]);
        let bb = bb_with_queue(queue(&[("core", 60.0), ("helm", 40.0), ("science", 10.0)]));
        let eng = StationId("engineering".into());
        let projected = v.project_repair_blackboard(Some(&eng), &bb);
        assert_eq!(queued_stations(&projected), vec!["core", "helm"]);
        assert!(
            !queued_stations(&projected).contains(&"science"),
            "the reveal is per-station, not a blanket unlock"
        );
    }

    #[test]
    fn the_projection_names_every_field_so_none_can_ride_through_unprojected() {
        // A guard against reintroducing `..bb.clone()`. If a new detail-bearing
        // field is added and left unprojected, it shows up here as a field this
        // assertion does not account for and the test needs revisiting.
        let v = vis(vec![]);
        let bb = bb_with_queue(queue(&[("helm", 40.0)]));
        let eng = StationId("engineering".into());
        let p = v.project_repair_blackboard(Some(&eng), &bb);
        // Projected:
        assert_eq!(ids(&p.system_hull), vec!["core", "repair"]);
        assert!(p.queue_depth.is_empty());
        assert!(p.aggregate_hull_fraction.is_some());
        // Deliberately whole:
        assert_eq!(p.damageable_systems, bb.damageable_systems);
        assert_eq!(p.teams, bb.teams);
        assert_eq!(p.travel_duration_secs, bb.travel_duration_secs);
    }

    #[test]
    fn only_the_engineering_holder_receives_the_repair_blackboard() {
        let v = vis(vec![]);
        assert!(v.may_receive_repair_blackboard(Some(&StationId("engineering".into()))));
        for other in ["helm", "science", "captain"] {
            assert!(
                !v.may_receive_repair_blackboard(Some(&StationId(other.into()))),
                "{other} must not be sent the repair blackboard"
            );
        }
        assert!(!v.may_receive_repair_blackboard(None));
    }

    #[test]
    fn a_non_engineering_viewer_is_sent_the_empty_blackboard_not_a_filtered_one() {
        // Withholding has to be an overwrite, not silence: a player who has just
        // moved off Engineering still has the previous payload on their phone.
        let v = vis(vec![]);
        let bb = SystemBlackboard::Repair(bb_with_queue(queue(&[("core", 60.0)])));
        let helm = Some(StationId("helm".into()));
        let SystemBlackboard::Repair(out) =
            project_blackboard_for_token(Some(&v), helm.as_ref(), &bb)
        else {
            unreachable!()
        };
        assert!(out.system_hull.is_empty());
        assert!(out.queue_depth.is_empty());
        assert!(out.damageable_systems.is_empty());
        assert!(out.teams.is_empty());
        assert!(out.aggregate_hull_fraction.is_none());
    }

    #[test]
    fn a_station_change_invalidates_the_cached_projection() {
        let mut cache = LastVisibleRepairBlackboard::default();
        let viewers = vec![("eng".to_string(), Some(StationId("engineering".into())))];
        assert!(
            cache.stations_changed(&viewers),
            "a token never seen before counts as changed"
        );
        cache.record_stations(&viewers);
        assert!(!cache.stations_changed(&viewers));

        let moved = vec![("eng".to_string(), Some(StationId("helm".into())))];
        assert!(
            cache.stations_changed(&moved),
            "moving station must invalidate the projection even though the \
             internal blackboard did not change"
        );
    }

    // ── AC#3 / AC#4: the on-site gate driven by the real state machine ────────
    //
    // These exercise `RepairTeams` transitions rather than hand-set `on_site`
    // lists, so travel/arrival/recall are asserted against the code that
    // actually moves teams around.

    fn hull_with(entries: &[(&str, f32)]) -> SystemHull {
        SystemHull::from_config(
            &entries
                .iter()
                .map(|(id, hp)| (SystemId((*id).into()), *hp))
                .collect::<Vec<_>>(),
        )
    }

    fn vis_with_teams(teams: &RepairTeams) -> HullVisibility {
        let mut v = vis(vec![]);
        v.on_site = teams.on_site_systems().cloned().collect();
        v
    }

    fn eng() -> StationId {
        StationId("engineering".into())
    }

    #[test]
    fn travelling_team_does_not_reveal_non_core_detail_to_engineering() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_with(&[("helm-radar", 100.0)]);
        hull.set_hp(&SystemId("helm-radar".into()), 40.0);
        teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
        // Part-way through travel: en route is not on site.
        teams.tick(1.0, &mut hull);

        let v = vis_with_teams(&teams);
        assert!(!v.can_see(Some(&eng()), &SystemId("helm-radar".into())));
        assert_eq!(ids(&v.entries_for(Some(&eng()))), vec!["core", "repair"]);
    }

    #[test]
    fn arrival_reveals_non_core_detail_to_engineering() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_with(&[("helm-radar", 100.0)]);
        hull.set_hp(&SystemId("helm-radar".into()), 40.0);
        teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
        assert!(!vis_with_teams(&teams).can_see(Some(&eng()), &SystemId("helm-radar".into())));

        // Travel completes (default travel_duration is 5s) → Repairing.
        teams.tick(6.0, &mut hull);
        let v = vis_with_teams(&teams);
        assert!(
            v.can_see(Some(&eng()), &SystemId("helm-radar".into())),
            "a team on site must reveal exact detail for the system it is at"
        );
        assert_eq!(
            ids(&v.entries_for(Some(&eng()))),
            vec!["core", "helm-radar", "repair"]
        );
    }

    #[test]
    fn recall_before_arrival_never_reveals_detail() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_with(&[("helm-radar", 100.0)]);
        hull.set_hp(&SystemId("helm-radar".into()), 40.0);
        teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
        teams.tick(2.0, &mut hull);
        // Dispatching to the same system while Travelling is a recall.
        teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());

        // Walk the whole return leg: at no point may the detail appear.
        for _ in 0..10 {
            teams.tick(1.0, &mut hull);
            assert!(
                !vis_with_teams(&teams).can_see(Some(&eng()), &SystemId("helm-radar".into())),
                "a recalled team never arrives, so it must never reveal detail"
            );
        }
    }

    #[test]
    fn detail_is_withdrawn_again_once_the_team_leaves() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_with(&[("helm-radar", 100.0)]);
        hull.set_hp(&SystemId("helm-radar".into()), 40.0);
        teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
        teams.tick(6.0, &mut hull);
        assert!(vis_with_teams(&teams).can_see(Some(&eng()), &SystemId("helm-radar".into())));

        // Recall from Repairing → Returning: detail goes away immediately.
        teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
        assert!(
            !vis_with_teams(&teams).can_see(Some(&eng()), &SystemId("helm-radar".into())),
            "detail must be withdrawn the moment the team stops being on site"
        );
    }

    #[test]
    fn on_site_reveal_does_not_extend_to_other_systems_of_that_station() {
        // helm-radar is on site; the visibility grant is per-system, so a
        // second helm-owned system stays hidden from Engineering.
        let mut v = vis(vec!["helm-radar"]);
        v.entries.push(status("helm-thrust", 10.0));
        v.owner_of.insert(
            SystemId("helm-thrust".into()),
            Some(StationId("helm".into())),
        );
        assert!(!v.can_see(Some(&eng()), &SystemId("helm-thrust".into())));
    }

    // ── World-level: the actual wire path, live and on reconnect ─────────────

    /// A minimal but *real* ship config: helm owns `helm-radar`, engineering
    /// owns `repair`, and `core` is declared as a hull entry with no owning
    /// `[[system]]` — exactly the ownerless-bucket shape the shipped hulls use.
    fn world_ship_config() -> ShipConfig {
        ShipConfig::from_toml(
            r#"
[[station]]
id = "helm"
name = "Helm"
description = "Flying."
rank = "Ltn."

[[station]]
id = "engineering"
name = "Engineering"
description = "Fixing."
rank = "Ltn."

[[system]]
id = "helm-radar"
kind = "helm_radar"
station = "helm"

[[system]]
id = "repair"
kind = "repair"
station = "engineering"
"#,
            &["helm_radar", "repair"],
        )
        .expect("test ship config must parse")
    }

    /// Spawn a LocalShip carrying the config/hull/teams the projection reads,
    /// plus two connected sessions: `eng` at Engineering, `pilot` at Helm.
    fn world_app(teams: RepairTeams) -> App {
        use crate::entity_spawner::EntitySystemHull;
        use crate::server_app::LocalShip;
        use crate::ship_plugin::ShipConfigComponent;

        let mut app = App::new();
        app.init_resource::<crate::core::broadcast::LastBroadcastHull>();
        app.init_resource::<crate::simulation::SimOutbox>();

        let mut sessions = crate::lobby::session::SessionManager::new();
        sessions.register("eng".into(), "Bob".into()).unwrap();
        sessions.register("pilot".into(), "Ada".into()).unwrap();
        sessions.set_station("eng", Some(StationId("engineering".into())));
        sessions.set_station("pilot", Some(StationId("helm".into())));
        app.insert_resource(Sessions(sessions));

        let mut hull = hull_with(&[("core", 100.0), ("helm-radar", 100.0), ("repair", 100.0)]);
        hull.set_hp(&SystemId("helm-radar".into()), 30.0);
        hull.set_hp(&SystemId("core".into()), 60.0);

        app.world_mut().spawn((
            crate::simulation::Ship,
            LocalShip,
            ShipConfigComponent(world_ship_config()),
            EntitySystemHull(hull),
            super::super::server::ShipRepairTeams(teams),
        ));
        app
    }

    fn sent_hull(app: &mut App) -> Vec<(String, Vec<String>, Option<f32>)> {
        push_hull_updates(app.world_mut());
        app.world()
            .resource::<crate::simulation::SimOutbox>()
            .0
            .iter()
            .filter_map(|(target, msg)| match (target, msg) {
                (
                    Target::Token(token),
                    ServerMessage::SystemHullUpdate {
                        entries,
                        aggregate_fraction,
                    },
                ) => Some((
                    token.clone(),
                    entries.iter().map(|e| e.system_id.0.clone()).collect(),
                    *aggregate_fraction,
                )),
                _ => None,
            })
            .collect()
    }

    fn rows_for<'a>(
        sent: &'a [(String, Vec<String>, Option<f32>)],
        token: &str,
    ) -> &'a Vec<String> {
        &sent
            .iter()
            .find(|(t, _, _)| t == token)
            .unwrap_or_else(|| panic!("expected a SystemHullUpdate for {token}"))
            .1
    }

    #[test]
    fn live_broadcast_gives_engineering_core_only_with_no_team_on_site() {
        let mut app = world_app(RepairTeams::new(1));
        let sent = sent_hull(&mut app);
        assert_eq!(
            rows_for(&sent, "eng"),
            &vec!["core".to_string(), "repair".to_string()]
        );
        assert!(
            !rows_for(&sent, "eng").contains(&"helm-radar".to_string()),
            "Engineering must not receive non-Core detail with no team on site"
        );
    }

    #[test]
    fn live_broadcast_gives_a_station_owner_only_its_own_systems() {
        let mut app = world_app(RepairTeams::new(1));
        let sent = sent_hull(&mut app);
        assert_eq!(rows_for(&sent, "pilot"), &vec!["helm-radar".to_string()]);
    }

    #[test]
    fn every_recipient_receives_the_same_ship_wide_aggregate() {
        let mut app = world_app(RepairTeams::new(1));
        let sent = sent_hull(&mut app);
        // 60 + 30 + 100 out of 300 — spans systems no single recipient sees.
        let expected = 190.0 / 300.0;
        for (token, _, aggregate) in &sent {
            let got = aggregate.unwrap_or_else(|| panic!("{token} got no aggregate"));
            assert!((got - expected).abs() < 1e-6, "{token} aggregate {got}");
        }
    }

    #[test]
    fn live_broadcast_reveals_non_core_detail_once_a_team_is_on_site() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_with(&[("helm-radar", 100.0)]);
        hull.set_hp(&SystemId("helm-radar".into()), 30.0);
        teams.dispatch(0, SystemId("helm-radar".into()), "Radar".into());
        teams.tick(6.0, &mut hull); // arrive

        let mut app = world_app(teams);
        let sent = sent_hull(&mut app);
        assert!(rows_for(&sent, "eng").contains(&"helm-radar".to_string()));
        // Helm's own view is unchanged by someone else's team arriving.
        assert_eq!(rows_for(&sent, "pilot"), &vec!["helm-radar".to_string()]);
    }

    #[test]
    fn reconnect_resync_honours_the_same_gating_as_the_live_broadcast() {
        let mut app = world_app(RepairTeams::new(1));

        let live = sent_hull(&mut app);
        for token in ["eng", "pilot"] {
            let ServerMessage::SystemHullUpdate {
                entries,
                aggregate_fraction,
            } = hull_update_for_token(app.world_mut(), token).expect("resync payload")
            else {
                unreachable!()
            };
            let resync: Vec<String> = entries.iter().map(|e| e.system_id.0.clone()).collect();
            assert_eq!(
                &resync,
                rows_for(&live, token),
                "reconnect must not hand {token} detail the live path withholds"
            );
            assert!(aggregate_fraction.is_some());
        }
    }

    #[test]
    fn repair_blackboard_fans_out_per_token_while_others_stay_broadcast() {
        let v = vis(vec![]);
        let repair_bb = SystemBlackboard::Repair(RepairBlackboard {
            teams: vec![],
            travel_duration_secs: 5.0,
            system_hull: vec![
                status("core", 40.0),
                status("helm-radar", 60.0),
                status("repair", 100.0),
            ],
            damageable_systems: vec![
                SystemId("core".into()),
                SystemId("helm-radar".into()),
                SystemId("repair".into()),
            ],
            queue_depth: vec![],
            aggregate_hull_fraction: None,
        });
        let viewers = vec![
            ("eng".to_string(), Some(StationId("engineering".into()))),
            ("pilot".to_string(), Some(StationId("helm".into()))),
        ];
        let mut cache = LastVisibleRepairBlackboard::default();
        let out = project_repair_blackboards(
            vec![(SystemId("repair".into()), repair_bb)],
            Some(&v),
            &viewers,
            &mut cache,
        );

        assert!(
            !out.iter().any(|(t, _)| matches!(t, Target::All)),
            "a repair blackboard carrying hull detail must never go to Target::All"
        );
        for (target, msg) in &out {
            let (Target::Token(token), ServerMessage::BlackboardUpdate { updates }) = (target, msg)
            else {
                panic!("expected a token-targeted BlackboardUpdate")
            };
            let SystemBlackboard::Repair(bb) = &updates[0].1 else {
                unreachable!()
            };
            let rows = ids(&bb.system_hull);
            match token.as_str() {
                "eng" => {
                    assert_eq!(rows, vec!["core", "repair"]);
                    assert!(bb.aggregate_hull_fraction.is_some());
                }
                // Helm is not Engineering: the repair blackboard is the
                // Engineering console's payload and nothing else renders it,
                // so Helm receives the empty one — not a filtered one.
                "pilot" => {
                    assert!(rows.is_empty());
                    assert!(bb.damageable_systems.is_empty());
                    assert!(bb.teams.is_empty());
                    assert!(bb.queue_depth.is_empty());
                    assert!(bb.aggregate_hull_fraction.is_none());
                }
                other => panic!("unexpected recipient {other}"),
            }
        }

        // Unchanged projections are not re-sent.
        assert!(cache.projections.contains_key("eng"));
    }

    #[test]
    fn reconnect_resync_gives_an_unknown_token_no_detail() {
        let mut app = world_app(RepairTeams::new(1));
        let ServerMessage::SystemHullUpdate { entries, .. } =
            hull_update_for_token(app.world_mut(), "stranger").expect("resync payload")
        else {
            unreachable!()
        };
        assert!(entries.is_empty());
    }

    #[test]
    fn ship_with_no_engineering_station_reveals_nothing_extra() {
        let v = HullVisibility::new(
            vec![status("core", 50.0)],
            [(SystemId("core".into()), None)].into_iter().collect(),
            None,
            vec![SystemId("core".into())],
        );
        assert!(v.entries_for(Some(&StationId("pilot".into()))).is_empty());
    }
}
