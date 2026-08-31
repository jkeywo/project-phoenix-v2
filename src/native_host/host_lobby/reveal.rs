//! When the native host's lobby surface is on screen (issue #1325).
//!
//! The surface is **permanent**. It is composited onto the viewscreen window
//! once, at boot, and it is never torn down: on mission start the lobby chrome
//! *yields* — the page renders nothing, the host stops compositing its texture,
//! and it drops out of the input router so a click over it reaches the
//! viewscreen — and one host key brings it back. Later slices put the QR
//! overlay, the settings and the layout rows on this same surface, so "the
//! lobby went away" has to mean "the chrome yielded", not "the view was
//! destroyed and something must rebuild it".
//!
//! That is three separate observable answers from one small state, so the state
//! is here, pure, rather than as three booleans scattered across an
//! `--features ultralight` frame loop that no CI job compiles:
//!
//! | answer | who reads it |
//! |---|---|
//! | [`RevealState::composited`] | the Bevy node's `display`, and whether the frame is copied at all |
//! | [`RevealState::routes_input`] | whether the surface is placed in the `#1124` router |
//! | [`RevealState::force_chrome`] | pushed to the page, which ORs it into the panel's own phase-driven visibility |
//!
//! # The rules, and why each one
//!
//! * **In the lobby phase the chrome is unconditionally on.** That is the
//!   surface's whole job at that moment, and it is what makes the acceptance
//!   criterion "on return to the lobby phase the lobby chrome comes back" hold
//!   without anybody pressing anything.
//! * **Any phase change clears the manual latch.** Mission start is the case
//!   the issue names — the chrome yields to the 3-D viewscreen — but the same
//!   rule is what stops a key pressed *during* the lobby (where it does nothing
//!   visible, because the chrome is already on) from silently arming a reveal
//!   that springs open the moment the mission starts.
//! * **The key is a toggle, not a hold.** A bridge operator reaching for a
//!   reference during play should not have to keep a finger on a key.
//!
//! Nothing here knows about Ultralight, Bevy or a window; the adapter that does
//! is `panes::ultralight`, behind the SDK feature.

use crate::core::messages::GamePhase;

/// What the host should be doing with the lobby surface right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfacePresence {
    /// Draw the surface's texture onto the window.
    ///
    /// `false` is the *invisible* half of "present but yielded": the view is
    /// still alive and still being pushed to, its Bevy node is simply not
    /// displayed. Nothing is rebuilt when it comes back.
    pub composited: bool,
    /// Place the surface in the pane input router (issue #1124).
    ///
    /// `false` is the *input-transparent* half: with no placement, a pointer or
    /// a touch over that region resolves to no pane and is left to the
    /// viewscreen, so a yielded lobby cannot swallow a click.
    pub routes_input: bool,
    /// Tell the page to render its lobby chrome even though the phase says
    /// otherwise — `renderHostLobby`'s `revealChrome` option.
    ///
    /// Only ever `true` outside the lobby phase: inside it the page's own view
    /// model already shows the panel, and forcing what is already true would
    /// make the two answers indistinguishable in a log.
    pub force_chrome: bool,
}

/// The lobby surface's reveal state.
///
/// Starts in the lobby phase with no manual reveal, which is what a host boots
/// into — `GamePhase::default()` is `Lobby`.
///
/// `Clone` but not `Copy`: `GamePhase` is a wire type deriving neither, and
/// making this `Copy` would mean holding a second notion of the phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevealState {
    phase: GamePhase,
    /// The host key's latch. Meaningful only outside the lobby phase; cleared
    /// by every phase change, so it can never survive one.
    revealed_in_play: bool,
}

impl Default for RevealState {
    fn default() -> Self {
        Self {
            phase: GamePhase::Lobby,
            revealed_in_play: false,
        }
    }
}

impl RevealState {
    /// A fresh state: lobby phase, nothing manually revealed.
    pub fn new() -> Self {
        Self::default()
    }

    /// The phase this state currently believes it is in.
    pub fn phase(&self) -> &GamePhase {
        &self.phase
    }

    /// Whether the host key has been used to reveal the surface in play.
    ///
    /// The latch itself rather than its effect — [`presence`](Self::presence)
    /// is what says whether anything is on screen.
    pub fn revealed_in_play(&self) -> bool {
        self.revealed_in_play
    }

    /// Observe the simulation's phase. Returns whether anything changed.
    ///
    /// A phase change clears the manual latch, in **both** directions: leaving
    /// the lobby is the chrome yielding to the mission, and returning to it is
    /// the chrome coming back on its own terms rather than on a key pressed
    /// twenty minutes ago.
    pub fn observe_phase(&mut self, phase: &GamePhase) -> bool {
        if &self.phase == phase {
            return false;
        }
        self.phase = phase.clone();
        self.revealed_in_play = false;
        true
    }

