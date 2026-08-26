// Pure Rust module for managing the server-side Comms inbox.
// No Bevy dependency. Owns all server-side comms message state.
//
// PRD #342: legacy multi-scenario layering is gone. Messages are just
// CommsMessage records; the Comms officer clears read messages via
// `ClearComms`.
//
// Public surface:
//   - `CommsInbox::inject` — add a message
//   - `CommsInbox::clear` — remove orphaned and read messages
//   - `CommsInbox::messages` — ordered snapshot of current inbox
//   - `CommsInbox::is_dirty` / `CommsInbox::mark_clean` — change tracking

use crate::core::messages::{CommsMessage, CommsPriority};

/// Server-side record for a single inbox message.
#[derive(Clone, Debug)]
struct InboxRecord {
    message: CommsMessage,
}

/// Server-side Comms inbox: stores messages for the active world.
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

    /// Inject a comms message into the inbox.
    ///
    /// If a message with the same `id` already exists it is **not** duplicated;
    /// the call is a no-op. Returns `true` when the message was newly inserted.
    pub fn inject(&mut self, msg: CommsMessage) -> bool {
        if self.records.iter().any(|r| r.message.id == msg.id) {
            return false;
        }
        self.records.push(InboxRecord { message: msg });
        self.dirty = true;
        true
    }

    /// Remove all orphaned and read messages from the inbox.
    ///
    /// Returns the number of messages removed.
    pub fn clear(&mut self) -> usize {
        let before = self.records.len();
        self.records
            .retain(|r| !r.message.is_orphaned && !r.message.is_read);
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

    /// Remove a single message by id (no-op if not found). Used by the
    /// pending-follow-up timer to retire the `...` placeholder on expiry.
    pub fn remove(&mut self, message_id: &str) {
        let before = self.records.len();
        self.records.retain(|r| r.message.id != message_id);
        if self.records.len() != before {
            self.dirty = true;
        }
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

    /// Return the authoritative priority of `message_id`, including the legacy
    /// boolean fallback for hand-built old values.
    pub fn priority_for(&self, message_id: &str) -> Option<CommsPriority> {
        self.records
            .iter()
            .find(|r| r.message.id == message_id)
            .map(|r| r.message.effective_priority())
    }

    /// Remove Critical importance without removing the historical message.
    /// Used when an explicit inbox clear invalidates its dialogue surface.
    pub fn acknowledge_priority(&mut self, message_id: &str) {
        if let Some(rec) = self.records.iter_mut().find(|r| r.message.id == message_id) {
            if rec.message.effective_priority() == CommsPriority::Critical {
                rec.message.priority = CommsPriority::Routine;
                rec.message.is_urgent = false;
                self.dirty = true;
            }
        }
    }

    /// True while the latest live message in any thread is Critical.
    ///
    /// "Live" is presentation-independent: reading or visiting Comms does not
    /// acknowledge a safety interruption. A response, orphaning/removal, or a
    /// later message in the same thread does. Empty legacy thread ids fall back
    /// to the message id, matching the client grouping rule.
    pub fn has_live_critical_thread(&self) -> bool {
        let mut latest = std::collections::BTreeMap::<&str, &CommsMessage>::new();
        for record in &self.records {
            let message = &record.message;
            let thread = if message.thread_id.is_empty() {
                message.id.as_str()
            } else {
                message.thread_id.as_str()
            };
            latest.insert(thread, message);
        }
        latest.values().any(|message| {
            !message.is_orphaned
                && message.selected_response.is_none()
                && message.effective_priority() == CommsPriority::Critical
        })
    }

    /// Return the `sender_name` of the message with `message_id`, or `None`.
    pub fn sender_name_for(&self, message_id: &str) -> Option<String> {
        self.records
            .iter()
            .find(|r| r.message.id == message_id)
            .map(|r| r.message.sender_name.clone())
    }

    /// Return all messages that share `thread_id`, in insertion order.
    pub fn messages_for_thread(&self, thread_id: &str) -> Vec<&CommsMessage> {
        self.records
            .iter()
            .filter(|r| r.message.thread_id == thread_id)
            .map(|r| &r.message)
            .collect()
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
            body_params: Default::default(),
            responses: vec![
                crate::core::messages::CommsResponseView {
                    text: "Understood".into(),
                    important: false,
                    available: true,
                },
                crate::core::messages::CommsResponseView {
                    text: "On our way".into(),
                    important: false,
                    available: true,
                },
            ],
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: true,
            thread_id: id.into(),
            priority: CommsPriority::Routine,
            is_urgent: false,
        }
    }

    #[test]
    fn new_inbox_is_empty_and_clean() {
        let inbox = CommsInbox::new();
        assert!(inbox.messages().is_empty());
        assert!(!inbox.is_dirty());
    }

    #[test]
    fn inject_adds_message_and_marks_dirty() {
        let mut inbox = CommsInbox::new();
        let inserted = inbox.inject(msg("m1"));
        assert!(inserted);
        assert_eq!(inbox.messages().len(), 1);
        assert_eq!(inbox.messages()[0].id, "m1");
        assert!(inbox.is_dirty());
    }

    #[test]
    fn inject_is_idempotent_for_same_id() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"));
        inbox.mark_clean();
        let second = inbox.inject(msg("m1"));
        assert!(!second);
        assert_eq!(inbox.messages().len(), 1);
        assert!(!inbox.is_dirty());
    }

    #[test]
    fn clear_removes_orphaned_and_read_messages() {
        let mut inbox = CommsInbox::new();
        let mut orphaned = msg("m1");
        orphaned.is_orphaned = true;
        inbox.inject(orphaned);
        let mut read = msg("m2");
        read.is_read = true;
        inbox.inject(read);
        inbox.inject(msg("m3")); // active: stays
        inbox.mark_clean();

        let removed = inbox.clear();
        assert_eq!(removed, 2);
        let remaining: Vec<_> = inbox.messages().into_iter().map(|m| m.id).collect();
        assert_eq!(remaining, vec!["m3"]);
        assert!(inbox.is_dirty());
    }

    #[test]
    fn clear_leaves_active_unread_messages() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"));
        inbox.mark_clean();
        let removed = inbox.clear();
        assert_eq!(removed, 0);
        assert!(!inbox.is_dirty());
        assert_eq!(inbox.messages().len(), 1);
    }

    #[test]
    fn mark_clean_resets_dirty() {
        let mut inbox = CommsInbox::new();
        inbox.inject(msg("m1"));
        assert!(inbox.is_dirty());
        inbox.mark_clean();
        assert!(!inbox.is_dirty());
    }

    #[test]
    fn messages_for_thread_returns_matching_messages_in_order() {
        let mut inbox = CommsInbox::new();
        let mut m1 = msg("m1");
        m1.thread_id = "thread-a".into();
        let mut m2 = msg("m2");
        m2.thread_id = "thread-b".into();
        let mut m3 = msg("m3");
        m3.thread_id = "thread-a".into();
        inbox.inject(m1);
        inbox.inject(m2);
        inbox.inject(m3);

        let thread_a = inbox.messages_for_thread("thread-a");
        assert_eq!(thread_a.len(), 2);
        assert_eq!(thread_a[0].id, "m1");
        assert_eq!(thread_a[1].id, "m3");

        let thread_b = inbox.messages_for_thread("thread-b");
        assert_eq!(thread_b.len(), 1);
        assert_eq!(thread_b[0].id, "m2");

        assert!(inbox.messages_for_thread("missing").is_empty());
    }

    #[test]
    fn critical_is_latest_live_thread_state_not_an_unread_edge() {
        let mut inbox = CommsInbox::new();
        let mut critical = msg("critical");
        critical.thread_id = "safety".into();
        critical.priority = CommsPriority::Critical;
        critical.is_urgent = true;
        critical.is_read = true;
        inbox.inject(critical);

        assert!(inbox.has_live_critical_thread());
        assert!(
            inbox.has_live_critical_thread(),
            "reading or visiting Comms is not an acknowledgement"
        );

        let mut superseding = msg("later");
        superseding.thread_id = "safety".into();
        inbox.inject(superseding);
        assert!(
            !inbox.has_live_critical_thread(),
            "the latest message owns the thread's current priority"
        );
    }

    #[test]
    fn response_or_invalidation_clears_critical_idempotently() {
        let mut inbox = CommsInbox::new();
        let mut critical = msg("critical");
        critical.priority = CommsPriority::Critical;
        critical.is_urgent = true;
        inbox.inject(critical);
        inbox.record_response("critical", 0);
        assert!(!inbox.has_live_critical_thread());

        let mut invalidated = msg("invalidated");
        invalidated.priority = CommsPriority::Critical;
        invalidated.is_urgent = true;
        inbox.inject(invalidated);
        assert!(inbox.has_live_critical_thread());
        inbox.acknowledge_priority("invalidated");
        inbox.acknowledge_priority("invalidated");
        assert!(!inbox.has_live_critical_thread());
    }
}
