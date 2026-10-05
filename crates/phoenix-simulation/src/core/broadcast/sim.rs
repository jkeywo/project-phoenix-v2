use bevy::prelude::*;

use crate::core::broadcast::broadcaster::{dispatch, BroadcastKind, Broadcaster};
use crate::core::messages::DeliveryClass;

/// Marker for the simulation (`InProgress`) broadcast phase.
///
/// - Delivery: `Snapshot` (lossy, latest-wins).
/// - Phase gate: none inline — the SimSet chain's
///   `.run_if(in_state(GamePhase::InProgress))` gates the whole set.
/// - Schedule: `FixedUpdate` (where `SimSet` lives since issue #895), inside
///   `SimSet::Broadcast`.
pub struct Sim;

impl BroadcastKind for Sim {
    fn delivery() -> DeliveryClass {
        DeliveryClass::Snapshot
    }

    fn add_dispatch(app: &mut App) {
        app.add_systems(
            FixedUpdate,
            dispatch::<Sim>
                .in_set(crate::sim_sets::SimSet::Broadcast)
                .in_set(crate::sim_sets::FixedStep::SimDispatch),
        );
    }
}

/// Bevy plugin that broadcasts `ServerMessage`s during the `InProgress` phase.
///
/// Use [`Broadcaster::register`] before adding the plugin to `App` to enqueue
/// producers. Each producer is called at the requested cadence and its output
/// is routed to the `Target` resolved from the `Audience`.
pub type SimBroadcaster = Broadcaster<Sim>;

/// Production callback order inside the one simulation dispatch (#1400).
/// Preserve the canonical composition's wire order even when the owning
/// plugins register in a different order. This does not order ECS systems.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimProducer {
    Repair,
    Power,
    Shields,
    Weapons,
    SimState,
    Modifier,
    Outbox,
}

impl SimProducer {
    fn rank(self) -> usize {
        match self {
            Self::Repair => 0,
            Self::Power => 1,
            Self::Shields => 2,
            Self::Weapons => 3,
            Self::SimState => 4,
            Self::Modifier => 5,
            Self::Outbox => 6,
        }
    }
}

impl SimBroadcaster {
    /// Build the registration for one production owner. Its single callback
    /// keeps its own audience, cadence and message sequence. Duplicate owners
    /// are refused; unlabelled generic registrations retain insertion order.
    pub(crate) fn for_producer(owner: SimProducer) -> Self {
        Self::for_order(owner.rank())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;
