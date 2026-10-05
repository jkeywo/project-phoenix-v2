use super::*;

fn msg(id: &str) -> CommsMessage {
    CommsMessage {
        id: id.into(),
        sender_uuid: "s-uuid".into(),
        sender_name: "Station Alpha".into(),
        subject: "Distress".into(),
        body: "We are under attack!".into(),
        body_params: Default::default(),
        recipient_ship: None,
        literal_body: false,
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
