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
mod tests {
    use super::*;
    use crate::console_bridge::AudioCueEvent;
    use crate::gm_restore::AcceptedRestore;

    fn publish_audio_lifecycle(world: &mut World) {
        advance_audio_lifecycle(world);
    }

    #[test]
    fn accepted_restore_and_recovery_rebase_without_cue_history_or_digest_changes() {
        let mut app = App::new();
        app.init_resource::<RoomAudioLifecycle>()
            .init_resource::<GmLiveRestore>()
            .init_resource::<MeshRestoreArm>()
            .add_message::<AudioCueEvent>()
            .add_message::<crate::gm_presentation::sound::LiveSoundRequest>()
            .insert_resource(State::new(GamePhase::InProgress))
            .add_systems(Update, publish_audio_lifecycle);
        app.update();
        let generation = app
            .world()
            .resource::<RoomAudioLifecycle>()
            .state
            .generation;
        app.update();
        assert_eq!(
            app.world()
                .resource::<RoomAudioLifecycle>()
                .state
                .generation,
            generation
        );
        let digest = crate::sim_digest::world_digest(app.world());
        app.world_mut()
            .write_message(crate::gm_presentation::sound::LiveSoundRequest {
                ship: "alpha".into(),
                source: None,
                definition: crate::gm_presentation::sound::LiveSoundCatalog::default()
                    .resolve("weapons")
                    .unwrap(),
            });
        app.world_mut()
            .resource_mut::<Messages<AudioCueEvent>>()
            .write(AudioCueEvent {
                json: "old shot".into(),
            });
        app.world_mut()
            .resource_mut::<GmLiveRestore>()
            .accept(AcceptedRestore {
                operator_id: "gm".into(),
                correlation: "restore".into(),
                candidate_slot: "save".into(),
                requested_tick: 12,
                initiator: Default::default(),
                order: Default::default(),
            });
        app.update();
        let state = &app.world().resource::<RoomAudioLifecycle>().state;
        assert!(state.running && state.suspended);
        assert_eq!(state.generation, generation + 1);
        assert!(app.world().resource::<Messages<AudioCueEvent>>().is_empty());
        assert!(app
            .world()
            .resource::<Messages<crate::gm_presentation::sound::LiveSoundRequest>>()
            .is_empty());
        assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
        // Returning to Idle ends the hold; the real driver's Restored/RolledBack
        // -> explicit Resume path is exercised by tests/gm_restore.rs.
        app.world_mut().insert_resource(GmLiveRestore::default());
        app.update();
        assert!(!app.world().resource::<RoomAudioLifecycle>().state.suspended);
        assert_eq!(
            app.world()
                .resource::<RoomAudioLifecycle>()
                .state
                .generation,
            generation + 2
        );
        app.world_mut()
            .resource_mut::<MeshRestoreArm>()
            .arm(Default::default());
        app.update();
        assert!(app.world().resource::<RoomAudioLifecycle>().state.suspended);
        app.world_mut().resource_mut::<MeshRestoreArm>().disarm();
        app.update();
        assert!(!app.world().resource::<RoomAudioLifecycle>().state.suspended);
    }

    #[test]
    fn an_unarmed_snapshot_refusal_does_not_interrupt_current_audio() {
        let mut app = App::new();
        app.init_resource::<RoomAudioLifecycle>()
            .init_resource::<MeshSnapshotReceiver>()
            .init_resource::<MeshRestoreArm>()
            .add_message::<AudioCueEvent>()
            .insert_resource(State::new(GamePhase::InProgress));
        publish_audio_lifecycle(app.world_mut());
        let before = app.world().resource::<RoomAudioLifecycle>().state.clone();
        app.world_mut()
            .resource_mut::<Messages<AudioCueEvent>>()
            .write(AudioCueEvent {
                json: "current shot".into(),
            });
        for chunk in crate::lockstep::transfer::chunk("uninvited record", Default::default(), 1, 0)
        {
            app.world_mut()
                .resource_mut::<MeshSnapshotReceiver>()
                .accept_chunk(&chunk)
                .unwrap();
        }
        crate::lockstep::snapshot_relay::drain_mesh_restore(app.world_mut());
        assert_eq!(
            app.world()
                .resource::<MeshSnapshotReceiver>()
                .last_outcome(),
            Some(&MeshRestoreOutcome::RefusedUnarmed)
        );
        publish_audio_lifecycle(app.world_mut());
        assert_eq!(app.world().resource::<RoomAudioLifecycle>().state, before);
        assert_eq!(app.world().resource::<Messages<AudioCueEvent>>().len(), 1);
        let committed = MeshRestoreOutcome::Committed {
            tick: 6,
            digest: 42,
        };
        assert_eq!(
            continuation_outcome(Some(&MeshRestoreOutcome::RefusedUnarmed), Some(&committed)),
            Some(committed)
        );
    }

    #[test]
    fn startup_restore_uses_the_existing_capture_gate_and_then_rebases() {
        let mut app = App::new();
        app.init_resource::<RoomAudioLifecycle>()
            .add_systems(Update, publish_audio_lifecycle);
        crate::save_slots_lifecycle::begin_startup_restore(app.world_mut());
        app.update();
        assert!(app.world().resource::<RoomAudioLifecycle>().state.suspended);
        crate::save_slots_lifecycle::cancel_startup_restore(app.world_mut());
        app.update();
        assert!(!app.world().resource::<RoomAudioLifecycle>().state.suspended);
    }

    #[test]
    fn shared_gm_projections_follow_the_boundary_and_keep_absent_owner_baselines() {
        use crate::console_bridge::{GmAttentionChanged, GmHealthChanged};
        let mut app = App::new();
        app.init_resource::<crate::sim_tick::SimTick>()
            .insert_resource(crate::gm_projection::BrowserGameMaster)
            .insert_resource(State::new(GamePhase::InProgress))
            .add_plugins((
                crate::gm_health::GmHealthPlugin,
                crate::gm_attention::GmAttentionPlugin,
            ));
        // The shared producers remain usable without a room adapter. A missing
        // continuation owner is an explicit, silent baseline, not a fake epoch.
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<GmHealthChanged>>()
                .drain()
                .last()
                .unwrap()
                .payload
                .presentation_generation,
            None,
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<GmAttentionChanged>>()
                .drain()
                .last()
                .unwrap()
                .payload
                .presentation_generation,
            None,
        );
        app.init_resource::<RoomAudioLifecycle>()
            .init_resource::<GmLiveRestore>()
            .add_systems(
                PostUpdate,
                publish_audio_lifecycle.in_set(AudioLifecyclePublished),
            );
        for generation in [1, 2] {
            if generation == 2 {
                app.world_mut()
                    .resource_mut::<GmLiveRestore>()
                    .accept(AcceptedRestore {
                        operator_id: "gm".into(),
                        correlation: "restore".into(),
                        candidate_slot: "save".into(),
                        requested_tick: 12,
                        initiator: Default::default(),
                        order: Default::default(),
                    });
            }
            app.update();
            assert_eq!(
                app.world_mut()
                    .resource_mut::<Messages<GmHealthChanged>>()
                    .drain()
                    .last()
                    .unwrap()
                    .payload
                    .presentation_generation,
                Some(generation),
            );
            assert_eq!(
                app.world_mut()
                    .resource_mut::<Messages<GmAttentionChanged>>()
                    .drain()
                    .last()
                    .unwrap()
                    .payload
                    .presentation_generation,
                Some(generation),
            );
        }
        assert!(app.world().resource::<RoomAudioLifecycle>().state.suspended);
    }
}
