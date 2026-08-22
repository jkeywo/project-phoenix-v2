/**
 * gui/battleship/tactical.console.js — the Harrow battleship's Tactical seat
 * (issue #1234). The reference hull: a single-family flat `tactical` payload,
 * the full weapons kit, and the target footer. No bespoke tail.
 *
 * The `.html` imports `renderStation` and hands it to `initConsole`; a vitest
 * suite imports the same `renderStation` to assert the radar contract.
 */
import { makeTacticalRender } from '../stations/tactical-console.js';

export const renderStation = makeTacticalRender({
  // Flat `tactical` family: the panels read fields straight off the payload.
  weaponsView: (s) => s,
  ids: {
    radar: 'tactical-radar',
    phasers: 'phasers-controls',
    blasters: 'blasters-controls',
    torpedo: 'torpedo-controls',
    stationDamage: 'station-damage',
    autoBadge: 'tactical-auto-badge',
  },
  // The battleship mounts no blaster bank, so the panel stays hidden for it; a
  // reused hull whose Tactical seat owns blasters gets its real state from the
  // same flat payload (issue #925).
  blastersHideWhenEmpty: true,
  torpedoMaxDefault: 20,
  footer: { id: 'footer-target' },
});
