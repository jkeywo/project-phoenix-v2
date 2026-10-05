use super::*;

#[test]
fn receivers_apply_the_same_order_despite_different_arrival_orders() {
    let input = [(4, 2, 1, "last"), (4, 1, 8, "first"), (9, 1, 9, "future")];
    let mut a = CommandQueue::default();
    let mut b = CommandQueue::default();
    for &(tick, origin, seq, value) in &input {
        a.insert((tick, CommandOrder::new(HostSlot(origin), seq)), value);
    }
    for &(tick, origin, seq, value) in input.iter().rev() {
        b.insert((tick, CommandOrder::new(HostSlot(origin), seq)), value);
    }
    assert_eq!(a.drain_due(4).collect::<Vec<_>>(), vec!["first", "last"]);
    assert_eq!(b.drain_due(4).collect::<Vec<_>>(), vec!["first", "last"]);
    assert_eq!(a.values().copied().collect::<Vec<_>>(), vec!["future"]);
    assert_eq!(
        a.drain_due(9).collect::<Vec<_>>(),
        b.drain_due(9).collect::<Vec<_>>()
    );
}

#[test]
fn retransmission_does_not_duplicate_input_and_run_reset_preserves_identity() {
    let mut queue = CommandQueue::default();
    queue.set_origin(HostSlot(7));
    let key = (3, queue.next_order());
    assert_eq!(queue.insert(key, "first"), None);
    assert_eq!(queue.insert(key, "replacement"), Some("first"));
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.drain_due(3).collect::<Vec<_>>(), vec!["replacement"]);
    queue.clear();
    assert_eq!(queue.next_order(), CommandOrder::new(HostSlot(7), 0));
    assert!(queue.is_empty());
}
