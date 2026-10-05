use super::*;

#[test]
fn default_view_mode_is_camera() {
    assert_eq!(ViewMode::default(), ViewMode::Camera(CameraView::default()));
}
