//! Shared Bevy adapter for restoring a save into a freshly bootstrapped App.
//!
//! Browser and native edges validate and stage their local record, then call
//! [`advance`] once per frame and report its terminal outcome. This module owns
//! the whole restore lifecycle; neither storage nor presentation decides when
//! to reconcile, rebuild, verify, or release the capture gate.

use bevy::prelude::*;

use crate::save_slots_lifecycle;
use crate::snapshot::{self, LayerReconcileStatus, StoredRun};

/// Post-bootstrap waiting frames, including layer reconstruction. Lobby time
/// before the GameStart roster walk does not consume this local patience budget.
/// This retains the existing host restore limit; it is not simulation time.
pub const RESTORE_DEADLINE_FRAMES: u32 = 1_800;

#[derive(Resource)]
struct StagedRestore {
    run: StoredRun,
    waited_frames: u32,
}

/// A terminal failure for target-local reporting. Restore may already have
/// changed the World: cancellation resumes capture at its current continuation
/// and does not claim to roll back those changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreFailure {
    NoSnapshot,
    LayerFailed { path: String },
    NotReady { tick: u64, entities: usize },
    DigestMismatch { expected: u64, actual: u64 },
    Incomplete { tick: u64, gaps: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    Applied { tick: u64 },
    Failed(RestoreFailure),
}

/// Whether this fresh App still owes a terminal restore outcome.
pub fn is_pending(world: &World) -> bool {
    world.contains_resource::<StagedRestore>()
}

/// Hand off a fully validated record before the fresh App's first update.
/// Callers own compatibility/boot identity checks and refuse live-session
/// staging. The browser calls this only while constructing its one new App;
/// native staging also checks SimTick and refuses a second staged record.
pub(crate) fn stage(world: &mut World, run: StoredRun) {
    assert!(!is_pending(world), "startup restore already staged");
    world
        .get_resource_or_insert_with(crate::authoritative::StateCensus::default)
        .declare(
            std::any::type_name::<StagedRestore>(),
            crate::authoritative::StateClass::Timer,
            "startup-save-restore-driver",
        );
    world.insert_resource(StagedRestore {
        run,
        waited_frames: 0,
    });
    save_slots_lifecycle::begin_startup_restore(world);
}

/// Advance the shared lifecycle once per rendered frame. A terminal outcome is
/// returned exactly once, after verification and capture-gate resolution.
pub(crate) fn advance(world: &mut World) -> Option<RestoreOutcome> {
    // Own the record while probing/mutating the World, avoiding a full snapshot
    // clone on every waiting frame. Only an unresolved restore is reinserted.
    let mut staged = world.remove_resource::<StagedRestore>()?;
    let outcome = advance_staged(world, &mut staged);
    match &outcome {
        None => world.insert_resource(staged),
        Some(RestoreOutcome::Applied { tick }) => {
            save_slots_lifecycle::complete_startup_restore(world, *tick);
        }
        Some(RestoreOutcome::Failed(_)) => {
            save_slots_lifecycle::cancel_startup_restore(world);
        }
    }
    outcome
}

fn advance_staged(world: &mut World, staged: &mut StagedRestore) -> Option<RestoreOutcome> {
    let Some(snapshot) = staged.run.snapshot.as_ref() else {
        return Some(RestoreOutcome::Failed(RestoreFailure::NoSnapshot));
    };

    // OnEnter's commands are deferred, and scripts may subsequently leave
    // InProgress. This roster marker records the completed walk even when it
    // was empty, so it is the durable prerequisite shared by both hosts.
    if !world.contains_resource::<crate::server_app::GameStartEntityUuids>() {
        return None;
    }

    let layers_ready = match snapshot::reconcile_world_layers(world, &snapshot.state) {
        LayerReconcileStatus::Ready => true,
        LayerReconcileStatus::Waiting => false,
        LayerReconcileStatus::Failed(path) => {
            return Some(RestoreOutcome::Failed(RestoreFailure::LayerFailed { path }));
        }
    };
    let ready = layers_ready && snapshot::ready_to_restore(world, &snapshot.state);
    if !ready {
        staged.waited_frames = staged.waited_frames.saturating_add(1);
        if staged.waited_frames < RESTORE_DEADLINE_FRAMES {
            return None;
        }
        // Entity rebuilding cannot substitute for missing layer ASTs or a
        // still-pending topology change. Both kinds of wait spend one budget.
        if !layers_ready || !snapshot::ready_to_rebuild(world, &snapshot.state) {
            return Some(RestoreOutcome::Failed(RestoreFailure::NotReady {
                tick: snapshot.tick,
                entities: snapshot.state.entities.len(),
            }));
        }
    }

    let report = snapshot::restore(world, &snapshot.state);
    let actual = crate::sim_digest::world_digest(world);
    let outcome = if actual != snapshot.digest {
        RestoreOutcome::Failed(RestoreFailure::DigestMismatch {
            expected: snapshot.digest,
            actual,
        })
    } else if !report.is_complete() {
        RestoreOutcome::Failed(RestoreFailure::Incomplete {
            tick: snapshot.tick,
            gaps: report.gaps.len(),
        })
    } else {
        RestoreOutcome::Applied {
            tick: snapshot.tick,
        }
    };
    Some(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_snapshot_resolves_once_even_before_bootstrap() {
        let mut run = snapshot::run_for(
            snapshot::PhoenixSnapshot::default(),
            0,
            42,
            "assets/worlds/duel.toml",
            vellum_save::Versions::new(1, "test", 0),
        );
        run.snapshot = None;
        let mut world = World::new();
        stage(&mut world, run);
        assert!(save_slots_lifecycle::startup_restore_pending(&world));
        assert_eq!(
            advance(&mut world),
            Some(RestoreOutcome::Failed(RestoreFailure::NoSnapshot))
        );
        assert!(!is_pending(&world));
        assert!(!save_slots_lifecycle::startup_restore_pending(&world));
        assert_eq!(advance(&mut world), None);
    }
}

#[cfg(all(test, feature = "headless", not(target_arch = "wasm32")))]
#[path = "startup_restore_integration_tests.rs"]
mod integration_tests;
