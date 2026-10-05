use super::*;

#[test]
fn delayed_and_duplicate_peer_delivery_preserves_canonical_movement() {
    let mut first = Grid::new(HostSlot(1), [HostSlot(2)]);
    let mut second = Grid::new(HostSlot(2), [HostSlot(1)]);
    let right = first.submit(Move { dx: 1, dy: 0 }).unwrap();
    let left = second.submit(Move { dx: -1, dy: 0 }).unwrap();
    assert!(first.advance());
    assert!(first.advance());
    assert!(
        !first.advance(),
        "the barrier waits for the peer before applying delayed input"
    );
    first.receive(left.clone());
    first.receive(left.clone());
    second.receive(right.clone());
    first.session.observe(HostSlot(2), 10);
    second.session.observe(HostSlot(1), 10);
    for _ in 0..5 {
        first.advance();
    }
    for _ in 0..7 {
        second.advance();
    }
    assert_eq!(first.state, second.state);
    assert_eq!(first.ledger.first_divergence(&second.ledger), None);
    assert_eq!(
        first.state.x, 0,
        "right then left, irrespective of receiver order"
    );
}

#[test]
fn recovery_preserves_pending_moves_and_issuer_sequence() {
    let mut original = Grid::new(HostSlot(1), []);
    original.submit(Move { dx: 1, dy: 0 });
    original.advance();
    let chunks = original.recovery_chunks().unwrap();
    let mut resumed = Grid::new(HostSlot(1), []);
    resumed.recover(&chunks).unwrap();
    let next = Move { dx: 0, dy: 1 };
    assert_eq!(original.submit(next.clone()), resumed.submit(next));
    for _ in 0..4 {
        original.advance();
        resumed.advance();
    }
    assert_eq!(original.state, resumed.state);
    assert_eq!((resumed.state.x, resumed.state.y), (1, 1));
}

#[test]
fn damaged_or_invalid_recovery_leaves_the_running_game_unchanged() {
    let mut grid = Grid::new(HostSlot(1), []);
    grid.advance();
    let before = grid.checkpoint().unwrap();
    let mut chunks = grid.recovery_chunks().unwrap();
    chunks[0].text.push('x');
    assert!(grid.recover(&chunks).is_err());
    assert_eq!(grid.checkpoint().unwrap(), before);
    assert!(grid
        .submit(Move {
            dx: i32::MIN,
            dy: i32::MAX
        })
        .is_none());
}

#[test]
fn shared_digest_history_detects_and_recovers_a_divergence() {
    let mut lead = Grid::new(HostSlot(1), []);
    let mut resumed = Grid::new(HostSlot(1), []);
    lead.advance();
    resumed.advance();
    resumed.state.x = 4;
    lead.advance();
    resumed.advance();
    let divergence = lead.ledger.first_divergence(&resumed.ledger).unwrap();
    assert_eq!((divergence.after, divergence.tick), (Some(1), 2));
    resumed.recover(&lead.recovery_chunks().unwrap()).unwrap();
    lead.submit(Move { dx: 1, dy: 0 });
    resumed.submit(Move { dx: 1, dy: 0 });
    for _ in 0..4 {
        lead.advance();
        resumed.advance();
    }
    assert_eq!(lead.digest(), resumed.digest());
    assert!(lead.ledger.first_divergence(&resumed.ledger).is_none());
}
