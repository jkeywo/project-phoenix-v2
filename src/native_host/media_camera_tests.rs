use super::*;
#[test]
fn camera_identity_survives_same_named_neighbour_disconnection() {
    let first = CameraDevice {
        id: "interface-a".into(),
        name: "Camera".into(),
    };
    let alone = discovered_cameras(std::slice::from_ref(&first));
    let pair = discovered_cameras(&[
        first,
        CameraDevice {
            id: "interface-b".into(),
            name: "Camera".into(),
        },
    ]);
    assert_eq!(alone[0].identity, pair[0].identity);
    assert_ne!(pair[0].identity, pair[1].identity);
}
#[test]
fn preview_refuses_capture_before_a_native_window_exists() {
    let mut preview = CameraPreview::default();
    assert!(preview
        .start("endpoint", false)
        .unwrap_err()
        .contains("not ready"));
    assert!(preview.capture.is_none());
    assert!(preview.pending.is_none());
}
#[test]
fn terminal_failure_survives_explicit_teardown() {
    let mut preview = CameraPreview::default();
    preview.fail("stop operation failed".into());
    preview.stop();
    assert_eq!(preview.failure.as_deref(), Some("stop operation failed"));
    assert!(preview.frame.is_none());
    assert!(preview.capture.is_none());
}
