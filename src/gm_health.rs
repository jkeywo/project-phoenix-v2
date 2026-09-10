//! Peer-local public technical health for the Game Master desk (issue #1437 /
//! PRD #1419 story 10).
//!
//! One advisory projection answers "is the fleet actually healthy right now" —
//! who is connected, which Stations have a human on them, how fresh every
//! peer's tick watermark is, and whether the world is deliberately paused,
//! waiting on a lagging peer, restoring after a divergence, or short a peer
//! entirely. It publishes on the page-local `gm_health` Host Channel, exactly
//! as [`crate::gm_attention`] publishes the attention queue.
//!
//! # What this is NOT
//!
//! Not a `ServerMessage`, not a mesh frame, not a snapshot field, not a digest
//! fold, and not a policy. Nothing here mutates the world, evicts a Session,
//! reassigns a Station or ends a recovery: a condition appears because it holds
//! and disappears because it stopped holding. This issue *observes* the
//! membership and recovery machinery #1116/#1118/#1119 already own; M5's
//! restore-specific eviction is not here and must not be inferred from here.
//!
//! # What may cross, and what may never
//!
//! Everything on the wire is already crew-public: authored entity names and
//! uuids, [`crate::core::messages::StationId`]s, a player's chosen display
//! name, and the public [`crate::gm_roster::GmOperator`] id. Deliberately
//! absent, and covered by test:
//!
//! - Session tokens and reconnect credentials ([`crate::lobby::server::Sessions`]
//!   keys, `Player::token`).
//! - Physical transport identity ([`crate::session_connections::ConnectionId`],
//!   rendezvous peer ids).
//! - The technical fleet slot ([`crate::command_admission::log::HostSlot`]).
//!   `crate::gm_journal` already holds that line for the action journal, and a
//!   health panel is no more entitled to it: a peer is named here by the hull
//!   it flies or the public operator bound to it, and an anonymous peer gets a
//!   display ordinal instead.
//!
//! # The four states are distinct on purpose
//!
//! A paused world and a lagging peer look identical if you only watch the tick
//! counter, and reporting a deliberate pause as a fault is how a facilitator
//! learns to ignore the health panel. So:
//!
//! - **Paused** — [`SimulationPaused`] is set. Nobody is advancing, and that is
//!   the point.
//! - **Stale** — running, but this peer's declared watermark is far enough
//!   behind that the barrier itself would withhold the next tick. The threshold
//!   is not invented here: it is [`LockstepSession::stall_at`], the same
//!   question [`crate::lockstep::gate_lockstep_ticks`] asks every frame.
//! - **Recovering** — [`crate::lockstep::recovery::RecoveryState`] has a plan in
//!   flight; the fleet is holding at a boundary on purpose.
//! - **Disconnected** — the peer departed the barrier ([`LockstepSession::has_departed`],
//!   issue #1119), the operator's public row says `connected = false`, or the
//!   human who held a Station has dropped.
//!
//! # The seam M5 reuses (#1446/#1447)
//!
//! [`GmHealthProjection::alerts`] is the whole technical-failure surface, and it
//! is deliberately a list of self-describing rows rather than a set of booleans:
//! a live restore adds a [`GmHealthAlertKind`] variant and a String Table
//! sentence, and both the unfilterable banner region and the Urgent Station rows
//! in the attention queue pick it up with no further wiring. The browser half of
//! the seam is `gui/gm-health-banner.js`, which renders any alert row into any
//! container.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::console_bridge::GmHealthChanged;
use crate::core::messages::StationId;
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::gm_action::SimulationPaused;
use crate::gm_projection::GmEntityReference;
use crate::gm_roster::GmRoster;
use crate::lobby::server::Sessions;
use crate::lockstep::recovery::RecoveryState;
use crate::lockstep::{FleetLockstep, FleetRoster, FleetSlotOf};
use crate::ship::components::ShipConfigComponent;

/// Presentation bound on one published health panel. A fleet cannot exceed the
/// rendezvous roster ceiling, but an unbounded projection is an unbounded page.
pub const MAX_GM_HEALTH_ROWS: usize = 64;

