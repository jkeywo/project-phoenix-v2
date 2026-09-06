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
import { SENSORS_SCAN_ACTION } from '../../gui/stations/sensors-actions.js';

function keyboardChord(binding) {
  const modifiers = [
    binding.ctrlKey && 'Control',
    binding.shiftKey && 'Shift',
    binding.altKey && 'Alt',
    binding.metaKey && 'Meta',
  ].filter(Boolean);
  return [...modifiers, binding.code].join('+');
}

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
  const commsPayload = JSON.stringify({
    contacts: [],
    messages: [
      {
        id: 'hail-1', sender_name: 'RELAY STATION', body: 'Do you copy?', is_read: true,
        responses: [{ text: 'Acknowledge', available: true, important: false }],
      },
      {
        id: 'hail-2', sender_name: 'OUTER RELAY', body: 'Routine traffic follows.', is_read: false,
        responses: [],
      },
    ],
  });
  await page.evaluate((json) => window.__updateConsole('comms', json), commsPayload);

  // ── Tab reaches the hail list — it is one Tab stop (AC #1) ──────────────────
  expect(await tabTo(page, '#comms-hail-list')).toBe(true);

  // The unread hail is the automatic thread, and since issue #1380 the inbox
  // is SORTED — unread first — so it is also the first row and the one focus
  // starts on. Arrow down to the read hail and Enter on it: that proves the
  // shared local selection actually repaints both panels, from the keyboard,
  // with no unconsumed host command leaving the console.
  await expect(page.locator('ph-comms-current-message #sender-label')).toHaveText('OUTER RELAY');
  await expect(page.locator('ph-comms-hail-list .row').first())
    .toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(page.locator('ph-comms-hail-list .row').nth(1))
    .toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('ph-comms-current-message #sender-label')).toHaveText('RELAY STATION');
  expect(await page.evaluate(
    () => window.__sent.some((a) => a.action === 'select_comms_message')
  )).toBe(false);
  expect(await page.evaluate(() => window.__sent)).toEqual([]);

  // ── A repaint must not evict the operator from the list ─────────────────────
  // The shell pushes console state ~10x a second. If a render re-inserted rows
  // whose order had not changed, that remove+insert would blur the focused row
  // between keystrokes and no one could arrow or Enter at all. Push the SAME
  // payload again; the same row must still hold focus. (Only a real browser
  // runs the focus-fixup rule this depends on, which is why the check lives
  // here and not in the jsdom suite.)
  const focusedRowId = () => page.evaluate(() => {
    const host = document.getElementById('comms-hail-list');
    const el = host && host.shadowRoot ? host.shadowRoot.activeElement : null;
    return el ? (el.dataset.id || null) : null;
  });
  const heldRow = await focusedRowId();
  expect(heldRow).toBeTruthy();
  await page.evaluate((json) => window.__updateConsole('comms', json), commsPayload);
  expect(await focusedRowId()).toBe(heldRow);

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
    system_ids: ['captain', 'sensors'],
    system_families: { captain: 'captain', sensors: 'sensors' },
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
  expect(scan).toMatchObject({
    action: 'scan_target',
    console: 'captain',
    uuid: 'contact-1',
    semantic_action: 'sensors.scan',
  });
  expect(scan.correlation).toMatch(/^[\x21-\x7e]{1,64}$/);
  expect(Number.isFinite(scan.__input_ms)).toBe(true);

  // Shared feedback is correlation-specific and never substitutes for the
  // authoritative scan projection. First prove a refusal, then invoke the
  // default semantic binding itself and prove a later accepted occurrence.
  const scanStatus = page.locator(
    '.semantic-action-feedback__item[data-action-id="sensors.scan"]',
  );
  await expect(scanStatus).toHaveAttribute('data-state', 'Pending');
  await page.evaluate((correlation) => window.__updateActionFeedback({
    correlation, state: 'Refused',
  }), scan.correlation);
  await expect(scanStatus).toHaveAttribute('data-state', 'Refused');
  await expect(page.locator('#scan-readout').locator('#reason')).toBeHidden();

  const scanBinding = SENSORS_SCAN_ACTION.bindings[0];
  expect(scanBinding).toMatchObject({
    type: 'keyboard',
    code: 'KeyN',
    ctrlKey: false,
    shiftKey: true,
    altKey: false,
    metaKey: false,
  });
  await page.keyboard.press(keyboardChord(scanBinding));
  await expect.poll(() => page.evaluate(
    () => window.__sent.filter((a) => a.action === 'scan_target').length,
  )).toBe(2);
  const accepted = await page.evaluate(
    () => window.__sent.filter((a) => a.action === 'scan_target').at(-1),
  );
  expect(accepted.correlation).not.toBe(scan.correlation);
  await expect(scanStatus).toHaveAttribute('data-state', 'Pending');
  await page.evaluate((correlation) => window.__updateActionFeedback({
    correlation, state: 'Applied',
  }), accepted.correlation);
  await expect(scanStatus).toHaveAttribute('data-state', 'Applied');
  await expect(page.locator('#scan-readout').locator('#reason')).toBeHidden();

  // Not one pointer event was used to get here.
  expect(await page.evaluate(() => window.__pointerEvents)).toBe(0);
});
