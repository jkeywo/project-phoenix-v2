// Pure Rust module for managing the server-side Comms inbox.
// No Bevy dependency. Owns all server-side comms message state.
//
// Messages are owned by the scenario that created them. On scenario unload,
// owned messages are marked orphaned (responses disabled, "transmission ended"
// marker). The Comms officer can clear orphaned and read messages via
// `ClearComms`.
//
// Public surface:
//   - `CommsInbox::inject` — add a message owned by a scenario
//   - `CommsInbox::unload_scenario` — orphan all messages for that scenario
//   - `CommsInbox::clear` — remove orphaned and read messages
//   - `CommsInbox::messages` — ordered snapshot of current inbox
//   - `CommsInbox::is_dirty` / `CommsInbox::mark_clean` — change tracking

use crate::messages::CommsMessage;

/// Server-side record for a single inbox message.
#[derive(Clone, Debug)]
struct InboxRecord {
    message: CommsMessage,
    /// The scenario ID that owns this message.
    scenario_id: String,
}

/// Server-side Comms inbox: stores messages across scenario boundaries.
#[derive(Clone, Debug, Default)]
pub struct CommsInbox {
    records: Vec<InboxRecord>,
    dirty: bool,
}

impl CommsInbox {
    /// Create an empty `CommsInbox`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject a comms message owned by `scenario_id` into the inbox.
    ///
    /// If a message with the same `id` already exists it is **not** duplicated;
    /// the call is a no-op. Returns `true` when the message was newly inserted.
    pub fn inject(&mut self, msg: CommsMessage, scenario_id: impl Into<String>) -> bool {
        if self.records.iter().any(|r| r.message.id == msg.id) {
            return false;
        }
        self.records.push(InboxRecord { message: msg, scenario_id: scenario_id.into() });
        self.dirty = true;
        true
    }

    /// Orphan all messages owned by `scenario_id`.
    ///
    /// Orphaned messages have `is_orphaned = true` and their response list is
    /// cleared (responses disabled). Returns the number of messages affected.
    pub fn unload_scenario(&mut self, scenario_id: &str) -> usize {
        let mut count = 0;
        for rec in self.records.iter_mut() {
            if rec.scenario_id == scenario_id && !rec.message.is_orphaned {
                rec.message.is_orphaned = true;
                rec.message.responses.clear();
                rec.message.selected_response = None;
                count += 1;
                self.dirty = true;
            }
        }
        count
    }

    /// Remove all orphaned and read messages from the inbox.
    ///
    /// Returns the number of messages removed.
    pub fn clear(&mut self) -> usize {
        let before = self.records.len();
        self.records.retain(|r| !r.message.is_orphaned && !r.message.is_read);
        let removed = before - self.records.len();
        if removed > 0 {
            self.dirty = true;
        }
        removed
    }

    /// Current inbox as an ordered slice of `CommsMessage` references.
    pub fn messages(&self) -> Vec<CommsMessage> {
        self.records.iter().map(|r| r.message.clone()).collect()
    }

    /// Returns `true` if the inbox has changed since last `mark_clean`.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the inbox as clean (no pending changes to broadcast).
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Explicitly mark the inbox as dirty (pending broadcast).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Record that the player chose `response_index` for message `message_id`.
    ///
    /// Sets `selected_response` and marks the message as read. No-op if the
    /// message is not found.
    pub fn record_response(&mut self, message_id: &str, response_index: usize) {
        if let Some(rec) = self.records.iter_mut().find(|r| r.message.id == message_id) {
            rec.message.selected_response = Some(response_index);
            rec.message.is_read = true;
            self.dirty = true;
        }
    }

    /// Return the `sender_uuid` of the message with `message_id`, or `None`.
    pub fn sender_uuid_for(&self, message_id: &str) -> Option<String> {
        self.records
            .iter()
            .find(|r| r.message.id == message_id)
            .map(|r| r.message.sender_uuid.clone())
    }