/// The complete state vocabulary, ordered least to most severe.
///
/// Closed and system-defined: no authored config names one of these, and no
/// authored config may reorder them (see the module note and
/// [`GmHealthAlert`]).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmHealthState {
    /// Connected and keeping up.
    Live,
    /// The world is deliberately held. Not a fault.
    Paused,
    /// Running, but behind far enough that the barrier would withhold a tick.
    Stale,
    /// A divergence recovery is in flight for this peer.
    Recovering,
    /// The peer, operator or Station human is gone.
    Disconnected,
}

impl GmHealthState {
    /// The wire spelling, which is also the `data-state` the page draws with.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Paused => "paused",
            Self::Stale => "stale",
            Self::Recovering => "recovering",
            Self::Disconnected => "disconnected",
        }
    }
}

/// A short human reason, as a String Table id plus its runtime parameters.
///
/// Never prose: the projection has no locale, and the parameter values are
/// authored ids the page resolves at its own presentation boundary — the same
/// contract [`crate::gm_attention::GmAttentionReason`] holds.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmHealthReason {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

impl GmHealthReason {
    fn new(id: &str, params: impl IntoIterator<Item = (&'static str, String)>) -> Self {
        Self {
            id: id.to_string(),
            params: params
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        }
    }
}

/// What kind of technical failure one alert reports.
///
/// Append-only, and deliberately not authorable: an advisory `[[gm_*]]` table
/// can choose bands for its own Comms/beat/NPC items, but it can neither add a
/// kind here nor suppress one. M5's live restore adds variants beside these.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmHealthAlertKind {
    /// The human holding a Station dropped. The seat still exists and nothing
    /// has been taken away from them.
    StationDisconnected,
    /// A whole fleet peer left the barrier (issue #1119); its hull is still in
    /// the world with nobody flying it.
    ShipPeerLost,
    /// A Game Master's own public row says they are gone.
    OperatorDisconnected,
    /// A divergence recovery is in flight; the fleet is holding at a boundary.
    RecoveryInProgress,
    /// A recovery ended without healing the split, honestly reported.
    RecoveryFailed,
}

impl GmHealthAlertKind {
    /// The severity a row of this kind carries. `Recovering` is a held fleet,
    /// not a lost one, and reporting it as `Disconnected` would cry wolf.
    fn severity(self) -> GmHealthState {
        match self {
            Self::StationDisconnected | Self::ShipPeerLost | Self::OperatorDisconnected => {
                GmHealthState::Disconnected
            }
            Self::RecoveryInProgress => GmHealthState::Recovering,
            Self::RecoveryFailed => GmHealthState::Disconnected,
        }
    }

    /// Whether a row of this kind also earns an Urgent Station row in the
    /// attention queue (issue #1433).
    ///
    /// Station-scoped losses do; the recovery and operator rows are technical
    /// conditions with no Station to open, and they are reported by the banner
    /// alone rather than by a queue row a filter could narrow away from.
    pub fn is_station_attention(self) -> bool {
        matches!(self, Self::StationDisconnected | Self::ShipPeerLost)
    }
}

/// One persistent technical failure, self-describing.
///
/// This is the single source both treatments read: the unfilterable banner
/// region renders every alert verbatim, and the Urgent Station rows in the
/// attention queue are built from the [`GmHealthAlertKind::is_station_attention`]
/// subset. They can therefore never disagree about whether something is wrong.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmHealthAlert {
    /// Stable while the condition holds; a recurrence mints a different one, so
    /// a snooze taken against the previous outage cannot hide the new one.
    pub id: String,
    pub kind: GmHealthAlertKind,
    pub severity: GmHealthState,
    pub reason: GmHealthReason,
    /// The hull this is about, so the banner's action is a selection on the
    /// map that already exists rather than a new surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship: Option<GmEntityReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<StationId>,
    /// The simulation tick this peer first observed the condition.
    pub first_seen_tick: u64,
}

/// One crewed seat and whether a human is on it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmStationHealth {
    /// `<ship uuid>/<station id>` — public, stable, and never a session token.
    pub id: String,
    pub station_id: StationId,
    /// The authored Station name (a String Table id or authored label).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship: Option<GmEntityReference>,
    /// The display name the human chose. Never their token.
    pub operator: String,
    pub state: GmHealthState,
}

/// One equal Game Master's public presence.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmOperatorHealth {
    /// The crew-public operator id, exactly as [`GmRoster`] holds it.
    pub id: String,
    pub name: String,
    pub state: GmHealthState,
}

