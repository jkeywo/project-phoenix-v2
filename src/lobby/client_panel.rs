//! Pure client-side lobby state model.
//!
//! Maintains a `LobbyState` (mirroring the server's `GameState` shape) by
//! applying inbound `ServerMessage`s, and derives a `LobbyView` from that
//! state plus the local player's token. Both pieces are deliberately
//! Bevy-free so they can be unit-tested on native and reused by the
//! WASM client UI layer.

use crate::messages::{ClientMessage, Console, GamePhase, GameState, Player, ServerMessage};
use crate::stations_config::ShipStations;
use bevy::prelude::Resource;
use std::collections::HashMap;

/// The client's authoritative model of the shared lobby state.
///
/// Built incrementally from `ServerMessage`s — every message that the
/// server can send while in the lobby phase is applied here so the UI
/// layer only ever reads from this struct.
#[derive(Clone, Debug, PartialEq, Resource)]
pub struct LobbyState {
    pub phase: GamePhase,
    pub players: Vec<Player>,
    /// Station configuration received from the server on `Welcome`.
    pub ship_stations: ShipStations,
    /// Current per-console complexity preset selection.
    pub complexity: HashMap<Console, String>,
}

impl Default for LobbyState {
    fn default() -> Self {
        Self {
            phase: GamePhase::Lobby,
            players: Vec::new(),
            ship_stations: ShipStations::default(),
            complexity: HashMap::new(),
        }
    }
}

impl LobbyState {
    /// Replace the entire lobby state — used on `Welcome`, which is the
    /// authoritative initial sync.
    pub fn replace_from(&mut self, state: GameState, ship_stations: ShipStations) {
        self.phase = state.phase;
        self.players = state.players;
        self.ship_stations = ship_stations;
        self.complexity = state.complexity;
    }

    /// Apply a single inbound `ServerMessage`. Variants that don't affect
    /// the lobby (e.g. `SimState`, `WorldSetup`) are ignored.
    pub fn apply(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::Welcome { state, ship_stations } => {
                self.replace_from(state.clone(), ship_stations.clone());
            }
            ServerMessage::PlayerJoined { player } => {
                if let Some(existing) = self.players.iter_mut().find(|p| p.token == player.token) {
                    *existing = player.clone();
                } else {
                    self.players.push(player.clone());
                }
            }
            ServerMessage::PlayerLeft { token } => {
                self.players.retain(|p| &p.token != token);
            }
            ServerMessage::NameChanged { token, name } => {
                if let Some(p) = self.players.iter_mut().find(|p| &p.token == token) {
                    p.name = name.clone();
                }
            }
            ServerMessage::StationAssigned { token, consoles, .. } => {
                // The server's authoritative StationAssigned carries the
                // *holder's* full console list, but the same console can
                // only be held by one player — so first clear any other
                // player who used to hold any of these consoles, then
                // assign to the named token.
                for c in consoles {
                    for p in self.players.iter_mut() {
                        if &p.token != token {
                            p.consoles.retain(|existing| existing != c);
                        }
                    }
                }
                if let Some(p) = self.players.iter_mut().find(|p| &p.token == token) {
                    p.consoles = consoles.clone();
                }
            }
            ServerMessage::GameStarted => {
                self.phase = GamePhase::InProgress;
            }
            ServerMessage::ComplexityChanged { console, preset_name } => {
                // Update the per-console preset in ship_stations or local state.
                // The ship_stations carries complexity_presets (available presets),
                // while we track the current selection in the clientside cache.
                self.complexity.insert(console.clone(), preset_name.clone());
            }
            ServerMessage::SimState { .. }
            | ServerMessage::WorldSetup { .. }
            | ServerMessage::TargetLock { .. }
            | ServerMessage::WeaponsUpdate { .. }
            | ServerMessage::BeamStarted { .. }
            | ServerMessage::BeamEnded { .. }
            | ServerMessage::AsteroidDestroyed { .. }
            | ServerMessage::ScienceTargetSuggestion { .. }
            | ServerMessage::SensorsTargetSuggestion { .. }
            | ServerMessage::PhaserFired { .. }
            | ServerMessage::RepairState { .. }
            | ServerMessage::ShieldStatus { .. }
            | ServerMessage::TorpedoLaunched { .. }
            | ServerMessage::TorpedoDestroyed { .. }
            | ServerMessage::ModifierAdded { .. }
            | ServerMessage::ModifierRemoved { .. }
            | ServerMessage::AsteroidSpawned { .. }
            | ServerMessage::PowerState { .. }
            | ServerMessage::EntitySpawned { .. }
            | ServerMessage::EntityDespawned { .. }
            | ServerMessage::FrequencyHint { .. }
            | ServerMessage::StationSpawned { .. }
            | ServerMessage::StationDestroyed { .. }
            | ServerMessage::ObjectiveSummary { .. }
            | ServerMessage::CommsState { .. }
            | ServerMessage::ShipDestroyed => {
                // Not relevant to the lobby model.
            }
        }
    }
}

/// All consoles the lobby UI knows how to render. Listed in the order
/// they appear on screen.
pub const ALL_CONSOLES: [Console; 9] = [Console::CaptainChair, Console::Helm, Console::Tactical, Console::Repair, Console::Sensors, Console::Shields, Console::Navigation, Console::Power, Console::Comms];