    /// The host key. Returns the presence that results, so a caller can log
    /// what the operator just did without asking a second question.
    ///
    /// Inert in the lobby phase by construction: the latch flips, but
    /// [`presence`](Self::presence) is already showing the chrome and the next
    /// phase change clears the latch again.
    pub fn toggle(&mut self) -> SurfacePresence {
        self.revealed_in_play = !self.revealed_in_play;
        self.presence()
    }

    /// What the host should be doing with the surface right now.
    pub fn presence(&self) -> SurfacePresence {
        let in_lobby = self.phase == GamePhase::Lobby;
        let force_chrome = !in_lobby && self.revealed_in_play;
        let on = in_lobby || force_chrome;
        SurfacePresence {
            composited: on,
            routes_input: on,
            force_chrome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_boots_showing_the_lobby_chrome() {
        // `GamePhase::default()` is Lobby and the surface's first job is the
        // crew lobby, so the very first frame must already be showing it —
        // there is no phase push to wait for.
        let state = RevealState::new();
        assert_eq!(
            state.presence(),
            SurfacePresence {
                composited: true,
                routes_input: true,
                force_chrome: false,
            }
        );
    }

    #[test]
    fn mission_start_yields_the_chrome_without_destroying_the_surface() {
        // The acceptance criterion, in one assertion: after start the surface
        // is invisible AND transparent to input. Nothing here says "closed" —
        // the frame loop keeps the view, which is the point of a permanent
        // surface.
        let mut state = RevealState::new();
        assert!(state.observe_phase(&GamePhase::InProgress));
        let presence = state.presence();
        assert!(!presence.composited);
        assert!(!presence.routes_input);
        assert!(!presence.force_chrome);
    }

    #[test]
    fn the_loading_phase_yields_too_because_the_web_lobby_does() {
        // `hostLobbyViewModel` shows the panel only in Lobby, so Loading is
        // already a hidden panel on the web host. The native surface agrees
        // rather than inventing a third answer for the two seconds of asset
        // pre-cache.
        let mut state = RevealState::new();
        state.observe_phase(&GamePhase::Loading);
        assert!(!state.presence().composited);
    }

    #[test]
    fn the_host_key_reveals_and_hides_the_surface_in_play() {
        let mut state = RevealState::new();
        state.observe_phase(&GamePhase::InProgress);

        let revealed = state.toggle();
        assert!(revealed.composited);
        assert!(revealed.routes_input);
        assert!(
            revealed.force_chrome,
            "the page has to be told to render chrome its own phase would hide"
        );

        let hidden = state.toggle();
        assert!(!hidden.composited);
        assert!(!hidden.routes_input);
        assert!(!hidden.force_chrome);
    }

    #[test]
    fn returning_to_the_lobby_brings_the_chrome_back() {
        let mut state = RevealState::new();
        state.observe_phase(&GamePhase::InProgress);
        state.observe_phase(&GamePhase::GameOver);
        assert!(!state.presence().composited);

        state.observe_phase(&GamePhase::Lobby);
        let presence = state.presence();
        assert!(presence.composited);
        assert!(presence.routes_input);
        assert!(
            !presence.force_chrome,
            "in the lobby the page's own view model shows the panel; forcing it \
             would hide which of the two answers was in play"
        );
    }

    #[test]
    fn a_reveal_does_not_survive_the_next_phase_change() {
        // Otherwise a key pressed to read something during a mission would
        // still be latched when the crew returned to the lobby and started a
        // second one — the chrome would spring back open mid-flight.
        let mut state = RevealState::new();
        state.observe_phase(&GamePhase::InProgress);
        state.toggle();
        assert!(state.revealed_in_play());

        state.observe_phase(&GamePhase::GameOver);
        assert!(!state.revealed_in_play());
        assert!(!state.presence().composited);
    }

    #[test]
    fn a_key_pressed_in_the_lobby_cannot_arm_a_reveal_for_the_mission() {
        // The trap this rule exists for: in the lobby the chrome is already on,
        // so the key looks inert — and if the latch survived, the operator's
        // idle keypress would blank the viewscreen the moment the crew launched.
        let mut state = RevealState::new();
        state.toggle();
        assert!(
            state.presence().composited,
            "the lobby phase shows the chrome whatever the latch says"
        );
        state.observe_phase(&GamePhase::InProgress);
        assert!(!state.presence().composited);
    }

    #[test]
    fn observing_the_same_phase_again_changes_nothing() {
        // The phase is read every frame from the simulation's state, so this is
        // the common case; it must not clear a reveal the operator just made.
        let mut state = RevealState::new();
        state.observe_phase(&GamePhase::InProgress);
        state.toggle();
        assert!(!state.observe_phase(&GamePhase::InProgress));
        assert!(state.presence().composited);
    }
}
