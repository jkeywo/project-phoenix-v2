/**
 * gui/destroyer/helm.console.js — the alliance destroyer's Helm seat
 * (issue #1235).
 *
 * Lateral-thrust joystick, no target-footer touch (the footer stays a
 * static "NO TARGET"), plus a bespoke tail for two panels the other hulls'
 * Helm seats do not mount: the contextual Dock control (issue #1159) and
 * the under-tow-load banner (issue #1157). Each hides itself entirely when
 * this hull's payload carries no matching view, so a hull without one is
 * unchanged.
 *
 * The Dock button toggles dock/undock through the action map — human and AI
 * issue the SAME admitted command, this control just picks which. The click
 * handler lives in `helm.html` (it calls `sendAction`, which only exists
 * after `initConsole` runs) and reads the "which one" plus the authored Dock
 * SystemId off the button's own render-refreshed state — never a second id
 * inference in the action map (the #1235 chunk-1 tractor-button precedent).
 */
import { makeHelmRender } from '../stations/helm-console.js';

export const renderStation = makeHelmRender({
  ids: {
    radar: 'helm-radar',
    joystick: 'helm-joystick',
    lateral: 'lateral-thrust-joystick',
    impulse: 'impulse-btn',
    boost: 'boost-btn',
    stationDamage: 'station-damage',
    autoBadge: 'helm-auto-badge',
  },
  tail: (s, doc, t) => {
    // ── Contextual dock control (issue #1159) ──────────────────────────
    const dockPanel = doc.getElementById('dock-panel');
    const d = s.dock || null;
    const dockBtn = doc.getElementById('dock-btn');
    if (dockBtn) dockBtn.dataset.systemId = d?.system_id || '';
    if (dockPanel) {
      if (!d || (!d.available && !d.engaged && !d.docked)) {
        dockPanel.hidden = true;
      } else {
        dockPanel.hidden = false;
        const docked = !!d.docked;
        if (dockBtn) {
          dockBtn.classList.toggle('docked', docked);
          dockBtn.textContent = t(docked ? 'console.dock.undock' : 'console.dock.dock');
        }
        const dockStatus = doc.getElementById('dock-status');
        if (dockStatus) {
          dockStatus.textContent = docked
            ? t('console.dock.docked') + (d.docked_to_name ? ' · ' + t(d.docked_to_name) : '')
            : t('console.dock.available') + (d.available_target_name ? ' · ' + t(d.available_target_name) : '');
        }
        const dockRefusal = doc.getElementById('dock-refusal');
        if (dockRefusal) {
          if (d.refusal) { dockRefusal.hidden = false; dockRefusal.textContent = t(d.refusal); }
          else { dockRefusal.hidden = true; dockRefusal.textContent = ''; }
        }
      }
    }

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
