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
#[path = "keyboard_tests.rs"]
mod tests;
