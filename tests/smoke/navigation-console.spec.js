import { test, expect } from './fixtures';
import { ts } from './strings';

const CONSOLE_URL = '/gui/battleship/navigation.html';

test('navigation console: tapping the map alone never sends a waypoint action', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.locator('ph-navigation-map').locator('canvas').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(0);
});

test('navigation console: Set Waypoint pick mode places a free waypoint on tap', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
    // #1287 routes the placement through the shared semantic action, whose
    // handler resolves its view from console state and refuses outright when
    // there is none — so a spec that never pushed a payload silently sends
    // nothing. The sibling clear-waypoint test has always done this.
    window.__updateConsole('navigation', JSON.stringify({
      blips: [],
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await page.locator('ph-navigation-map').locator('#btn-set-waypoint').click();
  await page.locator('ph-navigation-map').locator('canvas').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_navigation_waypoint');
  expect(parsed.console).toBe('navigation');
  expect(typeof parsed.x).toBe('number');
  expect(typeof parsed.z).toBe('number');
  expect(parsed.source_uuid).toBeUndefined();
});

test('navigation console: clear waypoint sends clear_navigation_waypoint', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
    window.__updateConsole('navigation', JSON.stringify({
      blips: [],
      waypoint: { x: 100, z: 200 },
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await page.locator('ph-navigation-map').locator('#btn-clear-waypoint').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('clear_navigation_waypoint');
  expect(parsed.console).toBe('navigation');
});

test('navigation console: selected entity name stays NONE until the operator taps a target', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__updateConsole('navigation', JSON.stringify({
      blips: [
        { uuid: 'station-alpha', name: 'Alpha Station', kind: 'station', stance: 'friendly', radar_x: 0.2, radar_y: -0.1, world_x: 200, world_z: -100, selectable: true },
      ],
      waypoint: null,
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await expect(page.locator('#ent-name')).toHaveText(ts('console.navigation.none'));
});

test('navigation console: tapping a visible entity selects it and Set as Waypoint anchors it', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
    window.__updateConsole('navigation', JSON.stringify({
      blips: [
        { uuid: 'station-alpha', name: 'Alpha Station', kind: 'station', stance: 'friendly', radar_x: 0.0, radar_y: 0.0, world_x: 0, world_z: 0, selectable: true },
      ],
      waypoint: null,
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await page.locator('ph-navigation-map').locator('canvas').click();
  // Selecting alone does not set the waypoint.
  await expect(page.locator('#ent-name')).toHaveText('Alpha Station');
  let sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(0);

  await page.locator('ph-navigation-map').locator('#btn-set-selected').click();
  sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  const parsed = JSON.parse(sent[0]);
  expect(parsed.action).toBe('set_navigation_waypoint');
  expect(parsed.console).toBe('navigation');
  expect(parsed.source_uuid).toBe('station-alpha');
});

test('navigation console: waypoint state renders the waypoint label in the side panel', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__updateConsole('navigation', JSON.stringify({
      blips: [
        { uuid: 'station-bravo', name: 'Bravo Station', kind: 'station', stance: 'friendly', radar_x: 0.4, radar_y: -0.3, world_x: 300, world_z: -200, selectable: true },
      ],
      waypoint: { x: 300, z: -200, name: 'Bravo Station' },
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });
  await expect(page.locator('#waypoint-name')).toHaveText('Bravo Station');
});

// ── Phone TRAFFIC | MISSION segment (issue #1379) ───────────────────────────

test('navigation console: the phone segment switches TRAFFIC and MISSION and keeps one tab stop', { tag: '@core' }, async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(CONSOLE_URL);

  const trafficTab = page.locator('#nav-seg-traffic');
  const missionTab = page.locator('#nav-seg-mission');
  const trafficPanel = page.locator('#nav-panel-traffic');
  const missionPanel = page.locator('#nav-panel-mission');

  // Default: TRAFFIC selected and visible, MISSION hidden and out of the tab order.
  await expect(trafficTab).toHaveAttribute('aria-selected', 'true');
  await expect(missionTab).toHaveAttribute('aria-selected', 'false');
  await expect(trafficPanel).toBeVisible();
  await expect(missionPanel).toBeHidden();
  expect(await missionTab.getAttribute('tabindex')).toBe('-1');

  await missionTab.click();

  await expect(missionTab).toHaveAttribute('aria-selected', 'true');
  await expect(trafficTab).toHaveAttribute('aria-selected', 'false');
  await expect(missionPanel).toBeVisible();
  await expect(trafficPanel).toBeHidden();
  expect(await trafficTab.getAttribute('tabindex')).toBe('-1');
  expect(await missionTab.getAttribute('tabindex')).toBe('0');

  // Arrow-key roving moves the single tab stop AND re-selects (issue #1379's
  // automatic activation — same as the Station Bar's own tabs).
  await missionTab.focus();
  await page.keyboard.press('ArrowLeft');
  await expect(trafficTab).toBeFocused();
  await expect(trafficTab).toHaveAttribute('aria-selected', 'true');
  await expect(trafficPanel).toBeVisible();
  await expect(missionPanel).toBeHidden();
});

test('navigation console: TRAFFIC and MISSION both stay on screen at desktop width', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(CONSOLE_URL);
  await expect(page.locator('#nav-seg')).toBeHidden();
  await expect(page.locator('#nav-panel-traffic')).toBeVisible();
  await expect(page.locator('#nav-panel-mission')).toBeVisible();
});

// ── Waypoint bar wrap at phone width (issue #1379 review finding 2) ────────
// AC #3's first half ("At 390px the waypoint bar wraps so Set / Set as /
// Clear stay reachable") had no automated regression coverage: nothing pushed
// a waypoint + a differing selection at 390px and checked the bar itself
// (`ph-navigation-map`'s shadow-DOM `.wp-bar`, `flex-wrap: wrap`) or the
// `--nav-chart-corner-clear` reservation that keeps it clear of ON SCREEN.
test('navigation console: at 390px the waypoint bar wraps Set as Waypoint and Clear Waypoint onto separate rows clear of the chart corner', { tag: '@core' }, async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__updateConsole('navigation', JSON.stringify({
      // A selectable blip at the ship's own position (ship_heading 0) so a
      // centre-canvas tap selects it — same trick the existing "tapping a
      // visible entity selects it" test above uses.
      blips: [
        { uuid: 'station-alpha', name: 'Alpha Station', kind: 'station', stance: 'friendly', radar_x: 0.0, radar_y: 0.0, world_x: 0, world_z: 0, selectable: true },
      ],
      // An existing waypoint (different from the blip above) so Clear
      // Waypoint is already showing before the selection joins it.
      waypoint: { x: 4000, z: -4000, name: 'Existing Waypoint' },
      ship_x: 0,
      ship_z: 0,
      ship_heading: 0,
      ship_speed: 0,
      radar_range: 5000,
    }));
  });

  // Select the blip: both Set as Waypoint and Clear Waypoint now show at once
  // (Set Waypoint stays hidden — a waypoint is already set).
  await page.locator('ph-navigation-map').locator('canvas').click();

  const setSelected = page.locator('ph-navigation-map').locator('#btn-set-selected');
  const clearWaypoint = page.locator('ph-navigation-map').locator('#btn-clear-waypoint');
  await expect(setSelected).toBeVisible();
  await expect(clearWaypoint).toBeVisible();

  const [selectedBox, clearBox, cornerBox] = await Promise.all([
    setSelected.boundingBox(),
    clearWaypoint.boundingBox(),
    page.locator('.chart-corner').boundingBox(),
  ]);

  function overlaps(a, b) {
    return a.x < b.x + b.width && a.x + a.width > b.x
      && a.y < b.y + b.height && a.y + a.height > b.y;
  }

  // The bar wraps rather than overlapping itself: the two buttons land on
  // separate rows (a shared row would put their tops within a few px of
  // each other; wrapped rows are a full button-height plus row-gap apart).
  expect(Math.abs(selectedBox.y - clearBox.y)).toBeGreaterThan(clearBox.height / 2);

  // Neither wrapped row overlaps the ON SCREEN corner's reserved footprint.
  expect(overlaps(selectedBox, cornerBox)).toBe(false);
  expect(overlaps(clearBox, cornerBox)).toBe(false);
});

// ── AUTO badge growing the chart corner (issue #1379 review finding 1) ─────
// Navigation runs `human_seeking: true` on every hull, so an unmanned seat
// handing to AI mid-shift — `setAutoState` flipping the AUTO badge's
// `hidden` attribute — is the mainline trigger, not an edge case. At 375px
// (the reviewer's own repro width) a single Clear Waypoint button sits close
// enough to the ON SCREEN corner that the AUTO badge appearing renders it
// flush against the button with zero gap. A stale `--nav-chart-corner-clear`
// was one half of that (fixed by the ResizeObserver above); the other half
// is structural: `.wp-bar` is `justify-content: flex-end`, so a *lone*
// button that doesn't fit in the reserved space simply overflows past `left`
// rather than moving anywhere — CSS never wraps a single flex item onto a
// line of its own. `.wp-bar-spacer` (ph-navigation-map.js) is what actually
// forces the lone button onto a second row, clear of the corner.
test('navigation console: the AUTO badge appearing keeps the waypoint bar clear of the chart corner', { tag: '@core' }, async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto(CONSOLE_URL);

  function pushState(navigationAuto) {
    return page.evaluate((isAuto) => {
      window.__updateConsole('navigation', JSON.stringify({
        blips: [],
        waypoint: { x: 4000, z: -4000, name: 'Existing Waypoint' },
        navigation_auto: isAuto,
        ship_x: 0,
        ship_z: 0,
        ship_heading: 0,
        ship_speed: 0,
        radar_range: 5000,
      }));
    }, navigationAuto);
  }

  await pushState(false);
  const badge = page.locator('#navigation-auto-badge');
  const clearWaypoint = page.locator('ph-navigation-map').locator('#btn-clear-waypoint');
  await expect(badge).toBeHidden();
  await expect(clearWaypoint).toBeVisible();

  // The seat goes to AI without a page reload — the same render pipeline just
  // pushes navigation_auto: true on the next state update.
  await pushState(true);
  await expect(badge).toBeVisible();

  function overlaps(a, b) {
    return a.x < b.x + b.width && a.x + a.width > b.x
      && a.y < b.y + b.height && a.y + a.height > b.y;
  }

  // The badge's hidden -> visible toggle grows `.chart-corner`'s own
  // rendered box; Clear Waypoint must land clear of it — on a row below, not
  // flush against it on the same row (a load-time/resize/font-load-only
  // recompute, or a `left`-only reservation with no spacer, both leave this
  // failing — see the fix comments above the IIFE in
  // gui/battleship/navigation.html and above `.wp-bar-spacer` in
  // gui/components/ph-navigation-map.js).
  await expect(async () => {
    const [badgeBox, clearBox, cornerBox] = await Promise.all([
      badge.boundingBox(),
      clearWaypoint.boundingBox(),
      page.locator('.chart-corner').boundingBox(),
    ]);
    expect(overlaps(clearBox, cornerBox)).toBe(false);
    expect(overlaps(clearBox, badgeBox)).toBe(false);
  }).toPass();
});
