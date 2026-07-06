// Single-station helpers for the client GUI shell (issue #626).
//
// When a player is in-game with exactly one station assigned, the tab bar
// should be hidden entirely and the console section fills the full screen.
// These pure functions are Vitest-testable and consumed by client.html.

/**
 * Returns true when the tab bar should be hidden.
 * Hides in-game when the player owns 0 or 1 console (single-station mode).
 * In the lobby the bar is always visible (for the station-select list).
 *
 * @param {string[]} myConsoles - station ids owned by the local player
 * @param {boolean}  inGame     - true when phase is InProgress or GameOver
 * @returns {boolean}
 */
export function shouldHideTabBar(myConsoles, inGame) {
  if (!inGame) return false;
  return !myConsoles || myConsoles.length <= 1;
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.shouldHideTabBar = shouldHideTabBar;
}
