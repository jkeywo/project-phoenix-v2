use super::*;

fn identity(n: u8) -> PaneIdentity {
    PaneIdentity::adopt(
        format!("3f1a6c2e-0a11-4b3c-9d55-00000000000{n}"),
        format!("crew-{n}"),
    )
    .unwrap()
}

fn snapshot(tag: &str) -> PaneDispatch {
    snapshot_of(ServerMessageDiscriminants::GameStarted, tag)
}

fn snapshot_of(kind: ServerMessageDiscriminants, tag: &str) -> PaneDispatch {
    PaneDispatch {
        json: tag.to_string(),
        delivery: DeliveryClass::Snapshot,
        kind,
        pending: PendingSnapshot::Replace,
    }
}

fn reliable(tag: &str) -> PaneDispatch {
    PaneDispatch {
        json: tag.to_string(),
        delivery: DeliveryClass::Reliable,
        kind: ServerMessageDiscriminants::Welcome,
        pending: PendingSnapshot::Preserve,
    }
}

#[test]
fn opening_a_pane_creates_no_session_and_claims_no_station() {
    // A pane is a logical client, not a seat. It becomes a participant by
    // sending `Identify` through the seam like a phone, and this is the
    // "before" half of that claim.
    let mut registry = PaneRegistry::default();
    let id = registry.open(identity(1));
    let pane = registry.get(id).unwrap();
    assert_eq!(pane.lifecycle(), PaneLifecycle::Loading);
    assert_eq!(pane.queued_outbound(), 0);
    assert_eq!(registry.open_count(), 1);
}

#[test]
fn two_panes_have_different_identities_and_different_handles() {
    let mut registry = PaneRegistry::default();
    let a = registry.open(identity(1));
    let b = registry.open(identity(2));
    assert_ne!(a, b);
    assert_ne!(
        registry.get(a).unwrap().token(),
        registry.get(b).unwrap().token()
    );
}

#[test]
fn a_loading_pane_queues_its_backlog_instead_of_losing_it() {
    // `load_url` returns before the document's own scripts have run, so a
    // push aimed at a page in that window throws and the message is gone.
    // The one that matters is `Welcome`: a pane that missed it sits in the
    // lobby forever with a clean log.
    let mut registry = PaneRegistry::default();
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.push_outbound(reliable("welcome"));
    pane.push_outbound(snapshot("state"));
    assert!(
        pane.drain_outbound().is_empty(),
        "a loading document has no bridge to push into"
    );
    assert_eq!(pane.queued_outbound(), 2);

    pane.mark_live();
    let delivered: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
    assert_eq!(
        delivered,
        vec!["welcome".to_string(), "state".to_string()],
        "the backlog arrives in order, Welcome first"
    );
}

#[test]
fn an_overfull_pane_preserves_every_unsuperseded_message_and_reports_the_fault() {
    let mut registry = PaneRegistry::new(3);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.mark_live();
    assert_eq!(
        pane.push_outbound(reliable("welcome")),
        OutboundVerdict::Queued
    );
    // Three DIFFERENT kinds, so the cap is what decides here and not the
    // same-kind coalescing rule (tested on its own below).
    assert_eq!(
        pane.push_outbound(snapshot_of(ServerMessageDiscriminants::ShieldStatus, "s1")),
        OutboundVerdict::Queued
    );
    assert_eq!(
        pane.push_outbound(snapshot_of(
            ServerMessageDiscriminants::SystemHullUpdate,
            "s2"
        )),
        OutboundVerdict::Queued
    );
    assert_eq!(
        pane.push_outbound(snapshot_of(ServerMessageDiscriminants::RepairState, "s3")),
        OutboundVerdict::Overflowed
    );
    let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
    assert_eq!(
        queued,
        vec!["welcome".to_string(), "s1".to_string(), "s2".to_string()],
        "neither change-only projection was discarded without a replacement"
    );
}

