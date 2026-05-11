//! Pure client-side lobby state model.
//!
//! Maintains a `LobbyState` (mirroring the server's `GameState` shape) by
//! applying inbound `ServerMessage`s, and derives a `LobbyView` from that
//! state plus the local player's token. Both pieces are deliberately
//! Bevy-free so they can be unit-tested on native and reused by the
//! WASM client UI layer.

use crate::messages::{ClientMessage, Console, GamePhase, GameState, Player, ServerMessage};
use bevy::prelude::Resource;

/// The client's authoritative model of the shared lobby state.
///
/// Built incrementally from `ServerMessage`s — every message that the
/// server can send while in the lobby phase is applied here so the UI
/// layer only ever reads from this struct.
#[derive(Clone, Debug, PartialEq, Resource)]
pub struct LobbyState {
    pub phase: GamePhase,
    pub players: Vec<Player>,
}

impl Default for LobbyState {
    fn default() -> Self {
        Self {
            phase: GamePhase::Lobby,
            players: Vec::new(),
        }
    }
}

impl LobbyState {
    /// Replace the entire lobby state — used on `Welcome`, which is the
    /// authoritative initial sync.
    pub fn replace_from(&mut self, state: GameState) {
        self.phase = state.phase;
        self.players = state.players;
    }

    /// Apply a single inbound `ServerMessage`. Variants that don't affect
    /// the lobby (e.g. `SimState`, `WorldSetup`) are ignored.
    pub fn apply(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::Welcome { state } => {
                self.replace_from(state.clone());
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
            ServerMessage::ConsoleSelected { token, consoles } => {
                // The server's authoritative ConsoleSelected carries the
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
            ServerMessage::ConsoleCleared { token } => {
                if let Some(p) = self.players.iter_mut().find(|p| &p.token == token) {
                    p.consoles.clear();
                }
            }
            ServerMessage::GameStarted => {
                self.phase = GamePhase::InProgress;
            }
            ServerMessage::SimState { .. }
            | ServerMessage::WorldSetup { .. }
            | ServerMessage::TargetLock { .. }
            | ServerMessage::WeaponsUpdate { .. }
            | ServerMessage::BeamStarted { .. }
            | ServerMessage::BeamEnded { .. }
            | ServerMessage::AsteroidDestroyed { .. }
            | ServerMessage::ScienceTargetSuggestion { .. }
            | ServerMessage::PhaserFired { .. }
            | ServerMessage::RepairState { .. }
            | ServerMessage::ShieldStatus { .. }
            | ServerMessage::TorpedoLaunched { .. }
            | ServerMessage::TorpedoDestroyed { .. } => {
                // Not relevant to the lobby model.
            }
        }
    }
}

/// All consoles the lobby UI knows how to render. Listed in the order
/// they appear on screen.
pub const ALL_CONSOLES: [Console; 5] = [Console::CaptainChair, Console::Helm, Console::Tactical, Console::Engineering, Console::Science];

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

    /// True if the local player currently holds the science console.
    pub fn is_science(&self) -> bool {
        self.my_consoles().contains(&Console::Science)
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
}

/// Returns the `ClientMessage` to send when the lobby UI activates the
/// given console slot. `Occupied` slots are unclickable, so they yield
/// `None`; `Available` and `Mine` both send `SelectConsole`, which the
/// server treats as a toggle.
pub fn message_for_slot_click(slot: &ConsoleSlot) -> Option<ClientMessage> {
    match slot {
        ConsoleSlot::Available { console } | ConsoleSlot::Mine { console } => {
            Some(ClientMessage::SelectConsole { console: console.clone() })
        }
        ConsoleSlot::Occupied { .. } => None,
    }
}

/// `ClientMessage` to send when the captain presses the Engage button.
pub fn engage_message() -> ClientMessage {
    ClientMessage::StartGame
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
                world: None,
            },
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
    fn console_selected_assigns_to_named_player() {
        let mut s = LobbyState::default();
        s.players = vec![p("a", "Alice", vec![]), p("b", "Bob", vec![])];
        s.apply(&ServerMessage::ConsoleSelected {
            token: "a".into(),
            consoles: vec![Console::CaptainChair],
        });
        assert_eq!(s.players[0].consoles, vec![Console::CaptainChair]);
        assert!(s.players[1].consoles.is_empty());
    }

    #[test]
    fn console_selected_steals_from_previous_holder() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::Helm]),
            p("b", "Bob",   vec![]),
        ];
        s.apply(&ServerMessage::ConsoleSelected {
            token: "b".into(),
            consoles: vec![Console::Helm],
        });
        assert!(s.players[0].consoles.is_empty(), "old holder loses the console");
        assert_eq!(s.players[1].consoles, vec![Console::Helm]);
    }

    #[test]
    fn console_cleared_empties_named_players_consoles() {
        let mut s = LobbyState::default();
        s.players = vec![p("a", "Alice", vec![Console::CaptainChair, Console::Helm])];
        s.apply(&ServerMessage::ConsoleCleared { token: "a".into() });
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
                hull_integrity: 100,
                authorized_repair_console: None,
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
    fn clicking_available_slot_sends_select_console() {
        let msg = message_for_slot_click(&ConsoleSlot::Available { console: Console::Helm });
        assert_eq!(msg, Some(ClientMessage::SelectConsole { console: Console::Helm }));
    }

    #[test]
    fn clicking_my_slot_sends_select_console_to_toggle_off() {
        let msg = message_for_slot_click(&ConsoleSlot::Mine { console: Console::CaptainChair });
        assert_eq!(msg, Some(ClientMessage::SelectConsole { console: Console::CaptainChair }));
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
    fn is_science_is_true_only_when_i_hold_the_science_console() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::Helm]),
            p("b", "Bob",   vec![Console::Science]),
        ];
        assert!(!LobbyView::new(&s, "a").is_science());
        assert!( LobbyView::new(&s, "b").is_science());
        assert!(!LobbyView::new(&s, "ghost").is_science());
    }

    #[test]
    fn all_consoles_filled_is_false_when_any_console_is_available() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair]),
            p("b", "Bob",   vec![Console::Helm]),
            p("c", "Carol", vec![Console::Tactical]),
            // Engineering not taken
        ];
        assert!(!LobbyView::new(&s, "a").all_consoles_filled());
    }

    #[test]
    fn all_consoles_filled_is_true_when_all_five_consoles_are_held() {
        let mut s = LobbyState::default();
        s.players = vec![
            p("a", "Alice", vec![Console::CaptainChair]),
            p("b", "Bob",   vec![Console::Helm]),
            p("c", "Carol", vec![Console::Tactical]),
            p("d", "Dave",  vec![Console::Engineering]),
            p("e", "Eve",   vec![Console::Science]),
        ];
        assert!(LobbyView::new(&s, "a").all_consoles_filled());
    }
}
