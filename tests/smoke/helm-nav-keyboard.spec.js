// Issue #1176 — Keyboard-only smoke for the helm family and navigation composites.
//
// The sweep's end-to-end claim: a player with no pointing device can fly the
// ship and work the chart. Two consoles carry the proof — a helm course change
// on the Destroyer's Helm console, and a contact-select-and-waypoint on the
// Battleship's Navigation console — each driven entirely from the keyboard,
// each with a pointer-event guard asserting not one mouse/pointer/touch event
// was used.
//
// Shape mirrors tactical-keyboard.spec.js: drive the standalone console page,
// stub window.__sendAction to capture the action envelopes, push state through
// window.__updateConsole, and interact with keyboard.* only — never
// click/tap/hover/mouse.

import { test, expect } from './fixtures';

/** The active element's identity, reaching through the shadow boundary. */
async function activeId(page) {
  return page.evaluate(() => {
    const el = document.activeElement;
    if (!el) return null;
    return el.id ? '#' + el.id : el.tagName;
  });
}

/** Tab until the focused element is `wantId`, up to `max` presses. */
async function tabTo(page, wantId, max = 16) {
  for (let i = 0; i < max; i += 1) {
    if (await activeId(page) === wantId) return true;
    await page.keyboard.press('Tab');
  }
  return await activeId(page) === wantId;
}

/** Install the sent-action capture and the pointer-event guard. */
async function instrument(page) {
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(JSON.parse(json));
    window.__pointerEvents = 0;
    for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'touchstart', 'pointermove']) {
      window.addEventListener(type, () => { window.__pointerEvents += 1; }, { capture: true });
    }
  });
}

test('Helm console: a course change fires from the keyboard, with no pointer', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/gui/destroyer/helm.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function'
    && !!customElements.get('ph-helm-joystick'));
  await instrument(page);

  // helm_auto:false so the stick answers to the operator, not the autopilot.
  await page.evaluate(() => window.__updateConsole('helm', JSON.stringify({
    blips: [], range: 500, x: 0, z: 0, ship_heading: 0, speed: 0,
    helm_auto: false, lateral_auto: false,
    impulse_charge_progress: 0, boost_enabled: true, boost_active: false, boost_battery: 1,
  })));

  // ── Tab reaches the joystick — it is a real Tab stop (AC #1) ────────────────
  expect(await tabTo(page, '#helm-joystick')).toBe(true);

  // ── ArrowUp flies forward: the SAME set_helm the pointer drag emits ─────────
  await page.keyboard.down('ArrowUp');
  await expect.poll(() => page.evaluate(
    () => window.__sent.some((a) => a.action === 'set_helm' && a.thrust > 0)
  )).toBe(true);
  await page.keyboard.up('ArrowUp');

  const helm = await page.evaluate(() => window.__sent.find((a) => a.action === 'set_helm' && a.thrust > 0));
  expect(helm).toMatchObject({ action: 'set_helm', console: 'helm' });
  expect(helm.thrust).toBeGreaterThan(0);

  // Not one pointer event was used to get here.
  expect(await page.evaluate(() => window.__pointerEvents)).toBe(0);
});

test('Navigation console: a map contact is selected and made a waypoint from the keyboard, with no pointer', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/gui/battleship/navigation.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function'
    && !!customElements.get('ph-navigation-map'));
  await instrument(page);

  await page.evaluate(() => window.__updateConsole('navigation', JSON.stringify({
    blips: [
      { uuid: 'station-alpha', name: 'Alpha Station', kind: 'station', stance: 'friendly',
        radar_x: 0.2, radar_y: -0.1, world_x: 200, world_z: -100, selectable: true },
    ],
    regions: [], waypoint: null,
    ship_x: 0, ship_z: 0, ship_heading: 0, ship_speed: 0, radar_range: 5000,
  })));

  // ── Tab reaches the chart — it is a real Tab stop (AC #1) ───────────────────
  expect(await tabTo(page, '#navigation-map')).toBe(true);

  // ── ArrowDown selects the contact; Enter commits it as the waypoint ─────────
  await page.keyboard.press('ArrowDown');   // select Alpha Station
  await page.keyboard.press('Enter');       // set_navigation_waypoint, anchored to it

  await expect.poll(() => page.evaluate(
    () => window.__sent.some((a) => a.action === 'set_navigation_waypoint')
  )).toBe(true);

  const wp = await page.evaluate(() => window.__sent.find((a) => a.action === 'set_navigation_waypoint'));
  expect(wp).toMatchObject({ action: 'set_navigation_waypoint', console: 'navigation', source_uuid: 'station-alpha' });

  // Not one pointer event was used to get here.
  expect(await page.evaluate(() => window.__pointerEvents)).toBe(0);
});
