use crate::messages::{Player, StationId};
use crate::ship::config::ShipConfig;

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
///   `process_disconnect_with_stations` / `process_message`'s `Identify`
///   handler (`src/lobby/handler.rs`).
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
pub struct SessionManager {
    players: Vec<Player>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            players: Vec::new(),
        }
    }

    fn idx(&self, token: &str) -> Option<usize> {
        self.players.iter().position(|p| p.token == token)
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
    pub fn set_station(&mut self, token: &str, station: Option<StationId>) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].station = station;
        }
    }

    /// C3: Record the rating the player held at a station just before disconnect.
    /// Cleared to None once a reconnect restore has applied it.
    pub fn set_last_rating(&mut self, token: &str, rating: Option<String>) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].last_rating = rating;
        }
    }

    /// Station IDs not held by any connected player, in ship-config declaration
    /// order. Sits alongside `holder_for_station` as the station-keyed
    /// replacement for the legacy console-keyed API.
    pub fn available_stations(&self, ship_config: &ShipConfig) -> Vec<StationId> {
        let held_stations: Vec<&StationId> = self
            .players
            .iter()
            .filter(|p| p.connected)
            .filter_map(|p| p.station.as_ref())
            .collect();

        ship_config
            .stations
            .iter()
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

    /// True when every connected player is ready, or when
    /// zero connected players exist (zero-human auto-start is allowed).
    pub fn all_ready(&self) -> bool {
        let connected: Vec<&Player> = self.players.iter().filter(|p| p.connected).collect();
        if connected.is_empty() {
            return true; // zero-human start
        }
        connected.iter().all(|p| p.ready)
    }

    /// Reset all players' ready flags to false (e.g. when a new scenario loads).
    pub fn reset_ready(&mut self) {
        for p in &mut self.players {
            p.ready = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::stations_config::{ShipStations, StationDef};
    use crate::messages::{PowerGroupId, SystemId};
    use crate::ship::config::{PowerGroupConfig, StationConfig, SystemInstanceConfig};

    fn sm() -> SessionManager {
        SessionManager::new()
    }

    fn test_stations() -> ShipStations {
        ShipStations {
            stations: vec![
                StationDef {
                    id: StationId("captain".into()),
                    name: "Captain".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
                StationDef {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
                StationDef {
                    id: StationId("tactical".into()),
                    name: "Tactical".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
                StationDef {
                    id: StationId("repair".into()),
                    name: "Repair".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
                StationDef {
                    id: StationId("sensors".into()),
                    name: "Sensors".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
                StationDef {
                    id: StationId("shields".into()),
                    name: "Shields".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
                StationDef {
                    id: StationId("navigation".into()),
                    name: "Navigation".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
                StationDef {
                    id: StationId("power".into()),
                    name: "Power".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
                StationDef {
                    id: StationId("comms".into()),
                    name: "Comms".into(),
                    description: "".into(),
                    rank: "".into(),
                    short_code: "".into(),
                    console: None,
                },
            ],
        }
    }

    fn test_ship_config() -> ShipConfig {
        ShipConfig {
            stations: test_stations()
                .stations
                .into_iter()
                .map(|station| StationConfig {
                    id: station.id,
                    name: station.name,
                    description: station.description,
                    rank: station.rank,
                    short_code: station.short_code,
                    ratings: vec![],
                    console: None,
                })
                .collect(),
            systems: vec![SystemInstanceConfig {
                id: SystemId("dummy".into()),
                kind: "dummy".into(),
                station: None,
                ai_only: true,
                power_group: None,
                marker: None,
                config: None,
            }],
            power_groups: std::iter::once((
                PowerGroupId("ops".into()),
                PowerGroupConfig {
                    label: "Ops".into(),
                    default_level: 2,
                    min_level: 1,
                    max_level: 4,
                },
            ))
            .collect(),
            coordination_lag_secs: 2.0,
        }
    }

    #[test]
    fn register_new_player() {
        let mut sm = sm();
        let p = sm.register("t1".into(), "Alice".into()).unwrap();
        assert_eq!(p.token, "t1");
        assert_eq!(p.name, "Alice");
        assert!(p.connected);
        assert!(p.station.is_none());
    }

    #[test]
    fn duplicate_token_fails() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(matches!(
            sm.register("t1".into(), "Bob".into()),
            Err(RegisterError::DuplicateToken)
        ));
    }

    #[test]
    fn disconnect_marks_player_as_disconnected() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.disconnect("t1");
        assert!(!sm.players()[0].connected);
    }

    #[test]
    fn disconnect_clears_ready_flag() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_ready("t1", true);
        assert!(sm.players()[0].ready);
        sm.disconnect("t1");
        assert!(!sm.players()[0].ready);
    }

    #[test]
    fn reconnect_marks_player_as_connected() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.disconnect("t1");
        assert!(!sm.players()[0].connected);
        sm.reconnect("t1");
        assert!(sm.players()[0].connected);
    }

    #[test]
    fn set_name_updates_name() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_name("t1", "Alicia".into());
        assert_eq!(sm.players()[0].name, "Alicia");
    }

    #[test]
    fn players_returns_all_registered() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        assert_eq!(sm.players().len(), 2);
    }

    #[test]
    fn holder_for_station_returns_player_at_captain_chair() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        assert_eq!(
            sm.holder_for_station(&StationId("captain".into())),
            Some("t1")
        );
    }

    #[test]
    fn holder_for_station_returns_none_when_unclaimed() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert_eq!(sm.holder_for_station(&StationId("captain".into())), None);
    }

    #[test]
    fn holder_for_station_returns_correct_helm() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_station("t2", Some(StationId("helm".into())));
        assert_eq!(sm.holder_for_station(&StationId("helm".into())), Some("t2"));
    }

    #[test]
    fn holder_for_station_returns_none_when_holder_disconnected() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        sm.disconnect("t1");
        assert_eq!(sm.holder_for_station(&StationId("captain".into())), None);
    }

    #[test]
    fn available_stations_returns_all_when_none_claimed() {
        let ship_config = test_ship_config();
        let sm = sm();
        let available = sm.available_stations(&ship_config);
        assert!(available.contains(&StationId("captain".into())));
        assert!(available.contains(&StationId("helm".into())));
        assert!(available.contains(&StationId("tactical".into())));
        assert!(available.contains(&StationId("repair".into())));
        assert!(available.contains(&StationId("sensors".into())));
        assert!(available.contains(&StationId("shields".into())));
        assert!(available.contains(&StationId("navigation".into())));
        assert!(available.contains(&StationId("power".into())));
        assert!(available.contains(&StationId("comms".into())));
    }

    #[test]
    fn available_stations_excludes_claimed_stations() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        let available = sm.available_stations(&ship_config);
        assert!(!available.contains(&StationId("captain".into())));
        assert!(available.contains(&StationId("helm".into())));
    }

    #[test]
    fn available_stations_reappears_on_disconnect() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        assert!(!sm
            .available_stations(&ship_config)
            .contains(&StationId("captain".into())));
        sm.disconnect("t1");
        assert!(sm
            .available_stations(&ship_config)
            .contains(&StationId("captain".into())));
    }

    #[test]
    fn available_stations_excludes_disconnected_station_holders() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        sm.set_station("t2", Some(StationId("helm".into())));
        assert!(!sm
            .available_stations(&ship_config)
            .contains(&StationId("captain".into())));
        assert!(!sm
            .available_stations(&ship_config)
            .contains(&StationId("helm".into())));
        sm.disconnect("t1");
        assert!(sm
            .available_stations(&ship_config)
            .contains(&StationId("captain".into())));
    }

    #[test]
    fn station_for_token_returns_none_when_no_station() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert_eq!(sm.station_for_token("t1"), None);
    }

    #[test]
    fn set_station_sets_and_clears() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        assert_eq!(
            sm.station_for_token("t1"),
            Some(&StationId("captain".into()))
        );
        sm.set_station("t1", None);
        assert_eq!(sm.station_for_token("t1"), None);
    }

    #[test]
    fn set_last_rating_stores_and_clears() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(sm.players()[0].last_rating.is_none());
        sm.set_last_rating("t1", Some("Assisted".into()));
        assert_eq!(sm.players()[0].last_rating.as_deref(), Some("Assisted"));
        sm.set_last_rating("t1", None);
        assert!(sm.players()[0].last_rating.is_none());
    }

    // ── Ready state ──────────────────────────────────────────────────

    #[test]
    fn set_ready_marks_player_ready() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(!sm.players()[0].ready);
        sm.set_ready("t1", true);
        assert!(sm.players()[0].ready);
        sm.set_ready("t1", false);
        assert!(!sm.players()[0].ready);
    }

    #[test]
    fn set_ready_unknown_token_is_noop() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_ready("nonexistent", true); // should not panic
        assert!(!sm.players()[0].ready);
    }

    #[test]
    fn all_ready_returns_true_when_zero_players() {
        let sm = sm();
        assert!(sm.all_ready(), "zero players → all_ready must be true");
    }

    #[test]
    fn all_ready_returns_true_when_single_player_ready() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_ready("t1", true);
        assert!(sm.all_ready());
    }

    #[test]
    fn all_ready_returns_false_when_single_player_not_ready() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(!sm.all_ready());
    }

    #[test]
    fn all_ready_requires_all_players_ready() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_ready("t1", true);
        assert!(!sm.all_ready(), "t2 not ready → all_ready false");
        sm.set_ready("t2", true);
        assert!(sm.all_ready(), "both ready → all_ready true");
    }

    #[test]
    fn all_ready_ignores_disconnected_players() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_ready("t1", true);
        sm.disconnect("t2");
        assert!(
            sm.all_ready(),
            "disconnected player should not block all_ready"
        );
    }

    #[test]
    fn reset_ready_clears_all() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_ready("t1", true);
        sm.set_ready("t2", true);
        sm.reset_ready();
        assert!(!sm.players()[0].ready);
        assert!(!sm.players()[1].ready);
    }

    #[test]
    fn register_sets_ready_false() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(
            !sm.players()[0].ready,
            "newly registered player must have ready=false"
        );
    }
}
