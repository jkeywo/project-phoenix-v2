import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

function source(path) {
  return readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');
}

describe('Navigation semantic-action structural coverage', () => {
  it('routes every direct chart/map/traffic control through the shared Navigation dispatcher', () => {
    const map = source('gui/components/ph-navigation-map.js');
    for (const id of [
      'NAVIGATION_CONTACT_ACTION_ID',
      'NAVIGATION_WAYPOINT_PLACE_ACTION_ID',
      'NAVIGATION_WAYPOINT_ANCHOR_ACTION_ID',
      'NAVIGATION_WAYPOINT_CLEAR_ACTION_ID',
      'NAVIGATION_PAN_LEFT_ACTION_ID',
      'NAVIGATION_PAN_RIGHT_ACTION_ID',
      'NAVIGATION_PAN_UP_ACTION_ID',
      'NAVIGATION_PAN_DOWN_ACTION_ID',
      'NAVIGATION_ZOOM_IN_ACTION_ID',
      'NAVIGATION_ZOOM_OUT_ACTION_ID',
    ]) {
      expect(map).toContain(id);
    }
    expect(map).toContain('activateNavigationAction');
    const mousePan = map.slice(map.indexOf('#boundMouseMove ='), map.indexOf('#boundMouseUp ='));
    const wheelZoom = map.slice(map.indexOf('#boundWheel ='), map.indexOf('#boundTouchStart ='));
    const touchGestures = map.slice(map.indexOf('#boundTouchMove ='), map.indexOf('#boundTouchEnd ='));
    expect(mousePan).toContain('#activatePan');
    expect(wheelZoom).toContain('#activateZoom');
    expect(touchGestures).toContain('#activatePan');
    expect(touchGestures).toContain('#activateZoom');
    // A submitted request is not an authoritative waypoint projection.
    expect(map).not.toContain("#showToast(t('console.navigation.waypoint_set'))");

    const traffic = source('gui/components/ph-civilian-traffic.js');
    expect(traffic).toContain('activateNavigationAction');
    expect(traffic).toContain('NAVIGATION_CIVILIAN_ORDER_ACTION_ID');

    const direct = source('gui/battleship/navigation.html');
    expect(direct).toContain('activateNavigationAction');
    expect(direct).toContain('NAVIGATION_CHART_ACTION_ID');
  });

  it('keeps all four shipped hull ownership variants attached to a Navigation-aware document', () => {
    const battleship = source('assets/entities/alliance_battleship.toml');
    const destroyer = source('assets/entities/alliance_destroyer.toml');
    const cruiser = source('assets/entities/alliance_cruiser.toml');
    const courier = source('assets/entities/alliance_courier.toml');

    expect(battleship).toMatch(/id = "navigation"[\s\S]*?console = "gui\/battleship\/navigation\.html"/);
    expect(destroyer).toMatch(/id = "navigation"[\s\S]*?console = "gui\/battleship\/navigation\.html"/);
    expect(cruiser).toMatch(/id = "navigation"[\s\S]*?console = "gui\/cruiser\/comms\.html"/);
    expect(courier).toMatch(/id = "navigation"[\s\S]*?station = "captain"/);
    expect(courier).toContain('console = "gui/courier/captain.html"');

    const direct = source('gui/battleship/navigation.html');
    expect(direct).toContain("initConsole({ name: 'navigation'");
    const cruiserComms = source('gui/cruiser/comms.html');
    expect(cruiserComms).toContain("actionFamilies: ['comms', 'navigation']");
    const courierCaptain = source('gui/courier/captain.html');
    expect(courierCaptain).toContain("actionFamilies: ['captain', 'navigation']");
  });

  it('includes Navigation in product discovery and preserves correlated wire envelopes', () => {
    const catalogue = source('gui/client-semantic-actions.js');
    expect(catalogue).toContain('NAVIGATION_ACTIONS');

    const actionMap = source('gui/action-map.js');
    for (const action of [
      'set_navigation_chart:',
      'set_navigation_waypoint:',
      'clear_navigation_waypoint:',
      'order_civilian:',
    ]) {
      const start = actionMap.indexOf(action);
      expect(start).toBeGreaterThanOrEqual(0);
      const fragment = actionMap.slice(start, start + 1800);
      expect(fragment).toContain("'ControlSystemCorrelated'");
    }
  });
});
