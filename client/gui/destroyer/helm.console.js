/**
 * gui/destroyer/helm.console.js — the alliance destroyer's Helm seat
 * (issue #1235).
 *
 * Lateral-thrust joystick, no target-footer touch (the footer stays a
 * static "NO TARGET"), plus a bespoke tail for two panels: the contextual
 * Dock control (issue #1159) and the under-tow-load banner (issue #1157).
 * Both are shared with the cruiser — which mounts a dock since #1388 and a
 * tractor since #1390 — so their bodies are `renderDockPanel` and
 * `renderTowLoadPanel` in gui/stations/helm-console.js rather than copies
 * here. Each hides itself entirely when this hull's payload carries no
 * matching view, so a hull without one is unchanged.
 *
 * The Dock button toggles dock/undock through the action map — human and AI
 * issue the SAME admitted command, this control just picks which. The click
 * handler lives in `helm.html` and activates `helm.dock`; its adapter reads
 * the latest authoritative dock view and authored SystemId, so the variant
 * neither duplicates the verb decision nor dispatches stale rendered state.
 */
import { makeHelmRender, renderDockPanel, renderTowLoadPanel } from '../stations/helm-console.js';

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
    // ── Under-tow-load indicator (issues #1157, #1390) ─────────────────
    renderTowLoadPanel(s, doc, t);
  },
});
