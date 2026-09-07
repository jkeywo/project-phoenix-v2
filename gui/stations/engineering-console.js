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
 * owns Shields, and its bespoke tail adds a Field Repair dispatch panel the
 * cruiser's does not mount).
 *
 * Both player hulls whose Engineering seat owns coupling gear — the destroyer
 * since #1156/#1160, the cruiser since #1390 — render the SAME two Operations
 * panels, so their bodies live here as `renderTractorPanel` and
 * `renderUmbilicalPanel` rather than as a copy inside each hull's variant.
 * Same reason `renderDockPanel` moved into gui/stations/helm-console.js in
 * #1388: one control reading one authoritative view, and a copy per hull would
 * be a second place for the engage/release and start/stop decisions to drift.
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
 * @property {string} [ids.autoBadge]             the AUTO badge id
 * @property {function(object, {shields: object|null, power: object, repair: object}, Document, function): void} [tail]
 *   Bespoke per-hull rendering the shared core does not cover — the Operations
 *   panels (`renderTractorPanel` / `renderUmbilicalPanel` below) and the
 *   destroyer's Field-Repair-dispatch panel — called with `(s, views, doc, t)`
 *   after the common panels are set. `views.shields` is `null` on a hull with
 *   no Shields column.
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';
import { familySystemId, familyView } from '../console-payload.js';

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
      repairEl.state = {
        teams: r.teams || [],
        auto: !!r.repair_auto,
        targets: r.dispatch_targets || [],
        damaged: r.damaged_systems || [],
        // The field destination an open idle card offers (issue #1384) and the
        // team abroad on it (issue #1386) — the same shape
        // `gui/stations/repair-console.js` hands the component.
        external_dispatch: r.external_dispatch || null,
      };
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

/**
 * The Tractor beam control (issue #1156), shared by every hull whose
 * Engineering seat owns a `kind = "tractor"` System — the destroyer since #1156
 * and the cruiser since #1390.
 *
 * Hidden entirely unless this hull's payload carries a tractor system, so a
 * hull that authors no `[tractor]` shows nothing at all. The one button toggles
 * between the two admitted commands (engage / release) and its `.engaged` class
 * is set here on every render, which is what the click handler in the hull's
 * `engineering.html` reads to pick the verb — so it can never dispatch a stale
 * one. Every string written here is a `strings.csv` id through `tr()`; no
 * English crosses this seam.
 *
 * A hull opts in by calling this from its variant `tail`; its markup supplies
 * `tractor-panel` / `tractor-btn` / `tractor-status` / `tractor-refusal`.
 *
 * @param {object} s      the system-id-keyed Engineering console payload
 * @param {Document} doc
 * @param {function} tr   the string resolver (the shared `t`)
 */
export function renderTractorPanel(s, doc, tr) {
  const panel = doc.getElementById('tractor-panel');
  if (!panel) return;
  const tv = familyView(s, 'tractor');
  const tractorSystemId = tv.system_id || familySystemId(s, 'tractor');
  if (!tractorSystemId) {
    panel.hidden = true;
    return;
  }
  panel.hidden = false;
  const engaged = !!tv.engaged;
  const btn = doc.getElementById('tractor-btn');
  if (btn) {
    btn.classList.toggle('engaged', engaged);
    btn.textContent = tr(engaged ? 'console.tractor.release' : 'console.tractor.engage');
  }
  const status = doc.getElementById('tractor-status');
  if (status) {
    status.textContent = engaged
      ? tr('console.tractor.holding') + (tv.coupled_target_name ? ' · ' + tr(tv.coupled_target_name) : '')
      : tr('console.tractor.idle') + ' · ' + tr('console.tractor.range') + ' ' + Math.round(tv.range || 0);
  }
  const refusal = doc.getElementById('tractor-refusal');
  if (refusal) {
    if (tv.refusal) { refusal.hidden = false; refusal.textContent = tr(tv.refusal); }
    else { refusal.hidden = true; refusal.textContent = ''; }
  }
}

/**
 * The Transfer umbilical control (issue #1160), shared by every hull whose
 * Engineering seat owns a `kind = "umbilical"` System — the destroyer since
 * #1160 and the cruiser since #1390. Hidden, toggled and localized on exactly
 * the terms `renderTractorPanel` above describes.
 *
 * The status line reports BOTH ends' levels — operator then partner — with a
 * '—' where a ledger is absent (undocked, or a partner that carries no such
 * capacity), so the operator can see the capacity actually crossing.
 *
 * A hull opts in by calling this from its variant `tail`; its markup supplies
 * `umbilical-panel` / `umbilical-btn` / `umbilical-status` / `umbilical-refusal`.
 *
 * @param {object} s      the system-id-keyed Engineering console payload
 * @param {Document} doc
 * @param {function} tr   the string resolver (the shared `t`)
 */
export function renderUmbilicalPanel(s, doc, tr) {
  const panel = doc.getElementById('umbilical-panel');
  if (!panel) return;
  const um = familyView(s, 'umbilical');
  const umbilicalSystemId = um.system_id || familySystemId(s, 'umbilical');
  if (!umbilicalSystemId) {
    panel.hidden = true;
    return;
  }
  panel.hidden = false;
  const running = !!um.running;
  const btn = doc.getElementById('umbilical-btn');
  if (btn) {
    btn.classList.toggle('engaged', running);
    btn.textContent = tr(running ? 'console.umbilical.stop' : 'console.umbilical.start');
  }
  const lvl = (v) => (v == null) ? '—' : Math.round(v);
  const status = doc.getElementById('umbilical-status');
  if (status) {
    status.textContent = tr(running ? 'console.umbilical.flowing' : 'console.umbilical.idle')
      + ' · ' + tr('console.umbilical.rate') + ' ' + Math.round(um.rate || 0)
      + ' · ' + tr('console.umbilical.levels') + ' ' + lvl(um.operator_level) + ' → ' + lvl(um.partner_level);
  }
  const refusal = doc.getElementById('umbilical-refusal');
  if (refusal) {
    if (um.refusal) { refusal.hidden = false; refusal.textContent = tr(um.refusal); }
    else { refusal.hidden = true; refusal.textContent = ''; }
  }
}
