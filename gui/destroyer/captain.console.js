/**
 * gui/destroyer/captain.console.js — the alliance destroyer's Captain seat
 * (issue #1235).
 *
 * A system-id-keyed hull: the Captain view comes from the projected Captain
 * Console Family. The destroyer's Captain seat also
 * absorbs Sensors — there is no separate Sensors Station on this hull — so
 * the sensor radar, scan readout, deadline list and a locked-target-name
 * footer (reading the sensors view, not a contact count) are this hull's
 * bespoke tail. The AUTO badge conjuncts three views' own auto flags.
 */
import { makeCaptainRender } from '../stations/captain-console.js';
import { familyView } from '../console-payload.js';

export const renderStation = makeCaptainRender({
  captainView: (s) => familyView(s, 'captain'),
  ids: {
    camera: 'camera-select',
    redAlert: 'red-alert',
    objectives: 'objective-list',
    stationDamage: 'station-damage',
    autoBadge: 'captain-auto-badge',
  },
  // Station badge: the retired composite's captain_auto meant "station
  // is Backfill-rated". The per-system equivalent is every owned system
  // AI-run — the conjunction of the resolved views' *_auto flags
  // (controlSources can lag a rating change by one tick).
  autoState: (s, view) => !!(view.red_alert_auto && view.viewscreen_auto && familyView(s, 'sensors').sensors_auto),
  tail: (s, view, doc, t) => {
    const sensors = familyView(s, 'sensors');

    const sensorRadarEl = doc.getElementById('sensor-radar');
    if (sensorRadarEl) sensorRadarEl.state = sensors;
    const sensorEl = doc.getElementById('sensor-panel');
    if (sensorEl) sensorEl.state = sensors;

    // Visible mission deadlines, counted down server-side (issue #1024).
    const deadlineEl = doc.getElementById('deadline-list');
    if (deadlineEl) deadlineEl.state = { deadlines: view.deadlines || [] };

    // The science scan (issue #1032). The readout is DERIVED server-side
    // from the subject's own condition track — there is no authored scan
    // text anywhere behind it — and the button aims at whatever the captain
    // has selected on the sensor radar.
    const scanEl = doc.getElementById('scan-readout');
    if (scanEl) scanEl.state = { scan: sensors.scan || {}, target_uuid: sensors.target_uuid || null };

    const footerEl = doc.getElementById('footer-target');
    if (footerEl) {
      const targetName = sensors.target_name || null;
      footerEl.textContent = targetName ? '◉ ' + targetName : t('console.common.no_target');
      footerEl.style.color = targetName ? 'var(--tactical)' : 'var(--ink-faint)';
    }
  },
});
