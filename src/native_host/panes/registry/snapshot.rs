//! Queue-local snapshot semantics, after recipient projection.
//!
//! A delivery class says a transport may shed a frame; it does not say that a
//! later message of the same variant contains all of its state. Keep events in
//! order, replace complete projections, and compose sparse updates by key.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::messages::{DeliveryClass, EntityStateSnapshot, ServerMessage};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PendingSnapshot {
    /// Ordered events and unclassified payloads are never silently discarded.
    Preserve,
    /// The complete projected state for this recipient.
    Replace,
    /// Share the producer's typed delta until this recipient needs a merge.
    Delta(Arc<ServerMessage>),
}

impl PendingSnapshot {
    pub(super) fn for_message(delivery: DeliveryClass, message: &ServerMessage) -> Self {
        if delivery != DeliveryClass::Snapshot {
            return Self::Preserve;
        }
        match message {
            ServerMessage::BlackboardUpdate { .. } | ServerMessage::SimState { .. } => {
                Self::Delta(Arc::new(message.clone()))
            }
            ServerMessage::WeaponsUpdate { .. }
            | ServerMessage::RepairState { .. }
            | ServerMessage::PowerState { .. }
            | ServerMessage::ShieldStatus { .. }
            | ServerMessage::SystemHullUpdate { .. } => Self::Replace,
            // In particular ModifierAdded/Removed are events, despite arriving
            // on Snapshot. Unknown future variants get the conservative rule.
            _ => Self::Preserve,
        }
    }
}

/// Compose two delta batches in delivery order. Called only for one PaneId and
/// within one segment containing no reliable message or unclassified event.
pub(super) fn merge_delta(older: &ServerMessage, newer: &ServerMessage) -> Option<ServerMessage> {
    match (older, newer) {
        (
            ServerMessage::BlackboardUpdate { updates: old },
            ServerMessage::BlackboardUpdate { updates: new },
        ) => {
            // Each value is a COMPLETE blackboard, including withheld/empty
            // repair projections. Never merge fields inside a blackboard.
            let mut updates: BTreeMap<_, _> = old.iter().cloned().collect();
            updates.extend(new.iter().cloned());
            Some(ServerMessage::BlackboardUpdate {
                updates: updates.into_iter().collect(),
            })
        }
        (ServerMessage::SimState { snapshot: old }, ServerMessage::SimState { snapshot: new }) => {
            let mut entities: BTreeMap<_, _> = old
                .entity_states
                .iter()
                .map(|state| (state.uuid.clone(), state.clone()))
                .collect();
            for state in &new.entity_states {
                let merged = match entities.remove(&state.uuid) {
                    Some(previous) => merge_entity(previous, state.clone()),
                    None => state.clone(),
                };
                entities.insert(merged.uuid.clone(), merged);
            }
            // All non-entity fields are complete projections: empty means
            // clear, rather than retain an older Station/authority row.
            let mut snapshot = new.clone();
            snapshot.entity_states = entities.into_values().collect();
            Some(ServerMessage::SimState { snapshot })
        }
        _ => None,
    }
}

fn merge_entity(old: EntityStateSnapshot, new: EntityStateSnapshot) -> EntityStateSnapshot {
    // Explicit destructuring makes additions to the wire shape require an
    // omission-policy decision here instead of silently losing a new delta.
    let EntityStateSnapshot {
        uuid,
        position,
        yaw,
        hull_fraction,
        shield_fraction,
        flags,
        shields,
        shield_freq,
        warp_out_remaining_secs,
    } = new;
    EntityStateSnapshot {
        uuid,
        position: position.or(old.position),
        yaw: yaw.or(old.yaw),
        hull_fraction: hull_fraction.or(old.hull_fraction),
        shield_fraction: shield_fraction.or(old.shield_fraction),
        shields: shields.or(old.shields),
        shield_freq: shield_freq.or(old.shield_freq),
        // These are complete facts on a present entity row, not optional
        // position/health deltas; an empty flags list or absent warp timer clears.
        flags,
        warp_out_remaining_secs,
    }
}
