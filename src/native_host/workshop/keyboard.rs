//! Exact semantic key identity for the shared Authoring page, with native
//! editing defaults applied by the pane only when the page did not claim it.
use bevy::input::{
    keyboard::{Key, KeyCode, KeyboardInput},
    ButtonInput,
};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopKey {
    pub code: String,
    pub key: String,
    pub pressed: bool,
    pub repeat: bool,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub alt_key: bool,
    pub meta_key: bool,
    pub text: Option<String>,
}

pub fn from_input(event: &KeyboardInput, keys: &ButtonInput<KeyCode>) -> Option<WorkshopKey> {
    let ctrl_key = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if ctrl_key && event.key_code == KeyCode::Tab {
        return None;
    }
    let alt_key = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let meta_key = keys.pressed(KeyCode::SuperLeft) || keys.pressed(KeyCode::SuperRight);
    let key = match &event.logical_key {
        Key::Character(text) => text.to_string(),
        Key::Space => " ".into(),
        other => format!("{other:?}"),
    };
    // AltGr is represented as Ctrl+Alt by some native layouts. Only committed
    // printable text may use that exception; a claimed semantic chord still
    // consumes the key before the SDK sees any text.
    let alt_graph_text = ctrl_key
        && alt_key
        && event
            .text
            .as_ref()
            .is_some_and(|text| !text.is_empty() && text.chars().all(|ch| !ch.is_control()));
    let text =
        if event.state.is_pressed() && !meta_key && ((!ctrl_key && !alt_key) || alt_graph_text) {
            match &event.logical_key {
                Key::Character(text) => Some(
                    event
                        .text
                        .as_ref()
                        .map_or_else(|| text.to_string(), ToString::to_string),
                ),
                Key::Space => Some(" ".into()),
                _ => None,
            }
        } else {
            None
        };
    Some(WorkshopKey {
        code: format!("{:?}", event.key_code),
        key,
        text,
        pressed: event.state.is_pressed(),
        repeat: event.repeat,
        ctrl_key,
        alt_key,
        meta_key,
        shift_key: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
    })
}

/// Native editing accelerators and caret movement after shared semantic
/// dispatch declined the exact key. Other physical keys still reach the DOM.
#[cfg(feature = "ultralight")]
pub fn virtual_key(code: &str) -> Option<vellum_ultralight::runtime::VirtualKeyCode> {
    use vellum_ultralight::runtime::VirtualKeyCode as V;
    Some(match code {
        "KeyA" => V::A,
        "KeyB" => V::B,
        "KeyC" => V::C,
        "KeyD" => V::D,
        "KeyE" => V::E,
        "KeyF" => V::F,
        "KeyG" => V::G,
        "KeyH" => V::H,
        "KeyI" => V::I,
        "KeyJ" => V::J,
        "KeyK" => V::K,
        "KeyL" => V::L,
        "KeyM" => V::M,
        "KeyN" => V::N,
        "KeyO" => V::O,
        "KeyP" => V::P,
        "KeyQ" => V::Q,
        "KeyR" => V::R,
        "KeyS" => V::S,
        "KeyT" => V::T,
        "KeyU" => V::U,
        "KeyV" => V::V,
        "KeyW" => V::W,
        "KeyX" => V::X,
        "KeyY" => V::Y,
        "KeyZ" => V::Z,
        "Digit0" => V::Key0,
        "Digit1" => V::Key1,
        "Digit2" => V::Key2,
        "Digit3" => V::Key3,
        "Digit4" => V::Key4,
        "Digit5" => V::Key5,
        "Digit6" => V::Key6,
        "Digit7" => V::Key7,
        "Digit8" => V::Key8,
        "Digit9" => V::Key9,
        "Backspace" => V::Back,
        "Tab" => V::Tab,
        "Enter" | "NumpadEnter" => V::Return,
        "Escape" => V::Escape,
        "Space" => V::Space,
        "ArrowLeft" => V::Left,
        "ArrowRight" => V::Right,
        "ArrowUp" => V::Up,
        "ArrowDown" => V::Down,
        "Home" => V::Home,
        "End" => V::End,
        "PageUp" => V::Prior,
        "PageDown" => V::Next,
        "Delete" => V::Delete,
        "Insert" => V::Insert,
        "ShiftLeft" | "ShiftRight" => V::Shift,
        "ControlLeft" | "ControlRight" => V::Control,
        "AltLeft" | "AltRight" => V::Menu,
        "SuperLeft" => V::Lwin,
        "SuperRight" => V::Rwin,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
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
}
