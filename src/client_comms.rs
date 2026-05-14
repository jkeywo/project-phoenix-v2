//! Pure client-side Comms console state model.
//!
//! Maintains a `ClientCommsState` by applying inbound `ServerMessage`s, and
//! exposes outbound message builders. Deliberately Bevy-free so it can be
//! unit-tested on native.

use crate::messages::{ClientMessage, CommsContact, CommsMessage, ObjectiveSnapshot, ServerMessage};
use bevy::prelude::Resource;

/// The client's view of the Comms console state.
#[derive(Clone, Debug, PartialEq, Default, Resource)]
pub struct ClientCommsState {
    /// Inbox messages, in server-determined order.
    pub messages: Vec<CommsMessage>,
    /// Active objectives visible to the Comms operator.
    pub objectives: Vec<ObjectiveSnapshot>,
    /// Hailable contacts.
    pub contacts: Vec<CommsContact>,
    /// The message the operator has currently selected (opened in chat view).
    pub selected_message_id: Option<String>,
}

impl ClientCommsState {
    /// Apply a single inbound `ServerMessage`. Only `CommsState` is handled;
    /// all other variants are ignored.
    pub fn apply(&mut self, msg: &ServerMessage) {
        if let ServerMessage::CommsState { messages, objectives, contacts } = msg {
            self.messages = messages.clone();
            self.objectives = objectives.clone();
            self.contacts = contacts.clone();
            // Drop selected id if the message it pointed to no longer exists.
            if let Some(ref id) = self.selected_message_id {
                if !self.messages.iter().any(|m| &m.id == id) {
                    self.selected_message_id = None;
                }
            }
        }
    }

    /// Mark a message as the currently selected one (opens the chat view).
    /// Does nothing if `id` is not present in the current inbox.
    pub fn select_message(&mut self, id: &str) {
        if self.messages.iter().any(|m| m.id == id) {
            self.selected_message_id = Some(id.to_string());
        }
    }

    /// The message currently open in the chat view, if any.
    pub fn selected_message(&self) -> Option<&CommsMessage> {
        self.selected_message_id
            .as_ref()
            .and_then(|id| self.messages.iter().find(|m| &m.id == id))
    }

    /// Returns the available response texts for `msg`, or an empty slice if
    /// the operator has already responded (i.e. `selected_response` is set).
    pub fn available_responses<'a>(&self, msg: &'a CommsMessage) -> &'a [String] {
        if msg.selected_response.is_some() {
            &[]
        } else {
            &msg.responses
        }
    }

    /// Returns `true` if response buttons should be enabled for the currently
    /// selected message: the message exists and has not yet been responded to.
    pub fn response_buttons_enabled(&self) -> bool {
        match self.selected_message() {
            Some(msg) => msg.selected_response.is_none() && !msg.responses.is_empty(),
            None => false,
        }
    }
}

// ── Outbound message builders ──────────────────────────────────────────────

/// `ClientMessage` to send when the operator hails a target entity.
pub fn hail_message(target_uuid: &str) -> ClientMessage {
    ClientMessage::Hail { target_uuid: target_uuid.to_string() }
}

/// `ClientMessage` to send when the operator selects a message in the inbox.
pub fn select_comms_message(message_id: &str) -> ClientMessage {
    ClientMessage::SelectCommsMessage { message_id: message_id.to_string() }
}

/// `ClientMessage` to send when the operator chooses a response.
pub fn respond_to_message(message_id: &str, response_index: usize) -> ClientMessage {
    ClientMessage::RespondToMessage {
        message_id: message_id.to_string(),
        response_index,
    }
}

