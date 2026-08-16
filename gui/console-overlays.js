/**
 * gui/console-overlays.js — the hero-bar toggle / full-frame panel pair that a
 * console uses to show a surface it does not have room for inline.
 *
 * The pattern was written three times inline (destroyer Tactical, courier
 * Pilot, courier Captain) before issue #984 needed a fourth, fifth and sixth
 * copy: Comms and Navigation are human-seeking, so ANY station can be asked to
 * host them, and every destroyer console therefore grows the same two toggles.
 * Six copies of a mutual-exclusion rule is five too many, so the rule lives
 * here and the consoles carry only markup.
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
 * no strings. Which toggles are VISIBLE is a separate question, answered by
 * `setSoughtToggle` in gui/console-ui.js from the console's own payload.
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
 * Open `panelId`, or close it if it is already open. Every other panel closes
 * either way.
 */
export function toggleConsoleOverlay(panelId, doc) {
  const root = doc || (typeof document !== 'undefined' ? document : null);
  if (!root) return;
  const panel = root.getElementById(panelId);
  const toggle = root.querySelector('.overlay-toggle[data-overlay="' + panelId + '"]');
  const wasOpen = !!panel && panel.classList.contains('open');
  closeConsoleOverlays(root);
  if (wasOpen || !panel) return;
  panel.classList.add('open');
  if (toggle) {
    toggle.dataset.active = 'true';
    toggle.classList.add('active');
  }
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
