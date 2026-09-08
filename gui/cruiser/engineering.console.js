/**
 * gui/cruiser/engineering.console.js — the cruiser's Engineering seat
 * (issue #1235).
 *
 * Power + Repair, no Shields column (this hull hangs Shields off Science),
 * plus a bespoke tail of two Operations panels since #1390: the Tractor beam
 * and the Transfer umbilical this hull's engineering-owned `tractor` and
 * `umbilical` Systems publish for. Both hide themselves entirely when the
 * payload carries no matching system view, so a flight that never couples sees
 * the console it always had.
 *
 * The two panel bodies are `renderTractorPanel` / `renderUmbilicalPanel` in
 * gui/stations/engineering-console.js, SHARED with the destroyer rather than
 * copied here — one control reading one authoritative view.
 *
 * NO Field Repair dispatch panel: that reads `[repair.external_dispatch]`, and
 * this hull authors none.
 *
 * Each panel's one button toggles between two admitted commands (engage/
 * release, start/stop) — human and AI both issue the SAME command through the
 * one admission path, this control just picks which. The click handlers live in
 * `engineering.html` (they call `activateEngineeringAction`, which only exists
 * after `initConsole` runs) and read the "which one" off the button's own
 * `.engaged` class rather than a variable threaded back out of `render` — the
 * class is set on every render, so it is never stale.
 */
import {
  makeEngineeringRender,
  renderTractorPanel,
  renderUmbilicalPanel,
} from '../stations/engineering-console.js';

export const renderStation = makeEngineeringRender({
  ids: {
    power: 'power-controls',
    battery: 'battery-bar',
    hullIntegrity: 'hull-integrity',
    coreDamage: 'core-damage',
    repairTeams: 'repair-teams',
    autoBadge: 'engineering-auto-badge',
  },
  tail: (s, views, doc, t) => {
    renderTractorPanel(s, doc, t);
    renderUmbilicalPanel(s, doc, t);
  },
});
