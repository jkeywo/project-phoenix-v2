//! Bevy keyboard events translated into the SDK-independent pane input stream.

use bevy::input::keyboard::{Key, KeyboardInput};

use super::pane_thread::{PaneCommand, PaneInput, PaneKeyCode};
use super::registry::PaneId;

/// Forward pressed keys only to the focused page. Ctrl+Tab belongs to native
/// pane traversal; function keys such as the F9 chrome toggle remain host-owned.
/// Text and raw keys retain the existing adapter's modifier/release semantics.
pub(super) fn pane_keyboard_command(
    key: &KeyboardInput,
    ctrl: bool,
    focused: Option<PaneId>,
) -> Option<PaneCommand> {
    if !key.state.is_pressed() {
        return None;
    }
    let id = focused?;
    let input = match &key.logical_key {
        Key::Character(text) => PaneInput::KeyChar(text.to_string()),
        Key::Space => PaneInput::KeyChar(" ".to_string()),
        Key::Backspace => PaneInput::Key(PaneKeyCode::Back),
        Key::Enter => PaneInput::Key(PaneKeyCode::Return),
        // The shared page dialog owns dismissal. Deliver its ordinary keydown
        // instead of adding a native Settings-specific close command.
        Key::Escape => PaneInput::Key(PaneKeyCode::Escape),
        Key::ArrowLeft => PaneInput::Key(PaneKeyCode::Left),
        Key::ArrowRight => PaneInput::Key(PaneKeyCode::Right),
        Key::ArrowUp => PaneInput::Key(PaneKeyCode::Up),
        Key::ArrowDown => PaneInput::Key(PaneKeyCode::Down),
        Key::Home => PaneInput::Key(PaneKeyCode::Home),
        Key::End => PaneInput::Key(PaneKeyCode::End),
        Key::Delete => PaneInput::Key(PaneKeyCode::Delete),
        Key::Tab if !ctrl => PaneInput::Key(PaneKeyCode::Tab),
        _ => return None,
    };
    Some(PaneCommand::Input { id, input })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::{keyboard::KeyCode, ButtonState};
    use bevy::prelude::Entity;

    fn pressed(logical_key: Key) -> KeyboardInput {
        KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        }
    }

    fn input(command: Option<PaneCommand>) -> (PaneId, PaneInput) {
        match command.expect("the focused page receives a key") {
            PaneCommand::Input { id, input } => (id, input),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn escape_reaches_the_focused_page_but_release_and_no_focus_do_not() {
        let pane = PaneId(7);
        let mut event = pressed(Key::Escape);
        assert_eq!(
            input(pane_keyboard_command(&event, false, Some(pane))),
            (pane, PaneInput::Key(PaneKeyCode::Escape))
        );
        assert!(pane_keyboard_command(&event, false, None).is_none());
        event.state = ButtonState::Released;
        assert!(pane_keyboard_command(&event, false, Some(pane)).is_none());
    }

    #[test]
    fn host_shortcuts_stay_reserved_while_page_editing_and_tab_still_forward() {
        let pane = Some(PaneId(8));
        assert!(pane_keyboard_command(&pressed(Key::F9), false, pane).is_none());
        assert!(pane_keyboard_command(&pressed(Key::Tab), true, pane).is_none());
        for (key, expected) in [
            (Key::Tab, PaneInput::Key(PaneKeyCode::Tab)),
            (Key::ArrowLeft, PaneInput::Key(PaneKeyCode::Left)),
            (Key::Delete, PaneInput::Key(PaneKeyCode::Delete)),
            (Key::Character("é".into()), PaneInput::KeyChar("é".into())),
            (Key::Space, PaneInput::KeyChar(" ".into())),
        ] {
            assert_eq!(
                input(pane_keyboard_command(&pressed(key), false, pane)).1,
                expected
            );
        }
    }
}
