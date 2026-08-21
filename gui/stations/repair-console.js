/**
 * gui/stations/repair-console.js — the Repair renderer (issue #1235, T4.C3
 * chunk 1 of the console-seam programme).
 *
 * Only the battleship stations a dedicated Repair console today — the other
 * hulls fold repair into their Engineering seat (see
 * gui/stations/engineering-console.js, whose destroyer variant renders the
 * same External-dispatch panel off its own `repair` system view). This still
 * gets the tactical-console.js treatment — `makeRepairRender(variant)` →
 * `renderStation(s, doc)` — so a future hull's dedicated Repair seat costs a
 * small variant file rather than a second copy of this logic, and so this
 * renderer is vitest-testable without a browser.
 *
 * `s` is a flat `RepairConsolePayload` — the battleship's Repair seat is a
 * single-family station, so `initConsole` normalises it under
 * `family: 'repair'` before `render` ever sees it, and this renderer reads it
 * straight off the top level (no `systemView` indirection needed).
 *
 * @typedef {object} RepairVariant
 * @property {object} ids                       element ids present in this hull's markup
 * @property {string} ids.hullIntegrity           `ph-hull-integrity` id
 * @property {string} [ids.coreDamage]            ownerless "core" systems bar id
 * @property {string} ids.repairTeams             `ph-repair-teams` id
 * @property {string} [ids.stationDamage]         footer `ph-station-damage` id
 * @property {string} [ids.footerRight]           footer active/total-teams status text id
 * @property {string} [ids.autoBadge]             the AUTO badge id
 * @property {string} [ids.dispatchPanel]         External-dispatch panel id (issue #1161),
 *   shown only when the payload carries `external_dispatch`
 * @property {string} [ids.dispatchBtn]           dispatch/recall button id
 * @property {string} [ids.dispatchStatus]        dispatch status text id
 * @property {string} [ids.dispatchRefusal]       dispatch refusal text id
 * @property {string} [dispatchWorkingClass]      CSS class toggled on the dispatch
 *   button while a team is out (default `'working'`, this hull's class)
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';

/**
 * Build a Repair `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {RepairVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeRepairRender(variant) {
  const ids = variant.ids || {};

  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    const oh = s.overall_hull || {};
    const hullEl = doc.getElementById(ids.hullIntegrity);
    if (hullEl) hullEl.state = { total_pct: oh.pct != null ? oh.pct : 1, destroyed_pct: oh.destroyed_pct };

    if (ids.coreDamage) {
      const el = doc.getElementById(ids.coreDamage);
      if (el) el.state = { entries: s.core_systems || [] };
    }

    const teamsEl = doc.getElementById(ids.repairTeams);
    if (teamsEl) {
      teamsEl.state = { teams: s.teams || [], auto: !!s.repair_auto, targets: s.dispatch_targets || [], damaged: s.damaged_systems || [] };
    }

    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }

    if (ids.footerRight) {
      const el = doc.getElementById(ids.footerRight);
      if (el) {
        const teams = s.teams || [];
        const active = teams.filter(function(tm) { return tm && tm.status && tm.status !== 'idle'; }).length;
        el.textContent = t('console.repair.footer_status', {
          pct: Math.round((oh.pct != null ? oh.pct : 1) * 100), active: active, total: teams.length,
        });
      }
    }

    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      if (el) setAutoState(null, el, !!s.repair_auto);
    }

    // ── External repair-team dispatch (issue #1161) ─────────────────────
    // Shown only on a hull that authored `[repair.external_dispatch]` — a
    // hull without one carries no `external_dispatch` view, so the panel
    // stays hidden and the console is unchanged.
    if (ids.dispatchPanel) {
      const panel = doc.getElementById(ids.dispatchPanel);
      const ed = s.external_dispatch || null;
      if (panel) {
        if (!ed) {
          panel.hidden = true;
        } else {
          panel.hidden = false;
          const working = ed.target != null;
          const btn = ids.dispatchBtn ? doc.getElementById(ids.dispatchBtn) : null;
          if (btn) {
            btn.classList.toggle(variant.dispatchWorkingClass || 'working', working);
            btn.textContent = t(working ? 'console.repair.dispatch.recall' : 'console.repair.dispatch.send');
          }
          const status = ids.dispatchStatus ? doc.getElementById(ids.dispatchStatus) : null;
          if (status) {
            status.textContent = working
              ? t('console.repair.dispatch.working') + (ed.target_name ? ' · ' + t(ed.target_name) : '')
              : t('console.repair.dispatch.idle') + ' · ' + t('console.repair.dispatch.range') + ' ' + Math.round(ed.range || 0);
          }
          const refusal = ids.dispatchRefusal ? doc.getElementById(ids.dispatchRefusal) : null;
          if (refusal) {
            if (ed.refusal) { refusal.hidden = false; refusal.textContent = t(ed.refusal); }
            else { refusal.hidden = true; refusal.textContent = ''; }
          }
        }
      }
    }

    if (variant.tail) variant.tail(s, doc, t);
  };
}
