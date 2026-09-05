/**
 * gui/iframe-bridge.js — Generic helpers for iframe console state-push (ADR-0001 §2).
 *
 * push(iframeEl, consoleName, stateJson)
 *   Calls `window.__updateConsole(consoleName, stateJson)` on the iframe's
 *   contentWindow, swallowing cross-origin / not-yet-loaded errors.
 *
 * setOverlay(iframeEl, overlayId)
 *   Calls `window.__setConsoleOverlay(overlayId|null)` on the iframe's
 *   contentWindow, swallowing the same errors (issue #1373).
 *
 * wireLoad(iframeEl, refreshFn)
 *   Attaches a 'load' listener so the iframe console receives current state
 *   after a page reload or first load.
 *
 * All three are exposed as `window.iframeBridgePush` /
 * `window.iframeBridgeSetOverlay` / `window.iframeBridgeWireLoad` for
 * non-module inline scripts (client.html).
 */

/**
 * Push a state snapshot to a console iframe.
 *
 * @param {HTMLIFrameElement|null} iframeEl
 * @param {string} consoleName  Lowercase station id (e.g. 'tactical'), matches
 *                              the `name` passed to `initConsole` inside each
 *                              per-console iframe post issue #618.
 * @param {string} stateJson    JSON-serialised console state
 */
export function push(iframeEl, consoleName, stateJson) {
  if (!iframeEl || !iframeEl.contentWindow) return;
  try {
    const fn = iframeEl.contentWindow.__updateConsole;
    if (typeof fn === 'function') fn(consoleName, stateJson);
  } catch (_) {}
}

/**
 * Select which overlay panel a console iframe is showing (issue #1373).
 *
 * The exact twin of `push` above, and deliberately so: the Station Bar's
 * overlay tabs are a SECOND thing the shell tells a console, reaching it the
 * same way state does — a named function on the iframe's own window, called
 * directly, with a not-yet-loaded or cross-origin frame swallowed rather than
 * thrown. A frame that has not run `initConsole` yet has no hook, so the call
 * is a no-op rather than an error and the next selection lands.
 *
 * @param {HTMLIFrameElement|null} iframeEl
 * @param {string|null} overlayId  the panel's DOM id, or null to close them all
 */
export function setOverlay(iframeEl, overlayId) {
  if (!iframeEl || !iframeEl.contentWindow) return;
  try {
    const fn = iframeEl.contentWindow.__setConsoleOverlay;
    if (typeof fn === 'function') fn(overlayId || null);
  } catch (_) {}
}

/**
 * Wire a 'load' listener on an iframe so it re-receives the current state
 * snapshot whenever it (re)loads.
 *
 * @param {HTMLIFrameElement|null} iframeEl
 * @param {function():void} refreshFn  Called with no args on every iframe load
 */
export function wireLoad(iframeEl, refreshFn) {
  if (!iframeEl) return;
  iframeEl.addEventListener('load', refreshFn);
}

// Expose for non-module inline scripts (client.html).
if (typeof window !== 'undefined') {
  window.iframeBridgePush       = push;
  window.iframeBridgeSetOverlay = setOverlay;
  window.iframeBridgeWireLoad   = wireLoad;
}
