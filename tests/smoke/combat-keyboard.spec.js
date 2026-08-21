// Issue #1177 — Keyboard-only smoke for the combat family.
//
// The #1170 sweep's end-to-end claim for the combat consoles: a player with no
// pointing device can fire a weapon, adjust shields, order a repair and shift
// power entirely from the keyboard. The tracer's tactical-keyboard.spec proved
// the weapon toolbars; this proves the family's other conversions — the shield
// facing ring (arrow-cycled arc cursor), the power steppers (roving toolbar)
// and the repair dispatch — while a pointer-event guard on each page asserts
// that not one mouse/pointer/touch event was used.
//
// The combat family spans two console pages — weapons live on the Tactical
// console, shields/power/repair on Engineering — so the test drives both in
// turn, re-installing the capture + guard after each navigation (page globals
// reset on navigation) and asserting each page saw zero pointer events.
//
// Shape mirrors tactical-keyboard.spec.js: drive the standalone console page,
// stub window.__sendAction to capture the action envelopes, push state through
// window.__updateConsole, and interact only with keyboard.* calls.

import { test, expect } from './fixtures';

// The Destroyer's tactical.html reads its panels out of systems['tactical-radar'].
const TACTICAL_STATE = {
  systems: {
    'tactical-radar': {
      blips: [
        { uuid: 'raider-1', radar_x: 0.15, radar_y: 0.1, kind: 'ship', label: 'Raider', scaled_radius: 0.03 },
      ],
      ship_x: 0, ship_z: 0, ship_speed: 0, ship_heading: 0,
      banks: [{ id: 'omni', label: 'Omni', fire_ready: true, on_cooldown: false }],
      target_uuid: 'raider-1',
      phaser_mode: 'Manual',
    },
  },
};

// The Destroyer's engineering.html resolves shields/power/repair from these
// fine system ids (see its render()).
const ENGINEERING_STATE = {
  own_hull: null,
  systems: {
    'shields-system': {
      facings: [
        { arc_id: 'fore', id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'aft', id: 'aft', label: 'Aft', hp: 100, max_hp: 100, online: true },
      ],
      focused_facing: null,
      shields_auto: false,
    },
    'power-reactor': {
      // level 1 at the floor: − is disabled, so the roving toolbar parks its
      // single Tab stop on the enabled + stepper, which raises the group.
      consoles: [{ id: 'weapons', label: 'Weapons', level: 1, commanded_level: 1, min_level: 1, max_level: 4 }],
      power_auto: false,
      battery_charge: 80, battery_max: 100, battery_online: true, charging: false,
    },
    repair: {
      overall_hull: { pct: 0.9, destroyed_pct: 0 },
      core_systems: [],
      teams: [{ id: 1, status: 'idle', label: 'Alpha' }],
      dispatch_targets: [{ id: 'core', label: 'Core' }],
      damaged_systems: [],
      repair_auto: false,
    },
  },
};

/** Install the action capture + pointer-event guard on the current page. */
async function installCapture(page) {
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(JSON.parse(json));
    window.__pointerEvents = 0;
    for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'touchstart', 'pointermove']) {
      window.addEventListener(type, () => { window.__pointerEvents += 1; }, { capture: true });
    }
  });
}

/** The document-level active element's host tag (a shadow child reports its host). */
async function activeHostTag(page) {
  return page.evaluate(() => (document.activeElement ? document.activeElement.tagName : null));
}

/**
 * Press Tab (keyboard only) until the active element's host is `tag`, up to
 * `max` presses. Returns true once reached — robust to intervening Tab stops so
 * the test never pins an exact global tab order (which shifts as panels change).
 */
async function tabToHost(page, tag, max = 30) {
  for (let i = 0; i < max; i += 1) {
    await page.keyboard.press('Tab');
    if (await activeHostTag(page) === tag) return true;
  }
  return false;
}

test('Combat family: fire a weapon, adjust shields, order a repair — all from the keyboard, no pointer', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });

  // ── Weapon: the Tactical console (AC — fires a weapon) ───────────────────────
  await page.goto('/gui/destroyer/tactical.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await installCapture(page);
  await page.evaluate((s) => window.__updateConsole('tactical', JSON.stringify(s)), TACTICAL_STATE);

  expect(await tabToHost(page, 'PH-PHASERS-CONTROLS')).toBe(true);
  await page.keyboard.press('ArrowDown');           // mode toggle → FIRE button
  await page.keyboard.press('Enter');               // fire_phaser
  await expect.poll(() => page.evaluate(() => window.__sent.some((a) => a.action === 'fire_phaser')))
    .toBe(true);

  const tacticalActions = await page.evaluate(() => window.__sent.map((a) => a.action));
  const tacticalPointer = await page.evaluate(() => window.__pointerEvents);
  expect(tacticalActions).toContain('fire_phaser');
  expect(tacticalPointer).toBe(0);

  // ── Shields, power and repair: the Engineering console ───────────────────────
  await page.goto('/gui/destroyer/engineering.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await installCapture(page);
  await page.evaluate((s) => window.__updateConsole('engineering', JSON.stringify(s)), ENGINEERING_STATE);

  // Adjust shields: cycle the arc cursor and commit a facing (AC — adjust shields).
  expect(await tabToHost(page, 'PH-SHIELD-FACINGS')).toBe(true);
  await page.keyboard.press('ArrowRight');          // cursor → first facing
  await page.keyboard.press('Enter');               // set_shield_focus
  await expect.poll(() => page.evaluate(() => window.__sent.some((a) => a.action === 'set_shield_focus')))
    .toBe(true);

  // Shift power: the roving toolbar's single Tab stop is the enabled + stepper.
  expect(await tabToHost(page, 'PH-POWER-CONTROLS')).toBe(true);
  await page.keyboard.press('Enter');               // set_power (raise the group)
  await expect.poll(() => page.evaluate(() => window.__sent.some((a) => a.action === 'set_power')))
    .toBe(true);

  // Order a repair: the idle team's dispatch button is a native control (AC — order a repair).
  expect(await tabToHost(page, 'PH-REPAIR-TEAMS')).toBe(true);
  await page.keyboard.press('Enter');               // dispatch_repair_team
  await expect.poll(() => page.evaluate(() => window.__sent.some((a) => a.action === 'dispatch_repair_team')))
    .toBe(true);

  const engActions = await page.evaluate(() => window.__sent.map((a) => a.action));
  const engPointer = await page.evaluate(() => window.__pointerEvents);
  expect(engActions).toContain('set_shield_focus');
  expect(engActions).toContain('set_power');
  expect(engActions).toContain('dispatch_repair_team');

  // The envelopes carry the right named-action payloads (same as touch).
  expect(await page.evaluate(() => window.__sent.find((a) => a.action === 'set_shield_focus')))
    .toMatchObject({ action: 'set_shield_focus', console: 'engineering', arc_id: 'fore', focused: true });
  expect(await page.evaluate(() => window.__sent.find((a) => a.action === 'dispatch_repair_team')))
    .toMatchObject({ action: 'dispatch_repair_team', console: 'engineering', team_idx: 1, target: 'core' });

  // Not one pointer event was used on either console.
  expect(engPointer).toBe(0);
});
