/**
 * gui/stations/science-console.js — the Science renderer (issue #1235,
 * T4.C3 chunk 2 of the console-seam programme).
 *
 * The cruiser's Science seat is currently the only `science.html` — it
 * combines the Sensors and Shields families onto one station. This shared
 * renderer follows the same `make<Kind>Render(variant)` shape as its
 * siblings so a future hull's dedicated Science console slots in without a
 * second inline `render`.
 *
 * Science is a system-id-KEYED `SystemStationConsolePayload` — `initConsole`
 * is called with no `family` — so `renderStation` reads through
 * `familyView` rather than off `s` directly, exactly like
 * `gui/stations/engineering-console.js`.
 *
 * @typedef {object} ScienceVariant
 * @property {object} ids                     element ids present in this hull's markup
 * @property {string} [ids.sensorRadar]         `ph-sensor-radar` id
 * @property {string} [ids.sensorPanel]         `ph-sensor-panel` id
 * @property {string} [ids.shieldPanel]         `ph-shield-panel` id
 * @property {string} [ids.shieldFacings]       `ph-shield-facings` id
 * @property {string} [ids.threatRow]           threat-bearing readout row id (paired
 *   with `ids.threatBearing`)
 * @property {string} [ids.threatBearing]       threat-bearing value span id
 * @property {string} [ids.footer]              footer target-text id
 * @property {string} [ids.autoBadge]           the AUTO badge id
 * @property {string} [ids.stationDamage]       footer `ph-station-damage` id
 * @property {function(object, {sensors: object, shields: object}, Document, function): void} [tail]
 *   Bespoke per-hull rendering the shared core does not cover, called with
 *   `(s, views, doc, t)` after the common panels are set.
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';
import { familyView } from '../console-payload.js';

/**
 * Build a Science `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {ScienceVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeScienceRender(variant) {
  const ids = variant.ids || {};

  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    const sensors = familyView(s, 'sensors');
    const shields = familyView(s, 'shields');

    // ── Sensors ──────────────────────────────────────────────────────────
    if (ids.sensorRadar) { const el = doc.getElementById(ids.sensorRadar); if (el) el.state = sensors; }
    if (ids.sensorPanel) { const el = doc.getElementById(ids.sensorPanel); if (el) el.state = sensors; }

    // ── Shields ──────────────────────────────────────────────────────────
    if (ids.shieldPanel) { const el = doc.getElementById(ids.shieldPanel); if (el) el.state = shields; }
    if (ids.shieldFacings) {
      const el = doc.getElementById(ids.shieldFacings);
      if (el) el.state = { facings: shields.facings || [], focused_facing: shields.focused_facing || null, auto: !!shields.shields_auto };
    }
    if (ids.threatRow && ids.threatBearing) {
      const threatRow = doc.getElementById(ids.threatRow);
      const threatBearing = doc.getElementById(ids.threatBearing);
      if (threatRow && threatBearing) {
        if (shields.threat_bearing != null) {
          threatRow.classList.add('active');
          threatBearing.textContent = Math.round(shields.threat_bearing) + '°M';
        } else {
          threatRow.classList.remove('active');
          threatBearing.textContent = '—';
        }
      }
    }

    // ── Target footer ────────────────────────────────────────────────────
    if (ids.footer) {
      const el = doc.getElementById(ids.footer);
      if (el) {
        const tn = sensors.target_name || null;
        el.textContent = tn ? '◉ ' + tn : t('console.common.no_target');
        el.style.color = tn ? 'var(--tactical)' : 'var(--ink-faint)';
      }
    }

    // ── AUTO badge ───────────────────────────────────────────────────────
    // Station badge: the retired composite's science_auto meant "station is
    // Backfill-rated". The per-system equivalent is every owned system
    // AI-run — the conjunction of the resolved views' *_auto flags (the
    // controlSources these derive from can lag a rating change by one tick,
    // so the badge may flip one update late).
    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      if (el) setAutoState(null, el, !!(sensors.sensors_auto && shields.shields_auto));
    }

    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }

    // ── Bespoke per-hull tail ────────────────────────────────────────────
    if (variant.tail) variant.tail(s, { sensors, shields }, doc, t);
  };
}
