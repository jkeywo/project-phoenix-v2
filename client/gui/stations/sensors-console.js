/**
 * gui/stations/sensors-console.js — the Sensors renderer (issue #1235,
 * T4.C3 chunk 2 of the console-seam programme).
 *
 * The battleship's Sensors seat is currently the only surviving
 * `sensors.html` — the cruiser and destroyer fold Sensors into their
 * Science / (future) consoles instead. This shared renderer follows the
 * same `make<Kind>Render(variant)` shape as its siblings so a future hull's
 * dedicated Sensors console slots in without a second inline `render`.
 *
 * Sensors is a FLAT single-family payload — `initConsole` is called with
 * flat Sensors-family payload — so `renderStation` reads `s`'s fields directly.
 *
 * @typedef {object} SensorsVariant
 * @property {object} ids                       element ids present in this hull's markup
 * @property {string} [ids.radar]                `ph-sensor-radar` id
 * @property {string} [ids.sensorPanel]          `ph-sensor-panel` id
 * @property {string} [ids.stationDamage]        `ph-station-damage` id
 * @property {string} [ids.scanRange]            scan-range readout id
 * @property {string} [ids.contactSub]           contact-count readout id
 * @property {string} [ids.targetName]           target-name readout id
 * @property {string} [ids.targetKind]           target-kind readout id
 * @property {string} [ids.footer]               footer target-text id
 * @property {string} [ids.cancelImpulseBtn]     Cancel-Impulse button id (also the AUTO
 *   disable/readonly target — see `setAutoState`)
 * @property {string} [ids.autoBadge]            the AUTO badge id
 * @property {string} [ids.shieldFacings]        target shield-facings readout id
 * @property {string} [ids.shieldsTag]           target shield-status tag id
 * @property {string} [ids.shieldFreqTag]        target shield-frequency tag id
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';

/**
 * Build a Sensors `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {SensorsVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeSensorsRender(variant) {
  const ids = variant.ids || {};

  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    if (ids.radar) { const el = doc.getElementById(ids.radar); if (el) el.state = s; }
    if (ids.sensorPanel) { const el = doc.getElementById(ids.sensorPanel); if (el) el.state = s; }
    if (ids.stationDamage) { const el = doc.getElementById(ids.stationDamage); if (el) el.state = s.own_hull || null; }
    if (ids.scanRange) { const el = doc.getElementById(ids.scanRange); if (el) el.textContent = String(s.scan_range || 0); }

    const blipCount = (s.blips || []).length;
    if (ids.contactSub) {
      const el = doc.getElementById(ids.contactSub);
      if (el) {
        el.textContent = blipCount === 1
          ? t('console.common.contacts.one', { n: 1 })
          : t('console.common.contacts.other', { n: blipCount });
      }
    }
    if (ids.targetName) {
      const el = doc.getElementById(ids.targetName);
      if (el) el.textContent = s.target_name || t('console.common.no_target');
    }
    if (ids.targetKind) {
      const el = doc.getElementById(ids.targetKind);
      if (el) el.textContent = s.target_kind ? String(s.target_kind).toUpperCase() : t('console.common.no_contact');
    }
    if (ids.footer) {
      const el = doc.getElementById(ids.footer);
      if (el) el.textContent = s.target_name || t('console.common.no_target');
    }
    if (ids.cancelImpulseBtn) {
      const el = doc.getElementById(ids.cancelImpulseBtn);
      if (el) el.hidden = !(s.impulse_charge_progress > 0);
    }

    renderShieldFacings(ids, s, doc);

    if (ids.cancelImpulseBtn || ids.autoBadge) {
      setAutoState(
        ids.cancelImpulseBtn ? doc.getElementById(ids.cancelImpulseBtn) : null,
        ids.autoBadge ? doc.getElementById(ids.autoBadge) : null,
        !!s.sensors_auto,
      );
    }

    if (variant.tail) variant.tail(s, doc, t);
  };
}

/**
 * The target shield-facings readout: per-facing bars when the wire sends
 * them, a single aggregate percentage when it sends only a fraction, or a
 * "no shield data" placeholder when it sends neither.
 */
function renderShieldFacings(ids, s, doc) {
  if (!ids.shieldFacings) return;
  const target = doc.getElementById(ids.shieldFacings);
  if (!target) return;
  const shields = s.target_shields || [];
  const shieldsTag = ids.shieldsTag ? doc.getElementById(ids.shieldsTag) : null;
  const freqTag = ids.shieldFreqTag ? doc.getElementById(ids.shieldFreqTag) : null;
  if (freqTag) {
    freqTag.textContent = s.target_shield_freq != null
      ? Math.round(s.target_shield_freq * 100) + '%'
      : t('console.common.no_data');
  }
  if (!shields.length && s.target_shield_fraction == null) {
    target.textContent = t('console.common.no_shield_data');
    if (shieldsTag) shieldsTag.textContent = t('console.common.no_data');
    return;
  }
  if (!shields.length && s.target_shield_fraction != null) {
    const pct = Math.max(0, Math.round(s.target_shield_fraction * 100));
    target.innerHTML = '<div class="s-facing"><span class="lbl">' + t('console.shield.shld') + '</span> <span class="pct">' + (pct > 0 ? pct + '%' : t('console.shield.down')) + '</span></div>';
    if (shieldsTag) shieldsTag.textContent = pct > 0 ? t('console.shield.online') : t('console.shield.shield_down');
    return;
  }
  target.innerHTML = shields.map(function(f) {
    const pct = f.max_hp > 0 ? Math.round((f.hp / f.max_hp) * 100) : 0;
    return '<div class="s-facing"><span class="lbl">' + (f.label || '?').toUpperCase() + '</span> <span class="pct">' + (f.online === false ? t('console.shield.down') : pct + '%') + '</span></div>';
  }).join('');
  if (shieldsTag) {
    shieldsTag.textContent = shields.some(function(f) { return f.online === false; })
      ? t('console.shield.degraded')
      : t('console.shield.online');
  }
}
