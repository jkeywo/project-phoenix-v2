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

/* ────────────────────────────────────────────────────────────────────────────
 * The loading surface (issue #1368)
 *
 * Two of the four surfaces above are *loading* surfaces: they say "the mission
 * is coming, keep holding the phone". `#asset-loading` says it while the host
 * pre-caches models, icons and rig sidecars, and `#waiting-overlay` says it
 * while the host is still choosing a World. Both now carry the same anatomy —
 * a ring, a lead line, a progress bar, and the Session named underneath it.
 *
 * `loadingView` takes `prePlayView`'s decision as its INPUT rather than
 * re-reading the inputs behind it. That is the whole point: a second read of
 * `preloadFraction` here would be a second answer to "is there anything to
 * measure", and the two would disagree the first time either was touched.
 * There is one decision, and this is a projection of it.
 * ──────────────────────────────────────────────────────────────────────────── */

/**
 * The loading treatment, one row per surface that wears it.
 *
 * DATA, not a switch. A surface joins the treatment by gaining a row here —
 * `statusId` is the line in the corner that says what is being waited on,
 * `ticksLeftId` is the small print under the bar that says what the bar is
 * counting, and `namesSession` is whether this surface may name the World at
 * all. Neither of the first two is a lead line: every surface's headline is
 * written into its own markup as a `data-i18n` and rendered by
 * gui/strings.js's applyToDom, exactly as HEADLINES above explains, and this
 * module is not a second writer of it.
 *
 * `namesSession` is the third column because "what is the crew about to play"
 * is a DIFFERENT question per surface, not a property of the Session record.
 * `#asset-loading` comes up with a World already loaded — `Welcome` landed
 * before the pre-cache began — so what the Session carries is what is coming.
 * `#waiting-overlay` is the opposite by construction: it exists for the window
 * where the host has NOT chosen a World, and `ReturnedToLobby` deliberately
 * leaves `scenarioTitle`/`shipConfig` standing (see gui/lobby-state.js), so
 * the Session there still describes the mission that just ENDED. Naming it
 * under "Waiting for host to select a scenario…" is the same invented fact as
 * a fabricated percentage, arriving on the ordinary GameOver → ReturnToLobby
 * path. Emptiness cannot tell the two apart — a stale title is a present,
 * well-formed string — so the surface has to say whether it is entitled to
 * name one.
 */
const LOADING_TREATMENT = Object.freeze({
  'asset-loading': Object.freeze({
    statusId: 'client.loading_assets',
    ticksLeftId: 'client.loading_ticks_assets',
    namesSession: true,
  }),
  'waiting-overlay': Object.freeze({
    statusId: 'client.loading_waiting_host',
    ticksLeftId: 'client.loading_ticks_none',
    namesSession: false,
  }),
});

/** The pre-play surfaces that wear the loading treatment, in priority order. */
export const LOADING_SURFACES = Object.freeze(
  PRE_PLAY_SURFACES.filter((id) => id in LOADING_TREATMENT),
);

/**
 * Link states in which the page has lost the host and is trying again.
 *
 * This is the ONLY thing the link state decides, and it decides a LINE OF TEXT
 * rather than a surface. Which surface shows is still priority and nothing
 * else — see `prePlayView` — so a dropped link cannot raise, suppress or swap
 * a surface; it can only change what the surface that is already up says about
 * itself. A player watching a bar that has stopped moving is owed the reason.
 */
const RETRYING_STATES = Object.freeze(['disconnected', 'error']);

/** The class badge every hull already has a translated label under. */
const SHIP_CLASS_PREFIX = 'component.ship_picker.class.';

/** A trimmed string, or '' for anything that is not one. */
function text(value) {
  return typeof value === 'string' ? value.trim() : '';
}

/**
 * What the loading surface says, or null when the surface that is up is not a
 * loading surface (the scenario picker and the join field are inputs waiting
 * on a person, not work waiting on a machine).
 *
 * @param {{surface: string|null, measurable: boolean, pct: number|null}} view
 *        `prePlayView`'s decision, passed through rather than recomputed.
 * @param {{ connectionState?: string, scenarioTitle?: string,
 *           shipClass?: string, hullId?: string }} [context]
 *        What the Session knows about itself: the link state, and the World's
 *        scenario and the ship the crew is about to fly. All optional — a
 *        surface that comes up before `Welcome` has landed names what it has.
 * @returns {object|null}
 */
export function loadingView(view, context) {
  const v = view || {};
  const ctx = context || {};
  const treatment = LOADING_TREATMENT[v.surface];
  if (!treatment) return null;

  // Straight from the decision. `measurable` is true for the asset preload and
  // nothing else, which is exactly the rule the bar needs: a real fraction
  // fills it, and everything else sweeps without claiming a number.
  const measurable = !!v.measurable && typeof v.pct === 'number';
  const pct = measurable ? v.pct : null;
  const retrying = RETRYING_STATES.includes(text(ctx.connectionState));

  const scenario = text(ctx.scenarioTitle);
  const hull = text(ctx.hullId);
  const shipClass = text(ctx.shipClass);

  return {
    // Echoed so a renderer addresses the surface it was handed rather than
    // guessing, and so a caller can tell one model from another.
    surface: v.surface,
    // Advisory, like HEADLINES: the markup's own data-i18n renders this line.
    labelId: HEADLINES[v.surface],
    measurable,
    pct,
    // The bar. `indeterminate` is the honest state — motion without a number.
    // `width` is the measured fraction, and EMPTY when there is nothing to
    // measure: an indeterminate sweep's width is a stylesheet decision (it is
    // a third of the track, and it becomes the whole track under reduced
    // motion), and an inline width written from script would outrank both.
    bar: {
      indeterminate: !measurable,
      width: measurable ? `${pct}%` : '',
    },
    // The small print under the bar. The right-hand tick is the number again,
    // against its total, and exists only when there IS a number.
    ticks: {
      left: { id: measurable ? treatment.ticksLeftId : 'client.loading_ticks_none', params: {} },
      right: measurable ? { id: 'client.loading_of_total', params: { pct } } : null,
    },
    // The corner line: what is being waited on, or that the link went away and
    // the page is trying again.
    status: {
      id: retrying ? 'client.loading_retrying' : treatment.statusId,
      params: {},
    },
    retrying,
    // What the player is about to play. Two things have to be true for the
    // block to show: this surface may name a World at all (`namesSession` —
    // #waiting-overlay may not, because the Session it can see is the mission
    // that just ended), and there is something in the Session to name. Absent
    // rather than empty for the second: before `Welcome` lands there is
    // genuinely nothing to name, and a bordered empty box reads as a bug.
    context: {
      visible: !!treatment.namesSession && !!(scenario || hull || shipClass),
      scenario: scenario ? { text: scenario } : null,
      ship: {
        lineId: hull ? 'client.loading_ship' : 'client.loading_ship_class_only',
        classId: SHIP_CLASS_PREFIX + (shipClass || 'unknown'),
        hull,
      },
    },
  };
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.prePlayView = prePlayView;
  window.loadingView = loadingView;
  window.PRE_PLAY_SURFACES = PRE_PLAY_SURFACES;
  window.LOADING_SURFACES = LOADING_SURFACES;
}
