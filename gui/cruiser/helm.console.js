/**
 * gui/cruiser/helm.console.js — the cruiser's Helm seat (issue #1235).
 *
 * Adds the lateral-thrust joystick and the zero-contacts-fallback,
 * glyph-prefixed, tint-by-count footer the battleship's Helm does not
 * carry, plus one bespoke tail panel since #1388: the contextual Dock
 * control this hull's helm-owned `dock` system publishes for. It hides
 * itself entirely when the payload carries no dock view, so a flight that
 * never comes near a berth sees the console it always had.
 *
 * NO under-tow-load banner — that reads a `tractor` blackboard, and this hull
 * mounts no tractor, so it stays a destroyer-only panel.
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
  footer: { id: 'footer-target', zeroFallback: true, glyph: true, colorize: true },
  tail: renderDockPanel,
});
