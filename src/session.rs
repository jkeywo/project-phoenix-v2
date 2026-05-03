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
    last_consoles: HashMap<String, Console>,
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
        self.players.push(Player { token, name, console: None, connected: true });
        Ok(self.players.last().unwrap())
    }

    pub fn reconnect(&mut self, token: &str) -> Option<&mut Player> {
        let idx = self.idx(token)?;
        self.players[idx].connected = true;
        if let Some(last) = self.last_consoles.get(token).cloned() {
            let taken = self.players.iter()
                .any(|p| p.connected && p.token != token && p.console == Some(last.clone()));
            if !taken {
                self.players[idx].console = Some(last);
                self.last_consoles.remove(token);
            }
        }
        Some(&mut self.players[idx])
    }

    pub fn disconnect(&mut self, token: &str) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].connected = false;
            if let Some(console) = self.players[idx].console.take() {
                self.last_consoles.insert(token.to_string(), console);
            }
        }
    }

    pub fn set_name(&mut self, token: &str, name: String) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].name = name;
        }
    }

    pub fn select_console(&mut self, token: &str, console: Console) -> Result<(), ConflictError> {
        let taken = self.players.iter()
            .any(|p| p.connected && p.token != token && p.console == Some(console.clone()));
        if taken {
            return Err(ConflictError::ConsoleTaken);
        }
        if let Some(idx) = self.idx(token) {
            self.players[idx].console = Some(console);
        }
        Ok(())
    }

    pub fn clear_console(&mut self, token: &str) {
        if let Some(idx) = self.idx(token) {
            self.players[idx].console = None;
        }
    }

    pub fn available_consoles(&self) -> Vec<Console> {
        let taken: Vec<Console> = self.players.iter()
            .filter(|p| p.connected)
            .filter_map(|p| p.console.clone())
            .collect();
        [Console::CaptainChair]
            .into_iter()
            .filter(|c| !taken.contains(c))
            .collect()
    }

    pub fn players(&self) -> &[Player] {
        &self.players
    }

    pub fn captain_token(&self) -> Option<&str> {
        self.players.iter()
            .find(|p| p.connected)
            .map(|p| p.token.as_str())
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
        assert!(p.console.is_none());
    }

    #[test]
    fn duplicate_token_fails() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(matches!(sm.register("t1".into(), "Bob".into()), Err(RegisterError::DuplicateToken)));
    }

    #[test]
    fn select_console_assigns_it() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        assert_eq!(sm.players()[0].console, Some(Console::CaptainChair));
    }

    #[test]
    fn select_console_conflict_between_connected_players() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        assert_eq!(sm.select_console("t2", Console::CaptainChair), Err(ConflictError::ConsoleTaken));
    }

    #[test]
    fn clear_console_removes_assignment() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        sm.clear_console("t1");
        assert!(sm.players()[0].console.is_none());
    }

    #[test]
    fn disconnect_releases_console_and_marks_disconnected() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        assert!(!sm.players()[0].connected);
        assert!(sm.players()[0].console.is_none());
    }

    #[test]
    fn disconnected_console_becomes_available() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        assert!(sm.available_consoles().is_empty());
        sm.disconnect("t1");
        assert_eq!(sm.available_consoles(), vec![Console::CaptainChair]);
    }

    #[test]
    fn disconnected_console_can_be_claimed_by_another() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.select_console("t2", Console::CaptainChair).unwrap();
        assert_eq!(sm.players().iter().find(|p| p.token == "t2").unwrap().console, Some(Console::CaptainChair));
    }

    #[test]
    fn reconnect_marks_connected() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.disconnect("t1");
        assert!(!sm.players()[0].connected);
        sm.reconnect("t1").unwrap();
        assert!(sm.players()[0].connected);
    }

    #[test]
    fn reconnect_unknown_token_returns_none() {
        let mut sm = sm();
        assert!(sm.reconnect("nope").is_none());
    }

    #[test]
    fn available_consoles_all_free_when_none_taken() {
        let sm = sm();
        assert_eq!(sm.available_consoles(), vec![Console::CaptainChair]);
    }

    #[test]
    fn available_consoles_empty_when_all_taken() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        assert!(sm.available_consoles().is_empty());
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
    fn captain_token_returns_first_connected_player() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        assert_eq!(sm.captain_token(), Some("t1"));
    }

    #[test]
    fn captain_token_changes_when_first_player_disconnects() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.disconnect("t1");
        assert_eq!(sm.captain_token(), Some("t2"));
    }

    #[test]
    fn captain_token_returns_none_when_all_disconnected() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.disconnect("t1");
        assert_eq!(sm.captain_token(), None);
    }

    #[test]
    fn reconnect_does_not_restore_console_taken_by_another() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        // Another player claims the console while t1 is gone
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.select_console("t2", Console::CaptainChair).unwrap();
        sm.reconnect("t1");
        assert!(sm.players().iter().find(|p| p.token == "t1").unwrap().console.is_none());
        assert_eq!(
            sm.players().iter().find(|p| p.token == "t2").unwrap().console,
            Some(Console::CaptainChair)
        );
    }

    #[test]
    fn reconnect_restores_previous_console_when_still_free() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.select_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        sm.reconnect("t1");
        assert_eq!(sm.players()[0].console, Some(Console::CaptainChair));
    }
}
