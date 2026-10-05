use crate::lobby::start_policy::ReadinessTally;
use phoenix_model::messages::{Player, StationId};
use phoenix_model::wire::ShipStations;

#[derive(Debug)]
pub enum RegisterError {
    DuplicateToken,
}

/// Server-side record of every connected or recently-disconnected player.
/// Keyed by session token (a UUIDv4 persisted client-side), not peer ID —
/// see `CONTEXT.md`'s "Session" / "Session Token" glossary entries.
///
/// ## Why disconnected records are never pruned (issue #613, PRD story 9)
///
/// `disconnect()` only flips `connected = false` and clears `ready`; it
/// never removes the `Player` entry. This is intentional, not an oversight:
///
/// - Reconnection is matched purely by token lookup (`reconnect()` calls
///   `idx(token)`, which scans `players` for a matching `token` string). If a
///   disconnected player's entry were pruned, a later `Identify` with the
///   same token would miss `idx()` and fall through to `register()` as a
///   brand-new player — losing `last_rating` and, for a player who still
///   holds a station, breaking the reconnect-yield seat/rating restore in
///   `process_disconnect_with_stations` / `handle_identify` (the `Identify`
///   handler in `crates/phoenix-simulation/src/lobby/handler.rs`).
/// - Even a station-less disconnected player is not safe to prune purely on
///   "disconnected + no station": nothing in `SessionManager` distinguishes
///   "never held a station this game" from "held one and had it stolen by a
///   Backfill/other-claim path" — either way the record is the only memory
///   of that token's `name` / prior identity, and re-registering fabricates
///   a second, disconnected phantom entry for the same human the next time
///   they reconnect (since `register()` only rejects *duplicate* tokens, a
///   stale-but-pruned token would silently accept a fresh entry instead of
///   restoring the old one).
/// - Growth is bounded in practice: a ship's `players` list is bounded by
///   the fixed station roster (a handful of seats) plus whatever spectators
///   have ever connected during that single running game process — this is
///   not a public server accumulating unbounded distinct sessions over
///   months of uptime, it resets to empty on process restart.
///
/// Net: correctness of reconnect semantics outweighs the bookkeeping win of
/// pruning, so this module deliberately does not prune session records.
#[derive(Default)]
pub struct SessionManager {
    players: Vec<Player>,
    /// Host-owned native screen assignments. Kept apart from connected tenure:
    /// a rebuilding screen reserves its station while Backfill operates it.
    native_station_assignments: std::collections::HashMap<String, StationId>,
    /// Rating a player has chosen for a station while still in the Lobby
    /// (before the Ship entity — and thus `ActiveStationRatings` — exists).
    /// Keyed by station, not token: the choice belongs to whoever currently
    /// holds the seat. Consumed by `spawn_game_start_entities` at game start
    /// and cleared on station release / `ReturnToLobby`. Distinct from
    /// `Player.last_rating`, which is per-token and only matters for a
    /// mid-game (`InProgress`) disconnect/reconnect — the two never overlap
    /// in lifetime.
    pending_ratings: std::collections::HashMap<StationId, String>,
    /// Anonymous accessibility eligibility (issue #1103), token → the set of
    /// Station ids that token has reported itself INELIGIBLE for.
    ///
    /// A private side-map, DELIBERATELY OFF `Player`: `Player` is serialized and
    /// broadcast, so an eligibility field there would leak a derived accessibility
    /// fact to every peer. Only the anonymous ineligible SET ever reaches here
    /// (via `ClientMessage::ReportStationEligibility`); the profile and the
    /// functional reasons never leave the reporting client. An unknown token
    /// defaults to eligible ([`is_eligible`]) so a silent or legacy client is
    /// never locked out of a seat. Cleared on `ReturnToLobby` alongside
    /// `pending_ratings`.
    eligibility: std::collections::HashMap<String, std::collections::HashSet<StationId>>,
    /// The station rating a player held on their directly-owned Station at the
    /// instant they entered AFK (issue #1104), token → rating name. Captured
    /// BEFORE the AFK Backfill is applied and consumed (and cleared) when the
    /// player leaves AFK, restoring the prior coherent control configuration.
    ///
    /// Kept INDEPENDENT of `Player.last_rating` on purpose: `last_rating` is the
    /// disconnect/reconnect snapshot, and a disconnect that lands while a player
    /// is AFK writes Backfill into `last_rating` — reusing it would clobber the
    /// true pre-AFK rating. A private side-map (like `pending_ratings` /
    /// `eligibility`) is the seam that survives that interaction. An absent entry
    /// means "no AFK snapshot to restore".
    afk_prev_rating: std::collections::HashMap<String, String>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn idx(&self, token: &str) -> Option<usize> {
        self.players.iter().position(|p| p.token == token)
    }

