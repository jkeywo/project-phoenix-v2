// Phase toggle for the client GUI shell. Pure function (issue #440).
//
// Given a `GamePhase` string from the server, returns a visibility map for
// each top-level UI section. The render loop in `client.html` consumes this
// to flip CSS classes and `display` styles.
//
// Per PRD #438, the lobby is the only UI shown during the `Lobby` phase, and
// the game shell (bezel + per-console content) is shown for the `InProgress`
// and `GameOver` phases. Unknown phases default to the lobby (fail-safe so a
// future enum variant doesn't accidentally hide every section).
//
// This module is loaded as an ES module by `client.html` (for use via
// `window.sectionVisibility`) and imported by Vitest tests
// (`tests/client/phase-toggle.test.js`).

export const IN_GAME_PHASES = Object.freeze(['InProgress', 'GameOver']);

export function isInGame(phase) {
  return IN_GAME_PHASES.includes(phase);
}

/**
 * Phases in which an operator may NOT explicitly regenerate a join code
 * (issue #1115 AC2). `Loading` is grouped with `InProgress` rather than
 * treated as "not yet a mission": assets are already committed to a specific
 * run by then, and rotating the code out from under it is the same hazard as
 * doing so mid-mission. Server-side, `rendezvous-transport.js`'s `rotate()`
 * has no notion of GamePhase at all — this predicate is the gate a caller
 * (server.html) checks BEFORE ever sending that frame.
 */
export const NON_ROTATABLE_PHASES = Object.freeze(['Loading', 'InProgress']);

/**
 * True while the operator may rotate a join code: `Lobby`, `GameOver`, or the
 * empty/unknown phase before the first `LobbyStatePayload` push has arrived.
 * Fail-open on an unrecognised phase for the same reason `sectionVisibility`
 * defaults to the lobby view — a future GamePhase variant should not silently
 * lock the rotate lever rather than merely not knowing about it yet.
 */
export function codesRotatable(phase) {
  return !NON_ROTATABLE_PHASES.includes(phase);
}

export function sectionVisibility(phase) {
  const inGame = isInGame(phase);
  return {
    lobby: !inGame,
    game: inGame,
    bezel: inGame,
  };
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.sectionVisibility = sectionVisibility;
  window.isInGame = isInGame;
  window.IN_GAME_PHASES = IN_GAME_PHASES;
  window.codesRotatable = codesRotatable;
  window.NON_ROTATABLE_PHASES = NON_ROTATABLE_PHASES;
}