/// One technical simulation participant, named by what it is on the desk.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmPeerHealth {
    /// `ship:<uuid>`, `gm:<operator id>`, or `peer:<display ordinal>` for a
    /// participant with neither. Never the fleet slot.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship: Option<GmEntityReference>,
    /// Public operator ids bound to this peer, in id order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operators: Vec<String>,
    pub state: GmHealthState,
    /// How many ticks behind this peer's last declared watermark is, by the
    /// barrier's own measure. `None` for this host (which never waits for
    /// itself) and for a departed peer (whose watermark is meaningless).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind_ticks: Option<u64>,
    /// Whether this row is the peer the operator is sitting at.
    pub local: bool,
}

/// What a recovery in flight is doing, if one is.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmRecoveryHealth {
    pub divergence_tick: u64,
    /// The tick the fleet holds at. Absent when the split was refused before a
    /// leader — and therefore a boundary — was ever agreed, rather than
    /// reported as a tick nothing ever held at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_tick: Option<u64>,
    /// How many peers are restoring. The slots themselves stay private.
    pub recovering_peers: usize,
    pub failed: bool,
}

/// The whole public health picture, absolute.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmHealthProjection {
    /// The simulation tick this sample was taken at — the observability basis
    /// every freshness number below is measured against.
    pub tick: u64,
    /// The world is deliberately held.
    pub paused: bool,
    /// How far ahead of the current tick input is stamped. The barrier's own
    /// tolerance, and therefore the threshold `stale` is judged against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_delay_ticks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<GmRecoveryHealth>,
    pub peers: Vec<GmPeerHealth>,
    pub stations: Vec<GmStationHealth>,
    pub operators: Vec<GmOperatorHealth>,
    /// Every persistent technical failure. The banner region and the Urgent
    /// Station rows both read exactly this.
    pub alerts: Vec<GmHealthAlert>,
}

impl GmHealthProjection {
    /// The worst state anything in this projection is in — the one word the
    /// panel's summary sentence uses.
    pub fn worst_state(&self) -> GmHealthState {
        let from_rows = self
            .peers
            .iter()
            .map(|peer| peer.state)
            .chain(self.stations.iter().map(|station| station.state))
            .chain(self.operators.iter().map(|operator| operator.state))
            .chain(self.alerts.iter().map(|alert| alert.severity))
            .max()
            .unwrap_or(GmHealthState::Live);
        if self.paused {
            from_rows.max(GmHealthState::Paused)
        } else {
            from_rows
        }
    }
}

/// This peer's own health bookkeeping.
///
/// `Presentation`: derived entirely from the lockstep barrier, the public
/// rosters and the recovery machinery, all of which are already classified, and
/// read by nothing authoritative.
#[derive(Resource, Default)]
pub struct GmHealthWatch {
    /// How many times each alert key has occurred. A key that stops holding and
    /// starts again is a NEW occurrence, so its id changes and a stale snooze
    /// cannot hide it.
    generations: BTreeMap<String, u64>,
    /// The keys that held when this peer last sampled, so a re-occurrence is
    /// distinguishable from a condition that simply persisted.
    holding: BTreeSet<String>,
    /// The tick each currently-holding key was first observed at.
    first_seen: BTreeMap<String, u64>,
    last: Option<GmHealthProjection>,
}

impl GmHealthWatch {
    /// The projection this peer last published, for the attention queue and for
    /// a late mount that needs the current picture without waiting for a change.
    pub fn last(&self) -> Option<&GmHealthProjection> {
        self.last.as_ref()
    }
}

/// String Table ids for the sentences one alert explains itself with.
pub const STATION_DISCONNECTED_REASON: &str = "server.gm.health.reason.station_disconnected";
pub const SHIP_PEER_LOST_REASON: &str = "server.gm.health.reason.ship_peer_lost";
pub const OPERATOR_DISCONNECTED_REASON: &str = "server.gm.health.reason.operator_disconnected";
pub const RECOVERY_IN_PROGRESS_REASON: &str = "server.gm.health.reason.recovery_in_progress";
pub const RECOVERY_FAILED_REASON: &str = "server.gm.health.reason.recovery_failed";

