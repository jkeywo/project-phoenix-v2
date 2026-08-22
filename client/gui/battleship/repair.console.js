/**
 * gui/battleship/repair.console.js — the Harrow battleship's Repair seat
 * (issue #1235). The reference (and, for now, only) hull. Authors
 * `[repair.external_dispatch]`, so the Field Repair dispatch panel is live.
 */
import { makeRepairRender } from '../stations/repair-console.js';

export const renderStation = makeRepairRender({
  ids: {
    hullIntegrity: 'hull-integrity',
    coreDamage: 'core-damage',
    repairTeams: 'repair-teams',
    stationDamage: 'station-damage',
    footerRight: 'footer-right',
    autoBadge: 'repair-auto-badge',
    dispatchPanel: 'dispatch-panel',
    dispatchBtn: 'dispatch-btn',
    dispatchStatus: 'dispatch-status',
    dispatchRefusal: 'dispatch-refusal',
  },
  dispatchWorkingClass: 'working',
});
