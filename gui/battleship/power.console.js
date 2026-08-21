/**
 * gui/battleship/power.console.js — the Harrow battleship's Power seat
 * (issue #1235). The reference (and, for now, only) hull.
 */
import { makePowerRender } from '../stations/power-console.js';

export const renderStation = makePowerRender({
  ids: {
    controls: 'power-controls',
    battery: 'battery-bar',
    stationDamage: 'station-damage',
    autoBadge: 'power-auto-badge',
    batteryLabel: 'bat-val',
    dataEl: 'power-data',
  },
});
