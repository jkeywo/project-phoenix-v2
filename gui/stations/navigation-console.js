/**
 * gui/stations/navigation-console.js — the Navigation renderer (issue #1235,
 * T4.C3 chunk 2 of the console-seam programme).
 *
 * The battleship's Navigation seat is currently the only surviving
 * `navigation.html` — navigation became a hero-bar station on every other
 * hull (issues #1097/#1098), so this shared renderer has exactly one
 * variant today. It follows the same `make<Kind>Render(variant)` shape as
 * its siblings so a future hull that re-adds a dedicated Navigation console
 * slots in without a second inline `render`.
 *
 * Navigation is a FLAT single-family payload — `initConsole` is called with
 * as an authoritative flat Navigation-family payload — so `renderStation`
 * reads `s`'s fields directly.
 *
 * @typedef {object} NavigationVariant
 * @property {object} ids                     element ids present in this hull's markup
 * @property {string} ids.map                  `ph-navigation-map` id
 * @property {string} [ids.objectiveList]       `ph-objective-list` id
 * @property {string} [ids.civilianTraffic]     `ph-civilian-traffic` id
 * @property {string} [ids.stationDamage]       `ph-station-damage` id
 * @property {string} [ids.contactCount]        contact-count readout id
 * @property {string} [ids.waypointName]        waypoint-name readout id
 * @property {string} [ids.footer]              footer target-text id
 * @property {string} [ids.onScreenBtn]         the On-Screen button id (also the AUTO
 *   disable/readonly target — see `setAutoState`)
 * @property {string} [ids.autoBadge]           the AUTO badge id
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';

/**
 * Build a Navigation `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {NavigationVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeNavigationRender(variant) {
  const ids = variant.ids || {};

  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    // ── Chart ────────────────────────────────────────────────────────────
    // `regions` carries the area hulls (nebulae, belts, objective zones);
    // the objective rings ride the `objective_target` flag already stamped
    // on both the regions and the blips.
    if (ids.map) {
      const el = doc.getElementById(ids.map);
      if (el) {
        el.state = {
          blips: s.blips || [], regions: s.regions || [], range: s.radar_range || 5000,
          ship_pos: { x: s.ship_x || 0, z: s.ship_z || 0 }, ship_heading: s.ship_heading || 0,
          waypoint: s.waypoint || null,
        };
      }
    }
    if (ids.objectiveList) {
      const el = doc.getElementById(ids.objectiveList);
      if (el) el.state = { objectives: s.objectives || [] };
    }
    // Civilian traffic (issue #1028): who is on which lane, and who is not
    // doing as asked. Server-derived; the panel never infers it from the
    // chart, because a craft that has not turned yet and one that has
    // refused look identical on a map.
    if (ids.civilianTraffic) {
      const el = doc.getElementById(ids.civilianTraffic);
      if (el) el.state = { civilians: s.civilians || [] };
    }
    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }
    if (ids.contactCount) {
      const el = doc.getElementById(ids.contactCount);
      if (el) el.textContent = String((s.blips || []).length);
    }

    const wpName = s.waypoint && s.waypoint.name ? s.waypoint.name : (s.waypoint ? t('console.common.waypoint') : t('console.navigation.not_set'));
    if (ids.waypointName) {
      const el = doc.getElementById(ids.waypointName);
      if (el) el.textContent = wpName;
    }
    if (ids.footer) {
      const el = doc.getElementById(ids.footer);
      if (el) el.textContent = wpName;
    }

    if (ids.onScreenBtn || ids.autoBadge) {
      setAutoState(
        ids.onScreenBtn ? doc.getElementById(ids.onScreenBtn) : null,
        ids.autoBadge ? doc.getElementById(ids.autoBadge) : null,
        !!s.navigation_auto,
      );
    }

    if (variant.tail) variant.tail(s, doc, t);
  };
}