    /// Only the native display adapter may set these; no client message can.
    pub fn set_native_station_assignments(
        &mut self,
        assignments: impl IntoIterator<Item = (String, StationId)>,
    ) {
        self.native_station_assignments = assignments.into_iter().collect();
    }

    pub fn native_station_for_token(&self, token: &str) -> Option<&StationId> {
        self.native_station_assignments.get(token)
    }

    /// Applies to new claims and reconnect restoration, including while a
    /// reserved screen is disconnected and therefore has no connected holder.
    pub fn station_claim_allowed(&self, token: &str, station: &StationId) -> bool {
        self.native_station_for_token(token)
            .is_none_or(|bound| bound == station)
            && !self
                .native_station_assignments
                .iter()
                .any(|(owner, bound)| owner != token && bound == station)
    }

    pub fn register(&mut self, token: String, name: String) -> Result<&Player, RegisterError> {
        if self.idx(&token).is_some() {
            return Err(RegisterError::DuplicateToken);
        }
        self.players.push(Player {
            token,
            name,
            connected: true,
            ready: false,
            station: None,
            last_rating: None,
            spectator: false,
            afk: false,
        });
        Ok(self.players.last().unwrap())
    }

    pub fn reconnect(&mut self, token: &str) -> Option<&mut Player> {
        let idx = self.idx(token)?;
        self.players[idx].connected = true;
        Some(&mut self.players[idx])
    }

