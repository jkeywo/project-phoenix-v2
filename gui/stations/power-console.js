/**
 * gui/stations/power-console.js — the Power renderer (issue #1235, T4.C3
 * chunk 1 of the console-seam programme).
 *
 * Only the battleship stations a dedicated Power console today — the other
 * hulls fold power allocation into their Engineering seat (see
 * gui/stations/engineering-console.js). This still gets the tactical-console.js
 * treatment — `makePowerRender(variant)` → `renderStation(s, doc)` — so a
 * future hull's dedicated Power seat costs a small variant file rather than a
 * second copy of this logic, and so this renderer is vitest-testable without a
 * browser, exactly like every other migrated console.
 *
 * `s` is a flat `PowerConsolePayload` — the battleship's Power seat is a
 * single-family station, so `initConsole` normalises it under
 * `family: 'power'` before `render` ever sees it, and this renderer reads it
 * straight off the top level (no `systemView` indirection needed).
 *
 * @typedef {object} PowerVariant
 * @property {object} ids                    element ids present in this hull's markup
 * @property {string} ids.controls            `ph-power-controls` id
 * @property {string} ids.battery             `ph-battery-bar` id
 * @property {string} [ids.stationDamage]     footer `ph-station-damage` id
 * @property {string} [ids.autoBadge]         the AUTO badge id
 * @property {string} [ids.batteryLabel]      footer battery-percent text id
 * @property {string} [ids.dataEl]            hidden `#power-data` element id —
 *   a machine-readable mirror of `groups`/`total`/`draining` the Playwright
 *   smoke spec reads via `dataset` rather than scraping `ph-power-controls`'
 *   shadow DOM
 */

import { setAutoState } from '../console-ui.js';

/**
 * Build a Power `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {PowerVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makePowerRender(variant) {
  const ids = variant.ids || {};

  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    const groups = s.groups || s.consoles || [];
    const pct = s.battery_max > 0 ? (s.battery_charge / s.battery_max) * 100 : 0;

    const controlsEl = doc.getElementById(ids.controls);
    if (controlsEl) controlsEl.state = { groups: groups, auto: !!s.power_auto };

    const batteryEl = doc.getElementById(ids.battery);
    if (batteryEl) batteryEl.state = { level_pct: pct, charging: !!s.charging, emergency_threshold_pct: 20 };

    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }

    if (ids.batteryLabel) {
      const el = doc.getElementById(ids.batteryLabel);
      if (el) el.textContent = Math.round(pct) + '%';
    }

    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      if (el) setAutoState(null, el, !!s.power_auto);
    }

    // Machine-readable mirror for the console smoke spec.
    if (ids.dataEl) {
      const dataEl = doc.getElementById(ids.dataEl);
      if (dataEl) {
        dataEl.dataset.total = String(s.total || 0);
        dataEl.dataset.totalMax = String(s.total_max || 0);
        dataEl.dataset.draining = String(!!s.draining);
        dataEl.replaceChildren();
        groups.forEach(function(group) {
          const entry = doc.createElement('div');
          entry.className = 'power-entry';
          entry.dataset.id = group.id || '';
          entry.dataset.level = String(group.level || 0);
          // The standing order, which differs from `level` while a battery
          // floor is holding the group down (issue #952).
          entry.dataset.commandedLevel = String(group.commanded_level || group.level || 0);
          entry.dataset.maxLevel = String(group.max_level || 0);
          dataEl.appendChild(entry);
        });
      }
    }

    if (variant.tail) variant.tail(s, doc);
  };
}
