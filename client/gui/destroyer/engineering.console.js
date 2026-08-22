/**
 * gui/destroyer/engineering.console.js — the alliance destroyer's Engineering
 * seat (issue #1235).
 *
 * The Shields + Power + Repair core, plus a bespoke tail for three panels the
 * cruiser's Engineering seat does not mount: Tractor beam (issue #1156),
 * Transfer umbilical (issue #1160) and External repair-team dispatch (issue
 * #1161, `[repair.external_dispatch]`). Each panel hides itself entirely when
 * this hull's payload carries no matching system view, so a hull without one
 * stays unchanged.
 *
 * Every panel here toggles one button between two admitted commands (engage/
 * release, start/stop, dispatch/recall) — human and AI both issue the SAME
 * command through the one admission path, this control just picks which. The
 * click handlers live in `engineering.html` (they call `sendAction`, which
 * only exists after `initConsole` runs) and read the "which one" off the
 * button's own `.engaged` class rather than a variable threaded back out of
 * `render` — the class is set here on every render, so it is never stale.
 */
import { makeEngineeringRender } from '../stations/engineering-console.js';

export const renderStation = makeEngineeringRender({
  ids: {
    shieldFacings: 'shield-facings',
    threatRow: 'threat-row',
    threatBearing: 'threat-bearing',
    power: 'power-controls',
    battery: 'battery-bar',
    hullIntegrity: 'hull-integrity',
    coreDamage: 'core-damage',
    repairTeams: 'repair-teams',
    stationDamage: 'station-damage',
    autoBadge: 'engineering-auto-badge',
  },
  tail: (s, views, doc, t) => {
    // ── Tractor beam (issue #1156) ─────────────────────────────────────
    const panel = doc.getElementById('tractor-panel');
    const tr = (s.systems && s.systems['tractor']) || null;
    if (panel) {
      if (!tr) {
        panel.hidden = true;
      } else {
        panel.hidden = false;
        const engaged = !!tr.engaged;
        const btn = doc.getElementById('tractor-btn');
        if (btn) {
          btn.classList.toggle('engaged', engaged);
          btn.textContent = t(engaged ? 'console.tractor.release' : 'console.tractor.engage');
        }
        const status = doc.getElementById('tractor-status');
        if (status) {
          status.textContent = engaged
            ? t('console.tractor.holding') + (tr.coupled_target_name ? ' · ' + t(tr.coupled_target_name) : '')
            : t('console.tractor.idle') + ' · ' + t('console.tractor.range') + ' ' + Math.round(tr.range || 0);
        }
        const refusal = doc.getElementById('tractor-refusal');
        if (refusal) {
          if (tr.refusal) { refusal.hidden = false; refusal.textContent = t(tr.refusal); }
          else { refusal.hidden = true; refusal.textContent = ''; }
        }
      }
    }

    // ── Transfer umbilical (issue #1160) ───────────────────────────────
    const uPanel = doc.getElementById('umbilical-panel');
    const um = (s.systems && s.systems['umbilical']) || null;
    if (uPanel) {
      if (!um) {
        uPanel.hidden = true;
      } else {
        uPanel.hidden = false;
        const running = !!um.running;
        const uBtn = doc.getElementById('umbilical-btn');
        if (uBtn) {
          uBtn.classList.toggle('engaged', running);
          uBtn.textContent = t(running ? 'console.umbilical.stop' : 'console.umbilical.start');
        }
        // Both ends' levels — operator then partner — with a '—' where a
        // ledger is absent (undocked, or a partner that carries no such
        // capacity).
        const lvl = (v) => (v == null) ? '—' : Math.round(v);
        const uStatus = doc.getElementById('umbilical-status');
        if (uStatus) {
          uStatus.textContent = t(running ? 'console.umbilical.flowing' : 'console.umbilical.idle')
            + ' · ' + t('console.umbilical.rate') + ' ' + Math.round(um.rate || 0)
            + ' · ' + t('console.umbilical.levels') + ' ' + lvl(um.operator_level) + ' → ' + lvl(um.partner_level);
        }
        const uRefusal = doc.getElementById('umbilical-refusal');
        if (uRefusal) {
          if (um.refusal) { uRefusal.hidden = false; uRefusal.textContent = t(um.refusal); }
          else { uRefusal.hidden = true; uRefusal.textContent = ''; }
        }
      }
    }

    // ── External repair-team dispatch (issue #1161) ────────────────────
    // On this hull the repair system is engineering-owned, so the dispatch
    // view rides `views.repair` (the same payload the battleship's dedicated
    // Repair console reads off its own flat payload).
    const dPanel = doc.getElementById('dispatch-panel');
    const ed = views.repair.external_dispatch || null;
    if (dPanel) {
      if (!ed) {
        dPanel.hidden = true;
      } else {
        dPanel.hidden = false;
        const working = ed.target != null;
        const dBtn = doc.getElementById('dispatch-btn');
        if (dBtn) {
          dBtn.classList.toggle('engaged', working);
          dBtn.textContent = t(working ? 'console.repair.dispatch.recall' : 'console.repair.dispatch.send');
        }
        const dStatus = doc.getElementById('dispatch-status');
        if (dStatus) {
          dStatus.textContent = working
            ? t('console.repair.dispatch.working') + (ed.target_name ? ' · ' + t(ed.target_name) : '')
            : t('console.repair.dispatch.idle') + ' · ' + t('console.repair.dispatch.range') + ' ' + Math.round(ed.range || 0);
        }
        const dRefusal = doc.getElementById('dispatch-refusal');
        if (dRefusal) {
          if (ed.refusal) { dRefusal.hidden = false; dRefusal.textContent = t(ed.refusal); }
          else { dRefusal.hidden = true; dRefusal.textContent = ''; }
        }
      }
    }
  },
});
