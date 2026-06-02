//! Pure client-side Comms console state model.
//!
//! Maintains a `ClientCommsState` by applying inbound `ServerMessage`s, and
//! exposes outbound message builders. Deliberately Bevy-free so it can be
//! unit-tested on native.

use crate::messages::{ClientMessage, CommsContact, CommsMessage, ObjectiveSnapshot, ServerMessage};
use bevy::prelude::Resource;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Returns the effective thread id for a message.
///
/// Old wire payloads (pre-threading) have `thread_id = ""`. Treat those as
/// their own thread by falling back to the message's own `id`.
fn effective_thread_id(msg: &CommsMessage) -> &str {
    if msg.thread_id.is_empty() { &msg.id } else { &msg.thread_id }
}

// ── ThreadSummary ──────────────────────────────────────────────────────────

/// A summary of one conversation thread shown in the inbox list.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadSummary {
    /// The shared thread identifier.
    pub thread_id: String,
    /// Sender name taken from the latest message in the thread.
    pub sender_name: String,
    /// Subject taken from the latest message in the thread.
    pub subject: String,
    /// True if any message in the thread is unread.
    pub any_unread: bool,
    /// True if the latest message's sender is out of range.
    pub latest_out_of_range: bool,
    /// True if the latest message is orphaned.
    pub latest_orphaned: bool,
}

// ── ClientCommsState ───────────────────────────────────────────────────────

/// The client's view of the Comms console state.
#[derive(Clone, Debug, PartialEq, Default, Resource)]
pub struct ClientCommsState {
    /// Inbox messages, in server-determined order.
    pub messages: Vec<CommsMessage>,
    /// Active objectives visible to the Comms operator.
    pub objectives: Vec<ObjectiveSnapshot>,
    /// Hailable contacts.
    pub contacts: Vec<CommsContact>,
    /// The thread the operator has currently open (selected in the inbox).
    pub selected_thread_id: Option<String>,
    /// Monotonically-increasing version number incremented on each `apply()`.
    /// Used by the refresh systems to detect state changes.
    pub version: u64,
    /// The last version that was "cleaned" — UI refresh systems set this after
    /// repopulating. `is_dirty()` returns `version != clean_version`.
    clean_version: u64,
}

impl ClientCommsState {
    /// Apply a single inbound `ServerMessage`. Only `CommsState` is handled;
    /// all other variants are ignored.
    pub fn apply(&mut self, msg: &ServerMessage) {
        if let ServerMessage::CommsState { messages, objectives, contacts } = msg {
            self.messages = messages.clone();
            self.objectives = objectives.clone();
            self.contacts = contacts.clone();
            // Drop selected thread if no messages with that thread_id remain.
            if let Some(ref tid) = self.selected_thread_id {
                if !self.messages.iter().any(|m| effective_thread_id(m) == tid.as_str()) {
                    self.selected_thread_id = None;
                }
            }
            self.version += 1;
        }
    }

    /// Open a conversation thread in the chat view.
    ///
    /// Does nothing if no message with `thread_id` exists.
    pub fn select_thread(&mut self, thread_id: &str) {
        if self.messages.iter().any(|m| effective_thread_id(m) == thread_id) {
            self.selected_thread_id = Some(thread_id.to_string());
            self.version += 1;
        }
    }

    /// All messages belonging to `thread_id`, in insertion (chronological) order.
    pub fn thread_messages(&self, thread_id: &str) -> Vec<&CommsMessage> {
        self.messages
            .iter()
            .filter(|m| effective_thread_id(m) == thread_id)
            .collect()
    }

