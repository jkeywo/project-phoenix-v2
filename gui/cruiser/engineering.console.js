/**
 * gui/cruiser/engineering.console.js — the cruiser's Engineering seat
 * (issue #1235).
 *
 * The plain case: Power + Repair only, no Shields column and no bespoke tail
 * (Tractor / Umbilical / Field-Repair-dispatch are all destroyer-only panels).
 */
import { makeEngineeringRender } from '../stations/engineering-console.js';

export const renderStation = makeEngineeringRender({
  ids: {
    power: 'power-controls',
    battery: 'battery-bar',
    hullIntegrity: 'hull-integrity',
    coreDamage: 'core-damage',
    repairTeams: 'repair-teams',
    stationDamage: 'station-damage',
    autoBadge: 'engineering-auto-badge',
  },
});
