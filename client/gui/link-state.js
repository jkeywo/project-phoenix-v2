/**
 * gui/link-state.js — what a console looks like when it is not talking to the
 * ship (PRD #1023 module 4, user stories 14 and 15).
 *
 * Two failures, one cause: a console's chrome renders from local state, so it
 * looks identical whether the numbers behind it are live, stale or absent.
 *
 *   14. The phone loses its DataChannel. The console keeps its last frame,
 *       keeps its controls live, and keeps accepting taps — "I never keep
 *       issuing commands into the void" was not true, because nothing on the
 *       console said the void was there. The one existing signal is a 0.55rem
 *       dot in the corner.
 *
 *   15. The DataChannel opens but the first state has not arrived. Every panel
 *       renders its empty shape — zeroed bars, blank readouts — which is
 *       exactly what a BROKEN console looks like. An empty panel has to read
 *       as loading, and it cannot do that without being told to.
 *
 * Hence three modes rather than two. `connecting` is not a mild `dead`: one is
 * "not yet", which resolves on its own, and the other is "no longer", which
 * needs the player to know they are shouting at nobody.
 *
 * Pure and DOM-free. client.html maps the mode onto `[data-link]` on the
 * console container; the CSS dims and the banner speaks.
 */

/**
 * @param {'connecting'|'ready'|'disconnected'|'error'} status
 *        the connection-manager status, verbatim.
 * @param {boolean} hasFirstData
 *        whether any authoritative state has landed since the link came up.
 * @returns {{ mode: 'connecting'|'live'|'dead',
 *             bannerId: string|null, dim: boolean, disable: boolean }}
 *   `dim` and `disable` are separate on purpose: a connecting console is
 *   dimmed (it is not showing you anything real yet) but its controls are not
 *   struck through, because they are about to work. A dead one is both.
 */
export function linkView(status, hasFirstData) {
  if (status === 'disconnected') {
    return { mode: 'dead', bannerId: 'client.reconnecting', dim: true, disable: true };
  }
  if (status === 'error') {
    return { mode: 'dead', bannerId: 'client.conn_error', dim: true, disable: true };
  }
  // 'ready' with nothing received yet is still a blank console; it just has a
  // different reason to be blank, and a different thing to say about it.
  if (status !== 'ready' || !hasFirstData) {
    return { mode: 'connecting', bannerId: 'client.link_connecting', dim: true, disable: false };
  }
  return { mode: 'live', bannerId: null, dim: false, disable: false };
}

// Expose for the classic inline script in client.html.
if (typeof window !== 'undefined') {
  window.linkView = linkView;
}
