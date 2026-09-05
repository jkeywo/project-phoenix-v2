//! The landing's fullscreen control, on the native surface (issue #1367).
//!
//! A browser host's control forwards to `gui/page-chrome.js`'s one
//! `initFullscreen`, which asks a BROWSER to fill a screen. This window has no
//! browser chrome: what "fullscreen" means here is the primary window's
//! [`WindowMode`], which belongs to the host process — so the press crosses the
//! page->host queue as [`HostLobbyRecord::ToggleFullscreen`] and is answered
//! here.
//!
//! [`HostLobbyRecord::ToggleFullscreen`]: super::HostLobbyRecord::ToggleFullscreen
//!
//! # The decision is pure; the system that applies it is thin
//!
//! [`next_window_mode`] and [`monitor_to_restore`] take a mode and give one
//! back, with no `App`, no window and no winit — so what a press does is a unit
//! test on any machine, and the ordinary `cargo test` runs cover it. The
//! system below is the four lines that read the window, ask those two, and
//! write the answer.
//!
//! # It sets the mode the way the display-assignment law already does
//!
//! `bridge_display::follow_layout_viewscreen` puts the primary window into
//! `WindowMode::BorderlessFullscreen(MonitorSelection::Entity(..))` when the
//! monitor row assigns a display, and `apply_bridge_profile` spawns it that way
//! at boot. Going back to fullscreen therefore restores the **selection the
//! window was filling before it was windowed** rather than inventing one:
//! anything else would move a viewscreen off the display the operator assigned
//! it to, which is a layout decision this control has no business making.
//! `MonitorSelection::Current` is the fallback for a window that was never
//! fullscreen — the display it is on, which is the only honest answer when
//! nothing has assigned one.
//!
//! # It does not fight the layout applier
//!
//! `follow_layout_viewscreen` returns immediately unless the layout's
//! viewscreen differs from the one `BridgeDisplayApplied` recorded, and a press
//! here moves no layout. So a windowed host stays windowed until the operator
//! says otherwise or assigns a display — at which point the law wins, which is
//! the right way round: an assignment is a statement about which screen the
//! crew watches, and this control is a statement about one window's border.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, Window, WindowMode};

use crate::logging::{LogCat, LogFilterConfig};

/// What the fullscreen control has asked for, and what to give back.
///
/// A latch, for the reason `PendingForceStart` is one: `drain_surface_records`
/// is a dispatcher over a drained queue in `PreUpdate`, and an arm that reached
/// for the primary window there would make every other record in the same batch
/// wait behind a window query that most compositions cannot answer at all.
/// [`apply_window_mode_toggle`] answers it in `Update` of the same frame.
///
/// `restore` is the [`MonitorSelection`] the window was filling when it left
/// fullscreen. Held here rather than re-derived because it cannot be: a
/// windowed window reports nothing about the display it used to cover, and
/// guessing `Current` would move a viewscreen off the display the monitor row
/// assigned it to the first time an operator toggled twice.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WindowModeToggle {
    /// Set by a press, cleared by the frame that applies it.
    pub pending: bool,
    /// The selection to fill again, or `None` on a host that has never been
    /// fullscreen.
    pub restore: Option<MonitorSelection>,
}

/// The mode a window currently in `current` takes when the control is pressed.
///
/// Any fullscreen mode — borderless or exclusive — gives the border back;
/// [`WindowMode::Windowed`] fills `restore`, or the window's own display when
/// nothing has been remembered.
pub fn next_window_mode(current: WindowMode, restore: Option<MonitorSelection>) -> WindowMode {
    match current {
        WindowMode::Windowed => {
            WindowMode::BorderlessFullscreen(restore.unwrap_or(MonitorSelection::Current))
        }
        _ => WindowMode::Windowed,
    }
}

/// What to remember, given the mode the window is in `current` and what was
/// already remembered.
///
/// A fullscreen window names the display it is filling and that is what is
/// worth keeping; a windowed one names nothing, so the last answer stands. Kept
/// beside [`next_window_mode`] rather than folded into it because the caller
/// needs both answers about the SAME pre-press mode, and a function returning a
/// pair would make the order they are applied in matter.
pub fn monitor_to_restore(
    current: WindowMode,
    remembered: Option<MonitorSelection>,
) -> Option<MonitorSelection> {
    match current {
        WindowMode::BorderlessFullscreen(selection) => Some(selection),
        WindowMode::Fullscreen(selection, _) => Some(selection),
        WindowMode::Windowed => remembered,
    }
}

/// Apply a pending press to the primary window.
///
/// Everything optional, and deliberately: a headless composition carries no
/// window and a delivery-only host carries no toggle resource, and neither is a
/// reason for a system to panic. A press that arrives on a host with no window
/// is dropped with a line in the operator log rather than latched forever —
/// a queued mode change that fired the day a window appeared would be a control
/// acting minutes after it was pressed.
pub(crate) fn apply_window_mode_toggle(
    toggle: Option<ResMut<WindowModeToggle>>,
    mut primary: Query<&mut Window, With<PrimaryWindow>>,
    log: Option<Res<LogFilterConfig>>,
) {
    let Some(mut toggle) = toggle else {
        return;
    };
    if !toggle.pending {
        return;
    }
    toggle.pending = false;
    let Ok(mut window) = primary.single_mut() else {
        crate::pwarn!(
            log,
            LogCat::Lobby,
            "host lobby: the fullscreen control was pressed on a host with no primary window; \
             the press is dropped"
        );
        return;
    };
    let was = window.mode;
    toggle.restore = monitor_to_restore(was, toggle.restore);
    window.mode = next_window_mode(was, toggle.restore);
    crate::pinfo!(
        log,
        LogCat::Lobby,
        "host lobby: window mode {was:?} -> {:?}",
        window.mode
    );
}

#[cfg(test)]
mod tests {
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
}
