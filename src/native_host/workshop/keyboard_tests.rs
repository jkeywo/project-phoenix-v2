use super::*;
use bevy::{input::ButtonState, prelude::Entity};
#[test]
fn authoring_shortcuts_keep_code_modifiers_repeat_and_release_without_inserting_control_text() {
    let mut held = ButtonInput::default();
    held.press(KeyCode::ControlLeft);
    held.press(KeyCode::ShiftRight);
    let mut event = KeyboardInput {
        key_code: KeyCode::KeyZ,
        logical_key: Key::Character("Z".into()),
        state: ButtonState::Pressed,
        text: Some("z".into()),
        repeat: true,
        window: Entity::PLACEHOLDER,
    };
    let key = from_input(&event, &held).unwrap();
    assert_eq!(key.code, "KeyZ");
    assert!(key.ctrl_key && key.shift_key && key.repeat && key.pressed);
    assert!(key.text.is_none());
    event.state = ButtonState::Released;
    assert!(!from_input(&event, &held).unwrap().pressed);
    held.release_all();
    event.state = ButtonState::Pressed;
    assert_eq!(
        from_input(&event, &held).unwrap().text.as_deref(),
        Some("z")
    );
    held.press(KeyCode::ControlLeft);
    event.key_code = KeyCode::Tab;
    assert!(from_input(&event, &held).is_none());
    held.press(KeyCode::AltRight);
    event.key_code = KeyCode::KeyQ;
    event.text = Some("@".into());
    event.logical_key = Key::Character("@".into());
    assert_eq!(
        from_input(&event, &held).unwrap().text.as_deref(),
        Some("@")
    );
}
