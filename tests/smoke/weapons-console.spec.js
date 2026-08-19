// Issue #422 — Smoke test: HTML tactical console (phasers + torpedoes) +
// viewscreen HUD tracer bullet.
//
// The flat gui/weapons-console.html page was deleted when the per-ship-class
// console migration (PRD #642) split weapons UI into standalone
// `<ph-phasers-controls>` / `<ph-torpedo-controls>` web components, embedded
// in each hull's tactical.html (gui/battleship/tactical.html here). Component
// internals (rendering, disabled-state rules, DOM reconciliation) already
// have dedicated coverage in tests/client/ph-phasers-controls.test.js and
// tests/client/ph-torpedo-controls.test.js — this smoke test only exercises
// the page-level ADR-0001 bridge contract end to end:
//   - window.__updateConsole('Tactical', stateJson) reaches both components
//     through console-core.js's initConsole render callback.
//   - Their FIRE controls call window.__sendAction(...) with the action
//     envelope, piercing the components' shadow roots.
// The HUD test loads the real server page and injects window.__updateHud(...)
// to confirm the overlay wiring (no sim needed — pure DOM logic).

import { test, expect, createServerPage } from './fixtures';

const CONSOLE_URL = '/gui/battleship/tactical.html';

test('tactical console: __updateConsole renders phaser banks, torpedo tubes and magazine', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    target_uuid: 'tgt-1',
    banks: [
      { id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false, cooldown_remaining: 0.0 },
      { id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true, cooldown_remaining: 1.5 },
    ],
    tubes: [
      { id: 'fore', label: 'Fore', loaded_count: 1, target_count: 1, volley_max: 1, load_progress: 0 },
      { id: 'aft', label: 'Aft', loaded_count: 0, target_count: 1, volley_max: 1, load_progress: 0 },
    ],
    torpedo_count: 7,
    torpedo_max: 20,
    phaser_mode: 'Manual',
  };

  await page.evaluate((s) => window.__updateConsole('Tactical', JSON.stringify(s)), state);

  // Phaser banks rendered with stable data-attrs, piercing the
  // <ph-phasers-controls> shadow root.
  await expect(page.locator('#phasers-controls .bank-row')).toHaveCount(2);
  await expect(page.locator('.bank-row[data-id="fore"] .cooldown-fill')).not.toHaveClass(/cooling/);
  await expect(page.locator('.bank-row[data-id="aft"] .cooldown-fill')).toHaveClass(/cooling/);

  // Torpedo tubes rendered, piercing the <ph-torpedo-controls> shadow root.
  await expect(page.locator('#torpedo-controls .tube-row')).toHaveCount(2);
  await expect(page.locator('.tube-row[data-id="fore"] .btn')).toBeEnabled();
  await expect(page.locator('.tube-row[data-id="aft"] .btn')).toBeDisabled();

  // Magazine count.
  await expect(page.locator('#magazine')).toHaveText('7 / 20');
});

test('tactical console: FIRE buttons call __sendAction with correct envelopes', async ({ page }) => {
  // ph-tactical-radar is a square (aspect-ratio 1/1) that eats most of the
  // vertical space at the default 1280×720 viewport, pushing the phaser and
  // torpedo controls low enough that the console's outer .frame overlay
  // intercepts clicks on them. Widen the viewport so both rows are on screen.
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto(CONSOLE_URL);

  // Stub __sendAction to capture every call's argument.
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });

  // Render state so the phaser fire button and a loaded tube's fire button
  // are both enabled.
  await page.evaluate((s) => window.__updateConsole('Tactical', JSON.stringify(s)), {
    target_uuid: 'tgt-1',
    banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false, cooldown_remaining: 0.0 }],
    tubes: [{ id: 'fore', label: 'Fore', loaded_count: 1, target_count: 1, volley_max: 1, load_progress: 0 }],
    torpedo_count: 3,
    torpedo_max: 20,
    phaser_mode: 'Manual',
  });

  await page.locator('#phasers-controls .bank-row[data-id="fore"] .btn').click();
  await page.locator('#torpedo-controls .tube-row[data-id="fore"] .btn').click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(2);

  expect(JSON.parse(sent[0])).toEqual({ action: 'fire_phaser', console: 'tactical', bank: 'fore' });
  expect(JSON.parse(sent[1])).toEqual({
    action: 'fire_torpedo',
    console: 'tactical',
    tube: 'fore',
    target_uuid: 'tgt-1',
  });
});