/// One hull the fleet put in the world, as the projection sees it.
struct ShipRow {
    reference: GmEntityReference,
    slot: Option<crate::command_admission::log::HostSlot>,
    stations: Vec<(StationId, String)>,
}

/// One alert before this peer's own first-seen/generation bookkeeping.
struct PendingAlert {
    key: String,
    kind: GmHealthAlertKind,
    reason: GmHealthReason,
    ship: Option<GmEntityReference>,
    station: Option<StationId>,
}

/// The alert one recovery in flight explains itself with.
///
/// Split out so the mapping is testable against a real
/// [`crate::lockstep::recovery::RecoveryStatus`] without standing up the
/// three-host divergence fixture: a recovery that is still transferring is
/// `Recovering`, and only one that ended without healing the split is a
/// `Disconnected`-severity failure.
fn recovery_alert(status: &crate::lockstep::recovery::RecoveryStatus) -> PendingAlert {
    use crate::lockstep::recovery::RecoveryStatus;
    match status {
        // The one tick a terminal failure always knows: the divergence it could
        // not repair. Matched on the variant, so no placeholder can reach the
        // sentence a facilitator cannot filter away.
        RecoveryStatus::Failed {
            divergence_tick, ..
        } => PendingAlert {
            key: "recovery-failed".to_string(),
            kind: GmHealthAlertKind::RecoveryFailed,
            reason: GmHealthReason::new(
                RECOVERY_FAILED_REASON,
                [("tick", divergence_tick.to_string())],
            ),
            ship: None,
            station: None,
        },
        RecoveryStatus::InProgress {
            boundary_tick,
            recovering,
            ..
        } => PendingAlert {
            key: format!("recovery:{boundary_tick}"),
            kind: GmHealthAlertKind::RecoveryInProgress,
            reason: GmHealthReason::new(
                RECOVERY_IN_PROGRESS_REASON,
                [
                    ("tick", boundary_tick.to_string()),
                    ("peers", recovering.len().to_string()),
                ],
            ),
            ship: None,
            station: None,
        },
    }
}

/// Decide one peer's state from the facts the barrier already keeps.
///
/// Pure, and separated so the four-way distinction can be tested without a
/// `World`: a paused fleet is Paused rather than Stale, a departed peer is
/// Disconnected regardless of its last watermark, and a peer the recovery plan
/// names is Recovering even while it is nominally keeping up.
fn peer_state(
    paused: bool,
    departed: bool,
    recovering: bool,
    behind_ticks: Option<u64>,
    delay: u64,
) -> GmHealthState {
    if departed {
        return GmHealthState::Disconnected;
    }
    if recovering {
        return GmHealthState::Recovering;
    }
    // A held world is not a lagging one. Checked after the two real faults so a
    // pause cannot mask a peer that actually left.
    if paused {
        return GmHealthState::Paused;
    }
    // The barrier's own tolerance, not a number invented here: a peer may sit up
    // to `delay` ticks behind before `stall_at` withholds anything.
    match behind_ticks {
        Some(behind) if behind > delay => GmHealthState::Stale,
        _ => GmHealthState::Live,
    }
}

