use super::*;

#[test]
fn a_borderless_fullscreen_window_gives_its_border_back() {
    // The state every native host boots into: `apply_bridge_profile` spawns
    // the primary window `BorderlessFullscreen`, so the FIRST press an
    // operator ever makes is this one.
    assert_eq!(
        next_window_mode(
            WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
            None
        ),
        WindowMode::Windowed
    );
}

#[test]
fn an_exclusive_fullscreen_window_does_too() {
    // Nothing in this repository asks for exclusive fullscreen, but winit
    // and an operator's window manager can both put a window in it. A
    // control that only knew about the mode we set would do nothing at all
    // there, which is the worst of the three possible answers.
    assert_eq!(
        next_window_mode(
            WindowMode::Fullscreen(
                MonitorSelection::Primary,
                bevy::window::VideoModeSelection::Current
            ),
            None
        ),
        WindowMode::Windowed
    );
}

#[test]
fn a_windowed_host_fills_the_display_it_is_already_on() {
    // No assignment has been made, so there is no display to name — and
    // `Current` is the only honest answer. Naming `Primary` here would send
    // a viewscreen to a laptop panel on a bridge whose operator dragged the
    // window somewhere else.
    assert_eq!(
        next_window_mode(WindowMode::Windowed, None),
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    );
}

#[test]
fn a_toggle_and_a_toggle_back_lands_on_the_display_the_row_assigned() {
    // The whole reason `restore` exists. The monitor row put the viewscreen
    // on a named display; going windowed and back must not quietly move it
    // to whichever screen the window happened to be on.
    let assigned = MonitorSelection::Index(2);
    let was = WindowMode::BorderlessFullscreen(assigned);

    let restore = monitor_to_restore(was, None);
    assert_eq!(restore, Some(assigned));
    let windowed = next_window_mode(was, restore);
    assert_eq!(windowed, WindowMode::Windowed);

    let restore = monitor_to_restore(windowed, restore);
    assert_eq!(next_window_mode(windowed, restore), was);
}

#[test]
fn a_windowed_window_says_nothing_about_which_display_to_fill() {
    // `monitor_to_restore` is a memory, not a reading: asked about a
    // windowed window it must leave what is remembered alone rather than
    // clear it, or the second press of a pair would forget the first.
    assert_eq!(
        monitor_to_restore(WindowMode::Windowed, Some(MonitorSelection::Index(1))),
        Some(MonitorSelection::Index(1))
    );
    assert_eq!(monitor_to_restore(WindowMode::Windowed, None), None);
}
