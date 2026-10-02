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