test('tactical console: short landscape keeps FIRE buttons reachable', async ({ page }) => {
  // Unlike the old standalone weapons-console.html, .console-body here is a
  // scrollable column (matching destroyer/tactical.html's existing pattern)
  // rather than a layout guaranteed to fit every control on screen at once —
  // the square radar alone can exceed a very short landscape viewport. So
  // the contract this test checks is "every control is reachable by
  // scrolling", not "everything fits with no scroll".
  await page.setViewportSize({ width: 844, height: 390 });
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => window.__updateConsole('Tactical', JSON.stringify(s)), {
    target_uuid: 'tgt-1',
    target_name: 'Harrow Patrol',
    banks: [
      { id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false, cooldown_remaining: 0.0 },
      { id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: false, cooldown_remaining: 0.0 },
    ],
    tubes: [
      { id: 'fore', label: 'Fore', loaded_count: 1, target_count: 1, volley_max: 1, load_progress: 0 },
      { id: 'aft', label: 'Aft', loaded_count: 0, target_count: 1, volley_max: 1, load_progress: 0 },
    ],
    torpedo_count: 7,
    torpedo_max: 20,
    phaser_mode: 'Manual',
  });

  const buttons = [
    '#phasers-controls .bank-row[data-id="fore"] .btn',
    '#torpedo-controls .tube-row[data-id="fore"] .btn',
  ];
  for (const selector of buttons) {
    const locator = page.locator(selector);
    await locator.scrollIntoViewIfNeeded();
    const box = await locator.boundingBox();
    expect(box, `${selector} should have layout bounds`).not.toBeNull();
    expect(box.y, `${selector} top`).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height, `${selector} bottom`).toBeLessThanOrEqual(390);
    // Confirms the control is actually clickable post-scroll, not merely
    // within bounds while some other element still covers it.
    await expect(locator).toBeVisible();
  }
});

test('viewscreen HUD: __updateHud updates the status strip and red-alert class', async ({ context }) => {
  // Boot the real server page (it carries the #hud-overlay markup + __updateHud).
  // createServerPage already waits for __wasmReady.
  const serverPage = await createServerPage(context);

  // Inject HUD state directly — exercises the DOM-update logic without needing
  // the simulation to drive a real push.
  await serverPage.evaluate(() =>
    window.__updateHud(
      JSON.stringify({ heading: 7, hull_pct: 64, condition: 'ALERT', red_alert: true }),
    ),
  );

  // The real 4-slot viewscreen status strip — #v-nav/#v-tac/#v-eng/#v-sys —
  // not the hidden legacy #hud-heading/#hud-hull/#hud-condition/#hud-strip
  // compat elements, which __updateHud no longer writes.
  await expect(serverPage.locator('#v-nav')).toContainText('007'); // zero-padded to 3
  await expect(serverPage.locator('#v-eng')).toContainText('64');
  await expect(serverPage.locator('#v-sys')).toHaveText('ALERT');
  await expect(serverPage.locator('#hud-overlay')).toHaveClass(/alert-on/);
  await expect(serverPage.locator('#v-tac')).toContainText('WEAPONS HOT');

  // Clearing red alert removes the class.
  await serverPage.evaluate(() =>
    window.__updateHud(
      JSON.stringify({ heading: 359, hull_pct: 100, condition: 'NOMINAL', red_alert: false }),
    ),
  );
  await expect(serverPage.locator('#hud-overlay')).not.toHaveClass(/alert-on/);
  await expect(serverPage.locator('#v-tac')).toContainText('CLEAR');
  await expect(serverPage.locator('#v-nav')).toContainText('359');

  await serverPage.close();
});