    /// The active message for `thread_id`: the last message in the thread that
    /// still has pending responses (non-empty `responses`, no `selected_response`,
    /// not orphaned, sender in range).
    ///
    /// This is the message whose response buttons are shown at the bottom of the
    /// chat panel.
    pub fn active_message_for_thread(&self, thread_id: &str) -> Option<&CommsMessage> {
        self.thread_messages(thread_id)
            .into_iter()
            .rev()
            .find(|m| {
                !m.responses.is_empty()
                    && m.selected_response.is_none()
                    && !m.is_orphaned
                    && m.sender_in_range
            })
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

    /// True when the selected thread has an active message with pending responses.
    pub fn response_buttons_enabled(&self) -> bool {
        match &self.selected_thread_id {
            Some(tid) => self.active_message_for_thread(tid).is_some(),
            None => false,
        }
    }

    /// Returns `true` if a Hail click on `uuid` should produce an outbound
    /// message: the contact exists in the current snapshot and is in range.
    pub fn can_hail(&self, uuid: &str) -> bool {
        self.contacts.iter().any(|c| c.uuid == uuid && c.in_range)
    }

    /// Returns `true` if the state has changed since the last `mark_clean()`.
    pub fn is_dirty(&self) -> bool {
        self.version != self.clean_version
    }

    /// Mark the state as clean (no pending UI refresh needed).
    pub fn mark_clean(&mut self) {
        self.clean_version = self.version;
    }

    /// Clear the currently selected thread.
    pub fn clear_selection(&mut self) {
        if self.selected_thread_id.is_some() {
            self.selected_thread_id = None;
            self.version += 1;
        }
    }

    /// Thread summaries sorted for display: threads with any unread message
    /// first, then by position of first message in the inbox (stable, chronological).
    ///
    /// Each thread appears exactly once; its metadata reflects the **latest**
    /// message in the thread.
    pub fn sorted_threads(&self) -> Vec<ThreadSummary> {
        // Collect unique thread_ids in first-seen order (preserves inbox order).
        let mut seen: Vec<&str> = Vec::new();
        for msg in &self.messages {
            let tid = effective_thread_id(msg);
            if !seen.contains(&tid) {
                seen.push(tid);
            }
        }

        let mut summaries: Vec<ThreadSummary> = seen
            .into_iter()
            .map(|tid| {
                let thread_msgs = self.thread_messages(tid);
                let latest = thread_msgs.last().copied().expect("non-empty thread");
                let any_unread = thread_msgs.iter().any(|m| !m.is_read);
                ThreadSummary {
                    thread_id: tid.to_string(),
                    sender_name: latest.sender_name.clone(),
                    subject: latest.subject.clone(),
                    any_unread,
                    latest_out_of_range: !latest.sender_in_range,
                    latest_orphaned: latest.is_orphaned,
                }
            })
            .collect();

        // Sort: unread threads first, preserving relative order within each group.
        summaries.sort_by(|a, b| {
            let a_read = !a.any_unread as u8;
            let b_read = !b.any_unread as u8;
            a_read.cmp(&b_read)
        });

        summaries
    }
}

// ── Outbound message builders ──────────────────────────────────────────────

/// `ClientMessage` to send when the operator hails a target entity.
pub fn hail_message(target_uuid: &str) -> ClientMessage {
    ClientMessage::Hail { target_uuid: target_uuid.to_string() }
}

/// `ClientMessage` to send when the operator selects a thread in the inbox.
///
/// The server does not currently handle this message (it is forwarded but
/// ignored); it is sent for completeness. The `message_id` carries the
/// `thread_id` since only the client acts on this.
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
        CommsContact { uuid: uuid.into(), name: name.into(), in_range: true }
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
            is_orphaned: false,
            sender_in_range: true,
            thread_id: id.into(),
        }
    }

    fn msg_in_thread(id: &str, thread: &str) -> CommsMessage {
        let mut m = msg(id);
        m.thread_id = thread.into();
        m
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
        assert!(s.selected_thread_id.is_none());
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
    fn apply_comms_state_preserves_selected_thread_when_messages_remain() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1"), msg("m2")], vec![]));
        s.select_thread("m1");
        // Update still contains a message with thread_id "m1".
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        assert_eq!(s.selected_thread_id.as_deref(), Some("m1"));
    }

    #[test]
    fn apply_comms_state_clears_selected_thread_when_messages_removed() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        s.select_thread("m1");
        // New state does not contain any message with thread_id "m1".
        s.apply(&comms_state(vec![msg("m2")], vec![]));
        assert!(s.selected_thread_id.is_none());
    }

    // ── select_thread ──────────────────────────────────────────────────────

    #[test]
    fn select_thread_sets_id_when_present() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1"), msg("m2")], vec![]));
        s.select_thread("m2");
        assert_eq!(s.selected_thread_id.as_deref(), Some("m2"));
    }

    #[test]
    fn select_thread_does_nothing_for_unknown_id() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        s.select_thread("ghost");
        assert!(s.selected_thread_id.is_none());
    }

    // ── thread_messages ────────────────────────────────────────────────────

    #[test]
    fn thread_messages_returns_messages_in_thread_order() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(
            vec![
                msg_in_thread("m1", "t1"),
                msg_in_thread("m2", "t2"),
                msg_in_thread("m3", "t1"),
            ],
            vec![],
        ));
        let t1 = s.thread_messages("t1");
        assert_eq!(t1.len(), 2);
        assert_eq!(t1[0].id, "m1");
        assert_eq!(t1[1].id, "m3");
    }

    // ── active_message_for_thread ──────────────────────────────────────────

    #[test]
    fn active_message_is_last_unresponded_in_thread() {
        let mut s = ClientCommsState::default();
        let mut m1 = msg_in_thread("m1", "t1");
        m1.selected_response = Some(0); // already responded
        let m2 = msg_in_thread("m2", "t1"); // pending
        s.apply(&comms_state(vec![m1, m2], vec![]));
        let active = s.active_message_for_thread("t1").unwrap();
        assert_eq!(active.id, "m2");
    }

    #[test]
    fn active_message_none_when_all_responded() {
        let mut s = ClientCommsState::default();
        let mut m = msg_in_thread("m1", "t1");
        m.selected_response = Some(0);
        s.apply(&comms_state(vec![m], vec![]));
        assert!(s.active_message_for_thread("t1").is_none());
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

    #[test]
    fn response_buttons_enabled_when_thread_has_pending_response() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        s.select_thread("m1");
        assert!(s.response_buttons_enabled());
    }

    #[test]
    fn response_buttons_disabled_after_response() {
        let mut m = msg("m1");
        m.selected_response = Some(0);
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![m], vec![]));
        s.select_thread("m1");
        assert!(!s.response_buttons_enabled());
    }

    #[test]
    fn response_buttons_disabled_when_no_thread_selected() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        assert!(!s.response_buttons_enabled());
    }

    #[test]
    fn available_responses_returns_empty_when_already_responded() {
        let mut m = msg("m1");
        m.selected_response = Some(0);
        let s = ClientCommsState::default();
        let responses = s.available_responses(&m);
        assert!(responses.is_empty());
    }

    #[test]
    fn available_responses_returns_responses_when_not_responded() {
        let m = msg("m1");
        let s = ClientCommsState::default();
        let responses = s.available_responses(&m);
        assert_eq!(responses, &["Ack".to_string()]);
    }

    #[test]
    fn response_buttons_disabled_when_message_has_no_responses() {
        let mut m = msg("m1");
        m.responses = vec![];
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![m], vec![]));
        s.select_thread("m1");
        assert!(!s.response_buttons_enabled());
    }

    #[test]
    fn response_buttons_disabled_when_message_is_orphaned() {
        let mut m = msg("m1");
        m.is_orphaned = true;
        m.responses = vec![];
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![m], vec![]));
        s.select_thread("m1");
        assert!(!s.response_buttons_enabled());
    }

    // ── version / dirty tracking ──────────────────────────────────────────

    #[test]
    fn version_starts_at_zero() {
        let s = ClientCommsState::default();
        assert_eq!(s.version, 0);
    }

    #[test]
    fn apply_increments_version() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        assert_eq!(s.version, 1);
    }

    #[test]
    fn non_comms_message_does_not_increment_version() {
        let mut s = ClientCommsState::default();
        s.apply(&ServerMessage::GameStarted);
        assert_eq!(s.version, 0);
    }

    #[test]
    fn is_dirty_returns_true_after_apply() {
        let mut s = ClientCommsState::default();
        assert!(!s.is_dirty());
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        assert!(s.is_dirty());
    }

    #[test]
    fn mark_clean_clears_dirty() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        assert!(s.is_dirty());
        s.mark_clean();
        assert!(!s.is_dirty());
    }

    // ── clear_selection ────────────────────────────────────────────────────

    #[test]
    fn clear_selection_deselects_current_thread() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![msg("m1")], vec![]));
        s.select_thread("m1");
        assert!(s.selected_thread_id.is_some());
        s.clear_selection();
        assert!(s.selected_thread_id.is_none());
    }

    #[test]
    fn clear_selection_is_noop_when_nothing_selected() {
        let mut s = ClientCommsState::default();
        s.clear_selection();
        assert!(s.selected_thread_id.is_none());
    }

    // ── sorted_threads ─────────────────────────────────────────────────────

    fn read_msg(id: &str) -> CommsMessage {
        let mut m = msg(id);
        m.is_read = true;
        m
    }

    #[test]
    fn sorted_threads_returns_unread_first() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![read_msg("m1"), msg("m2")], vec![]));
        let threads = s.sorted_threads();
        assert_eq!(threads.len(), 2);
        assert!(threads[0].any_unread);
        assert_eq!(threads[0].thread_id, "m2");
        assert!(!threads[1].any_unread);
        assert_eq!(threads[1].thread_id, "m1");
    }

    #[test]
    fn sorted_threads_uses_latest_message_subject() {
        let mut s = ClientCommsState::default();
        let mut m1 = msg_in_thread("m1", "t1");
        m1.subject = "First".into();
        let mut m2 = msg_in_thread("m2", "t1");
        m2.subject = "Follow-up".into();
        s.apply(&comms_state(vec![m1, m2], vec![]));
        let threads = s.sorted_threads();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].subject, "Follow-up");
    }

    #[test]
    fn sorted_threads_groups_messages_by_thread_id() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(
            vec![
                msg_in_thread("m1", "t1"),
                msg_in_thread("m2", "t2"),
                msg_in_thread("m3", "t1"),
            ],
            vec![],
        ));
        let threads = s.sorted_threads();
        assert_eq!(threads.len(), 2);
    }

    // ── Slice 8: range flag passthrough + response gating ──────────────────

    #[test]
    fn apply_preserves_out_of_range_contact_flag() {
        let mut s = ClientCommsState::default();
        let mut c = contact("c1", "Far Station");
        c.in_range = false;
        s.apply(&comms_state(vec![], vec![c]));
        assert!(!s.contacts[0].in_range);
    }

    #[test]
    fn apply_preserves_out_of_range_message_flag() {
        let mut s = ClientCommsState::default();
        let mut m = msg("m1");
        m.sender_in_range = false;
        s.apply(&comms_state(vec![m], vec![]));
        assert!(!s.messages[0].sender_in_range);
    }

    #[test]
    fn response_buttons_disabled_when_sender_out_of_range() {
        let mut s = ClientCommsState::default();
        let mut m = msg("m1");
        m.sender_in_range = false;
        s.apply(&comms_state(vec![m], vec![]));
        s.select_thread("m1");
        assert!(!s.response_buttons_enabled());
    }

    #[test]
    fn can_hail_true_for_in_range_contact() {
        let mut s = ClientCommsState::default();
        s.apply(&comms_state(vec![], vec![contact("c1", "Alpha")]));
        assert!(s.can_hail("c1"));
    }

    #[test]
    fn can_hail_false_for_out_of_range_contact() {
        let mut s = ClientCommsState::default();
        let mut c = contact("c2", "Far");
        c.in_range = false;
        s.apply(&comms_state(vec![], vec![c]));
        assert!(!s.can_hail("c2"));
    }

    #[test]
    fn can_hail_false_for_unknown_contact() {
        let s = ClientCommsState::default();
        assert!(!s.can_hail("nope"));
    }
}
