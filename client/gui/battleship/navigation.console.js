/**
 * gui/battleship/navigation.console.js — the battleship's Navigation seat
 * (issue #1235).
 *
 * The only surviving Navigation seat (issues #1097/#1098 made navigation a
 * hero-bar station on every other hull) — no bespoke tail.
 */
import { makeNavigationRender } from '../stations/navigation-console.js';

export const renderStation = makeNavigationRender({
  ids: {
    map: 'navigation-map',
    objectiveList: 'objective-list',
    civilianTraffic: 'civilian-traffic',
    stationDamage: 'station-damage',
    contactCount: 'nav-contact-count',
    waypointName: 'waypoint-name',
    footer: 'footer-target',
    onScreenBtn: 'btn-on-screen',
    autoBadge: 'navigation-auto-badge',
  },
});
