/**
 * gui/mount-plan.js — Single home for the station-id → DOM-id naming scheme
 * and the console mount plan (issue #827).
 *
 * Replaces gui/console-registry.js: the manifest is the server-supplied
 * `ship_stations` (resolved to iframe URLs by gui/console-resolver.js), and
 * the section/iframe element ids follow the canonical `${id}-ui` /
 * `${id}-iframe` scheme with exactly one historical alias: the `tactical`
 * station keeps its `weapons-ui` / `weapons-iframe` element ids (the ids
 * predate the station rename and every downstream $() reference uses them).
 *
 * Consumers:
 *  - client.html mountConsoles()      — consumes planMounts()
 *  - client.html pushConsoleStateFor  — resolves iframe ids via iframeIdFor()
 *  - gui/content-switcher.js          — derives section visibility via sectionIdFor()
 */

import { resolveConsoleUrl } from './console-resolver.js';

/**
 * The one station whose DOM ids do not follow the `${id}-*` scheme.
 * tactical → weapons-ui / weapons-iframe.
 */
export const SECTION_ALIAS = Object.freeze({ tactical: 'weapons' });

/** DOM id base for a station id (applies the tactical → weapons alias). */
function domBase(stationId) {
  return SECTION_ALIAS[stationId] || stationId;
}

/** Section element id for a station id (e.g. 'helm' → 'helm-ui'). */
export function sectionIdFor(stationId) {
  if (!stationId) return null;
  return domBase(stationId) + '-ui';
}

/** Iframe element id for a station id (e.g. 'helm' → 'helm-iframe'). */
export function iframeIdFor(stationId) {
  if (!stationId) return null;
  return domBase(stationId) + '-iframe';
}

/**
 * Build the console mount plan for a ship.
 *
 * @param {{ stations?: Array<{ id?: string, name?: string, console?: string }> }|null} shipStations
 *        The server-supplied ship_stations from Welcome.
 * @returns {Array<{ stationId: string, sectionId: string, iframeId: string,
 *                   url: string, title: string }>}
 *        One entry per mountable station. Stations without an id or without a
 *        resolvable console URL are skipped (nothing to mount).
 */
export function planMounts(shipStations) {
  const stations = (shipStations && shipStations.stations) || [];
  const plan = [];
  for (const st of stations) {
    const stationId = st && st.id;
    if (!stationId) continue;
    const url = resolveConsoleUrl(shipStations, stationId);
    if (!url) continue;
    plan.push({
      stationId,
      sectionId: sectionIdFor(stationId),
      iframeId: iframeIdFor(stationId),
      url,
      title: st.name || stationId,
    });
  }
  return plan;
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.planMounts = planMounts;
  window.sectionIdFor = sectionIdFor;
  window.iframeIdFor = iframeIdFor;
}
