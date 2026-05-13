use std::collections::{HashMap, VecDeque};

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
    /// Available consoles based on the ship's EntityConfig.
    /// If None, all consoles are available (default for backward compatibility).
    available_consoles: Option<Vec<Console>>,
    /// FIFO queue of spectator tokens — players waiting for a station slot.
    spectator_queue: VecDeque<String>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self { 
            players: Vec::new(), 
            last_consoles: HashMap::new(),
            available_consoles: None,
            spectator_queue: VecDeque::new(),
        }
    }
    
    /// Create a new SessionManager with available consoles from EntityConfig.
    pub fn new_with_config(config: &crate::entity_config::EntityConfig) -> Self {
        let mut available = Vec::new();
        
        if config.captain_console.is_some() {
            available.push(Console::CaptainChair);
        }
        if config.helm_console.is_some() {
            available.push(Console::Helm);
        }
        if config.weapons_console.is_some() {
            available.push(Console::Tactical);
        }
        if config.engineering_console.is_some() {
            available.push(Console::Repair);
        }
        // Science has no dedicated TOML config section; always available when a ship entity is loaded.
        available.push(Console::Science);
        
        Self { 
            players: Vec::new(), 
            last_consoles: HashMap::new(),
            available_consoles: Some(available),
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
        self.players.push(Player { token, name, consoles: Vec::new(), connected: true });
        Ok(self.players.last().unwrap())
    }

    pub fn reconnect(&mut self, token: &str) -> Option<&mut Player> {
        let idx = self.idx(token)?;
        self.players[idx].connected = true;
        // Issue #130: reconnect treats the player as a fresh joiner — no preferential
        // reassignment. Discard any saved consoles.
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
    /// Also rejects selection of consoles that are not available in the configured list.
    pub fn toggle_console(&mut self, token: &str, console: Console) -> Result<bool, ConflictError> {
        // Check: is this console available in the configured list?
        if let Some(ref available) = self.available_consoles {
            if !available.contains(&console) {
                return Err(ConflictError::ConsoleTaken); // Reuse error type for "not available"
            }
        }
        
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
        
        // Use configured available consoles, or default to all if not configured
        let all_available = self.available_consoles.as_deref().unwrap_or(&[
            Console::CaptainChair,
            Console::Helm,
            Console::Tactical,
            Console::Repair,
            Console::Science,
            Console::Power,
        ]);
        
        all_available
            .iter()
            .filter(|c| !taken.contains(c))
            .cloned()
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
        // CaptainChair and Helm are taken; Weapons and Engineering remain free
        assert!(!sm.available_consoles().contains(&Console::CaptainChair));
        assert!(!sm.available_consoles().contains(&Console::Helm));
        sm.disconnect("t1");
        assert!(sm.available_consoles().contains(&Console::CaptainChair));
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
    fn reconnect_does_not_restore_consoles_fresh_joiner_semantics() {
        // Issue #130: reconnect is a fresh join — no console restore.
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        sm.reconnect("t1");
        assert!(sm.players()[0].consoles.is_empty());
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

    #[test]
    fn weapons_console_can_be_selected_and_is_available_when_free() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(sm.available_consoles().contains(&Console::Tactical));
        sm.toggle_console("t1", Console::Tactical).unwrap();
        assert!(!sm.available_consoles().contains(&Console::Tactical));
    }

    #[test]
    fn repair_console_can_be_selected_and_is_available_when_free() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        assert!(sm.available_consoles().contains(&Console::Repair));
        sm.toggle_console("t1", Console::Repair).unwrap();
        assert!(!sm.available_consoles().contains(&Console::Repair));
    }

    #[test]
    fn weapons_console_becomes_available_on_disconnect() {
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::Tactical).unwrap();
        sm.disconnect("t1");
        assert!(sm.available_consoles().contains(&Console::Tactical));
    }

    #[test]
    fn repair_console_selectable_by_another_player_after_disconnect() {
        // After disconnect the console is free; another player can take it.
        let mut sm = sm();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::Repair).unwrap();
        sm.disconnect("t1");
        sm.register("t2".into(), "Bob".into()).unwrap();
        sm.toggle_console("t2", Console::Repair).unwrap();
        assert!(sm.players()[1].consoles.contains(&Console::Repair));
    }
    
    // ── Console Selectability Tests ────────────────────────────────────────
    
    fn sm_with_config(consoles: Vec<Console>) -> SessionManager {
        let mut sm = SessionManager::new();
        sm.available_consoles = Some(consoles);
        sm
    }
    
    #[test]
    fn all_consoles_available_with_all_sections_present() {
        let sm = sm_with_config(vec![
            Console::CaptainChair,
            Console::Helm,
            Console::Tactical,
            Console::Repair,
        ]);
        let available = sm.available_consoles();
        assert!(available.contains(&Console::CaptainChair));
        assert!(available.contains(&Console::Helm));
        assert!(available.contains(&Console::Tactical));
        assert!(available.contains(&Console::Repair));
    }
    
    #[test]
    fn weapons_console_hidden_when_not_in_config() {
        let sm = sm_with_config(vec![
            Console::CaptainChair,
            Console::Helm,
            Console::Repair,
        ]);
        let available = sm.available_consoles();
        assert!(!available.contains(&Console::Tactical));
    }
    
    #[test]
    fn captain_console_hidden_when_not_in_config() {
        let sm = sm_with_config(vec![
            Console::Helm,
            Console::Tactical,
            Console::Repair,
        ]);
        let available = sm.available_consoles();
        assert!(!available.contains(&Console::CaptainChair));
    }
    
    #[test]
    fn selecting_unavailable_console_is_rejected() {
        let mut sm = sm_with_config(vec![
            Console::CaptainChair,
            Console::Helm,
            Console::Repair,
        ]);
        sm.register("t1".into(), "Alice".into()).unwrap();
        // Weapons console is not available
        let result = sm.toggle_console("t1", Console::Tactical);
        assert!(matches!(result, Err(ConflictError::ConsoleTaken)));
    }
    
    #[test]
    fn default_session_manager_has_all_consoles() {
        let sm = SessionManager::new();
        let available = sm.available_consoles();
        assert!(available.contains(&Console::CaptainChair));
        assert!(available.contains(&Console::Helm));
        assert!(available.contains(&Console::Tactical));
        assert!(available.contains(&Console::Repair));
        assert!(available.contains(&Console::Science));
        assert!(available.contains(&Console::Power));
    }

    #[test]
    fn new_with_config_includes_science_console() {
        use crate::entity_config::EntityConfig;
        let toml_str = "[helm_console]\n[weapons_console]\n[engineering_console]\n[captain_console]\n";
        let config = EntityConfig::from_toml(toml_str).unwrap();
        let sm = SessionManager::new_with_config(&config);
        let available = sm.available_consoles();
        assert!(available.contains(&Console::Science), "Science must be in available_consoles after new_with_config");
    }

    #[test]
    fn science_console_selectable_after_new_with_config() {
        use crate::entity_config::EntityConfig;
        let toml_str = "[helm_console]\n[weapons_console]\n[engineering_console]\n[captain_console]\n";
        let config = EntityConfig::from_toml(toml_str).unwrap();
        let mut sm = SessionManager::new_with_config(&config);
        sm.register("t1".into(), "Alice".into()).unwrap();
        let result = sm.toggle_console("t1", Console::Science);
        assert!(result.is_ok(), "Science console toggle must succeed, got: {:?}", result);
    }

    // ── Spectator queue ───────────────────────────────────────────────────

    #[test]
    fn spectator_queue_starts_empty() {
        let sm = SessionManager::new();
        assert!(sm.spectator_queue().is_empty());
    }

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

    #[test]
    fn reconnect_does_not_restore_consoles() {
        // Issue #130: reconnect treats player as fresh joiner — no console restore.
        let mut sm = SessionManager::new();
        sm.register("t1".into(), "Alice".into()).unwrap();
        sm.toggle_console("t1", Console::CaptainChair).unwrap();
        sm.disconnect("t1");
        sm.reconnect("t1");
        assert!(sm.players()[0].consoles.is_empty(), "reconnect must not restore consoles");
    }
}
