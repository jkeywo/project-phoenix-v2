/**
 * gui/loading-render.js — the loading surface's renderer (issue #1368).
 *
 * The DOM half of the pair whose pure half is `loadingView()` in
 * `gui/pre-play-view.js`. That function says what the surface says; this
 * writes it into a document. It is the exact sibling of
 * `gui/client-lobby-render.js` (#1369), `gui/host-lobby-render.js` (#1325) and
 * `gui/host-landing-render.js` (#1360), and is read the same way — no state of
 * its own, no transport, no reducer. Given a view model it writes a document,
 * and everything it cannot do itself it asks the caller for.
 *
 * ## There is only one decision behind it
 *
 * `loadingView` takes `prePlayView`'s decision as its input, so this renderer
 * cannot disagree with the module that decides which surface is up: it is
 * handed a projection of that same answer, or `null` when the surface showing
 * is not a loading surface at all. `renderLoading(doc, null, t)` writes
 * nothing — the caller does not have to know which surfaces wear the
 * treatment.
 *
 * ## `doc` is the first argument
 *
 * Every element is reached through the `doc` it is handed and never through a
 * free `document`, so a test renders into its own document and a second
 * surface — the host viewscreen carries an `#asset-loading` overlay of its
 * own — can share this renderer rather than growing a second implementation of
 * the same element ids.
 *
 * ## Every write is guarded on the element existing
 *
 * The two loading surfaces do not carry identical markup: `#waiting-overlay`
 * has no percentage, because nothing about a host still choosing a World is
 * measurable. So a branch whose element is absent must do nothing rather than
 * throw and abandon the rest of the render half-written. That is also what
 * makes the page's boot race survivable: `client.html` paints from the first
 * server message, and a partially mounted shell must write what it has.
 *
 * ## `t` is injected, never imported
 *
 * `client.html` holds a classic-script `t()` closed over `window.phStrings`; a
 * test imports `gui/strings.js` directly. Neither is this module's business.
 *
 * ## What it does NOT write
 *
 * The lead line. Every pre-play surface carries its own `data-i18n` and
 * gui/strings.js's `applyToDom` renders it — a renderer that also wrote
 * `labelId` would be the second writer of one string, which is the drift
 * #1359 wrote HEADLINES down to prevent. The model carries `labelId` so the
 * decision can SAY what the surface leads with; this reads it and leaves it
 * alone.
 */

/**
 * The parts of one loading surface, addressed as `<surface-id>-<part>`.
 *
 * Derived from the surface the model names rather than listed per surface, so
 * a surface joins the treatment by adding a row to `LOADING_TREATMENT` in
 * gui/pre-play-view.js and the matching ids to its markup — never by editing a
 * branch here. `#asset-loading-pct` is the id the shipped page already used,
 * which is why the scheme is a suffix on the surface id rather than a fresh
 * naming: the existing selector keeps working.
 */
export const LOADING_PARTS = Object.freeze([
  'pct-row', 'pct', 'bar', 'fill', 'tick-left', 'tick-right',
  'ctx', 'scenario', 'ship', 'status',
]);

/** Resolve a `{ id, params }` / `{ text }` label pair. Neither is a decision. */
function label(pair, t) {
  if (!pair) return '';
  if (typeof pair.text === 'string') return pair.text;
  return pair.id ? t(pair.id, pair.params || {}) : '';
}

/** Write `text` into `id` if this document has it. */
function setText(doc, id, value) {
  const el = doc.getElementById(id);
  if (el) el.textContent = value;
}

/** Show or hide `id` if this document has it. */
function setShown(doc, id, shown) {
  const el = doc.getElementById(id);
  if (el) el.hidden = !shown;
}

/**
 * Render one loading view model into `doc`.
 *
 * @param {Document} doc the document holding the loading surface's markup.
 * @param {object|null} vm the return of `loadingView()`; `null` renders
 *        nothing, which is what a non-loading pre-play surface returns.
 * @param {(id: string, params?: object) => string} t string-id resolver.
 */
export function renderLoading(doc, vm, t) {
  if (!doc || !vm || !vm.surface) return;
  const id = (part) => `${vm.surface}-${part}`;

  // ── The number ──────────────────────────────────────────────────────
  //
  // Only the asset preload has one. Every other state shows motion WITHOUT a
  // number rather than a percentage nobody measured, so the whole row goes
  // away instead of holding a stale or invented figure.
  setShown(doc, id('pct-row'), vm.measurable);
  if (vm.measurable) setText(doc, id('pct'), String(vm.pct));

  // ── The bar ─────────────────────────────────────────────────────────
  //
  // One class and one width, both from the model. The sweep itself is a
  // keyframe in the page's stylesheet with a reduced-motion counterpart beside
  // it — motion is a stylesheet decision, not a script one, which is what lets
  // the sweep stop without this module knowing anything about it.
  const bar = doc.getElementById(id('bar'));
  if (bar && bar.classList) bar.classList.toggle('indet', vm.bar.indeterminate);
  if (bar && bar.setAttribute) {
    // A determinate bar reports where it is; an indeterminate one reports that
    // it does not know, which is what an absent `aria-valuenow` means to a
    // screen reader. Saying "0%" would be the invented number in another form.
    //
    // The two attributes are EXCLUSIVE, and that is not a style choice: ARIA
    // says `aria-valuetext` SUPERSEDES `aria-valuenow` as the announced value.
    // Writing both would have published the percentage to the eye and hidden
    // it from assistive tech — "Loading — assets" announced forever while 62%
    // sat unread in the attribute beside it. So the number speaks when there
    // is one, and the status line speaks when there is not.
    if (vm.measurable) {
      bar.setAttribute('aria-valuenow', String(vm.pct));
      if (bar.removeAttribute) bar.removeAttribute('aria-valuetext');
    } else {
      if (bar.removeAttribute) bar.removeAttribute('aria-valuenow');
      bar.setAttribute('aria-valuetext', label(vm.status, t));
    }
  }
  const fill = doc.getElementById(id('fill'));
  if (fill && fill.style) fill.style.width = vm.bar.width;

  // ── The small print under it ────────────────────────────────────────
  setText(doc, id('tick-left'), label(vm.ticks.left, t));
  setText(doc, id('tick-right'), label(vm.ticks.right, t));

  // ── What the crew is about to play ──────────────────────────────────
  //
  // The World's scenario and the hull, so a player waiting with the phone in
  // their hand knows what they are waiting for. Hidden outright before
  // `Welcome` lands: a bordered box holding two empty lines reads as a defect,
  // where an absent one reads as "not known yet".
  setShown(doc, id('ctx'), vm.context.visible);
  setText(doc, id('scenario'), label(vm.context.scenario, t));
  setText(doc, id('ship'), t(vm.context.ship.lineId, {
    hull: vm.context.ship.hull,
    class: t(vm.context.ship.classId),
  }));

  // ── The corner line ─────────────────────────────────────────────────
  //
  // What is being waited on — or, when the link has gone, that the page is
  // trying again. A bar that has stopped moving with no explanation is the
  // failure this line exists to close.
  setText(doc, id('status'), label(vm.status, t));
  const status = doc.getElementById(id('status'));
  if (status && status.classList) status.classList.toggle('retrying', vm.retrying);
}

// Expose for the classic (non-module) script in client.html — the same
// self-registering pattern window.hostLandingRender uses.
if (typeof window !== 'undefined') {
  window.loadingRender = { renderLoading, LOADING_PARTS };
}