    pub fn disconnect(&mut self, token: &str) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].connected = false;
            self.players[idx].ready = false;
            // `afk` is DELIBERATELY preserved across a disconnect (issue #1104
            // AC5): an AFK holder that drops is already delegated (visiting
            // Stations re-resolved) and keeps the seat, so the presence flag —
            // and the `afk_prev_rating` snapshot that restores their prior
            // configuration — must survive the drop and the reconnect. Only the
            // transient `ready` flag is cleared here (unlike `afk`).
        }
    }

    pub fn set_name(&mut self, token: &str, name: String) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].name = name;
        }
    }

    /// C1: Return the stable station ID currently held by this player.
    pub fn station_for_token(&self, token: &str) -> Option<&StationId> {
        self.players
            .iter()
            .find(|p| p.token == token)
            .and_then(|p| p.station.as_ref())
    }

    /// C1: Set (or clear) the stable station ID for a player.
    ///
    /// Seating a player (`Some(..)`) also clears the spectator flag: a seat and
    /// the Spectator role are mutually exclusive (issue #1105 invariant, and the
    /// #1106 seam — a token that becomes seated is no longer a spectator).
    pub fn set_station(&mut self, token: &str, station: Option<StationId>) {
        if let Some(idx) = self.idx(token) {
            if station.is_some() {
                self.players[idx].spectator = false;
            }
            self.players[idx].station = station;
        }
    }

    /// Set (or clear) the explicit Spectator role for a player (issue #1105).
    /// No-op if the token is not found. Setting `true` also vacates any held
    /// Station to preserve the invariant `spectator ⇒ station == None` — the
    /// two roles are mutually exclusive.
    pub fn set_spectator(&mut self, token: &str, spectator: bool) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].spectator = spectator;
            if spectator {
                self.players[idx].station = None;
            }
        }
    }

    /// True when the player with `token` is currently a Spectator (issue #1105).
    /// False for an unknown token.
    pub fn is_spectator(&self, token: &str) -> bool {
        self.idx(token)
            .map(|idx| self.players[idx].spectator)
            .unwrap_or(false)
    }

    /// Enter or leave the AFK presence state for a player (issue #1104). No-op
    /// if the token is not found. Unlike `set_spectator`, this RETAINS any held
    /// Station — AFK delegates the seat's Systems without relinquishing it — so
    /// only the flag moves.
    pub fn set_afk(&mut self, token: &str, afk: bool) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].afk = afk;
        }
    }

    /// True when the player with `token` is currently AFK (issue #1104). False
    /// for an unknown token.
    pub fn is_afk(&self, token: &str) -> bool {
        self.idx(token)
            .map(|idx| self.players[idx].afk)
            .unwrap_or(false)
    }

    /// C3: Record the rating the player held at a station just before disconnect.
    /// Cleared to None once a reconnect restore has applied it.
    pub fn set_last_rating(&mut self, token: &str, rating: Option<String>) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].last_rating = rating;
        }
    }

    /// Record a station's chosen rating while still in the Lobby (pre-spawn).
    pub fn set_pending_rating(&mut self, station: &StationId, rating: String) {
        self.pending_ratings.insert(station.clone(), rating);
    }

    /// The pending (lobby-chosen) rating for a station, if any.
    pub fn pending_rating_for(&self, station: &StationId) -> Option<&String> {
        self.pending_ratings.get(station)
    }

    /// The same connected-seat choices ordinary ship boot consumes. This
    /// projection can cross the host mesh without player names or tokens.
    pub fn lobby_station_ratings(
        &self,
        stations: &crate::lobby::stations_config::ShipStations,
    ) -> Vec<(StationId, String)> {
        let mut crew: Vec<_> = stations
            .stations
            .iter()
            .filter(|station| {
                !station.auxiliary
                    && self.players.iter().any(|player| {
                        player.connected
                            && !player.spectator
                            && player.station.as_ref() == Some(&station.id)
                    })
            })
            .map(|station| {
                let rating = self
                    .pending_rating_for(&station.id)
                    .cloned()
                    .or_else(|| station.ratings.first().cloned())
                    .unwrap_or_else(|| "Std".into());
                (station.id.clone(), rating)
            })
            .collect();
        crew.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
        crew
    }

    /// Clear a single station's pending rating (e.g. on release/reassignment).
    pub fn clear_pending_rating(&mut self, station: &StationId) {
        self.pending_ratings.remove(station);
    }

    /// All pending (lobby-chosen) ratings, keyed by station.
    pub fn pending_ratings(&self) -> &std::collections::HashMap<StationId, String> {
        &self.pending_ratings
    }

    /// Clear every pending rating (e.g. on `ReturnToLobby` for a fresh round).
    pub fn clear_all_pending_ratings(&mut self) {
        self.pending_ratings.clear();
    }

    /// Record the anonymous set of Station ids a token is INELIGIBLE for
    /// (issue #1103). Replaces any prior report for that token, since the client
    /// re-sends the complete set whenever its profile or a required rating
    /// changes. An empty set means "eligible everywhere".
    pub fn set_eligibility(
        &mut self,
        token: &str,
        ineligible: std::collections::HashSet<StationId>,
    ) {
        self.eligibility.insert(token.to_string(), ineligible);
    }

    /// Is `token` eligible for `station`? DEFAULT TRUE for an unknown token or an
    /// unreported station, so a silent / legacy client is never locked out of a
    /// seat. Only a token that has explicitly reported `station` as ineligible
    /// returns `false`.
    pub fn is_eligible(&self, token: &str, station: &StationId) -> bool {
        self.eligibility
            .get(token)
            .is_none_or(|ineligible| !ineligible.contains(station))
    }

    /// Clear every token's eligibility report (e.g. on `ReturnToLobby` for a
    /// fresh round), alongside `clear_all_pending_ratings`.
    pub fn clear_all_eligibility(&mut self) {
        self.eligibility.clear();
    }

    /// Snapshot the rating a player held on their directly-owned Station just
    /// before entering AFK (issue #1104), so leaving AFK can restore the exact
    /// prior configuration. Replaces any earlier snapshot for the token.
    pub fn set_afk_prev_rating(&mut self, token: &str, rating: String) {
        self.afk_prev_rating.insert(token.to_string(), rating);
    }

    /// The rating snapshotted at AFK-entry for `token`, if any (issue #1104).
    pub fn afk_prev_rating_for(&self, token: &str) -> Option<&String> {
        self.afk_prev_rating.get(token)
    }

    /// Drop a token's AFK rating snapshot (issue #1104), once it has been
    /// restored on AFK-exit.
    pub fn clear_afk_prev_rating(&mut self, token: &str) {
        self.afk_prev_rating.remove(token);
    }

    /// Station IDs not held by any connected player, in ship-config declaration
    /// order. Sits alongside `holder_for_station` as the station-keyed
    /// replacement for the legacy console-keyed API.
    pub fn available_stations(&self, ship_config: &ShipStations) -> Vec<StationId> {
        let held_stations: Vec<&StationId> = self
            .players
            .iter()
            .filter(|p| p.connected)
            .filter_map(|p| p.station.as_ref())
            .collect();

        ship_config
            .stations
            .iter()
            .filter(|def| !def.auxiliary)
            .filter(|def| !held_stations.contains(&&def.id))
            .map(|def| def.id.clone())
            .collect()
    }

    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// Get the connected player token holding the given station id, or `None`
    /// when the station is unclaimed or its holder is disconnected.
    ///
    /// Sole holder lookup after issue #618: takes a `StationId` directly and
    /// no longer needs `ShipConfig` to translate a console variant into the
    /// owning station.
    pub fn holder_for_station(&self, station_id: &StationId) -> Option<&str> {
        self.players
            .iter()
            .find(|p| p.connected && p.station.as_ref() == Some(station_id))
            .map(|p| p.token.as_str())
    }

    /// Set the ready flag for a player. No-op if token not found.
    pub fn set_ready(&mut self, token: &str, ready: bool) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].ready = ready;
        }
    }

    /// Count connected, non-spectator crew and the ready subset.
    ///
    /// Station ownership is irrelevant: a connected participant who has not
    /// chosen a Station yet still belongs to collective readiness. Spectators
    /// and disconnected rows do not.
    pub fn readiness_tally(&self) -> ReadinessTally {
        let connected = self
            .players
            .iter()
            .filter(|player| player.connected && !player.spectator)
            .count() as u32;
        let ready = self
            .players
            .iter()
            .filter(|player| player.connected && !player.spectator && player.ready)
            .count() as u32;
        ReadinessTally { connected, ready }
    }

    /// True when every connected, non-spectator player is ready.
    ///
    /// Delegates to [`Self::readiness_tally`] so solo countdown, fleet
    /// projection and the host lobby cannot drift on who counts. Returns false
    /// at zero participants; GM-only start is a coordinated policy over the
    /// separate GM roster and deliberately does not change this local answer.
    pub fn all_ready(&self) -> bool {
        self.readiness_tally().all_ready()
    }

    /// Reset all players' ready flags to false (e.g. when a new scenario loads).
    pub fn reset_ready(&mut self) {
        for p in &mut self.players {
            p.ready = false;
        }
    }

    /// Clear every player's held station (e.g. on `ReturnToLobby` for a fresh
    /// round, issue #756). Identity fields (token / name / connected /
    /// last_rating) are untouched — only the seat claim is released so the
    /// next round starts from an empty roster.
    pub fn clear_all_stations(&mut self) {
        for p in &mut self.players {
            p.station = None;
        }
    }
}

#[derive(bevy::prelude::Resource)]
pub struct Sessions(pub SessionManager);
