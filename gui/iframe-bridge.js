/**
 * gui/iframe-bridge.js — Generic helpers for iframe console state-push (ADR-0001 §2).
 *
 * push(iframeEl, consoleName, stateJson)
 *   Calls `window.__updateConsole(consoleName, stateJson)` on the iframe's
 *   contentWindow, swallowing cross-origin / not-yet-loaded errors.
 *
 * wireLoad(iframeEl, refreshFn)
 *   Attaches a 'load' listener so the iframe console receives current state
 *   after a page reload or first load.
 *
 * Both functions are exposed as `window.iframeBridgePush` /
 * `window.iframeBridgeWireLoad` for non-module inline scripts (client.html).
 */

/**
 * Push a state snapshot to a console iframe.
 *
 * @param {HTMLIFrameElement|null} iframeEl
 * @param {string} consoleName  PascalCase Console enum variant (e.g. 'Tactical')
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
  window.iframeBridgePush     = push;
  window.iframeBridgeWireLoad = wireLoad;
}
