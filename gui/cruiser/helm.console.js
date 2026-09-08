/**
 * gui/cruiser/helm.console.js — the cruiser's Helm seat (issue #1235).
 *
 * Adds the lateral-thrust joystick and the zero-contacts-fallback,
 * glyph-prefixed, tint-by-count footer the battleship's Helm does not
 * carry, plus a bespoke tail of two panels: the contextual Dock control this
 * hull's helm-owned `dock` system publishes for (#1388), and the
 * under-tow-load banner its engineering-owned `tractor` system publishes for
 * (#1390). Both hide themselves entirely when the payload carries no matching
 * view, so a flight that never comes near a berth and never tows sees the
 * console it always had.
 *
 * Both bodies are shared with the destroyer (`renderDockPanel` /
 * `renderTowLoadPanel` in gui/stations/helm-console.js) rather than copied
 * here — one control reading one authoritative view.
 *
 * The tractor's CONTROL is Engineering's on this hull; only the mass penalty
 * the tow costs reaches this seat, which is why the banner is here and the
 * Engage/Release button is not.
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
  footer: { id: 'footer-target', zeroFallback: true, glyph: true, colorize: true },
  tail: (s, doc, t) => {
    // ── Contextual dock control (issues #1159, #1388) ──────────────────
    renderDockPanel(s, doc, t);
    // ── Under-tow-load indicator (issues #1157, #1390) ─────────────────
    renderTowLoadPanel(s, doc, t);
  },
});
