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
    for chunk in crate::lockstep::transfer::chunk("uninvited record", Default::default(), 1, 0) {
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
