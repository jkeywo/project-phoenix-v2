/**
 * gui/cruiser/tactical.console.js — the cruiser's Tactical seat (issue #1234).
 *
 * Like the battleship (flat `tactical` payload, phasers + torpedoes) but with
 * no blaster mount and a colorized target footer. This rewrite also eliminates
 * the old inline bug: `gui/cruiser/tactical.html` used to do
 * `var t = document.getElementById('torpedo-controls')`, shadowing the imported
 * String Table `t()` — so `t('console.common.locked')` threw when a locked
 * target had no name. The shared renderer owns the footer now, using the real
 * `t`; nothing here shadows it.
 */
import { makeTacticalRender } from '../stations/tactical-console.js';

export const renderStation = makeTacticalRender({
  weaponsView: (s) => s,
  ids: {
    radar: 'tactical-radar',
    phasers: 'phasers-controls',
    torpedo: 'torpedo-controls',
    stationDamage: 'station-damage',
    autoBadge: 'tactical-auto-badge',
  },
  torpedoMaxDefault: 20,
  footer: { id: 'footer-target', colorize: true },
});
