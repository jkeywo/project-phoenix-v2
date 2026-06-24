use std::collections::VecDeque;

use crate::messages::{Console, Player, StationId};
use crate::ship::config::ShipConfig;

#[derive(Debug)]
pub enum RegisterError {
    DuplicateToken,
}

pub struct SessionManager {
    players: Vec<Player>,
    /// FIFO queue of spectator tokens — players waiting for a station slot.
    spectator_queue: VecDeque<String>,
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
            spectator_queue: VecDeque::new(),
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

    /// Consoles not held by any connected player, derived from unclaimed stations.
    pub fn available_consoles(&self, ship_config: &ShipConfig) -> Vec<Console> {
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
            .filter_map(|def| Console::from_console_id(&def.console))
            .filter(|console| *console != Console::Core)
            .collect()
    }

    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// Check if a specific station (by StationId) owns the given console.
    pub fn station_has_console(
        &self,
        station_id: &StationId,
        console: &Console,
        ship_config: &ShipConfig,
    ) -> bool {
        ship_config
            .station(station_id)
            .and_then(|station| Console::from_console_id(&station.console))
            .as_ref()
            == Some(console)
    }

    /// Get the connected player token holding a station that owns this console.
    pub fn console_holder(&self, console: &Console, ship_config: &ShipConfig) -> Option<&str> {
        let station_id = ship_config
            .station_for_console(console.station_console_id())
            .map(|station| &station.id)?;

        self.players
            .iter()
            .find(|p| p.connected && p.station.as_ref() == Some(station_id))
            .map(|p| p.token.as_str())
    }

    /// Check if a specific token is assigned to a station that owns this console.
    pub fn player_has_console(
        &self,
        token: &str,
        console: &Console,
        ship_config: &ShipConfig,
    ) -> bool {
        let Some(station_id) = self.station_for_token(token) else {
            return false;
        };
        self.station_has_console(station_id, console, ship_config)
    }

    /// Append a token to the back of the spectator queue (if not already queued).
    pub fn push_spectator(&mut self, token: String) {
        if !self.spectator_queue.contains(&token) {
            self.spectator_queue.push_back(token);
        }
    }

    /// Remove and return the front of the spectator queue.
    pub fn pop_spectator(&mut self) -> Option<String> {
        self.spectator_queue.pop_front()
    }

    /// Read-only view of the spectator queue.
    pub fn spectator_queue(&self) -> &VecDeque<String> {
        &self.spectator_queue
    }

    /// Mutable access to the spectator queue (for applying cascade results).
    pub fn spectator_queue_mut(&mut self) -> &mut VecDeque<String> {
        &mut self.spectator_queue
    }

    /// Remove a token from the spectator queue (e.g. when they are promoted to a station).
    pub fn remove_spectator(&mut self, token: &str) {
        self.spectator_queue.retain(|t| t != token);
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
                    consoles: vec![Console::CaptainChair],
                    rank: "".into(),
                    short_code: "".into(),
                },
                StationDef {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "".into(),
                    consoles: vec![Console::Helm],
                    rank: "".into(),
                    short_code: "".into(),
                },
                StationDef {
                    id: StationId("tactical".into()),
                    name: "Tactical".into(),
                    description: "".into(),
                    consoles: vec![Console::Tactical],
                    rank: "".into(),
                    short_code: "".into(),
                },
                StationDef {
                    id: StationId("repair".into()),
                    name: "Repair".into(),
                    description: "".into(),
                    consoles: vec![Console::Repair],
                    rank: "".into(),
                    short_code: "".into(),
                },
                StationDef {
                    id: StationId("sensors".into()),
                    name: "Sensors".into(),
                    description: "".into(),
                    consoles: vec![Console::Sensors],
                    rank: "".into(),
                    short_code: "".into(),
                },
                StationDef {
                    id: StationId("shields".into()),
                    name: "Shields".into(),
                    description: "".into(),
                    consoles: vec![Console::Shields],
                    rank: "".into(),
                    short_code: "".into(),
                },
                StationDef {
                    id: StationId("navigation".into()),
                    name: "Navigation".into(),
                    description: "".into(),
                    consoles: vec![Console::Navigation],
                    rank: "".into(),
                    short_code: "".into(),
                },
                StationDef {
                    id: StationId("power".into()),
                    name: "Power".into(),
                    description: "".into(),
                    consoles: vec![Console::Power],
                    rank: "".into(),
                    short_code: "".into(),
                },
                StationDef {
                    id: StationId("comms".into()),
                    name: "Comms".into(),
                    description: "".into(),
                    consoles: vec![Console::Comms],
                    rank: "".into(),
                    short_code: "".into(),
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
                    console: station
                        .consoles
                        .first()
                        .map(Console::station_console_id)
                        .unwrap_or("core")
                        .to_string(),
                    ratings: vec![],
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
    fn console_holder_returns_player_at_captain_chair() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        assert_eq!(
            sm.console_holder(&Console::CaptainChair, &ship_config),
            Some("t1")
        );
    }

