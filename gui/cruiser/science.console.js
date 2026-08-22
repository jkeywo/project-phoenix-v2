/**
 * gui/cruiser/science.console.js — the cruiser's Science seat (issue #1235).
 *
 * The only surviving Science seat — Sensors + Shields, no bespoke tail.
 */
import { makeScienceRender } from '../stations/science-console.js';

export const renderStation = makeScienceRender({
  ids: {
    sensorRadar: 'sensor-radar',
    sensorPanel: 'sensor-panel',
    shieldPanel: 'shield-panel',
    shieldFacings: 'shield-facings',
    threatRow: 'threat-row',
    threatBearing: 'threat-bearing',
    footer: 'footer-target',
    autoBadge: 'science-auto-badge',
    stationDamage: 'station-damage',
  },
});
