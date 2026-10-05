use crate::messages::{DeliveryClass, ServerMessage};
use crate::session_io::Target;
use bevy::prelude::*;
/// Carries the reason string — and, since #843, the structured
/// [`Outcome`](crate::outcome::Outcome) — when the game ends. Set before
/// transitioning to `GamePhase::GameOver`. The `OnEnter(GameOver)` system reads
/// `.0` and broadcasts the reason to all clients; the headless exit report
/// reads `.1` to classify victory vs defeat without string-matching the
/// per-world reason.
///
/// Field `.0` (display string) is unchanged — every existing site that reads or
/// writes it keeps working. Field `.1` is the outcome: `Some(Defeat)` at the
/// built-in player-death sites, whatever a scenario declared on its `game_over`
/// action, or `None` for an undeclared scripted end (the classifier defaults
/// that to victory).
#[derive(Resource, Default)]
pub struct GameOverReason(pub Option<String>, pub Option<crate::outcome::Outcome>);

/// Pending outbound messages produced by simulation systems.
/// Drained each logical fixed tick by the `SimBroadcaster` dispatch in
/// `SimSet::Broadcast` while the in-progress fixed loop advances.
///
/// ## Migration note (T1 Architecture PRD #1249, issue #1262)
/// The old preamble pattern (`MessageWriter<OutboundMessage>`) has been
/// eliminated from simulation domain plugins. Producers now choose
/// [`SimOutbox::push_snapshot`] or [`SimOutbox::push_reliable`] at the point
/// where cadence and loss semantics are known. The raw queue is private, so a
/// new producer cannot compile without making that choice. The
/// `sim_outbox_broadcaster()` drains the already-classified entries to the
/// `OutboundMessage` bus without inspecting their payload variants.
/// The intentional direct-message exception is
/// `debug_overlay::report_debug_state`: it emits Reliable `DebugState` from
/// `PreUpdate`, because a pause stops the fixed loop that drains this outbox
/// and the client still has to receive confirmation.
/// PRD #253 introduced the earlier broadcaster seam; #1249 is the parent PRD
/// for this explicit producer-owned delivery classification.
#[derive(Resource, Default)]
pub struct SimOutbox {
    entries: Vec<SimOutboxEntry>,
}

pub struct SimOutboxEntry {
    pub target: Target,
    pub message: ServerMessage,
    pub delivery: DeliveryClass,
}

impl SimOutbox {
    /// Queue a lossy latest-state projection for the snapshot DataChannel.
    pub fn push_snapshot(&mut self, (target, message): (Target, ServerMessage)) {
        self.push(target, message, DeliveryClass::Snapshot);
    }

    /// Queue an ordered event that must be delivered reliably.
    pub fn push_reliable(&mut self, (target, message): (Target, ServerMessage)) {
        self.push(target, message, DeliveryClass::Reliable);
    }

    /// Queue a batch of latest-state projections with one explicit class.
    pub fn extend_snapshot(&mut self, entries: impl IntoIterator<Item = (Target, ServerMessage)>) {
        self.entries
            .extend(entries.into_iter().map(|(target, message)| SimOutboxEntry {
                target,
                message,
                delivery: DeliveryClass::Snapshot,
            }));
    }

    /// Queue a batch of ordered events with one explicit class.
    pub fn extend_reliable(&mut self, entries: impl IntoIterator<Item = (Target, ServerMessage)>) {
        self.entries
            .extend(entries.into_iter().map(|(target, message)| SimOutboxEntry {
                target,
                message,
                delivery: DeliveryClass::Reliable,
            }));
    }

    /// Read queued entries without exposing a mutable raw insertion path.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&Target, &ServerMessage)> {
        self.entries
            .iter()
            .map(|entry| (&entry.target, &entry.message))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn push(&mut self, target: Target, message: ServerMessage, delivery: DeliveryClass) {
        self.entries.push(SimOutboxEntry {
            target,
            message,
            delivery,
        });
    }

    pub fn drain(&mut self) -> Vec<SimOutboxEntry> {
        std::mem::take(&mut self.entries)
    }
}
