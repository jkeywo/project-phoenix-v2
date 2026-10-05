use super::*;

#[test]
fn default_args_point_at_a_real_model() {
    assert_eq!(
        ViewerArgs::default().model.as_deref(),
        Some("assets/models/alliance_cruiser.glb")
    );
}

fn command_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_resource::<LightingMode>()
        .init_resource::<ViewerArgs>()
        .init_resource::<subject::SubjectState>()
        .init_resource::<LodMode>()
        .init_resource::<LadderState>()
        .init_resource::<crate::entities::planet::PlanetLightingOverride>()
        .add_systems(Update, lighting::apply_lighting);
    app
}
fn apply_queued(app: &mut App) {
    // The browser queue belongs to its calling thread. Native Bevy schedules
    // may execute systems on another worker, so drain at the actual JS seam.
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut().run_system_once(apply_commands).unwrap();
    app.update();
}
fn queue(command: ViewerCommand) {
    COMMAND_QUEUE.with(|q| q.borrow_mut().push(command));
}
fn lights(app: &mut App) -> Vec<Entity> {
    app.world_mut()
        .query_filtered::<Entity, With<lighting::ViewerLight>>()
        .iter(app.world())
        .collect()
}
#[test]
fn camera_gizmo_and_lod_commands_preserve_lights_but_lighting_edits_update_both_renderers() {
    let mut app = command_app();
    app.update();
    let initial = lights(&mut app);
    assert!(!initial.is_empty());
    queue(ViewerCommand::SetGizmos(true));
    queue(ViewerCommand::SetCameraDistance(42.0));
    queue(ViewerCommand::SetLodMode(LodMode::Base));
    apply_queued(&mut app);
    assert_eq!(lights(&mut app), initial);
    queue(ViewerCommand::SetAmbient {
        color: [0.2, 0.3, 0.4],
        brightness: 0.0,
    });
    apply_queued(&mut app);
    assert_ne!(lights(&mut app), initial);
    assert_eq!(
        app.world()
            .resource::<crate::entities::planet::PlanetLightingOverride>()
            .ambient_floor,
        0.0
    );
    assert!(app
        .world_mut()
        .query::<&AmbientLight>()
        .iter(app.world())
        .all(|l| l.brightness == 0.0));
}
#[test]
fn immutable_preview_rejects_legacy_mutation_until_explicit_opt_in() {
    let mut app = command_app();
    queue(ViewerCommand::SetLadder(vec![
        crate::entities::config::LodLevel::default(),
    ]));
    queue(ViewerCommand::ReloadAssets);
    queue(ViewerCommand::CaptureBillboard {
        views: 4,
        resolution: 16,
        pitch_deg: 0.0,
    });
    apply_queued(&mut app);
    assert!(app.world().resource::<LadderState>().levels.is_empty());
    assert!(!app.world().resource::<subject::SubjectState>().reloading);
    assert!(!app.world().contains_resource::<capture::CaptureRequest>());
    app.add_plugins(LegacyViewerWorkflowPlugin);
    queue(ViewerCommand::SetLadder(vec![
        crate::entities::config::LodLevel::default(),
    ]));
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut().run_system_once(apply_commands).unwrap();
    assert!(app.world().contains_resource::<capture::CaptureRequest>());
    assert!(app.world().contains_resource::<capture::CaptureState>());
    assert_eq!(app.world().resource::<LadderState>().levels.len(), 1);
}
