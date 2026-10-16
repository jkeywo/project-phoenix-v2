use super::*;
use crate::core::messages::{HelmBlackboard, SystemBlackboard, SystemId};

fn packet(generation: u64, tick: u64, ids: &[&str]) -> ServerMessage {
    ServerMessage::BlackboardUpdate {
        presentation_generation: Some(generation),
        console_scope_tick: Some(tick),
        console_scope: Some(ids.iter().map(|id| SystemId((*id).into())).collect()),
        updates: ids
            .iter()
            .map(|id| {
                (
                    SystemId((*id).into()),
                    SystemBlackboard::Helm(HelmBlackboard {
                        x: tick as f32,
                        ..Default::default()
                    }),
                )
            })
            .map(|(id, board)| (id, board.into()))
            .collect(),
    }
}

fn same(actual: ServerMessage, expected: &ServerMessage) {
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn scoped_queue_rejects_stale_ticks_and_preserves_revocation() {
    let current = packet(2, 10, &["b"]);
    same(
        merge_delta(&current, &packet(2, 9, &["a", "b"])).unwrap(),
        &current,
    );
    let revoked = packet(2, 11, &[]);
    same(merge_delta(&current, &revoked).unwrap(), &revoked);
    same(merge_delta(&revoked, &current).unwrap(), &revoked);
    let regrant = packet(2, 12, &["a"]);
    same(merge_delta(&revoked, &regrant).unwrap(), &regrant);
}

#[test]
fn scoped_queue_rebases_generation_at_tick_zero_without_carrying_old_boards() {
    let old = packet(2, 99, &["a", "b"]);
    let rebased = packet(3, 0, &["b"]);
    same(merge_delta(&old, &rebased).unwrap(), &rebased);
    same(merge_delta(&rebased, &old).unwrap(), &rebased);
}

#[test]
fn unscoped_packet_cannot_replace_accepted_scope_even_with_newer_generation() {
    let current = packet(2, 10, &["b"]);
    for generation in [None, Some(2), Some(3)] {
        let unscoped = ServerMessage::BlackboardUpdate {
            presentation_generation: generation,
            console_scope: None,
            console_scope_tick: None,
            updates: ([(
                SystemId("a".into()),
                SystemBlackboard::Helm(HelmBlackboard::default()),
            )])
            .into_iter()
            .map(|(id, board)| (id, board.into()))
            .collect(),
        };
        same(merge_delta(&current, &unscoped).unwrap(), &current);
    }
}
