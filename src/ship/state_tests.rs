use super::*;

#[test]
fn ship_view_mode_defaults_to_camera() {
    let vm = ShipViewMode::default();
    assert_eq!(vm.view_mode, ViewMode::Camera(CameraView::default()));
}

#[test]
fn ship_view_mode_request_toggles_correctly() {
    let mut vm = ShipViewMode::default();
    vm.request_view_mode(ViewMode::Camera(CameraView::new("camera_aft")));
    vm.request_view_mode(ViewMode::Radar);
    assert_eq!(vm.view_mode, ViewMode::Radar);
    vm.request_view_mode(ViewMode::Radar);
    assert_eq!(
        vm.view_mode,
        ViewMode::Camera(CameraView::new("camera_aft"))
    );
}
