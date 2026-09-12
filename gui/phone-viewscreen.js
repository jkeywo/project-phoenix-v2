/**
 * gui/phone-viewscreen.js — is THIS Viewscreen endpoint phone-shaped?
 * (issue #1429, PRD #1418 stories 18-21)
 *
 * `server.html` is the shared Viewscreen/GM host document; PRD #1418 names a
 * SECOND supported surface for it — "a secondary phone experience" beside the
 * room's full display. The two callers of `isPhoneViewscreen` below answer:
 *
 *   - the Display tab (`gui/server-settings.js` -> `phoneLimited` on
 *     `gui/viewscreen-presentation-panel.js`): a phone-shaped Viewscreen
 *     offers only text scale, contrast and the Reduce effects preset. The
 *     individual camera-shake/flash/decorative-motion controls PRD #1418
 *     reserves for larger surfaces are OMITTED rather than rendered inert —
 *     see `gui/visual-effects.js`'s "named, not dead" rule for the sibling
 *     case (a surface with no consumer at all).
 *   - level-3 AI-to-AI Coordination chatter (`server.html`'s
 *     `#chatter-container`, `gui/phone-chatter-reader.js`): stays compact at
 *     any text scale on a phone-shaped Viewscreen so background chatter
 *     cannot fill a small screen; the same bubble enlarges normally on a
 *     full display. The CSS half of that lives in server.html's
 *     `.chatter-bubble` phone media query, keyed off the identical pair of
 *     queries this module tests in JS, so the two never drift apart.
 *
 * The thresholds mirror `client.html`'s own EXISTING phone-shaped console
 * breakpoint — its `@media (orientation: portrait) and (max-width: 599px)` /
 * `(orientation: landscape) and (max-height: 500px)` pair around
 * `#station-hero`, an unrelated feature answering the identical "is this a
 * phone" question. Reused here rather than inventing a third number for the
 * same shape of screen.
 *
 * `server.html` is landscape-LOCKED (`data-phx-force-landscape`, commit
 * 384ca3f7), but that is a pure CSS rotate of the RENDERED layout — the
 * viewport `matchMedia` reports against is the physical, pre-rotate screen,
 * so a phone held upright still matches the portrait branch below and one
 * held sideways still matches the landscape branch. That is exactly the pair
 * `docs/acceptance/1421-device-matrix.md`'s phone rows exercise:
 * `phone-390x844-portrait` and `phone-844x390-landscape`.
 *
 * Not a device/UA sniff: viewport shape only, matching every other
 * responsive rule in this repository (issue #1421's device-matrix kit found
 * no browser-support list this repo maintains beyond viewport fixtures).
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

/** Matches a phone held upright. */
export const PHONE_PORTRAIT_QUERY = '(orientation: portrait) and (max-width: 599px)';
/** Matches a phone held sideways. */
export const PHONE_LANDSCAPE_QUERY = '(orientation: landscape) and (max-height: 500px)';

/**
 * Whether `win` (defaults to the global `window`) is running server.html on a
 * phone-shaped screen right now.
 *
 * Stateless and recomputed on every call: a rotate mid-session is reflected
 * the next time a caller asks, rather than cached from whatever the page
 * looked like when it booted. The two callers named above both ask fresh —
 * the settings panel on every open/repaint, the chatter widget once per
 * arriving bubble.
 *
 * @param {Window|object|null} [win]
 * @returns {boolean}
 */
export function isPhoneViewscreen(win) {
  const target = win || (typeof window !== 'undefined' ? window : null);
  if (!target || typeof target.matchMedia !== 'function') return false;
  try {
    return target.matchMedia(PHONE_PORTRAIT_QUERY).matches
      || target.matchMedia(PHONE_LANDSCAPE_QUERY).matches;
  } catch (_) {
    return false; // a matchMedia that throws reads as "not a phone", not a crash
  }
}

// Expose for the non-module inline scripts in server.html, matching the
// window.* convention the other shared gui modules (focus-trap.js,
// coordination-popup.js) follow.
if (typeof window !== 'undefined') {
  window.isPhoneViewscreen = isPhoneViewscreen;
}
