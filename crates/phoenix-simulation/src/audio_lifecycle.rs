//! Shared presentation continuation projection. A live restore remains in
//! InProgress, so neither a button press nor a phase-only browser inference can
//! identify it. Read accepted restore and recovery state, never cue history.
//!
//! GM and Station projections use this type even without the `server` feature.
//! The room adapter owns playback/config/HUD rebasing; this module owns only
//! the current boundary and the discard of obsolete presentation occurrences.

use bevy::prelude::*;

use crate::console_bridge::AudioLifecycleState;
use crate::core::messages::GamePhase;
use crate::gm_restore::GmLiveRestore;
use crate::lockstep::snapshot_relay::{MeshRestoreArm, MeshRestoreOutcome, MeshSnapshotReceiver};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Boundary {
    phase: GamePhase,
    startup: bool,
    request: Option<crate::gm_action::GmActionOrder>,
    restoring: bool,
    mesh_armed: bool,
    mesh_outcome: Option<MeshRestoreOutcome>,
}

/// Current presentation state and its comparison key. This contains no audio
/// events, backlog or replay frontier and does not enter the simulation digest.
#[derive(Resource, Default)]
pub struct RoomAudioLifecycle {
    pub state: AudioLifecycleState,
    boundary: Option<Boundary>,
}

/// PostUpdate boundary after the active presentation adapter has advanced the
/// generation, discarded old occurrences and rebuilt current room/HUD state.
/// Shared alert producers order after this set without importing a renderer.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioLifecyclePublished;

fn continuation_outcome(
    outcome: Option<&MeshRestoreOutcome>,
    previous: Option<&MeshRestoreOutcome>,
) -> Option<MeshRestoreOutcome> {
    match outcome {
        Some(
            value @ (MeshRestoreOutcome::Committed { .. }
            | MeshRestoreOutcome::RefusedIntegrity { .. }
            | MeshRestoreOutcome::Incomplete { .. }),
        ) => Some(value.clone()),
        // Rejected transfers that never restore the world are not presentation
        // boundaries. Preserve the last real continuation, even if a bad packet
        // overwrote the receiver's latest diagnostic outcome.
        Some(_) => previous.cloned(),
        None => None,
    }
}

/// Advance the accepted boundary and retire transient occurrences. Returns true
/// exactly when the presentation adapter must rebuild its current baseline.
pub fn advance_audio_lifecycle(world: &mut World) -> bool {
    let restore = world.get_resource::<GmLiveRestore>();
    let boundary = Boundary {
        phase: world
            .get_resource::<State<GamePhase>>()
            .map_or(GamePhase::Lobby, |phase| phase.get().clone()),
        startup: crate::save_slots_lifecycle::startup_restore_pending(world),
        request: restore
            .and_then(|restore| restore.request())
            .map(|request| request.order),
        restoring: restore.is_some_and(|restore| restore.phase().holds_world()),
        mesh_armed: world
            .get_resource::<MeshRestoreArm>()
            .is_some_and(MeshRestoreArm::is_armed),
        mesh_outcome: continuation_outcome(
            world
                .get_resource::<MeshSnapshotReceiver>()
                .and_then(MeshSnapshotReceiver::last_outcome),
            world
                .resource::<RoomAudioLifecycle>()
                .boundary
                .as_ref()
                .and_then(|boundary| boundary.mesh_outcome.as_ref()),
        ),
    };
    if world.resource::<RoomAudioLifecycle>().boundary.as_ref() == Some(&boundary) {
        return false;
    }
    let suspended = boundary.startup || boundary.restoring || boundary.mesh_armed;
    let running = boundary.phase == GamePhase::InProgress;
    let mut lifecycle = world.resource_mut::<RoomAudioLifecycle>();
    lifecycle.state.generation = lifecycle.state.generation.wrapping_add(1);
    lifecycle.state.running = running;
    lifecycle.state.suspended = suspended;
    lifecycle.boundary = Some(boundary);
    // The frame may already have produced samples from the old continuation.
    // Discard only presentation output; canonical command bookkeeping remains.
    if let Some(mut cues) =
        world.get_resource_mut::<Messages<crate::console_bridge::AudioCueEvent>>()
    {
        cues.clear();
    }
    if let Some(mut requests) =
        world.get_resource_mut::<Messages<crate::gm_presentation::sound::LiveSoundRequest>>()
    {
        requests.clear();
    }
    true
}

#[cfg(test)]
#[path = "audio_lifecycle_tests.rs"]
mod tests;
