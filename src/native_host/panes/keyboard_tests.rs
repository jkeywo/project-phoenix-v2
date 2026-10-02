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
