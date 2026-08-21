/**
 * gui/battleship/sensors.console.js — the battleship's Sensors seat
 * (issue #1235).
 *
 * The only surviving Sensors seat (the cruiser folds sensors into Science;
 * the destroyer has neither) — no bespoke tail.
 */
import { makeSensorsRender } from '../stations/sensors-console.js';

export const renderStation = makeSensorsRender({
  ids: {
    radar: 'sensor-radar',
    sensorPanel: 'sensor-panel',
    stationDamage: 'station-damage',
    scanRange: 'scan-range-val',
    contactSub: 'contact-sub',
    targetName: 'tgt-name',
    targetKind: 'tgt-kind-tag',
    footer: 'footer-target',
    cancelImpulseBtn: 'btn-cancel-impulse',
    autoBadge: 'sensors-auto-badge',
    shieldFacings: 'shield-facings',
    shieldsTag: 'shields-tag',
    shieldFreqTag: 'shield-freq-tag',
  },
});