#[test]
fn replacement_stops_at_reliable_messages_and_keeps_the_newest_within_each_segment() {
    // The page only ever applies the newest snapshot of a kind, and every
    // stale one it is handed is a synchronous script evaluation the frame
    // pays for. The survivor sits where the newest arrived — at the tail —
    // and no reliable message moves.
    let mut registry = PaneRegistry::new(8);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.mark_live();
    assert_eq!(
        pane.push_outbound(reliable("welcome")),
        OutboundVerdict::Queued
    );
    assert_eq!(pane.push_outbound(snapshot("s1")), OutboundVerdict::Queued);
    assert_eq!(
        pane.push_outbound(reliable("assigned")),
        OutboundVerdict::Queued
    );
    assert_eq!(pane.push_outbound(snapshot("s2")), OutboundVerdict::Queued);
    assert_eq!(
        pane.push_outbound(snapshot("s3")),
        OutboundVerdict::QueuedSuperseding
    );
    assert_eq!(pane.queued_outbound(), 4);
    let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
    assert_eq!(
        queued,
        vec![
            "welcome".to_string(),
            "s1".to_string(),
            "assigned".to_string(),
            "s3".to_string()
        ],
        "one snapshot survives per ordered segment; no snapshot crosses a reliable barrier"
    );
}

#[test]
fn snapshots_of_different_kinds_do_not_supersede_each_other() {
    // Two kinds carry two different pieces of state; the newest of each is
    // what the page needs, not the newest overall.
    let mut registry = PaneRegistry::new(8);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.mark_live();
    let a = ServerMessageDiscriminants::GameStarted;
    let b = ServerMessageDiscriminants::ShipDestroyed;
    assert_eq!(
        pane.push_outbound(snapshot_of(a, "a1")),
        OutboundVerdict::Queued
    );
    assert_eq!(
        pane.push_outbound(snapshot_of(b, "b1")),
        OutboundVerdict::Queued
    );
    assert_eq!(
        pane.push_outbound(snapshot_of(a, "a2")),
        OutboundVerdict::QueuedSuperseding
    );
    let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
    assert_eq!(queued, vec!["b1".to_string(), "a2".to_string()]);
}

#[test]
fn a_burst_of_one_kind_never_reaches_the_cap_or_touches_reliable_state() {
    // The feedback loop this rule breaks: a slow frame's worth of ticks
    // publishing the same snapshot kind over and over. However long the
    // burst, the queue holds one of them, and the cap never has to choose.
    let mut registry = PaneRegistry::new(3);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.mark_live();
    assert_eq!(
        pane.push_outbound(reliable("welcome")),
        OutboundVerdict::Queued
    );
    assert_eq!(pane.push_outbound(snapshot("s0")), OutboundVerdict::Queued);
    for n in 1..50 {
        assert_eq!(
            pane.push_outbound(snapshot(&format!("s{n}"))),
            OutboundVerdict::QueuedSuperseding
        );
        assert_eq!(pane.queued_outbound(), 2);
    }
    let queued: Vec<String> = pane.drain_outbound().into_iter().map(|d| d.json).collect();
    assert_eq!(queued, vec!["welcome".to_string(), "s49".to_string()]);
}

#[test]
fn requeue_reconciles_newer_snapshots_and_reliable_order_within_the_cap() {
    let mut registry = PaneRegistry::new(2);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.mark_live();
    pane.push_outbound(reliable("welcome"));
    pane.push_outbound(snapshot("old"));
    let batch = pane.drain_outbound();

    // These arrive while evaluate_script is working on the drained batch.
    pane.push_outbound(snapshot("new"));
    assert!(!pane.requeue_front(batch));
    assert_eq!(pane.queued_outbound(), 2);
    assert_eq!(
        pane.drain_outbound(),
        vec![reliable("welcome"), snapshot("new")]
    );
}

#[test]
fn requeue_never_merges_a_snapshot_across_a_reliable_barrier_to_avoid_overflow() {
    let mut registry = PaneRegistry::new(2);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.mark_live();
    pane.push_outbound(snapshot("old"));
    pane.push_outbound(reliable("assigned"));
    let batch = pane.drain_outbound();
    pane.push_outbound(snapshot("new"));
    assert!(pane.requeue_front(batch));
    assert_eq!(
        pane.drain_outbound(),
        vec![snapshot("old"), reliable("assigned")]
    );
}

