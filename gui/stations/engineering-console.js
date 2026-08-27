/**
 * gui/stations/engineering-console.js — one Engineering renderer, N hulls
 * (issue #1235, T4.C3 chunk 1 of the console-seam programme).
 *
 * The cruiser and alliance-destroyer Engineering seats each shipped their own
 * inline `render(s)` in their `engineering.html`. The two are ~80% the same —
 * power controls + battery, hull integrity, a "core" (ownerless-system) damage
 * bar, repair teams, a station-damage footer and an AUTO badge that is the
 * conjunction of every owned system's own auto flag — differing in which
 * system families the station owns (the destroyer's Engineering seat also
 * owns Shields, and reads a bespoke tail of Field Repair / Tractor / Umbilical
 * panels the cruiser's does not mount).
 *
 * A hull supplies a `variant` object (below) and gets back a
 * `renderStation(s, doc = document)` it hands straight to `initConsole`'s
 * `render`. `s` is always the system-id-keyed `SystemStationConsolePayload` —
 * neither hull's Engineering seat is a single-family flat payload, so
 * `initConsole` is called with no `family` (same as before this migration).
 *
 * @typedef {object} EngineeringVariant
 * @property {object} ids                       element ids present in this hull's markup
 * @property {string} [ids.shieldFacings]        `ph-shield-facings` id, only on a hull
 *   whose Engineering seat also owns Shields
 * @property {string} [ids.threatRow]            threat-bearing readout row id (paired
 *   with `ids.shieldFacings`)
 * @property {string} [ids.threatBearing]        threat-bearing value span id
 * @property {string} ids.power                  `ph-power-controls` id
 * @property {string} ids.battery                `ph-battery-bar` id
 * @property {string} ids.hullIntegrity           `ph-hull-integrity` id
 * @property {string} [ids.coreDamage]            ownerless "core" systems bar id
 * @property {string} ids.repairTeams             `ph-repair-teams` id
 * @property {string} [ids.stationDamage]         footer `ph-station-damage` id
 * @property {string} [ids.autoBadge]             the AUTO badge id
 * @property {function(object, {shields: object|null, power: object, repair: object}, Document, function): void} [tail]
 *   Bespoke per-hull rendering the shared core does not cover (Tractor,
 *   Umbilical, Field-Repair-dispatch panels), called with `(s, views, doc, t)`
 *   after the common panels are set. `views.shields` is `null` on a hull with
 *   no Shields column.
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';
import { familyView } from '../console-payload.js';

/**
 * Build an Engineering `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {EngineeringVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeEngineeringRender(variant) {
  const ids = variant.ids || {};

  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    // ── Shields (only on a hull whose Engineering seat owns Shields) ──────
    let sh = null;
    if (ids.shieldFacings) {
      sh = familyView(s, 'shields');
      const shieldEl = doc.getElementById(ids.shieldFacings);
      if (shieldEl) {
        shieldEl.state = { facings: sh.facings || [], focused_facing: sh.focused_facing || null, auto: !!sh.shields_auto };
      }
      if (ids.threatRow && ids.threatBearing) {
        const threatRow = doc.getElementById(ids.threatRow);
        const threatBearing = doc.getElementById(ids.threatBearing);
        if (threatRow && threatBearing) {
          if (sh.threat_bearing != null) {
            threatRow.classList.add('active');
            threatBearing.textContent = Math.round(sh.threat_bearing) + '°M';
          } else {
            threatRow.classList.remove('active');
            threatBearing.textContent = '—';
          }
        }
      }
    }

    // ── Power ───────────────────────────────────────────────────────────
    const p = familyView(s, 'power');
    const powerEl = doc.getElementById(ids.power);
    if (powerEl) powerEl.state = { groups: p.consoles || [], auto: !!p.power_auto };
    const batteryEl = doc.getElementById(ids.battery);
    if (batteryEl) {
      batteryEl.state = {
        level_pct: (p.battery_max > 0 ? (p.battery_charge / p.battery_max * 100) : 0),
        charging: !!p.battery_online && !!p.charging,
        emergency_threshold_pct: 20,
      };
    }

    // ── Repair / Hull ───────────────────────────────────────────────────
    const r = familyView(s, 'repair');
    // Overall ship-wide hull (every damageable system), not just this
    // station's own systems.
    const oh = r.overall_hull || {};
    const hullEl = doc.getElementById(ids.hullIntegrity);
    if (hullEl) hullEl.state = { total_pct: oh.pct != null ? oh.pct : 1, destroyed_pct: oh.destroyed_pct };
    // Ownerless "core" systems get their own click-to-expand bar that hides
    // itself entirely when there are none (issue #12).
    if (ids.coreDamage) {
      const el = doc.getElementById(ids.coreDamage);
      if (el) el.state = { entries: r.core_systems || [] };
    }
    const repairEl = doc.getElementById(ids.repairTeams);
    if (repairEl) {
      repairEl.state = { teams: r.teams || [], auto: !!r.repair_auto, targets: r.dispatch_targets || [], damaged: r.damaged_systems || [] };
    }

    // ── Station-damage footer ──────────────────────────────────────────
    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }

    // ── AUTO badge ──────────────────────────────────────────────────────
    // Station badge: the retired composite's engineering_auto meant "station
    // is Backfill-rated". The per-system equivalent is every owned system
    // AI-run — the conjunction of the resolved views' *_auto flags
    // (controlSources can lag a rating change by one tick). A hull with no
    // Shields column has nothing to conjoin there.
    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      const auto = (sh ? !!sh.shields_auto : true) && !!p.power_auto && !!r.repair_auto;
      if (el) setAutoState(null, el, auto);
    }

    // ── Bespoke per-hull tail ───────────────────────────────────────────
    if (variant.tail) variant.tail(s, { shields: sh, power: p, repair: r }, doc, t);
  };
}
