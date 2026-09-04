/**
 * gui/pre-play-view.js — the ONE decision behind every full-screen surface
 * that can precede play on client.html (issue #1359).
 *
 * Four surfaces can cover the phone before a mission is running, and until
 * this module each of them toggled its own `display` from its own call site:
 *
 *   #asset-loading            z-index 200  — the `show-loading`/`hide-loading`
 *                                            reducer effects (gui/lobby-state.js
 *                                            on `LoadingProgress`/`GameStarted`),
 *                                            applied by client.html's
 *                                            applySideEffect().
 *   #scenario-picker-overlay  z-index 186  — render(), from
 *                                            lobbyState.showScenarioPicker().
 *   #waiting-overlay          z-index 185  — render(), from
 *                                            lobbyState.waitingForScenario.
 *   #join-entry               z-index  30  — the `.open` class, added by
 *                                            showEntry() and removed by
 *                                            attempt() in startPhoenixJoin().
 *
 * Nothing stopped two of them being displayed at once. A catalogue arriving
 * over the waiting overlay left BOTH up (render() returns early on the picker
 * branch, above the line that hides the waiting overlay), and a refused join
 * opened the code field UNDERNEATH either of them, where the guest could see
 * it was there and could not reach it. The only thing that decided what a
 * player actually saw was the z-index order — a stylesheet accident standing
 * in for a rule.
 *
 * So the rule is written down here instead: given whether the page is asking
 * for a join code, the lobby state and the asset preload fraction,
 * `prePlayView` names the ONE surface that shows, the string id it leads with,
 * and whether its progress is measurable. The priority is the z-index order
 * those four surfaces already had, and priority is the WHOLE rule — no input
 * suppresses a surface some other input asked for — so what a player sees in
 * any combination is exactly what they saw before; the surface underneath is
 * now genuinely hidden rather than merely covered.
 *
 * NOT in the set: **#game-over-overlay**. It is the post-play surface — it is
 * shown only in `GamePhase::GameOver`, it is the *result* of a mission rather
 * than a stage before one, and gui/game-over-view.js is already the one pure
 * decision behind it (outcome, headline, body, report rows). Folding it in
 * would mean this module deciding the shape of an ending report, which is a
 * different question with different inputs. The connection chrome
 * (#top-bar's #conn-label/#conn-dot/#retry-now-btn, #status, #conn-diag-row)
 * is not in the set either: it is corner chrome shared with server.html via
 * gui/page-chrome.js, deliberately visible ALONGSIDE whatever surface is up,
 * so it is not a competitor for the screen.
 *
 * Pure: no DOM, no client state, no imports. Model: gui/lobby-view.js and
 * gui/host-lobby-view.js — text that still needs resolving comes back as an
 * `{ id, params }` pair, exactly like their `statusLine`, and client.html's
 * glue is the only thing that touches an element.
 */

/**
 * Every pre-play surface, by element id, in decision priority — which is the
 * z-index order they already had, highest first. A consumer shows the one
 * `prePlayView` names and hides the rest of this list; that is the whole of
 * "exactly one surface".
 */
export const PRE_PLAY_SURFACES = Object.freeze([
  'asset-loading',
  'scenario-picker-overlay',
  'waiting-overlay',
  'join-entry',
]);

/**
 * The lead line each surface carries in its own markup, by element id.
 *
 * ADVISORY, not a render instruction: the page's own `data-i18n` attributes
 * own that text and gui/strings.js's `applyToDom` writes it, so a consumer
 * that also wrote `headline` into the element would be the second writer of
 * the same string. It is here so the decision can SAY what the surface it
 * names leads with — and tests/client/pre-play-view.test.js checks each id
 * against the `data-i18n` on that element in client.html, so the two cannot
 * drift apart in silence.
 */
const HEADLINES = Object.freeze({
  'asset-loading': 'client.preparing_scenario',
  'scenario-picker-overlay': 'client.select_scenario',
  'waiting-overlay': 'client.waiting_scenario',
  'join-entry': 'client.join.title',
});

/**
 * The preload fraction as a number in [0, 1], or null when no preload is in
 * flight. `0` is a real loading state (the overlay opens at 0%), so only an
 * absent or non-numeric value means "nothing to measure".
 */
function normaliseFraction(value) {
  if (value === null || value === undefined || value === '') return null;
  const n = Number(value);
  if (!Number.isFinite(n)) return null;
  return Math.min(1, Math.max(0, n));
}

function decision(surface, pct) {
  const measurable = surface !== null && pct !== null;
  return {
    // Element id of the surface that shows, or null when none of them does.
    surface,
    // What it says: an { id, params } pair for the string table, like
    // gui/lobby-view.js's statusLine. Null when no surface is up. Advisory —
    // the markup's own data-i18n already renders this line; see HEADLINES.
    headline: surface ? { id: HEADLINES[surface], params: {} } : null,
    // Whether this surface has progress worth a number.
    measurable,
    // The number, 0-100, and ONLY when it is measurable. Every other state —
    // a connecting link, a catalogue waiting on a tap, a host still choosing,
    // a guest typing a code — has nothing to measure and reports nothing,
    // rather than a stale or invented percentage.
    pct: measurable ? pct : null,
  };
}

/**
 * Decide the one pre-play surface.
 *
 * @param {{ joinPrompt?: boolean }} connection
 *        `joinPrompt` is true while the page is asking the guest for a join
 *        code — it has none to try, or the last one came back refused for a
 *        reason a retry cannot fix. It is the ONLY field read. A connection
 *        record carrying anything else (a `state`, say) is accepted and
 *        ignored: which surface a player sees does not depend on the link
 *        state, and the cross-product test holds that line by driving every
 *        state through every other input.
 * @param {{ pickingScenario?: boolean, waitingForScenario?: boolean }} lobby
 *        The two lobby facts that raise a surface:
 *        `lobbyState.showScenarioPicker()` and `lobbyState.waitingForScenario`.
 * @param {number|null} preloadFraction
 *        Asset preload progress in [0, 1] while the host is pre-caching, else
 *        null. The ONLY input that carries a measurable quantity.
 * @returns {{ surface: string|null, headline: {id: string, params: object}|null,
 *             measurable: boolean, pct: number|null }}
 */
export function prePlayView(connection, lobby, preloadFraction) {
  const conn = connection || {};
  const lob = lobby || {};
  const fraction = normaliseFraction(preloadFraction);

  // Highest z-index first, so the surface a player sees today is the surface
  // this returns. Assets pre-caching is opaque and covers the whole screen: it
  // beat everything before this module and it still does.
  if (fraction !== null) return decision('asset-loading', Math.round(fraction * 100));
  if (lob.pickingScenario) return decision('scenario-picker-overlay', null);
  if (lob.waitingForScenario) return decision('waiting-overlay', null);
  // Priority and nothing else: if the page is asking for a code and no surface
  // above outranks it, the field shows. There is deliberately no second rule
  // suppressing it — #join-entry is the page's only way in, showEntry() has
  // already written the refusal into it and taken focus by the time this runs,
  // and a decision that refused to raise it would leave a guest reading an
  // invisible error on a blank phone. That is the failure this module exists
  // to remove, not one to reintroduce from the other side.
  if (conn.joinPrompt) return decision('join-entry', null);
  return decision(null, null);
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.prePlayView = prePlayView;
  window.PRE_PLAY_SURFACES = PRE_PLAY_SURFACES;
}
