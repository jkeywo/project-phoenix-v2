/**
 * gui/device-orientation.js — Pure JS port of the Bevy detect_orientation()
 * system (formerly src/client/phone_border/framing.rs; ported in #462, the
 * Rust original deleted in #463).
 *
 * Computes a singleton orientation ('portrait' | 'landscape') from the window
 * aspect ratio: landscape when width / height >= 1.0, otherwise portrait
 * (matching the Rust threshold). The value is recomputed on every
 * `window.resize` and exposed as `window.currentOrientation()` — the data
 * source client.html's render() already reads (with an inline fallback that
 * this module supersedes).
 *
 * DOM-light: the computation is pure (takes width/height) and unit-testable;
 * the singleton + listener wiring is installed only when a real `window` is
 * present.
 */

/**
 * Pure orientation classifier. Landscape when `width / height >= 1.0`
 * (square counts as landscape, mirroring framing.rs `aspect >= 1.0`).
 * A zero/invalid height falls back to portrait.
 *
 * @param {number} width
 * @param {number} height
 * @returns {'portrait' | 'landscape'}
 */
export function orientationFor(width, height) {
  if (!(height > 0)) return 'portrait';
  return width / height >= 1.0 ? 'landscape' : 'portrait';
}

// ── Singleton ────────────────────────────────────────────────────────────────

let _current = 'portrait';

/** Read the cached orientation singleton. */
export function currentOrientation() {
  return _current;
}

/**
 * Recompute the singleton from `win` (defaults to the global `window`).
 * Returns the new value. Callers may pass an explicit object with
 * `innerWidth` / `innerHeight` for testing.
 *
 * @param {{ innerWidth: number, innerHeight: number }} [win]
 * @returns {'portrait' | 'landscape'}
 */
export function updateOrientation(win) {
  win = win || (typeof window !== 'undefined' ? window : null);
  if (!win) return _current;
  _current = orientationFor(win.innerWidth, win.innerHeight);
  return _current;
}

// ── Install on window ────────────────────────────────────────────────────────
// Compute an initial value and keep it fresh on resize. The resize handler is
// debounced through `window.scheduleRender` when present (the existing
// rAF-coalesced render scheduler in client.html) so we don't thrash render on
// every resize event; the orientation value itself is updated synchronously so
// any reader sees the current value immediately.

if (typeof window !== 'undefined') {
  updateOrientation(window);

  const onViewportChange = function() {
    updateOrientation(window);
    if (typeof window.scheduleRender === 'function') {
      window.scheduleRender();
    }
  };
  // `orientationchange` is the reliable signal on phone rotation; `resize`
  // covers desktop window resizing and browsers that don't fire the former.
  window.addEventListener('resize', onViewportChange);
  window.addEventListener('orientationchange', onViewportChange);

  // Expose the reader client.html's render() consumes.
  window.currentOrientation = currentOrientation;
}
