// Issue #1178 — Keyboard-only smoke for the comms, ops and sensors surfaces.
//
// The sweep's end-to-end claim (AC #3): a player with no pointing device can
// answer a comms hail and run a sensor scan. Two consoles carry the proof —
// the Battleship's Comms console (select a hail from the converted list, then
// answer it) and the Destroyer's Captain console (boost an objective, then take
// a science scan) — each driven entirely from the keyboard, each with a
// pointer-event guard asserting not one mouse/pointer/touch event was used.
//
// Shape mirrors helm-nav-keyboard.spec.js: drive the standalone console page,
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
async function tabTo(page, wantId, max = 40) {
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

test('Comms console: a hail is selected and answered from the keyboard, with no pointer', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/gui/battleship/comms.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function'
    && !!customElements.get('ph-comms-hail-list')
    && !!customElements.get('ph-comms-current-message'));
  await instrument(page);

  // Flat `comms`-family payload — fields at the top level, as
  // buildCommsConsoleState emits them (not nested under a `comms` key).
  await page.evaluate(() => window.__updateConsole('comms', JSON.stringify({
    contacts: [],
    messages: [
      {
        id: 'hail-1', sender_name: 'RELAY STATION', body: 'Do you copy?', is_read: false,
        responses: [{ text: 'Acknowledge', available: true, important: false }],
      },
    ],
  })));

  // ── Tab reaches the hail list — it is one Tab stop (AC #1) ──────────────────
  expect(await tabTo(page, '#comms-hail-list')).toBe(true);

  // Enter opens the focused hail: the SAME select_comms_message a tap sends.
  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(
    () => window.__sent.some((a) => a.action === 'select_comms_message' && a.message_id === 'hail-1')
  )).toBe(true);

  // ── Tab on to the open thread and answer it from the keyboard ───────────────
  expect(await tabTo(page, '#comms-current-message')).toBe(true);
  await page.keyboard.press('Enter');   // the response button: respond_to_message

  await expect.poll(() => page.evaluate(
    () => window.__sent.some((a) => a.action === 'respond_to_message' && a.message_id === 'hail-1')
  )).toBe(true);

  const answer = await page.evaluate(() => window.__sent.find((a) => a.action === 'respond_to_message'));
  expect(answer).toMatchObject({ action: 'respond_to_message', console: 'comms', message_id: 'hail-1', response_index: 0 });

  // Not one pointer event was used to get here.
  expect(await page.evaluate(() => window.__pointerEvents)).toBe(0);
});

test('Captain console: an objective is boosted and a scan taken from the keyboard, with no pointer', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/gui/destroyer/captain.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function'
    && !!customElements.get('ph-objective-list')
    && !!customElements.get('ph-scan-readout'));
  await instrument(page);

  await page.evaluate(() => window.__updateConsole('captain', JSON.stringify({
    systems: {
      captain: {
        objectives: [
          { id: 'obj-1', text: 'Hold the line', done: false },
          { id: 'obj-2', text: 'Escort the convoy', done: false },
        ],
        boosted_objective_id: null,
        camera_views: [], operations: {}, deadlines: [],
      },
      sensors: {
        scan: { capable: true }, target_uuid: 'contact-1', target_name: 'Contact One',
        blips: [], regions: [],
      },
    },
    own_hull: null,
  })));

  // ── Ops: Tab to the objective list, rove to the second, boost it ────────────
  expect(await tabTo(page, '#objective-list')).toBe(true);
  await page.keyboard.press('ArrowDown');   // move to obj-2
  await page.keyboard.press('Enter');        // set_objective_priority, obj-2

  await expect.poll(() => page.evaluate(
    () => window.__sent.some((a) => a.action === 'set_objective_priority' && a.id === 'obj-2')
  )).toBe(true);

  // ── Sensors: Tab to the scan readout, take the scan of the sensor target ─────
  expect(await tabTo(page, '#scan-readout')).toBe(true);
  await page.keyboard.press('Enter');        // scan_target, contact-1

  await expect.poll(() => page.evaluate(
    () => window.__sent.some((a) => a.action === 'scan_target' && a.uuid === 'contact-1')
  )).toBe(true);

  const scan = await page.evaluate(() => window.__sent.find((a) => a.action === 'scan_target'));
  expect(scan).toMatchObject({ action: 'scan_target', console: 'captain', uuid: 'contact-1' });

  // Not one pointer event was used to get here.
  expect(await page.evaluate(() => window.__pointerEvents)).toBe(0);
});