#[test]
fn requeue_reports_overflow_instead_of_shedding_unsuperseded_snapshots() {
    let mut registry = PaneRegistry::new(2);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.mark_live();
    pane.push_outbound(reliable("welcome"));
    pane.push_outbound(snapshot("old"));
    let batch = pane.drain_outbound();

    pane.push_outbound(reliable("assigned"));
    assert!(pane.requeue_front(batch));
    assert_eq!(pane.queued_outbound(), 2);
    assert_eq!(
        pane.drain_outbound(),
        vec![reliable("welcome"), snapshot("old")]
    );
}

#[test]
fn requeue_reports_reliable_overflow_and_keeps_the_oldest_messages_in_order() {
    let mut registry = PaneRegistry::new(2);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.mark_live();
    pane.push_outbound(reliable("welcome"));
    pane.push_outbound(reliable("assigned"));
    let batch = pane.drain_outbound();

    pane.push_outbound(reliable("started"));
    assert!(pane.requeue_front(batch));
    assert_eq!(pane.queued_outbound(), 2);
    assert_eq!(
        pane.drain_outbound(),
        vec![reliable("welcome"), reliable("assigned")]
    );
}

#[test]
fn a_pane_that_is_all_reliable_and_full_reports_rather_than_dropping_state() {
    // Nothing here may be dropped without breaking the page, so the honest
    // answer is a refusal the caller can act on — not a silently lost
    // `StationAssigned`.
    let mut registry = PaneRegistry::new(2);
    let id = registry.open(identity(1));
    let pane = registry.get_mut(id).unwrap();
    pane.push_outbound(reliable("a"));
    pane.push_outbound(reliable("b"));
    assert_eq!(
        pane.push_outbound(snapshot("s")),
        OutboundVerdict::Overflowed
    );
    assert_eq!(
        pane.push_outbound(reliable("c")),
        OutboundVerdict::Overflowed
    );
    assert_eq!(pane.queued_outbound(), 2);
}

#[test]
fn closing_a_pane_reports_the_token_the_lobby_is_owed_a_disconnect_for() {
    let mut registry = PaneRegistry::default();
    let id = registry.open(identity(1));
    let token = registry.get(id).unwrap().token().to_string();
    assert_eq!(registry.close(id), Some((token, Vec::new())));
    assert_eq!(registry.open_count(), 0);
    assert_eq!(
        registry.close(id),
        None,
        "closing twice owes the lobby one disconnect, not two"
    );
}

#[test]
fn a_closed_panes_handle_is_never_reissued() {
    // A message naming a closed pane must be a message for a pane that has
    // gone — never one quietly delivered to whoever inherited the number.
    let mut registry = PaneRegistry::default();
    let first = registry.open(identity(1));
    registry.close(first);
    let second = registry.open(identity(2));
    assert_ne!(first, second);
    assert!(registry.get_mut(first).is_none());
    assert_eq!(
        registry.get(first).map(|p| p.lifecycle()),
        Some(PaneLifecycle::Closed),
        "the record stays, so the id resolves to 'gone' rather than to nothing"
    );
}

#[test]
fn a_document_that_finishes_loading_after_its_pane_closed_does_not_reopen_it() {
    let mut registry = PaneRegistry::default();
    let id = registry.open(identity(1));
    registry.close(id);
    // Reach past `get_mut`'s own guard to prove `mark_live` refuses too:
    // the two are separate defences and only one of them is on the path a
    // late Ultralight load callback takes.
    let pane = registry.panes.iter_mut().find(|p| p.id == id).unwrap();
    pane.mark_live();
    assert_eq!(pane.lifecycle(), PaneLifecycle::Closed);
}

#[test]
fn a_pane_is_found_by_the_token_it_presents_and_a_closed_one_is_not() {
    let mut registry = PaneRegistry::default();
    let id = registry.open(identity(1));
    let token = registry.get(id).unwrap().token().to_string();
    assert_eq!(registry.find_by_token(&token).map(|p| p.id()), Some(id));
    registry.close(id);
    assert!(registry.find_by_token(&token).is_none());
}
