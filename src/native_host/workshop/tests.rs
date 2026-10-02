#[test]
fn offline_workshop_shell_installs_the_actual_pane_upload_consumer() {
    let mut app = super::build_shell(crate::boot::NativeRenderSurface::Contract).unwrap();
    assert!(app.is_plugin_added::<crate::native_host::panes::upload::PaneUploadPlugin>());
    assert!(app
        .world()
        .contains_resource::<crate::native_host::panes::upload::PanePendingUploads>());
    app.update();
}
