// Issue #1170 — Keyboard-only smoke for the Tactical (Weapons) console.
//
// The tracer's end-to-end claim: a player with no pointing device can run the
// Destroyer's Tactical console entirely from the keyboard. This spec proves it
// against the real console page — Tab walks between the console's components,
// arrow keys move within a composite, and Enter/Space fire the principal
// actions — while a pointer-event guard asserts that not one mouse, pointer or
// touch event was used to do it.
//
// Shape mirrors weapons-console.spec.js: drive the standalone console page,
// stub window.__sendAction to capture the action envelopes the components emit,
// and push state through window.__updateConsole. The ONE rule this spec adds is
// that every interaction is a keyboard.* call — never click/tap/hover/focus.
//
// The Destroyer's tactical.html reads its panels out of `systems['tactical-radar']`
// (unlike the battleship page, which reads top-level), so the fixture state is
// nested under that fine system id.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/destroyer/tactical.html';

// Everything the panels read comes from the first present fine-system object;
// `tactical-radar` is that object for the Destroyer.
const STATE = {
  system_ids: ['tactical-radar'],
  system_families: { 'tactical-radar': 'tactical' },
  systems: {
    'tactical-radar': {
      blips: [
        { uuid: 'raider-1', radar_x: 0.15, radar_y: 0.1, kind: 'ship', label: 'Raider', scaled_radius: 0.03 },
      ],
      ship_x: 0, ship_z: 0, ship_speed: 0, ship_heading: 0,
      banks: [{ id: 'omni', label: 'Omni', fire_ready: true, on_cooldown: false }],
      blasters: [{ id: 'port', label: 'Port', fire_ready: true, on_cooldown: false }],
      tubes: [{ id: 'fore', label: 'Fore', loaded_count: 1, target_count: 1, volley_max: 1, load_progress: 0 }],
      torpedo_count: 4, torpedo_max: 8,
      // A server-provided lock, so the weapons render enabled and the FIRE keys
      // have something to fire at; the radar's own keyboard lock is exercised
      // separately below and does not depend on it.
      target_uuid: 'raider-1',
      phaser_mode: 'Manual',
    },
  },
};

/** The active element's identity, reaching through the shadow boundary. */
async function activeId(page) {
  return page.evaluate(() => {
    const el = document.activeElement;
    if (!el) return null;
    return el.id ? '#' + el.id : el.tagName;
  });
}

test('Tactical console: every principal action fires from the keyboard, with no pointer', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto(CONSOLE_URL);

  // Capture the outbound action envelopes, and prove no pointer path was taken:
  // a keyboard-activated button emits a synthetic `click`, but never a
  // mousedown/pointerdown/touchstart — so those are the honest "a pointer was
  // used" signal to count.
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(JSON.parse(json));
    window.__pointerEvents = 0;
    for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'touchstart', 'pointermove']) {
      window.addEventListener(type, () => { window.__pointerEvents += 1; }, { capture: true });
    }
  });

  await page.evaluate((s) => window.__updateConsole('tactical', JSON.stringify(s)), STATE);

  // ── Tab walks between the console's components (AC #2) ──────────────────────
  // Roving tabindex leaves each composite a single Tab stop, so the sequence is
  // one stop per component: the Intel toggle, the radar scope, then the three
  // weapon toolbars.
  await page.keyboard.press('Tab');
  expect(await activeId(page)).toBe('#intel-toggle');

  await page.keyboard.press('Tab');
  expect(await activeId(page)).toBe('#tactical-radar');

  // ── Arrow keys move within the radar composite; Enter locks a target ────────
  await page.keyboard.press('ArrowDown');            // cursor → first contact
  await page.keyboard.press('Enter');                // set_target raider-1
  await expect.poll(() => page.evaluate(() => window.__sent.some((a) => a.action === 'set_target')))
    .toBe(true);

  // ── Phasers toolbar: arrow to a FIRE button, Enter fires it ─────────────────
  await page.keyboard.press('Tab');
  expect(await activeId(page)).toBe('#phasers-controls');
  await page.keyboard.press('ArrowDown');            // mode toggle → FIRE button
  await page.keyboard.press('Enter');                // fire_phaser

  // ── Blasters toolbar: Space is the hold-to-fire the pointer does ────────────
  await page.keyboard.press('Tab');
  expect(await activeId(page)).toBe('#blasters-controls');
  await page.keyboard.press('Space');                // charge_blaster_start + fire_blaster

  // ── Torpedoes toolbar: End jumps to the tube's FIRE button, Enter fires ─────
  await page.keyboard.press('Tab');
  expect(await activeId(page)).toBe('#torpedo-controls');
  await page.keyboard.press('End');                  // − → FIRE (last control in the tube)
  await page.keyboard.press('Enter');                // fire_torpedo

  // ── Every principal action arrived, from the keyboard alone ─────────────────
  const actions = await page.evaluate(() => window.__sent.map((a) => a.action));
  expect(actions).toContain('set_target');
  expect(actions).toContain('fire_phaser');
  expect(actions).toContain('charge_blaster_start');
  expect(actions).toContain('fire_blaster');
  expect(actions).toContain('fire_torpedo');

  // The envelopes carry the right named-action payloads (same as touch).
  expect(await page.evaluate(() => window.__sent.find((a) => a.action === 'fire_phaser')))
    .toMatchObject({ action: 'fire_phaser', console: 'tactical', bank: 'omni' });
  expect(await page.evaluate(() => window.__sent.find((a) => a.action === 'set_target')))
    .toMatchObject({ action: 'set_target', console: 'tactical', uuid: 'raider-1' });

  // Not one pointer event was used to get here.
  expect(await page.evaluate(() => window.__pointerEvents)).toBe(0);
});