/// One console row as the lobby UI should render it.
#[derive(Clone, Debug, PartialEq)]
pub enum ConsoleSlot {
    /// Console is unclaimed; clicking it sends `SelectConsole{console}`.
    Available { console: Console },
    /// Console is held by another player; the row is disabled.
    Occupied { console: Console, holder_name: String },
    /// Console is held by the local player; clicking releases it
    /// (the server treats `SelectConsole` as a toggle).
    Mine { console: Console },
}

/// One station row as the station-based lobby UI should render it.
#[derive(Clone, Debug, PartialEq)]
pub enum StationSlot {
    /// Station is unclaimed; clicking sends `SelectStation { station }`.
    Available { station: String, short_code: String, description: String, rank: String, consoles: Vec<Console>, preset_names: Vec<String> },
    /// Station is held by another player; row is disabled.
    Occupied { station: String, short_code: String, description: String, rank: String, consoles: Vec<Console>, holder_name: String, preset_names: Vec<String> },
    /// Station is held by the local player; a "Leave" affordance should appear.
    Mine { station: String, short_code: String, description: String, rank: String, consoles: Vec<Console>, preset_names: Vec<String> },
    /// A spectator (player with no station assignment), shown in arrival order.
    Spectator { player_name: String },
}

/// View-model derived from `LobbyState` plus the local player's token.
/// Everything the lobby UI needs to render comes from here, so the UI
/// layer stays trivial and this layer stays unit-testable.
pub struct LobbyView<'a> {
    state: &'a LobbyState,
    my_token: &'a str,
}

impl<'a> LobbyView<'a> {
    pub fn new(state: &'a LobbyState, my_token: &'a str) -> Self {
        Self { state, my_token }
    }

    /// True if the local player currently holds the captain's chair.
    pub fn is_captain(&self) -> bool {
        self.my_consoles().contains(&Console::CaptainChair)
    }

    /// True if the local player currently holds the helm console.
    pub fn is_helm(&self) -> bool {
        self.my_consoles().contains(&Console::Helm)
    }

    /// True if the local player currently holds the Sensors console.
    pub fn is_sensors(&self) -> bool {
        self.my_consoles().contains(&Console::Sensors)
    }

    /// True if the local player currently holds the Shields console.
    pub fn is_shields(&self) -> bool {
        self.my_consoles().contains(&Console::Shields)
    }

    /// True if the local player currently holds the Navigation console.
    pub fn is_navigation(&self) -> bool {
        self.my_consoles().contains(&Console::Navigation)
    }

    /// True if the local player currently holds the repair console.
    pub fn is_repair(&self) -> bool {
        self.my_consoles().contains(&Console::Repair)
    }

    /// True if the local player currently holds the power console.
    pub fn is_power(&self) -> bool {
        self.my_consoles().contains(&Console::Power)
    }

    /// Consoles held by the local player (empty if no matching token).
    pub fn my_consoles(&self) -> &[Console] {
        self.state
            .players
            .iter()
            .find(|p| p.token == self.my_token)
            .map(|p| p.consoles.as_slice())
            .unwrap_or(&[])
    }

    /// True if every console in `ALL_CONSOLES` is held by someone.
    pub fn all_consoles_filled(&self) -> bool {
        self.console_slots()
            .iter()
            .all(|slot| !matches!(slot, ConsoleSlot::Available { .. }))
    }

    /// One slot per console in `ALL_CONSOLES`, classified by who holds it.
    pub fn console_slots(&self) -> Vec<ConsoleSlot> {
        ALL_CONSOLES
            .iter()
            .map(|console| {
                let holder = self
                    .state
                    .players
                    .iter()
                    .find(|p| p.consoles.contains(console));
                match holder {
                    Some(p) if p.token == self.my_token => ConsoleSlot::Mine { console: console.clone() },
                    Some(p) => ConsoleSlot::Occupied { console: console.clone(), holder_name: p.name.clone() },
                    None => ConsoleSlot::Available { console: console.clone() },
                }
            })
            .collect()
    }

