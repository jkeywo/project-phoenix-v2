use super::*;
fn begun() -> Continuation {
    let mut state = Continuation::default();
    state
        .begin(
            2,
            HostSlot(1),
            HostSlot(2),
            vec![HostSlot(2), HostSlot(3)],
            HostSlot(1),
            HostSlot(2),
            vec![HostSlot(1), HostSlot(2), HostSlot(3)],
        )
        .unwrap();
    state
}
#[test]
fn continuation_holds_until_validated_effects_are_committed() {
    let mut state = begun();
    assert!(state
        .prepare_commit(
            2,
            HostSlot(1),
            HostSlot(2),
            72,
            &[HostSlot(2), HostSlot(3)],
            Some(71),
            70
        )
        .is_err());
    state.acknowledge_replay(2, Some(71)).unwrap();
    assert_eq!(state.status().loss_tick, Some(72));
    assert!(state
        .prepare_commit(
            2,
            HostSlot(1),
            HostSlot(2),
            72,
            &[HostSlot(2)],
            Some(71),
            70
        )
        .is_err());
    assert!(state
        .prepare_commit(
            2,
            HostSlot(1),
            HostSlot(2),
            73,
            &[HostSlot(2), HostSlot(3)],
            Some(71),
            70
        )
        .is_err());
    let prepared = state
        .prepare_commit(
            2,
            HostSlot(1),
            HostSlot(2),
            72,
            &[HostSlot(3), HostSlot(2)],
            Some(71),
            70,
        )
        .unwrap();
    assert!(state.held());
    state.commit(prepared).unwrap();
    assert!(!state.held());
    state.mark_pending();
    assert!(state.retry_commit(2, HostSlot(1), HostSlot(2), 72));
    assert!(!state.retry_commit(2, HostSlot(1), HostSlot(2), 73));
    assert_eq!(state.status().status, ContinuationPhase::Committed);
}
#[test]
fn refusal_cannot_be_cleared_by_replay_or_a_prepared_commit() {
    let mut state = begun();
    state.acknowledge_replay(2, Some(71)).unwrap();
    let prepared = state
        .prepare_commit(
            2,
            HostSlot(1),
            HostSlot(2),
            72,
            &[HostSlot(2), HostSlot(3)],
            Some(71),
            70,
        )
        .unwrap();
    state.refuse("bad-tail");
    state.mark_pending();
    assert!(!state.accepts_replay(2));
    assert!(state.acknowledge_replay(2, Some(71)).is_err());
    assert!(state.commit(prepared).is_err());
    assert_eq!(state.status().status, ContinuationPhase::Refused);
    assert!(state.held());
}
#[test]
fn replay_without_begin_and_conflicting_begin_are_refused() {
    assert!(Continuation::default()
        .acknowledge_replay(2, Some(71))
        .is_err());
    let mut state = begun();
    assert!(state
        .begin(
            3,
            HostSlot(1),
            HostSlot(2),
            vec![HostSlot(2), HostSlot(3)],
            HostSlot(1),
            HostSlot(2),
            vec![HostSlot(1), HostSlot(2), HostSlot(3)]
        )
        .is_err());
    assert_eq!(state.transaction().unwrap().epoch, 2);
}
