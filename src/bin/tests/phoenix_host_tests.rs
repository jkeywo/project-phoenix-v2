use super::*;
use project_phoenix::delivery::args::SaveOperatorAction;

const SLOT: &str = "00000000-0000-4000-8000-000000000123";

#[test]
fn native_operator_actions_reach_the_installed_catalogue_service() {
    let dir =
        std::env::temp_dir().join(format!("phoenix-host-save-actions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let versions = vellum_save::Versions::new(7, "rules", 0x1234);
    let run = project_phoenix::snapshot::run_for(
        project_phoenix::snapshot::PhoenixSnapshot {
            tick: 41,
            ..Default::default()
        },
        0xfeed,
        17,
        "assets/worlds/probe.toml",
        versions.clone(),
    );
    project_phoenix::save_slots::write_manual_save(
        &vellum_save::FileStore::new(&dir),
        SLOT,
        "Before",
        &run,
    )
    .unwrap();
    let mut app = bevy::prelude::App::new();
    app.init_resource::<project_phoenix::save_slots_lifecycle::PendingStoredRuns>();
    project_phoenix::save_slots_store::install_local_save_store(
        &mut app,
        vellum_save::FileStore::new(&dir),
    );

    let listed = apply_native_save_action(
        &mut app,
        &SaveOperatorAction::List,
        &versions,
        "assets/worlds/probe.toml",
    )
    .unwrap();
    assert!(listed.iter().any(|line| line.contains("Before")));

    apply_native_save_action(
        &mut app,
        &SaveOperatorAction::Rename {
            slot_id: SLOT.into(),
            display_name: "After".into(),
        },
        &versions,
        "assets/worlds/probe.toml",
    )
    .unwrap();
    let export = dir.with_extension("export.ron");
    let _ = std::fs::remove_file(&export);
    apply_native_save_action(
        &mut app,
        &SaveOperatorAction::Export {
            slot_id: SLOT.into(),
            path: export.to_string_lossy().into_owned(),
        },
        &versions,
        "assets/worlds/probe.toml",
    )
    .unwrap();
    let exported = std::fs::read_to_string(&export).unwrap();
    assert_eq!(
        project_phoenix::snapshot::StoredRun::from_ron(&exported).unwrap(),
        run
    );

    let unconfirmed = apply_native_save_action(
        &mut app,
        &SaveOperatorAction::Delete {
            slot_id: SLOT.into(),
            confirmed: false,
        },
        &versions,
        "assets/worlds/probe.toml",
    )
    .unwrap_err();
    assert!(unconfirmed.contains("--confirm-delete"));
    apply_native_save_action(
        &mut app,
        &SaveOperatorAction::Delete {
            slot_id: SLOT.into(),
            confirmed: true,
        },
        &versions,
        "assets/worlds/probe.toml",
    )
    .unwrap();
    assert!(app
        .world()
        .resource::<project_phoenix::save_slots_store::SaveSlotService>()
        .list_for_loaded_scenario(&versions, "assets/worlds/probe.toml")
        .unwrap()
        .is_empty());

    drop(app);
    let _ = std::fs::remove_file(export);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn native_operator_reporter_surfaces_and_drains_manual_refusals() {
    let dir = std::env::temp_dir().join(format!(
        "phoenix-host-save-refusal-report-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = bevy::prelude::App::new();
    app.init_resource::<project_phoenix::save_slots_lifecycle::PendingStoredRuns>();
    project_phoenix::save_slots_store::install_local_save_store(
        &mut app,
        vellum_save::FileStore::new(&dir),
    );
    app.add_systems(bevy::prelude::Update, report_save_outcomes);

    let slot_id = project_phoenix::save_slots_store::request_named_manual_save(
        app.world_mut(),
        "Refused operator save",
    )
    .expect("the native operator reserves a manual slot");
    project_phoenix::save_slots_lifecycle::begin_startup_restore(app.world_mut());

    // Frame one transfers the lifecycle refusal into the service's bounded
    // recent ring during PostUpdate. Frame two reports and drains it during
    // Update, matching the native binary's ordinary one-frame outcome lag.
    app.update();
    let refusal = app
        .world()
        .resource::<project_phoenix::save_slots_store::SaveSlotService>()
        .manual_refusals()
        .next()
        .expect("the bounded service ring receives the refusal");
    assert_eq!(refusal.slot_id, slot_id);
    assert!(describe_manual_save_refusal(refusal).contains("startup restore"));

    app.update();
    assert!(app
        .world()
        .resource::<project_phoenix::save_slots_store::SaveSlotService>()
        .manual_refusals()
        .next()
        .is_none());

    drop(app);
    let _ = std::fs::remove_dir_all(dir);
}
