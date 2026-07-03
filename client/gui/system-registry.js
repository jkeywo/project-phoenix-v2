/**
 * gui/system-registry.js - HTML fragment registry for station/system panels.
 *
 * The registry maps system kinds to the small DOM fragments that render them
 * inside a station's cohesive console. Issue #490 starts with Red Alert as the
 * first coarse system fragment; issue #529 adds Viewscreen.
 */

export const RED_ALERT_SYSTEM_ID = 'red-alert';
export const RED_ALERT_KIND = 'red_alert';

export const VIEWSCREEN_SYSTEM_ID = 'viewscreen';
export const VIEWSCREEN_KIND = 'viewscreen';

export function renderRedAlertFragment(doc, state) {
  const root = doc || document;
  const button = root.getElementById('red-alert-btn');
  const badge = root.getElementById('red-alert-auto-badge');
  if (!button) return;

  const systemId = state?.red_alert_system_id || RED_ALERT_SYSTEM_ID;
  const auto = !!state?.red_alert_auto;

  button.dataset.systemId = systemId;
  button.dataset.auto = String(auto);
  button.disabled = auto;
  button.classList.toggle('readonly', auto);

  if (badge) {
    badge.hidden = !auto;
    badge.textContent = 'AUTO';
  }
}

/**
 * Render the Viewscreen system fragment inside the Captain console.
 *
 * The fragment does not own a dedicated interactive button — the direction pad
 * and view-selector serve that role. Instead this function updates a dataset
 * attribute and shows/hides an AUTO badge on a dedicated viewscreen-indicator
 * element (id="viewscreen-auto-badge"), if present.
 *
 * @param {Document|null} doc  - document root; defaults to `document`.
 * @param {object} state       - parsed CaptainConsoleState.
 */
export function renderViewscreenFragment(doc, state) {
  const root = doc || document;
  const badge = root.getElementById('viewscreen-auto-badge');
  if (!badge) return;

  const auto = !!state?.viewscreen_auto;
  badge.hidden = !auto;
  badge.textContent = 'AUTO';
  badge.dataset.systemId = state?.viewscreen_system_id || VIEWSCREEN_SYSTEM_ID;
  badge.dataset.auto = String(auto);
}

export const SYSTEM_REGISTRY = Object.freeze({
  [RED_ALERT_KIND]: Object.freeze({
    kind: RED_ALERT_KIND,
    systemId: RED_ALERT_SYSTEM_ID,
    station: 'captain',
    fragmentId: 'red-alert-btn',
    render: renderRedAlertFragment,
  }),
  [VIEWSCREEN_KIND]: Object.freeze({
    kind: VIEWSCREEN_KIND,
    systemId: VIEWSCREEN_SYSTEM_ID,
    station: 'captain',
    fragmentId: 'viewscreen-auto-badge',
    render: renderViewscreenFragment,
  }),
});

if (typeof window !== 'undefined') {
  window.SYSTEM_REGISTRY = SYSTEM_REGISTRY;
}
