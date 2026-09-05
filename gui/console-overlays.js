/**
 * gui/console-overlays.js — the toggle / full-frame panel pair a console uses
 * to show a surface it does not have room for inline.
 *
 * The pattern was written three times inline (destroyer Tactical, courier
 * Pilot, courier Captain) before issue #984 grew it onto every destroyer
 * console for Comms and Navigation, which were human-seeking systems any
 * station could be asked to host. Both have since become complete hero-bar
 * Stations of their own (issues #1097, #1098).
 *
 * SINCE ISSUE #1374 NO CONSOLE THIS MODULE DRIVES AUTHORS A TOGGLE. The
 * shell's Station Bar is the selector — a console DECLARES its panels (issue
 * #1373) and the bar pushes the answer back in through `__setConsoleOverlay` →
 * `setConsoleOverlay` — and the panel's own Back button is the one in-console
 * way out. (gui/cruiser/comms.html still has a chart button of its own, driven
 * by that document's own script rather than this module; issue #1371's
 * Navigation slice is where it and its panel get rebuilt.) The toggle half of
 * the convention below is kept, tested and supported all the same: it is the
 * shape a console reaches for when it needs a surface of its own that the bar
 * has no business offering.
 *
 * THE CONVENTION, and it is the whole API:
 *
 *   <button class="overlay-toggle" data-overlay="comms-overlay">Comms</button>
 *   <div class="overlay-panel" id="comms-overlay">
 *     <button class="overlay-back" data-overlay-back>Back</button>
 *     …
 *   </div>
 *
 * A toggle names its panel; a panel's back button names nothing. Opening one
 * panel closes every other, because these panels cover the console — two open
 * at once is not a layout, it is a bug you cannot see.
 *
 * The module reads the DOM and nothing else: no console payload, no simState,
 * no strings. Which toggles are VISIBLE was a separate question for the
 * human-seeking Comms/Navigation toggles this pattern used to cover; both are
 * complete hero-bar Stations now (issues #1097, #1098), so nothing in this
 * module answers that question any more.
 */

/**
 * Every panel closed, every toggle unlit.
 *
 * `doc` is a Document (the console iframe's own), defaulted for callers inside
 * one; the parameter exists so the behaviour is testable against a jsdom
 * document rather than only in a browser.
 */
export function closeConsoleOverlays(doc) {
  const root = doc || (typeof document !== 'undefined' ? document : null);
  if (!root) return;
  root.querySelectorAll('.overlay-panel').forEach(function (panel) {
    panel.classList.remove('open');
  });
  root.querySelectorAll('.overlay-toggle').forEach(function (btn) {
    btn.dataset.active = 'false';
    btn.classList.remove('active');
  });
}

/**
 * Open exactly `panelId` — or, with a nullish id, nothing. Every other panel
 * closes either way.
 *
 * SET, not toggle, and that is the whole reason this exists beside the toggle
 * below. The shell's Station Bar owns which overlay tab is selected (issue
 * #1373) and pushes that answer in through `__setConsoleOverlay`; a toggle
 * asked to reach a KNOWN state makes the caller read the DOM back first and
 * lands on the wrong panel whenever two pushes arrive in one frame.
 * `toggleConsoleOverlay` is expressed in terms of this one, so the open rule
 * exists once and the two can never drift.
 *
 * @param {string|null|undefined} panelId
 * @param {Document} [doc]
 * @returns {string|null} the id actually opened, or null when nothing is open.
 *   An id naming no panel in this document closes everything and returns null
 *   rather than reporting an open panel that is not there.
 */
export function setConsoleOverlay(panelId, doc) {
  const root = doc || (typeof document !== 'undefined' ? document : null);
  if (!root) return null;
  closeConsoleOverlays(root);
  if (!panelId) return null;
  const panel = root.getElementById(panelId);
  if (!panel || !panel.classList.contains('overlay-panel')) return null;
  panel.classList.add('open');
  const toggle = root.querySelector('.overlay-toggle[data-overlay="' + panelId + '"]');
  if (toggle) {
    toggle.dataset.active = 'true';
    toggle.classList.add('active');
  }
  return panelId;
}

/**
 * Which panel is covering the console right now, or null.
 *
 * The console's own render asks this to learn whether the seat is LOOKING at a
 * surface: Intel's unread badge clears on being read, not on the state that
 * filled it (issue #1373), and "is the panel open" is a DOM fact this module
 * already owns rather than something to re-derive per console.
 */
export function openConsoleOverlayId(doc) {
  const root = doc || (typeof document !== 'undefined' ? document : null);
  if (!root) return null;
  const open = root.querySelector('.overlay-panel.open');
  return open && open.id ? open.id : null;
}

/**
 * Open `panelId`, or close it if it is already open. Every other panel closes
 * either way.
 */
export function toggleConsoleOverlay(panelId, doc) {
  const root = doc || (typeof document !== 'undefined' ? document : null);
  if (!root) return;
  const panel = root.getElementById(panelId);
  const wasOpen = !!panel && panel.classList.contains('open');
  setConsoleOverlay(wasOpen ? null : panelId, root);
}

/**
 * Wire every `data-overlay` toggle and every `data-overlay-back` button in the
 * document. Call once, at console module scope.
 *
 * Listeners are delegated to the root rather than bound per element, so a
 * toggle that is `hidden` at load — which every seeking system's toggle is —
 * still works the moment the seek reveals it.
 */
export function initConsoleOverlays(doc) {
  const root = doc || (typeof document !== 'undefined' ? document : null);
  if (!root) return;
  root.addEventListener('click', function (ev) {
    const target = ev.target;
    if (!target || typeof target.closest !== 'function') return;
    const back = target.closest('[data-overlay-back]');
    if (back) {
      closeConsoleOverlays(root);
      return;
    }
    const toggle = target.closest('.overlay-toggle[data-overlay]');
    if (toggle) toggleConsoleOverlay(toggle.dataset.overlay, root);
  });
}