/// Publish the public health picture onto the page-local Host Channel when it
/// changes.
///
/// Never emits an unchanged payload, for the reason
/// [`crate::gm_attention::publish_attention_projection`] does not: a banner
/// region that re-announced itself every frame would be an `aria-live` region
/// shouting at a facilitator who already knows.
pub fn publish_health_projection(
    roster: Option<Res<FleetRoster>>,
    lockstep: Option<Res<FleetLockstep>>,
    gms: Option<Res<GmRoster>>,
    sessions: Option<Res<Sessions>>,
    paused: Option<Res<SimulationPaused>>,
    recovery: Option<Res<RecoveryState>>,
    ships: Query<(
        &EntityUuid,
        Option<&EntityName>,
        Option<&FleetSlotOf>,
        Option<&ShipConfigComponent>,
    )>,
    local_ship: Query<&EntityUuid, With<crate::server_app::LocalShip>>,
    tick: Res<crate::sim_tick::SimTick>,
    mut state: ResMut<GmHealthWatch>,
    mut writer: MessageWriter<GmHealthChanged>,
) {
    let paused = paused.is_some_and(|paused| paused.0);
    let now = tick.0;

    // Every hull the frozen roster put in the world, keyed by the slot that
    // flies it. Ordinary NPCs carry no `FleetSlotOf` and are simply not peers.
    let mut fleet_ships: BTreeMap<crate::command_admission::log::HostSlot, ShipRow> =
        BTreeMap::new();
    let mut local_hull: Option<ShipRow> = None;
    let local_uuid = local_ship.iter().next().map(|uuid| uuid.0.clone());
    for (uuid, name, slot, config) in ships.iter() {
        let row = ShipRow {
            reference: GmEntityReference {
                entity_id: uuid.0.clone(),
                name: name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
            },
            slot: slot.map(|slot| slot.0),
            stations: config.map_or_else(Vec::new, |config| {
                config
                    .0
                    .stations
                    .iter()
                    .map(|station| (station.id.clone(), station.name.clone()))
                    .collect()
            }),
        };
        if local_uuid.as_deref() == Some(uuid.0.as_str()) {
            local_hull = Some(ShipRow {
                reference: row.reference.clone(),
                slot: row.slot,
                stations: row.stations.clone(),
            });
        }
        if let Some(slot) = row.slot {
            fleet_ships.insert(slot, row);
        }
    }

    let session = lockstep.as_deref().map(|fleet| &fleet.0);
    let delay = session.map_or(0, |session| session.delay());
    let local_slot = session
        .map(|session| session.local())
        .or_else(|| roster.as_deref().map(|roster| roster.local()));
    // The hull whose Stations this host's own Sessions sit on: the fleet ship
    // flying this slot, else the marked local hull.
    let host_hull = local_slot
        .and_then(|slot| fleet_ships.get(&slot))
        .map(|row| ShipRow {
            reference: row.reference.clone(),
            slot: row.slot,
            stations: row.stations.clone(),
        })
        .or(local_hull);

    let recovery_status = recovery.as_deref().and_then(RecoveryState::status);
    let recovering_slots: BTreeSet<_> = recovery_status
        .as_ref()
        .map(|status| status.recovering().iter().copied().collect())
        .unwrap_or_default();

    // Public operator bindings per peer, so a stationless GM host is still a
    // named row rather than an anonymous ordinal.
    let mut operators_by_slot: BTreeMap<crate::command_admission::log::HostSlot, Vec<String>> =
        BTreeMap::new();
    if let Some(roster) = roster.as_deref() {
        for gm in roster.gms() {
            operators_by_slot
                .entry(gm.host)
                .or_default()
                .push(gm.operator_id.clone());
        }
    }

    let participants = roster
        .as_deref()
        .map(FleetRoster::participants)
        .unwrap_or_else(|| local_slot.into_iter().collect());

    let mut alerts: Vec<PendingAlert> = Vec::new();
    let mut peers = Vec::new();
    for (ordinal, slot) in participants.iter().enumerate() {
        let ship = fleet_ships.get(slot);
        let bound = operators_by_slot.get(slot).cloned().unwrap_or_default();
        let is_local = local_slot == Some(*slot);
        let departed = session.is_some_and(|session| session.has_departed(*slot));
        // `ready_through` is what this host declares for the current tick; a
        // peer's own last declaration measured against it is the whole freshness
        // question, and it is the barrier's number rather than a wall clock.
        let behind_ticks = (!is_local && !departed)
            .then(|| {
                session.and_then(|session| {
                    session
                        .observed(*slot)
                        .map(|observed| session.ready_through(now).saturating_sub(observed))
                })
            })
            .flatten();
        let state = peer_state(
            paused,
            departed,
            recovering_slots.contains(slot),
            behind_ticks,
            delay,
        );
        let id = match (ship, bound.first()) {
            (Some(ship), _) => format!("ship:{}", ship.reference.entity_id),
            (None, Some(operator)) => format!("gm:{operator}"),
            // A participant with no hull and no public operator binding. The
            // ordinal is a DISPLAY position in this list, deliberately not the
            // technical slot.
            (None, None) => format!("peer:{}", ordinal + 1),
        };
        if departed {
            if let Some(ship) = ship {
                alerts.push(PendingAlert {
                    key: format!("ship-peer-lost:{}", ship.reference.entity_id),
                    kind: GmHealthAlertKind::ShipPeerLost,
                    reason: GmHealthReason::new(
                        SHIP_PEER_LOST_REASON,
                        [("ship", ship.reference.name.clone())],
                    ),
                    ship: Some(ship.reference.clone()),
                    station: None,
                });
            }
        }
        peers.push(GmPeerHealth {
            id,
            ship: ship.map(|ship| ship.reference.clone()),
            operators: bound,
            state,
            behind_ticks,
            local: is_local,
        });
    }

    // Station assignments. A record is never pruned on disconnect (see
    // `SessionManager`), which is precisely what lets a dropped human still be
    // reported as holding their seat rather than silently vanishing.
    //
    // A seat is a seat, not a record. The lobby lets a live human claim a seat
    // whose previous holder is merely disconnected, and leaves the departed
    // record still pointing at it: occupancy counts only connected players, and
    // the stale `station` is cleared by that player's own reconnect rather than
    // by the claim (`crate::lobby::handler`). So several records can name one
    // `StationId` at once, and the seat's true holder is the connected one —
    // the same resolution [`crate::lobby::session::SessionManager::holder_for_station`]
    // and the viewscreen border already use. Resolving per seat is what keeps
    // the panel to one row per Station and stops an ordinary mid-game backfill
    // from parking a permanent, unfilterable Disconnected warning under the
    // departed operator's name while a live human flies the seat.
    let mut stations = Vec::new();
    if let (Some(sessions), Some(hull)) = (sessions.as_deref(), host_hull.as_ref()) {
        let mut seats: BTreeMap<&str, (&StationId, &crate::core::messages::Player)> =
            BTreeMap::new();
        for player in sessions.0.players() {
            let Some(station_id) = player.station.as_ref() else {
                continue;
            };
            match seats.get(station_id.0.as_str()) {
                // A connected holder always wins; among records of equal
                // standing the later one is the more recent claim.
                Some((_, held)) if held.connected && !player.connected => {}
                _ => {
                    seats.insert(station_id.0.as_str(), (station_id, player));
                }
            }
        }
        for (station_id, player) in seats.into_values() {
            let name = hull
                .stations
                .iter()
                .find(|(id, _)| id == station_id)
                .map_or_else(|| station_id.0.clone(), |(_, name)| name.clone());
            let state = if player.connected {
                if paused {
                    GmHealthState::Paused
                } else {
                    GmHealthState::Live
                }
            } else {
                GmHealthState::Disconnected
            };
            if !player.connected {
                alerts.push(PendingAlert {
                    key: format!(
                        "station-disconnected:{}/{}",
                        hull.reference.entity_id, station_id.0
                    ),
                    kind: GmHealthAlertKind::StationDisconnected,
                    reason: GmHealthReason::new(
                        STATION_DISCONNECTED_REASON,
                        [
                            ("operator", player.name.clone()),
                            ("station", name.clone()),
                            ("ship", hull.reference.name.clone()),
                        ],
                    ),
                    ship: Some(hull.reference.clone()),
                    station: Some(station_id.clone()),
                });
            }
            stations.push(GmStationHealth {
                id: format!("{}/{}", hull.reference.entity_id, station_id.0),
                station_id: station_id.clone(),
                name,
                ship: Some(hull.reference.clone()),
                operator: player.name.clone(),
                state,
            });
        }
    }

    let mut operator_rows = Vec::new();
    if let Some(gms) = gms.as_deref() {
        for operator in gms.operators() {
            let state = if operator.connected {
                if paused {
                    GmHealthState::Paused
                } else {
                    GmHealthState::Live
                }
            } else {
                GmHealthState::Disconnected
            };
            if !operator.connected {
                alerts.push(PendingAlert {
                    key: format!("operator-disconnected:{}", operator.id),
                    kind: GmHealthAlertKind::OperatorDisconnected,
                    reason: GmHealthReason::new(
                        OPERATOR_DISCONNECTED_REASON,
                        [(
                            "operator",
                            if operator.name.is_empty() {
                                operator.id.clone()
                            } else {
                                operator.name.clone()
                            },
                        )],
                    ),
                    ship: None,
                    station: None,
                });
            }
            operator_rows.push(GmOperatorHealth {
                id: operator.id.clone(),
                name: operator.name.clone(),
                state,
            });
        }
    }

    let recovery_health = recovery_status.as_ref().map(|status| GmRecoveryHealth {
        divergence_tick: status.divergence_tick(),
        boundary_tick: status.boundary_tick(),
        recovering_peers: status.recovering().len(),
        failed: status.failed(),
    });
    if let Some(status) = recovery_status.as_ref() {
        alerts.push(recovery_alert(status));
    }

    // Bookkeeping: a key that stopped holding is forgotten, and a key that
    // starts holding again takes a new generation, so its id — and therefore
    // its attention-queue occurrence — is genuinely fresh.
    let holding: BTreeSet<String> = alerts.iter().map(|alert| alert.key.clone()).collect();
    for key in holding
        .difference(&state.holding)
        .cloned()
        .collect::<Vec<_>>()
    {
        *state.generations.entry(key.clone()).or_insert(0) += 1;
        state.first_seen.insert(key, now);
    }
    state.first_seen.retain(|key, _| holding.contains(key));
    state.holding = holding;

    let mut alert_rows: Vec<GmHealthAlert> = alerts
        .into_iter()
        .map(|alert| {
            let generation = state.generations.get(&alert.key).copied().unwrap_or(1);
            let first_seen_tick = state.first_seen.get(&alert.key).copied().unwrap_or(now);
            GmHealthAlert {
                id: format!("{}#{generation}", alert.key),
                severity: alert.kind.severity(),
                kind: alert.kind,
                reason: alert.reason,
                ship: alert.ship,
                station: alert.station,
                first_seen_tick,
            }
        })
        .collect();
    // Worst first, then oldest, then by id: the same deterministic shape the
    // attention queue orders on, so the banner list never shuffles under a
    // reading operator.
    alert_rows.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.first_seen_tick.cmp(&b.first_seen_tick))
            .then_with(|| a.id.cmp(&b.id))
    });
    alert_rows.truncate(MAX_GM_HEALTH_ROWS);
    peers.truncate(MAX_GM_HEALTH_ROWS);
    stations.truncate(MAX_GM_HEALTH_ROWS);
    operator_rows.truncate(MAX_GM_HEALTH_ROWS);

    let next = GmHealthProjection {
        tick: now,
        paused,
        input_delay_ticks: session.map(|session| session.delay()),
        recovery: recovery_health,
        peers,
        stations,
        operators: operator_rows,
        alerts: alert_rows,
    };
    // The tick alone is not a change: it advances every step by construction.
    // Compare what a Game Master actually reads.
    let changed = state.last.as_ref().is_none_or(|last| {
        last.paused != next.paused
            || last.recovery != next.recovery
            || last.input_delay_ticks != next.input_delay_ticks
            || last.peers != next.peers
            || last.stations != next.stations
            || last.operators != next.operators
            || last.alerts != next.alerts
    });
    if changed {
        state.last = Some(next.clone());
        writer.write(GmHealthChanged { payload: next });
    } else {
        state.last = Some(next);
    }
}

