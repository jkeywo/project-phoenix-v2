use std::collections::HashMap;

use crate::messages::{Console, Player};

#[derive(Debug)]
pub enum RegisterError {
    DuplicateToken,
}

#[derive(Debug, PartialEq)]
pub enum ConflictError {
    ConsoleTaken,
}

pub struct SessionManager {
    players: Vec<Player>,
    /// Last consoles assigned before disconnect — used for auto-reconnect restore.
    last_consoles: HashMap<String, Vec<Console>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self { players: Vec::new(), last_consoles: HashMap::new() }
    }

    fn idx(&self, token: &str) -> Option<usize> {
        self.players.iter().position(|p| p.token == token)
    }

    pub fn register(&mut self, token: String, name: String) -> Result<&Player, RegisterError> {
        if self.idx(&token).is_some() {
            return Err(RegisterError::DuplicateToken);
        }
        self.players.push(Player { token, name, consoles: Vec::new(), connected: true });
        Ok(self.players.last().unwrap())
    }

    pub fn reconnect(&mut self, token: &str) -> Option<&mut Player> {
        let idx = self.idx(token)?;
        self.players[idx].connected = true;
        if let Some(last) = self.last_consoles.get(token).cloned() {
            for console in last {
                let taken = self.players.iter()
                    .any(|p| p.connected && p.token != token && p.consoles.contains(&console));
                if !taken {
                    self.players[idx].consoles.push(console.clone());
                }
            }
        }
        self.last_consoles.remove(token);
        Some(&mut self.players[idx])
    }

    pub fn disconnect(&mut self, token: &str) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].connected = false;
            if !self.players[idx].consoles.is_empty() {
                self.last_consoles.insert(token.to_string(), self.players[idx].consoles.clone());
            }
            self.players[idx].consoles.clear();
        }
    }

    pub fn set_name(&mut self, token: &str, name: String) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].name = name;
        }
    }

    /// Add console if absent, remove if owned, error if another connected player holds it.
    pub fn toggle_console(&mut self, token: &str, console: Console) -> Result<bool, ConflictError> {
        // Check: is it held by someone else who is connected?
        let taken_by_other = self.players.iter()
            .any(|p| p.connected && p.token != token && p.consoles.contains(&console));
        if taken_by_other {
            return Err(ConflictError::ConsoleTaken);
        }
        if let Some(idx) = self.idx(token) {
            let player = &mut self.players[idx];
            if let Some(pos) = player.consoles.iter().position(|c| c == &console) {
                player.consoles.remove(pos);
                Ok(false) // was present → removed
            } else {
                player.consoles.push(console);
                Ok(true) // was absent → added
            }
        } else {
            Err(ConflictError::ConsoleTaken)
        }
    }

    /// Remove a single console by value — only if the player owns it; no-op otherwise.
    pub fn clear_console(&mut self, token: &str, console: Console) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].consoles.retain(|c| c != &console);
        }
    }

    /// Remove all consoles for this player.
    pub fn clear_consoles(&mut self, token: &str) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].consoles.clear();
        }
    }

    /// Consoles not held by any connected player.
    pub fn available_consoles(&self) -> Vec<Console> {
        let taken: Vec<Console> = self.players.iter()
            .filter(|p| p.connected)
            .flat_map(|p| p.consoles.clone())
            .collect();
        [Console::CaptainChair, Console::Helm]
            .into_iter()
            .filter(|c| !taken.contains(c))
            .collect()
    }

    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// Check if any connected player holds this console.
    pub fn has_console(&self, console: Console) -> bool {
        self.players.iter().any(|p| p.connected && p.consoles.contains(&console))
    }

    /// Get the connected player token(s) holding this console.  
    /// Returns the first one if multiple players hold it (shared console).
    pub fn console_holder(&self, console: Console) -> Option<&str> {
        self.players.iter()
            .find(|p| p.connected && p.consoles.contains(&console))
            .map(|p| p.token.as_str())
    }

    /// Check if a specific token is assigned this console.
    pub fn player_has_console(&self, token: &str, console: Console) -> bool {
        self.players.iter()
            .find(|p| p.token == token && p.connected)
            .is_some_and(|p| p.consoles.contains(&console))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sm() -> SessionManager {
        SessionManager::new()
    }

    #[test]
    fn register_new_player() {
        let mut sm = sm();
        let p = sm.register("t1".into(), "Alice".into()).unwrap();
        assert_eq!(p.token, "t1");
        assert_eq!(p.name, "Alice");
        assert!(p.connected);
        assert!(p.consoles.is_empty());
    }

    #[test]
    fn duplicate_token_fails() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(matches!(sm.register("t1".into(), "Bob".into()), Err(RegisterError::DuplicateToken)));
    }

    #[test]
    fn toggle_console_adds() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(sm.toggle_console("t1", Console::CaptainChair).unwrap());
        assert!(sm.players()[0].consoles.contains(&Console::CaptainChair));
    }

    #[test]
    fn toggle_console_removes() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        assert!(!sm.toggle_console("t1", Console::CaptainChair).unwrap());
        assert!(!sm.players()[0].consoles.contains(&Console::CaptainChair));
    }

    #[test]
    fn toggle_console_conflict_between_connected_players() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        assert_eq!(sm.toggle_console("t2", Console::CaptainChair), Err(ConflictError::ConsoleTaken));
    }

    #[test]
    fn toggle_console_allows_shared_unconnected_player() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        // t2 can now take CaptainChair even though Alice (disconnected) had it
        assert!(sm.toggle_console("t2", Console::CaptainChair).unwrap());
    }

    #[test]
    fn clear_console_removes_single() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.toggle_console("t1", Console::Helm).unwrap();
        assert_eq!(sm.players()[0].consoles.len(), 2);
        sm.clear_console("t1", Console::CaptainChair);
        assert!(!sm.players()[0].consoles.contains(&Console::CaptainChair));
        assert!(sm.players()[0].consoles.contains(&Console::Helm));
    }

    #[test]
    fn clear_consoles_removes_all() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.toggle_console("t1", Console::Helm).unwrap();
        sm.clear_consoles("t1");
        assert!(sm.players()[0].consoles.is_empty());
    }

    #[test]
    fn disconnect_releases_consoles_and_marks_disconnected() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        assert!(!sm.players()[0].connected);
        assert!(sm.players()[0].consoles.is_empty());
    }

    #[test]
    fn disconnected_console_becomes_available() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.toggle_console("t2", Console::Helm).unwrap();
        assert!(sm.available_consoles().is_empty());
        sm.disconnect("t1");
        assert_eq!(sm.available_consoles(), vec![Console::CaptainChair]);
    }

    #[test]
    fn reconnect_still_free_restores_console() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        sm.reconnect("t1");
        assert!(sm.players()[0].consoles.contains(&Console::CaptainChair));
    }

    #[test]
    fn reconnect_multiple_consoles() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.toggle_console("t1", Console::Helm).unwrap();
        sm.disconnect("t1");
        sm.reconnect("t1");
        assert!(sm.players()[0].consoles.contains(&Console::CaptainChair));
        assert!(sm.players()[0].consoles.contains(&Console::Helm));
    }

    #[test]
    fn reconnect_partial_restore_when_some_taken() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.toggle_console("t1", Console::Helm).unwrap();
        sm.disconnect("t1");
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.toggle_console("t2", Console::Helm).unwrap();
        sm.reconnect("t1");
        assert!(sm.players()[0].consoles.contains(&Console::CaptainChair));
        assert!(!sm.players()[0].consoles.contains(&Console::Helm));
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
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        assert_eq!(sm.console_holder(Console::CaptainChair), Some("t1"));
    }

    #[test]
    fn console_holder_returns_none_when_no_captain_chair() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert_eq!(sm.console_holder(Console::CaptainChair), None);
    }

    #[test]
    fn console_holder_returns_correct_helm() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.toggle_console("t2", Console::Helm).unwrap();
        assert_eq!(sm.console_holder(Console::Helm), Some("t2"));
    }

    #[test]
    fn has_console_returns_true_when_assigned() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        assert!(sm.has_console(Console::CaptainChair));
    }

    #[test]
    fn has_console_returns_false_when_not_assigned() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(!sm.has_console(Console::CaptainChair));
    }

    #[test]
    fn has_console_returns_false_when_disconnected() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::Helm).unwrap();
        sm.disconnect("t1");
        assert!(!sm.has_console(Console::Helm));
    }

    #[test]
    fn player_has_console_returns_true_when_assigned() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        assert!(sm.player_has_console("t1", Console::CaptainChair));
    }

    #[test]
    fn player_has_console_returns_false_when_not_assigned() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(!sm.player_has_console("t1", Console::CaptainChair));
    }
}
