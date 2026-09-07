/**
 * gui/destroyer/helm.console.js — the alliance destroyer's Helm seat
 * (issue #1235).
 *
 * Lateral-thrust joystick, no target-footer touch (the footer stays a
 * static "NO TARGET"), plus a bespoke tail for two panels: the contextual
 * Dock control (issue #1159 — shared with the cruiser since #1388, so the
 * body of it is `renderDockPanel` in gui/stations/helm-console.js rather
 * than a copy here) and the under-tow-load banner (issue #1157), which
 * stays destroyer-only because no other player hull mounts a tractor. Each
 * hides itself entirely when this hull's payload carries no matching view,
 * so a hull without one is unchanged.
 *
 * The Dock button toggles dock/undock through the action map — human and AI
 * issue the SAME admitted command, this control just picks which. The click
 * handler lives in `helm.html` and activates `helm.dock`; its adapter reads
 * the latest authoritative dock view and authored SystemId, so the variant
 * neither duplicates the verb decision nor dispatches stale rendered state.
 */
import { makeHelmRender, renderDockPanel } from '../stations/helm-console.js';

export const renderStation = makeHelmRender({
  ids: {
    radar: 'helm-radar',
    joystick: 'helm-joystick',
    lateral: 'lateral-thrust-joystick',
    impulse: 'impulse-btn',
    boost: 'boost-btn',
    autoBadge: 'helm-auto-badge',
  },
  tail: (s, doc, t) => {
    // ── Contextual dock control (issues #1159, #1388) ──────────────────
    renderDockPanel(s, doc, t);

    // ── Under-tow-load indicator (issue #1157) ─────────────────────────
    const towPanel = doc.getElementById('tow-load-panel');
    const tl = s.tow_load || null;
    if (towPanel) {
      if (!tl || !tl.active) {
        towPanel.hidden = true;
      } else {
        towPanel.hidden = false;
        const towTarget = doc.getElementById('tow-load-target');
        if (towTarget) towTarget.textContent = tl.target_name ? '· ' + t(tl.target_name) : '';
      }
    }
  },
});