/// Registers the health projection on a GM-presenting peer, exactly as
/// [`crate::gm_attention::GmAttentionPlugin`] registers the attention queue.
pub struct GmHealthPlugin;

impl Plugin for GmHealthPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};

        app.init_resource::<GmHealthWatch>()
            .declare_state::<GmHealthWatch>(StateClass::Presentation, "gm-t3-peer-health")
            .add_message::<GmHealthChanged>()
            .add_systems(
                PostUpdate,
                publish_health_projection
                    .before(crate::gm_attention::publish_attention_projection)
                    .run_if(crate::gm_projection::gm_presentation_active),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four states are decided from four different facts, and a deliberate
    /// pause is not allowed to look like a fault — nor to hide one.
    #[test]
    fn paused_stale_recovering_and_disconnected_are_decided_separately() {
        // Running and keeping up.
        assert_eq!(
            peer_state(false, false, false, Some(0), 6),
            GmHealthState::Live
        );
        // Behind, but inside the barrier's own tolerance.
        assert_eq!(
            peer_state(false, false, false, Some(6), 6),
            GmHealthState::Live
        );
        // Behind far enough that the barrier would withhold.
        assert_eq!(
            peer_state(false, false, false, Some(7), 6),
            GmHealthState::Stale
        );
        // The same lag, while the world is deliberately held, is a pause.
        assert_eq!(
            peer_state(true, false, false, Some(7), 6),
            GmHealthState::Paused
        );
        // A recovery in flight outranks the pause it holds the fleet with.
        assert_eq!(
            peer_state(true, false, true, Some(7), 6),
            GmHealthState::Recovering
        );
        // And a peer that actually left outranks everything, pause included.
        assert_eq!(
            peer_state(true, true, true, None, 6),
            GmHealthState::Disconnected
        );
    }

    /// Severity ordering is what the panel's one-word summary reads off.
    #[test]
    fn the_summary_reports_the_worst_thing_present() {
        let mut projection = GmHealthProjection {
            peers: vec![GmPeerHealth {
                id: "ship:a".into(),
                ship: None,
                operators: Vec::new(),
                state: GmHealthState::Live,
                behind_ticks: None,
                local: true,
            }],
            ..Default::default()
        };
        assert_eq!(projection.worst_state(), GmHealthState::Live);
        projection.paused = true;
        assert_eq!(projection.worst_state(), GmHealthState::Paused);
        projection.stations.push(GmStationHealth {
            id: "a/helm".into(),
            station_id: StationId("helm".into()),
            name: "Helm".into(),
            ship: None,
            operator: "Morgan".into(),
            state: GmHealthState::Disconnected,
        });
        assert_eq!(projection.worst_state(), GmHealthState::Disconnected);
    }

    /// A recovery in flight reads as a held fleet; only one that gave up is a
    /// failure, and neither is ever mistaken for a lost peer.
    #[test]
    fn a_recovery_explains_itself_differently_while_it_is_still_working() {
        use crate::command_admission::log::HostSlot;
        use crate::lockstep::recovery::RecoveryStatus;
        let working = RecoveryStatus::InProgress {
            divergence_tick: 240,
            boundary_tick: 260,
            recovering: vec![HostSlot(3)],
            resolved: false,
        };
        let alert = recovery_alert(&working);
        assert_eq!(alert.kind, GmHealthAlertKind::RecoveryInProgress);
        assert_eq!(alert.kind.severity(), GmHealthState::Recovering);
        assert_eq!(alert.reason.id, RECOVERY_IN_PROGRESS_REASON);
        assert_eq!(alert.reason.params.get("tick").unwrap(), "260");
        assert_eq!(alert.reason.params.get("peers").unwrap(), "1");
        // The key carries the boundary, so a SECOND recovery at a later
        // boundary is a new condition rather than the same row aging on.
        assert_eq!(alert.key, "recovery:260");

        // A terminal failure names the divergence it could not repair — the
        // tick the recovery itself recorded, whether or not a boundary was ever
        // agreed. Both shapes are checked, because the no-safe-leader path
        // reaches the banner with no boundary at all.
        for boundary_tick in [Some(260), None] {
            let failed = RecoveryStatus::Failed {
                divergence_tick: 240,
                boundary_tick,
            };
            let alert = recovery_alert(&failed);
            assert_eq!(alert.kind, GmHealthAlertKind::RecoveryFailed);
            assert_eq!(alert.kind.severity(), GmHealthState::Disconnected);
            assert_eq!(alert.reason.id, RECOVERY_FAILED_REASON);
            assert_eq!(
                alert.reason.params.get("tick").unwrap(),
                &failed.divergence_tick().to_string(),
                "the unfilterable sentence quotes the recovery's own divergence tick"
            );
            assert_eq!(alert.reason.params.get("tick").unwrap(), "240");
        }
    }

    /// Only Station-scoped losses become queue rows; the technical conditions
    /// with no Station to open stay banner-only.
    #[test]
    fn station_scoped_alerts_are_the_ones_that_earn_a_queue_row() {
        assert!(GmHealthAlertKind::StationDisconnected.is_station_attention());
        assert!(GmHealthAlertKind::ShipPeerLost.is_station_attention());
        assert!(!GmHealthAlertKind::OperatorDisconnected.is_station_attention());
        assert!(!GmHealthAlertKind::RecoveryInProgress.is_station_attention());
        assert!(!GmHealthAlertKind::RecoveryFailed.is_station_attention());
        // A recovery in flight is a held fleet, not a lost one.
        assert_eq!(
            GmHealthAlertKind::RecoveryInProgress.severity(),
            GmHealthState::Recovering
        );
        assert_eq!(
            GmHealthAlertKind::StationDisconnected.severity(),
            GmHealthState::Disconnected
        );
    }
}