    #[test]
    fn console_holder_returns_none_when_no_captain_chair() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert_eq!(
            sm.console_holder(&Console::CaptainChair, &ship_config),
            None
        );
    }

    #[test]
    fn console_holder_returns_correct_helm() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_station("t2", Some(StationId("helm".into())));
        assert_eq!(sm.console_holder(&Console::Helm, &ship_config), Some("t2"));
    }

    #[test]
    fn player_has_console_returns_true_when_assigned() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        assert!(sm.player_has_console("t1", &Console::CaptainChair, &ship_config));
    }

    #[test]
    fn player_has_console_returns_false_when_not_assigned() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(!sm.player_has_console("t1", &Console::CaptainChair, &ship_config));
    }

    #[test]
    fn player_has_console_returns_false_for_different_station_console() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        assert!(!sm.player_has_console("t1", &Console::Helm, &ship_config));
    }

    #[test]
    fn available_consoles_returns_all_when_none_claimed() {
        let ship_config = test_ship_config();
        let sm = sm();
        let available = sm.available_consoles(&ship_config);
        assert!(available.contains(&Console::CaptainChair));
        assert!(available.contains(&Console::Helm));
        assert!(available.contains(&Console::Tactical));
        assert!(available.contains(&Console::Repair));
        assert!(available.contains(&Console::Sensors));
        assert!(available.contains(&Console::Shields));
        assert!(available.contains(&Console::Navigation));
        assert!(available.contains(&Console::Power));
        assert!(available.contains(&Console::Comms));
    }

    #[test]
    fn available_consoles_excludes_claimed_stations() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        let available = sm.available_consoles(&ship_config);
        assert!(!available.contains(&Console::CaptainChair));
        assert!(available.contains(&Console::Helm));
    }

    #[test]
    fn available_consoles_reappears_on_disconnect() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        assert!(!sm
            .available_consoles(&ship_config)
            .contains(&Console::CaptainChair));
        sm.disconnect("t1");
        assert!(sm
            .available_consoles(&ship_config)
            .contains(&Console::CaptainChair));
    }

    #[test]
    fn available_consoles_excludes_disconnected_station_holders() {
        let ship_config = test_ship_config();
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.set_station("t1", Some(StationId("captain".into())));
        sm.set_station("t2", Some(StationId("helm".into())));
        assert!(!sm
            .available_consoles(&ship_config)
            .contains(&Console::CaptainChair));
        assert!(!sm.available_consoles(&ship_config).contains(&Console::Helm));
        sm.disconnect("t1");
        assert!(sm
            .available_consoles(&ship_config)
            .contains(&Console::CaptainChair));
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

    // ── Spectator queue ───────────────────────────────────────────────────

    #[test]
    fn push_spectator_appends_token() {
        let mut sm = SessionManager::new();
        sm.push_spectator("t1".into());
        assert_eq!(sm.spectator_queue().len(), 1);
        assert_eq!(sm.spectator_queue().front().map(|s| s.as_str()), Some("t1"));
    }

    #[test]
    fn push_spectator_does_not_add_duplicate() {
        let mut sm = SessionManager::new();
        sm.push_spectator("t1".into());
        sm.push_spectator("t1".into());
        assert_eq!(sm.spectator_queue().len(), 1);
    }

    #[test]
    fn push_spectator_maintains_fifo_order() {
        let mut sm = SessionManager::new();
        sm.push_spectator("t1".into());
        sm.push_spectator("t2".into());
        sm.push_spectator("t3".into());
        let queue: Vec<_> = sm.spectator_queue().iter().cloned().collect();
        assert_eq!(queue, vec!["t1", "t2", "t3"]);
    }

    #[test]
    fn pop_spectator_returns_front() {
        let mut sm = SessionManager::new();
        sm.push_spectator("t1".into());
        sm.push_spectator("t2".into());
        assert_eq!(sm.pop_spectator(), Some("t1".into()));
        assert_eq!(sm.spectator_queue().len(), 1);
    }

    #[test]
    fn pop_spectator_returns_none_when_empty() {
        let mut sm = SessionManager::new();
        assert_eq!(sm.pop_spectator(), None);
    }

    #[test]
    fn remove_spectator_removes_token_from_queue() {
        let mut sm = SessionManager::new();
        sm.push_spectator("t1".into());
        sm.push_spectator("t2".into());
        sm.remove_spectator("t1");
        assert_eq!(sm.spectator_queue().len(), 1);
        assert_eq!(sm.spectator_queue().front().map(|s| s.as_str()), Some("t2"));
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