/// `ClientMessage` to send when the operator clears read/orphaned messages.
pub fn clear_comms_message() -> ClientMessage {
    ClientMessage::ClearComms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::ObjectiveStatus;

    fn contact(uuid: &str, name: &str) -> CommsContact {
        CommsContact { uuid: uuid.into(), name: name.into() }
    }

    fn msg(id: &str) -> CommsMessage {
        CommsMessage {
            id: id.into(),
            sender_uuid: "s-uuid".into(),
            sender_name: "Starbase".into(),
            subject: "Hello".into(),
            body: "Body text".into(),
            responses: vec!["Ack".into()],
            selected_response: None,
            is_read: false,
        }
    }

    fn comms_state(messages: Vec<CommsMessage>, contacts: Vec<CommsContact>) -> ServerMessage {
        ServerMessage::CommsState {
            messages,
            objectives: vec![],
            contacts,
        }
    }

    // ── apply ──────────────────────────────────────────────────────────────

    #[test]
    fn default_state_is_empty() {
        let s = ClientCommsState::default();
        assert!(s.messages.is_empty());
        assert!(s.objectives.is_empty());
        assert!(s.contacts.is_empty());
        assert!(s.selected_message_id.is_none());
    }

    #[test]
    fn apply_comms_state_replaces_messages_and_contacts() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![contact("c1", "Alpha Station")]));
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].id, "m1");
        assert_eq!(s.contacts.len(), 1);
        assert_eq!(s.contacts[0].name, "Alpha Station");
    }

    #[test]
    fn apply_comms_state_stores_objectives() {
        let mut s = ClientCommsState::default();
        let obj = ObjectiveSnapshot {
            id: "obj1".into(),
            text: "Make contact".into(),
            mandatory: true,
            status: ObjectiveStatus::Active,
        };
        s.apply(&ServerMessage::CommsState {
            messages: vec![],
            objectives: vec![obj.clone()],
            contacts: vec![],
        });
        assert_eq!(s.objectives, vec![obj]);
    }

    #[test]
    fn apply_comms_state_replaces_previous_data() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![contact("c1", "Alpha")]));
        s.apply(&comms_state(vec![msg("m2"), msg("m3")], vec![]));
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].id, "m2");
        assert!(s.contacts.is_empty());
    }

    #[test]
    fn apply_non_comms_message_does_not_affect_state() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        let before = s.clone();
        s.apply(&ServerMessage::GameStarted);
        assert_eq!(s, before);
    }

    #[test]
    fn apply_comms_state_preserves_selected_id_when_message_still_present() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1"), msg("m2")], vec![]));
        s.select_message("m1");
        // Update still contains m1.
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        assert_eq!(s.selected_message_id.as_deref(), Some("m1"));
    }

    #[test]
    fn apply_comms_state_clears_selected_id_when_message_removed() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        s.select_message("m1");
        // New state does not contain m1.
        s.apply(&comms_state(vec![msg("m2")], vec![]));
        assert!(s.selected_message_id.is_none());
    }

    // ── select_message ─────────────────────────────────────────────────────

    #[test]
    fn select_message_sets_selected_id_when_present() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1"), msg("m2")], vec![]));
        s.select_message("m2");
        assert_eq!(s.selected_message_id.as_deref(), Some("m2"));
    }

    #[test]
    fn select_message_does_nothing_for_unknown_id() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        s.select_message("ghost");
        assert!(s.selected_message_id.is_none());
    }

    // ── selected_message ───────────────────────────────────────────────────

    #[test]
    fn selected_message_returns_none_when_nothing_selected() {
        let s = ClientCommsState::default();
        assert!(s.selected_message().is_none());
    }

    #[test]
    fn selected_message_returns_the_selected_entry() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1"), msg("m2")], vec![]));
        s.select_message("m2");
        let sel = s.selected_message().unwrap();
        assert_eq!(sel.id, "m2");
    }

    // ── outbound builders ──────────────────────────────────────────────────

    #[test]
    fn hail_message_builder_produces_hail() {
        let m = hail_message("station-uuid");
        assert_eq!(m, ClientMessage::Hail { target_uuid: "station-uuid".into() });
    }

    #[test]
    fn select_comms_message_builder() {
        let m = select_comms_message("msg-42");
        assert_eq!(m, ClientMessage::SelectCommsMessage { message_id: "msg-42".into() });
    }

    #[test]
    fn respond_to_message_builder() {
        let m = respond_to_message("msg-1", 2);
        assert_eq!(m, ClientMessage::RespondToMessage { message_id: "msg-1".into(), response_index: 2 });
    }

    #[test]
    fn clear_comms_builder() {
        assert_eq!(clear_comms_message(), ClientMessage::ClearComms);
    }

    // ── response_buttons_enabled / available_responses ────────────────────

    // Cycle 40: buttons enabled when selected message has no response yet
    #[test]
    fn response_buttons_enabled_when_no_selected_response() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        s.select_message("m1");
        assert!(s.response_buttons_enabled());
    }

    // Cycle 41: buttons disabled after response is chosen (selected_response set)
    #[test]
    fn response_buttons_disabled_after_response() {
        let mut m = msg("m1");
        m.selected_response = Some(0);
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![m], vec![]));
        s.select_message("m1");
        assert!(!s.response_buttons_enabled());
    }

    // Cycle 42: buttons disabled when no message is selected
    #[test]
    fn response_buttons_disabled_when_no_message_selected() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        // Don't select any message
        assert!(!s.response_buttons_enabled());
    }

    // Cycle 43: available_responses returns empty slice when already responded
    #[test]
    fn available_responses_returns_empty_when_already_responded() {
        let mut m = msg("m1");
        m.selected_response = Some(0);
        let s = ClientCommsState::default();
        let responses = s.available_responses(&m);
        assert!(responses.is_empty());
    }

    // Cycle 44: available_responses returns full slice when not yet responded
    #[test]
    fn available_responses_returns_responses_when_not_responded() {
        let m = msg("m1");
        let s = ClientCommsState::default();
        let responses = s.available_responses(&m);
        assert_eq!(responses, &["Ack".to_string()]);
    }

    // Cycle 45: buttons disabled when message has no responses (empty list)
    #[test]
    fn response_buttons_disabled_when_message_has_no_responses() {
        let mut m = msg("m1");
        m.responses = vec![];
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![m], vec![]));
        s.select_message("m1");
        assert!(!s.response_buttons_enabled());
    }
}