    /// One slot per station in `ShipStations` at the current player count,
    /// classified by who holds it, followed by spectator rows in arrival order.
    ///
    /// Player count is the number of connected players, clamped to max_players.
    /// Spectator rows are only appended when connected players exceed max_players
    /// (i.e. there are more players than station slots).
    pub fn station_slots(&self) -> Vec<StationSlot> {
        let player_count = self.state.players.len() as u32;
        let max = self.state.ship_stations.max_players;

        // Always show the layout for the current connected-player count so a
        // newly-joined player who hasn't picked yet sees the right station list.
        // Clamp to max_players so we never request a non-existent config entry.
        let display_count = if player_count == 0 {
            self.state.ship_stations.min_players.max(1)
        } else if max > 0 {
            player_count.min(max)
        } else {
            player_count
        };

        let mut slots: Vec<StationSlot> = Vec::new();

        if let Some(defs) = self.state.ship_stations.configs.get(&display_count) {
            for def in defs {
                // Find the holder: the player whose consoles intersect this station's consoles.
                let holder = self.state.players.iter().find(|p| {
                    p.consoles.iter().any(|c| def.consoles.contains(c))
                });
                let preset_names: Vec<String> = def.consoles.iter()
                    .map(|c| self.complexity_preset_for(c).unwrap_or("Std").to_string())
                    .collect();
                let slot = match holder {
                    Some(p) if p.token == self.my_token => StationSlot::Mine {
                        station: def.name.clone(),
                        short_code: def.short_code.clone(),
                        description: def.description.clone(),
                        rank: def.rank.clone(),
                        consoles: def.consoles.clone(),
                        preset_names: preset_names.clone(),
                    },
                    Some(p) => StationSlot::Occupied {
                        station: def.name.clone(),
                        short_code: def.short_code.clone(),
                        description: def.description.clone(),
                        rank: def.rank.clone(),
                        consoles: def.consoles.clone(),
                        holder_name: p.name.clone(),
                        preset_names: preset_names.clone(),
                    },
                    None => StationSlot::Available {
                        station: def.name.clone(),
                        short_code: def.short_code.clone(),
                        description: def.description.clone(),
                        rank: def.rank.clone(),
                        consoles: def.consoles.clone(),
                        preset_names,
                    },
                };
                slots.push(slot);
            }
        }

        // Append spectator rows only when connected players exceed the total
        // number of station slots (max_players). Below that threshold every
        // unassigned player can still pick a station and is not a spectator.
        if max > 0 && player_count > max {
            for p in self.state.players.iter().filter(|p| p.consoles.is_empty()) {
                slots.push(StationSlot::Spectator { player_name: p.name.clone() });
            }
        }

        slots
    }

    /// True if the local player is currently a spectator (no consoles assigned).
    pub fn is_spectator(&self) -> bool {
        self.my_consoles().is_empty()
    }

    /// True when the lobby panel should be visible.
    ///
    /// The lobby panel is shown during the `Lobby` phase (everyone sees it)
    /// and during the `InProgress` phase for spectators who haven't been
    /// promoted to a station yet.
    pub fn show_lobby_panel(&self) -> bool {
        match self.state.phase {
            GamePhase::Lobby => true,
            GamePhase::InProgress => self.is_spectator(),
        }
    }

    /// True when the "Game in progress" banner should appear at the top of
    /// the lobby panel.  Only shown to spectators watching an active game.
    pub fn game_in_progress_banner(&self) -> bool {
        self.state.phase == GamePhase::InProgress && self.is_spectator()
    }

    /// Get the current complexity preset for a given console, if any.
    pub fn complexity_preset_for(&self, console: &Console) -> Option<&str> {
        self.state.complexity.get(console).map(|s| s.as_str())
    }

    /// True if every station slot at the display player count is filled.
    ///
    /// When connected players exceed `max_players`, the surplus become spectators
    /// and only the `max_players` slots need to be filled.
    pub fn all_stations_filled(&self) -> bool {
        let player_count = self.state.players.len() as u32;
        let max = self.state.ship_stations.max_players;
        let check_count = if max > 0 && player_count > max { max } else { player_count };

        let all_held: Vec<Console> = self
            .state
            .players
            .iter()
            .flat_map(|p| p.consoles.iter().cloned())
            .collect();

        crate::stations_config::all_stations_filled(&self.state.ship_stations, check_count, &all_held)
    }
}

/// Returns the `ClientMessage` to send when the lobby UI activates the
/// given console slot. `Occupied` slots are unclickable, so they yield
/// `None`; `Available` and `Mine` both send `SelectStation`, which the
/// server will process in a later slice.
pub fn message_for_slot_click(slot: &ConsoleSlot) -> Option<ClientMessage> {
    match slot {
        ConsoleSlot::Available { console } | ConsoleSlot::Mine { console } => {
            Some(ClientMessage::SelectStation { station: console.display_name().to_string() })
        }
        ConsoleSlot::Occupied { .. } => None,
    }
}

/// `ClientMessage` to send when the captain presses the Engage button.
pub fn engage_message() -> ClientMessage {
    ClientMessage::StartGame
}

/// `ClientMessage` to send when the player clicks "Leave station".
pub fn release_station_message() -> ClientMessage {
    ClientMessage::ReleaseStation
}

/// `ClientMessage` to send when the player clicks an available or occupied station row.
/// Returns `None` for occupied rows (no-op per the issue spec).
pub fn message_for_station_slot_click(slot: &StationSlot) -> Option<ClientMessage> {
    match slot {
        StationSlot::Available { station, .. } => {
            Some(ClientMessage::SelectStation { station: station.clone() })
        }
        StationSlot::Mine { .. } | StationSlot::Occupied { .. } | StationSlot::Spectator { .. } => {
            None
        }
    }
}

/// Decides which console to land on after a `StationAssigned` update.
///
/// Returns `current` if it is present in `new_consoles`; otherwise returns
/// `new_consoles[0]`. Called by `apply_inbound_messages` when a
/// `StationAssigned` targets the local player.
///
/// # Panics
/// Panics if `new_consoles` is empty (caller must guard against spectator
/// assignment with an empty bundle before calling).
pub fn reconcile_active_console(current: Option<Console>, new_consoles: &[Console]) -> Console {
    assert!(!new_consoles.is_empty(), "reconcile_active_console called with empty bundle");
    if let Some(c) = current {
        if new_consoles.contains(&c) {
            return c;
        }
    }
    new_consoles[0].clone()
}

