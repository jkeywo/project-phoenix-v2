use super::*;

#[test]
fn escape_maps_to_the_sdk_dismissal_key() {
    assert!(matches!(
        pane_virtual_key(PaneKeyCode::Escape),
        VirtualKeyCode::Escape
    ));
    assert!(matches!(
        pane_virtual_key(PaneKeyCode::Tab),
        VirtualKeyCode::Tab
    ));
}
