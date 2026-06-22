/**
 * gui/system-registry.js - HTML fragment registry for station/system panels.
 *
 * The registry maps system kinds to the small DOM fragments that render them
 * inside a station's cohesive console. Issue #490 starts with Red Alert as the
 * first coarse system fragment.
 */

export const RED_ALERT_SYSTEM_ID = 'red-alert';
export const RED_ALERT_KIND = 'red_alert';

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

export const SYSTEM_REGISTRY = Object.freeze({
  [RED_ALERT_KIND]: Object.freeze({
    kind: RED_ALERT_KIND,
    systemId: RED_ALERT_SYSTEM_ID,
    console: 'CaptainChair',
    fragmentId: 'red-alert-btn',
    render: renderRedAlertFragment,
  }),
});

if (typeof window !== 'undefined') {
  window.SYSTEM_REGISTRY = SYSTEM_REGISTRY;
}