    /// Return the `sender_name` of the message with `message_id`, or `None`.
    pub fn sender_name_for(&self, message_id: &str) -> Option<String> {
        self.records
            .iter()
            .find(|r| r.message.id == message_id)
            .map(|r| r.message.sender_name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str) -> CommsMessage {
        CommsMessage {
            id: id.into(),
            sender_uuid: "s-uuid".into(),
            sender_name: "Station Alpha".into(),
            subject: "Distress".into(),
            body: "We are under attack!".into(),
            responses: vec!["Understood".into(), "On our way".into()],
            selected_response: None,
            is_read: false,
            is_orphaned: false,
        }
    }

    // Cycle 3a: new inbox is empty and clean
    #[test]
    fn new_inbox_is_empty_and_clean() {
        let inbox = CommsInbox::new();
        assert!(inbox.messages().is_empty());
        assert!(!inbox.is_dirty());
    }

    // Cycle 3b: inject adds message and sets dirty
    #[test]
    fn inject_adds_message_and_marks_dirty() {
        let mut inbox = CommsInbox::new();
        let inserted = inbox.inject(msg("m1"), "scenario-1");
        assert!(inserted);
        assert_eq!(inbox.messages().len(), 1);
        assert_eq!(inbox.messages()[0].id, "m1");
        assert!(inbox.is_dirty());
    }

    // Cycle 3c: inject is idempotent for same id
    #[test]
    fn inject_is_idempotent_for_same_id() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"), "scenario-1");
        inbox.mark_clean();
        let second = inbox.inject(msg("m1"), "scenario-1");
        assert!(!second);
        assert_eq!(inbox.messages().len(), 1);
        assert!(!inbox.is_dirty());
    }

    // Cycle 3d: unload_scenario orphans owned messages
    #[test]
    fn unload_scenario_orphans_owned_messages() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"), "scenario-1");
        inbox.inject(msg("m2"), "scenario-2");
        inbox.mark_clean();

        let count = inbox.unload_scenario("scenario-1");
        assert_eq!(count, 1);
        let msgs = inbox.messages();
        let m1 = msgs.iter().find(|m| m.id == "m1").unwrap();
        assert!(m1.is_orphaned);
        assert!(m1.responses.is_empty());
        // scenario-2 message untouched
        let m2 = msgs.iter().find(|m| m.id == "m2").unwrap();
        assert!(!m2.is_orphaned);
        assert!(inbox.is_dirty());
    }

    // Cycle 3e: unload_scenario does not re-orphan already-orphaned messages
    #[test]
    fn unload_scenario_skips_already_orphaned() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"), "scenario-1");
        inbox.unload_scenario("scenario-1");
        inbox.mark_clean();

        let count = inbox.unload_scenario("scenario-1");
        assert_eq!(count, 0);
        assert!(!inbox.is_dirty());
    }

    // Cycle 3f: clear removes orphaned and read messages
    #[test]
    fn clear_removes_orphaned_and_read_messages() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"), "scenario-1");
        let mut read = msg("m2");
        read.is_read = true;
        inbox.inject(read, "scenario-1");
        inbox.inject(msg("m3"), "scenario-1"); // active: stays
        inbox.unload_scenario("scenario-1"); // m1, m2, m3 all orphaned
        // Reinject m3 as active (non-orphaned) from a new scenario
        let mut inbox2 = CommsInbox::new();
        inbox2.inject(msg("m3"), "scenario-2"); // fresh, non-orphaned
        let mut read_m4 = msg("m4");
        read_m4.is_read = true;
        inbox2.inject(read_m4, "scenario-2");
        inbox2.mark_clean();

        let removed = inbox2.clear();
        assert_eq!(removed, 1); // only m4 (read)
        let remaining: Vec<_> = inbox2.messages().into_iter().map(|m| m.id).collect();
        assert_eq!(remaining, vec!["m3"]);
        assert!(inbox2.is_dirty());
    }

    // Cycle 3g: clear leaves unread non-orphaned messages alone
    #[test]
    fn clear_leaves_active_unread_messages() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"), "s1");
        inbox.mark_clean();
        let removed = inbox.clear();
        assert_eq!(removed, 0);
        assert!(!inbox.is_dirty());
        assert_eq!(inbox.messages().len(), 1);
    }

    // Cycle 3h: mark_clean clears dirty flag
    #[test]
    fn mark_clean_resets_dirty() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"), "s1");
        assert!(inbox.is_dirty());
        inbox.mark_clean();
        assert!(!inbox.is_dirty());
    }
}