/// The local player's session token, set once by JS via the bridge.
/// Held as a separate resource so the lobby UI can derive "mine" without
/// polluting `LobbyState`.
#[derive(Clone, Debug, Default, Resource)]
pub struct LocalPlayerToken(pub String);

/// The console the local player is currently viewing, set by the JS tab bar.
/// `None` means "auto" — the panel for the sole held console (or none).
/// When a player holds 2+ consoles this is set by JS when a tab is clicked.
#[derive(Clone, Debug, Default, Resource)]
pub struct ActiveConsole(pub Option<Console>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{Console, GamePhase, GameState, Player};

    fn p(token: &str, name: &str, consoles: Vec<Console>) -> Player {
        Player { token: token.into(), name: name.into(), consoles, connected: true }
    }

    #[test]
    fn default_lobby_state_is_empty_and_in_lobby_phase() {
        let s = LobbyState::default();
        assert_eq!(s.phase, GamePhase::Lobby);
        assert!(s.players.is_empty());
    }

    #[test]
    fn welcome_replaces_state_wholesale() {
        let mut s = LobbyState::default();
        s.players.push(p("ghost", "Ghost", vec![]));
        s.apply(&ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::Lobby,
                players: vec![p("a", "Alice", vec![Console::CaptainChair])],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
        });
        assert_eq!(s.players.len(), 1);
        assert_eq!(s.players[0].name, "Alice");
    }

    #[test]
    fn player_joined_appends_new_player() {
        let mut s = LobbyState::default();
        s.apply(&ServerMessage::PlayerJoined { player: p("a", "Alice", vec![]) });
        s.apply(&ServerMessage::PlayerJoined { player: p("b", "Bob",   vec![]) });
        assert_eq!(s.players.iter().map(|p| p.token.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn player_joined_with_existing_token_replaces_in_place() {
        let mut s = LobbyState::default();
        s.apply(&ServerMessage::PlayerJoined { player: p("a", "Alice", vec![Console::Helm]) });
        s.apply(&ServerMessage::PlayerJoined { player: p("a", "Alice2", vec![]) });
        assert_eq!(s.players.len(), 1);
        assert_eq!(s.players[0].name, "Alice2");
        assert!(s.players[0].consoles.is_empty());
    }

    #[test]
    fn player_left_removes_by_token() {
        let mut s = LobbyState::default();
        s.players = vec![p("a", "Alice", vec![]), p("b", "Bob", vec![])];
        s.apply(&ServerMessage::PlayerLeft { token: "a".into() });
        assert_eq!(s.players.len(), 1);
        assert_eq!(s.players[0].token, "b");
    }

    #[test]
    fn name_changed_updates_only_named_player() {
        let mut s = LobbyState::default();
        s.players = vec![p("a", "Alice", vec![]), p("b", "Bob", vec![])];
        s.apply(&ServerMessage::NameChanged { token: "a".into(), name: "Alicia".into() });
        assert_eq!(s.players[0].name, "Alicia");
        assert_eq!(s.players[1].name, "Bob");
    }

    #[test]
    fn station_assigned_assigns_consoles_to_named_player() {
        let mut s = LobbyState::default();
        s.players = vec![p("a", "Alice", vec![]), p("b", "Bob", vec![])];
        s.apply(&ServerMessage::StationAssigned {
            token: "a".into(),
            station: Some("Captain".into()),
            consoles: vec![Console::CaptainChair],
        });
        assert_eq!(s.players[0].consoles, vec![Console::CaptainChair]);
        assert!(s.players[1].consoles.is_empty());
    }

    #[test]
    fn station_assigned_steals_from_previous_holder() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::Helm]),
            p("b", "Bob",   vec![]),
        ];
        s.apply(&ServerMessage::StationAssigned {
            token: "b".into(),
            station: Some("Helm".into()),
            consoles: vec![Console::Helm],
        });
        assert!(s.players[0].consoles.is_empty(), "old holder loses the console");
        assert_eq!(s.players[1].consoles, vec![Console::Helm]);
    }

    #[test]
    fn station_assigned_spectator_clears_consoles() {
        let mut s = LobbyState::default();
        s.players = vec![p("a", "Alice", vec![Console::CaptainChair, Console::Helm])];
        s.apply(&ServerMessage::StationAssigned {
            token: "a".into(),
            station: None,
            consoles: vec![],
        });
        assert!(s.players[0].consoles.is_empty());
    }

    #[test]
    fn game_started_flips_phase_to_in_progress() {
        let mut s = LobbyState::default();
        assert_eq!(s.phase, GamePhase::Lobby);
        s.apply(&ServerMessage::GameStarted);
        assert_eq!(s.phase, GamePhase::InProgress);
    }

    #[test]
    fn sim_state_does_not_disturb_lobby_model() {
        use crate::messages::{SimSnapshot, ViewMode};
        let mut s = LobbyState::default();
        s.players = vec![p("a", "Alice", vec![])];
        let before = s.clone();
        s.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: true,
                view_mode: ViewMode::default(),
                ship_x: 1.0, ship_z: 2.0, ship_yaw: 0.5,
                hull_integrity: 100.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: crate::messages::RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
            },
        });
        assert_eq!(s, before);
    }

    // ── LobbyView derivations ──────────────────────────────────

    #[test]
    fn is_captain_is_false_when_my_token_is_unknown() {
        let s = LobbyState::default();
        let v = LobbyView::new(&s, "missing");
        assert!(!v.is_captain());
        assert!(v.my_consoles().is_empty());
    }

    #[test]
    fn is_captain_is_true_only_when_i_hold_the_captain_chair() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair]),
            p("b", "Bob",   vec![Console::Helm]),
        ];
        assert!( LobbyView::new(&s, "a").is_captain());
        assert!(!LobbyView::new(&s, "b").is_captain());
    }

    #[test]
    fn is_helm_is_true_only_when_i_hold_the_helm_console() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair]),
            p("b", "Bob",   vec![Console::Helm]),
        ];
        assert!(!LobbyView::new(&s, "a").is_helm());
        assert!( LobbyView::new(&s, "b").is_helm());
        assert!(!LobbyView::new(&s, "ghost").is_helm());
    }

    #[test]
    fn console_slots_lists_all_consoles_in_canonical_order() {
        let s = LobbyState::default();
        let slots = LobbyView::new(&s, "x").console_slots();
        assert_eq!(slots.len(), ALL_CONSOLES.len());
        match &slots[0] {
            ConsoleSlot::Available { console } => assert_eq!(console, &Console::CaptainChair),
            other => panic!("expected CaptainChair available first, got {other:?}"),
        }
        match &slots[1] {
            ConsoleSlot::Available { console } => assert_eq!(console, &Console::Helm),
            other => panic!("expected Helm available second, got {other:?}"),
        }
    }

    #[test]
    fn console_slot_occupied_carries_other_holders_name() {
        let mut s = LobbyState::default();
        s.players = vec![p("a", "Alice", vec![Console::Helm])];
        let slots = LobbyView::new(&s, "me").console_slots();
        let helm_slot = slots.iter().find(|sl| matches!(sl, ConsoleSlot::Occupied { console, .. } if console == &Console::Helm));
        match helm_slot {
            Some(ConsoleSlot::Occupied { holder_name, .. }) => assert_eq!(holder_name, "Alice"),
            other => panic!("expected Helm occupied by Alice, got {other:?}"),
        }
    }

    #[test]
    fn console_slot_mine_when_local_player_is_the_holder() {
        let mut s = LobbyState::default();
        s.players = vec![p("me", "Me", vec![Console::CaptainChair])];
        let slots = LobbyView::new(&s, "me").console_slots();
        assert!(matches!(&slots[0], ConsoleSlot::Mine { console } if console == &Console::CaptainChair));
    }

    // ── Outbound messages ──────────────────────────────────────

    #[test]
    fn clicking_available_slot_sends_select_station() {
        let msg = message_for_slot_click(&ConsoleSlot::Available { console: Console::Helm });
        assert_eq!(msg, Some(ClientMessage::SelectStation { station: "Helm".into() }));
    }

    #[test]
    fn clicking_my_slot_sends_select_station_to_toggle_off() {
        let msg = message_for_slot_click(&ConsoleSlot::Mine { console: Console::CaptainChair });
        assert_eq!(msg, Some(ClientMessage::SelectStation { station: "Captain's Chair".into() }));
    }

    #[test]
    fn clicking_occupied_slot_yields_no_message() {
        let msg = message_for_slot_click(&ConsoleSlot::Occupied {
            console: Console::Helm,
            holder_name: "Alice".into(),
        });
        assert!(msg.is_none());
    }

    #[test]
    fn engage_message_is_start_game() {
        assert_eq!(engage_message(), ClientMessage::StartGame);
    }

    #[test]
    fn is_repair_is_true_only_when_i_hold_the_repair_console() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::Helm]),
            p("b", "Bob",   vec![Console::Repair]),
        ];
        assert!(!LobbyView::new(&s, "a").is_repair());
        assert!( LobbyView::new(&s, "b").is_repair());
        assert!(!LobbyView::new(&s, "ghost").is_repair());
    }

    #[test]
    fn is_power_is_true_only_when_i_hold_the_power_console() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::Helm]),
            p("b", "Bob",   vec![Console::Power]),
        ];
        assert!(!LobbyView::new(&s, "a").is_power());
        assert!( LobbyView::new(&s, "b").is_power());
        assert!(!LobbyView::new(&s, "ghost").is_power());
    }

    #[test]
    fn is_sensors_is_true_only_when_i_hold_the_sensors_console() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::Helm]),
            p("b", "Bob",   vec![Console::Sensors]),
        ];
        assert!(!LobbyView::new(&s, "a").is_sensors());
        assert!( LobbyView::new(&s, "b").is_sensors());
        assert!(!LobbyView::new(&s, "ghost").is_sensors());
    }

    #[test]
    fn is_shields_is_true_only_when_i_hold_the_shields_console() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::Helm]),
            p("b", "Bob",   vec![Console::Shields]),
        ];
        assert!(!LobbyView::new(&s, "a").is_shields());
        assert!( LobbyView::new(&s, "b").is_shields());
    }

    #[test]
    fn is_navigation_is_true_only_when_i_hold_the_navigation_console() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::Helm]),
            p("b", "Bob",   vec![Console::Navigation]),
        ];
        assert!(!LobbyView::new(&s, "a").is_navigation());
        assert!( LobbyView::new(&s, "b").is_navigation());
    }

    #[test]
    fn all_consoles_filled_is_false_when_any_console_is_available() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair]),
            p("b", "Bob",   vec![Console::Helm]),
            p("c", "Carol", vec![Console::Tactical]),
            // Repair not taken
        ];
        assert!(!LobbyView::new(&s, "a").all_consoles_filled());
    }

    #[test]
    fn all_consoles_filled_is_true_when_all_nine_consoles_are_held() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair]),
            p("b", "Bob",   vec![Console::Helm]),
            p("c", "Carol", vec![Console::Tactical]),
            p("d", "Dave",  vec![Console::Repair]),
            p("e", "Eve",   vec![Console::Sensors]),
            p("f", "Frank", vec![Console::Power]),
            p("g", "Grace", vec![Console::Comms]),
            p("h", "Heidi", vec![Console::Shields]),
            p("i", "Ivan",  vec![Console::Navigation]),
        ];
        assert!(LobbyView::new(&s, "a").all_consoles_filled());
    }

    // ── Station-based LobbyView ────────────────────────────────────────────

    fn two_station_ship() -> crate::stations_config::ShipStations {
        crate::stations_config::parse_and_validate(r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Captain"
description = "Solo command"
consoles = ["CaptainChair", "Helm"]
next = "Helm"

[[stations.2]]
name = "Helm"
description = "Pilot"
consoles = ["Helm", "CaptainChair"]
previous = "Captain"

[[stations.2]]
name = "Tactical"
description = "Weapons"
consoles = ["Tactical"]
"#).unwrap()
    }

    #[test]
    fn welcome_stores_ship_stations() {
        let mut s = LobbyState::default();
        let stations = two_station_ship();
        s.apply(&ServerMessage::Welcome {
            state: GameState { phase: GamePhase::Lobby, players: vec![], complexity: HashMap::new(), world: None },
            ship_stations: stations.clone(),
        });
        assert_eq!(s.ship_stations, stations);
    }

    #[test]
    fn station_slots_shows_one_row_per_station_at_current_player_count() {
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        // Two stationed players → 2P config (2 stations: Helm, Tactical)
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair, Console::Helm]),
            p("b", "Bob",   vec![Console::Tactical]),
        ];
        let slots = LobbyView::new(&s, "x").station_slots();
        let station_names: Vec<&str> = slots.iter().filter_map(|sl| match sl {
            StationSlot::Available { station, .. }
            | StationSlot::Occupied { station, .. }
            | StationSlot::Mine { station, .. } => Some(station.as_str()),
            StationSlot::Spectator { .. } => None,
        }).collect();
        assert_eq!(station_names, vec!["Helm", "Tactical"]);
    }

    #[test]
    fn station_slots_shows_available_for_empty_station() {
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        // 1 stationed player → 1P config (1 station: Captain)
        s.players = vec![p("a", "Alice", vec![Console::CaptainChair, Console::Helm])];
        let slots = LobbyView::new(&s, "x").station_slots();
        // x is unknown token → no mine, Alice holds "Captain" → occupied
        assert!(matches!(&slots[0], StationSlot::Occupied { station, holder_name, .. } if station == "Captain" && holder_name == "Alice"));
    }

    #[test]
    fn station_slots_mine_when_local_player_holds_the_station() {
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        s.players = vec![
            p("me", "Me",  vec![Console::CaptainChair, Console::Helm]),
            p("b",  "Bob", vec![Console::Tactical]),
        ];
        let slots = LobbyView::new(&s, "me").station_slots();
        // 2P: Helm=mine, Tactical=occupied
        assert!(matches!(&slots[0], StationSlot::Mine { station, .. } if station == "Helm"));
        assert!(matches!(&slots[1], StationSlot::Occupied { station, holder_name, .. } if station == "Tactical" && holder_name == "Bob"));
    }

    #[test]
    fn station_slots_includes_spectator_rows_in_arrival_order() {
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        // Two stationed players (2P), plus two spectators
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair, Console::Helm]),
            p("b", "Bob",   vec![Console::Tactical]),
            p("c", "Carol", vec![]),  // spectator 1
            p("d", "Dave",  vec![]),  // spectator 2
        ];
        let slots = LobbyView::new(&s, "x").station_slots();
        // 2 station rows + 2 spectator rows = 4 total
        assert_eq!(slots.len(), 4);
        assert!(matches!(&slots[2], StationSlot::Spectator { player_name } if player_name == "Carol"));
        assert!(matches!(&slots[3], StationSlot::Spectator { player_name } if player_name == "Dave"));
    }

    #[test]
    fn station_slots_shows_np_layout_when_second_player_joins_but_has_no_consoles() {
        // Regression: when player 2 joins before picking a station the UI must
        // still show the 2P layout (not revert to 1P) so the new player can see
        // and claim the available station.
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair, Console::Helm]), // advanced to 2P Helm
            p("b", "Bob",   vec![]),                                      // joined, not yet picked
        ];
        let slots = LobbyView::new(&s, "x").station_slots();
        // 2P config: Helm (occupied by Alice) + Tactical (available) — no spectator rows
        assert_eq!(slots.len(), 2, "must show 2P layout, not 1P");
        assert!(matches!(&slots[0], StationSlot::Occupied { station, .. } if station == "Helm"));
        assert!(matches!(&slots[1], StationSlot::Available { station, .. } if station == "Tactical"),
            "Tactical must be available for Bob to pick");
    }

    #[test]
    fn station_slots_propagates_short_code() {
        let ship = crate::stations_config::parse_and_validate(r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
consoles = ["CaptainChair"]
short_code = "BRG"
"#).unwrap();
        let mut s = LobbyState::default();
        s.ship_stations = ship;
        let slots = LobbyView::new(&s, "x").station_slots();
        assert!(matches!(&slots[0], StationSlot::Available { short_code, .. } if short_code == "BRG"));
    }

    #[test]
    fn station_slots_shows_station_description() {
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        s.players = vec![];
        // 0 stationed → display_count = min_players = 1 → 1P config: "Captain" with desc "Solo command"
        let slots = LobbyView::new(&s, "x").station_slots();
        assert!(matches!(&slots[0], StationSlot::Available { description, .. } if description == "Solo command"));
    }

    #[test]
    fn is_captain_uses_captainchair_console_membership() {
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        // 2P: Helm station has CaptainChair+Helm consoles
        s.players = vec![
            p("me", "Me",  vec![Console::CaptainChair, Console::Helm]),
            p("b",  "Bob", vec![Console::Tactical]),
        ];
        assert!( LobbyView::new(&s, "me").is_captain(), "me holds CaptainChair → is_captain");
        assert!(!LobbyView::new(&s, "b").is_captain(),  "bob holds Tactical only → not captain");
    }

    #[test]
    fn all_stations_filled_uses_ship_stations() {
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        // 2P with both stations filled
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair, Console::Helm]),
            p("b", "Bob",   vec![Console::Tactical]),
        ];
        assert!(LobbyView::new(&s, "a").all_stations_filled());
    }

    #[test]
    fn all_stations_filled_false_when_station_empty() {
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        // 2P: Alice has Helm, Bob has wrong consoles (Repair ≠ Tactical).
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair, Console::Helm]),
            p("b", "Bob",   vec![Console::Repair]),
        ];
        assert!(!LobbyView::new(&s, "a").all_stations_filled());
    }

    #[test]
    fn all_stations_filled_false_when_second_player_is_spectator() {
        // Two connected players but Bob hasn't taken a station. Player count is
        // 2, so the 2P config applies — Tactical is unfilled → Engage must not show.
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair, Console::Helm]),
            p("b", "Bob",   vec![]),
        ];
        assert!(!LobbyView::new(&s, "a").all_stations_filled());
    }

    #[test]
    fn all_stations_filled_true_when_overflow_spectator_present() {
        // Three players connected but max_players = 2. Alice and Bob have filled
        // both 2P slots; Carol is a spectator. Engage should still show.
        let mut s = LobbyState::default();
        s.ship_stations = two_station_ship();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair, Console::Helm]),
            p("b", "Bob",   vec![Console::Tactical]),
            p("c", "Carol", vec![]),
        ];
        assert!(LobbyView::new(&s, "a").all_stations_filled());
    }

    #[test]
    fn release_station_message_sends_release_station() {
        assert_eq!(release_station_message(), ClientMessage::ReleaseStation);
    }

    #[test]
    fn clicking_available_station_slot_sends_select_station() {
        let msg = message_for_station_slot_click(&StationSlot::Available {
            station: "Helm".into(),
            short_code: "HLM".into(),
            description: "Pilot".into(),
            rank: "Cpt.".into(),
            consoles: vec![Console::Helm],
            preset_names: vec!["Std".into()],
        });
        assert_eq!(msg, Some(ClientMessage::SelectStation { station: "Helm".into() }));
    }

    #[test]
    fn clicking_occupied_station_slot_yields_no_message() {
        let msg = message_for_station_slot_click(&StationSlot::Occupied {
            station: "Helm".into(),
            short_code: "HLM".into(),
            description: "Pilot".into(),
            rank: "Ltn.".into(),
            consoles: vec![Console::Helm],
            holder_name: "Alice".into(),
            preset_names: vec!["Std".into()],
        });
        assert!(msg.is_none());
    }

    #[test]
    fn clicking_mine_station_slot_yields_no_message() {
        let msg = message_for_station_slot_click(&StationSlot::Mine {
            station: "Helm".into(),
            short_code: "HLM".into(),
            description: "Pilot".into(),
            rank: "Cpt.".into(),
            consoles: vec![Console::Helm],
            preset_names: vec!["Std".into()],
        });
        assert!(msg.is_none());
    }

    #[test]
    fn clicking_spectator_slot_yields_no_message() {
        let msg = message_for_station_slot_click(&StationSlot::Spectator { player_name: "Alice".into() });
        assert!(msg.is_none());
    }

    // ── reconcile_active_console ────────────────────────────────────

    #[test]
    fn reconcile_keeps_current_when_present_in_bundle() {
        let result = reconcile_active_console(
            Some(Console::Helm),
            &[Console::CaptainChair, Console::Helm],
        );
        assert_eq!(result, Console::Helm);
    }

    #[test]
    fn reconcile_jumps_to_first_when_current_not_in_bundle() {
        let result = reconcile_active_console(
            Some(Console::Sensors),
            &[Console::CaptainChair, Console::Helm],
        );
        assert_eq!(result, Console::CaptainChair);
    }

    #[test]
    fn reconcile_none_lands_on_first_console() {
        let result = reconcile_active_console(None, &[Console::Tactical]);
        assert_eq!(result, Console::Tactical);
    }

    // ── Spectator UI: InProgress visibility ───────────────────

    #[test]
    fn is_spectator_true_when_local_player_has_no_consoles() {
        let mut s = LobbyState::default();
        s.players = vec![p("me", "Me", vec![])];
        assert!(LobbyView::new(&s, "me").is_spectator());
    }

    #[test]
    fn is_spectator_false_when_local_player_holds_a_console() {
        let mut s = LobbyState::default();
        s.players = vec![p("me", "Me", vec![Console::Helm])];
        assert!(!LobbyView::new(&s, "me").is_spectator());
    }

    #[test]
    fn is_spectator_true_when_token_not_found() {
        let s = LobbyState::default();
        assert!(LobbyView::new(&s, "ghost").is_spectator());
    }

    #[test]
    fn show_lobby_panel_true_during_lobby_phase_regardless_of_consoles() {
        let mut s = LobbyState::default();
        s.phase = GamePhase::Lobby;
        s.players = vec![p("me", "Me", vec![Console::Helm])];
        assert!(LobbyView::new(&s, "me").show_lobby_panel());
    }

    #[test]
    fn show_lobby_panel_true_during_in_progress_when_spectator() {
        let mut s = LobbyState::default();
        s.phase = GamePhase::InProgress;
        s.players = vec![p("me", "Me", vec![])];
        assert!(LobbyView::new(&s, "me").show_lobby_panel());
    }

    #[test]
    fn show_lobby_panel_false_during_in_progress_when_stationed() {
        let mut s = LobbyState::default();
        s.phase = GamePhase::InProgress;
        s.players = vec![p("me", "Me", vec![Console::Helm])];
        assert!(!LobbyView::new(&s, "me").show_lobby_panel());
    }

    #[test]
    fn game_in_progress_banner_true_when_in_progress_and_spectator() {
        let mut s = LobbyState::default();
        s.phase = GamePhase::InProgress;
        s.players = vec![p("me", "Me", vec![])];
        assert!(LobbyView::new(&s, "me").game_in_progress_banner());
    }

    #[test]
    fn game_in_progress_banner_false_during_lobby_phase() {
        let mut s = LobbyState::default();
        s.phase = GamePhase::Lobby;
        s.players = vec![p("me", "Me", vec![])];
        assert!(!LobbyView::new(&s, "me").game_in_progress_banner());
    }

    #[test]
    fn game_in_progress_banner_false_when_in_progress_and_stationed() {
        let mut s = LobbyState::default();
        s.phase = GamePhase::InProgress;
        s.players = vec![p("me", "Me", vec![Console::Helm])];
        assert!(!LobbyView::new(&s, "me").game_in_progress_banner());
    }

    // ── ComplexityChanged ─────────────────────────────────────────────

    #[test]
    fn complexity_changed_updates_local_state() {
        let mut s = LobbyState::default();
        s.apply(&ServerMessage::ComplexityChanged { console: Console::Helm, preset_name: "Low".into() });
        assert_eq!(s.complexity.get(&Console::Helm).map(|s| s.as_str()), Some("Low"));
    }

    #[test]
    fn complexity_changed_overwrites_previous_value() {
        let mut s = LobbyState::default();
        s.complexity.insert(Console::Helm, "Std".into());
        s.apply(&ServerMessage::ComplexityChanged { console: Console::Helm, preset_name: "Low".into() });
        assert_eq!(s.complexity.get(&Console::Helm).map(|s| s.as_str()), Some("Low"));
    }

    #[test]
    fn complexity_changed_for_unrelated_console_does_not_overwrite_others() {
        let mut s = LobbyState::default();
        s.complexity.insert(Console::Helm, "Std".into());
        s.apply(&ServerMessage::ComplexityChanged { console: Console::Tactical, preset_name: "Low".into() });
        assert_eq!(s.complexity.get(&Console::Helm).map(|s| s.as_str()), Some("Std"));
    }

    #[test]
    fn complexity_preset_for_returns_preset_when_set() {
        let mut s = LobbyState::default();
        s.complexity.insert(Console::Helm, "Low".into());
        let view = LobbyView::new(&s, "x");
        assert_eq!(view.complexity_preset_for(&Console::Helm), Some("Low"));
    }

    #[test]
    fn complexity_preset_for_returns_none_when_not_set() {
        let s = LobbyState::default();
        let view = LobbyView::new(&s, "x");
        assert!(view.complexity_preset_for(&Console::Helm).is_none());
    }
}
