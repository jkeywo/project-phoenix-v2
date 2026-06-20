// Issue #422 — Smoke test: HTML weapons console + viewscreen HUD tracer bullet.
//
// The weapons console page (gui/weapons-console.html) is a static HTML file
// copied into dist/gui/ by Trunk. It exercises the ADR-0001 bridge contract:
//   - window.__updateConsole(name, stateJson) renders Tactical state into DOM.
//   - The FIRE buttons call window.__sendAction(...) with the action envelope.
// The HUD test loads the real server page and injects window.__updateHud(...)
// to confirm the overlay wiring (no sim needed — pure DOM logic).

import { test, expect, createServerPage } from './fixtures';

const CONSOLE_URL = '/gui/weapons-console.html';

test('weapons console: __updateConsole renders banks, tubes and torpedo count', async ({ page }) => {
  await page.goto(CONSOLE_URL);

  const state = {
    target_uuid: 'tgt-1',
    banks: [
      { id: 'fore', fire_ready: true, on_cooldown: false, cooldown_remaining: 0.0 },
      { id: 'aft', fire_ready: false, on_cooldown: true, cooldown_remaining: 1.5 },
    ],
    tubes: [
      { id: 'fore', loaded: true, reload_secs: 0.0 },
      { id: 'aft', loaded: false, reload_secs: 3.5 },
    ],
    torpedo_count: 7,
    phaser_mode: 'Auto',
    // Per-bank cooldown bars need the ship-config bank list (carries cooldown_secs).
    phaser_arcs: [
      { id: 'fore', facing_deg: 0,   fire_arc_deg: 270, cooldown_secs: 6.0 },
      { id: 'aft',  facing_deg: 180, fire_arc_deg: 270, cooldown_secs: 6.0 },
    ],
  };

  await page.evaluate((s) => (window as any).__updateConsole('Tactical', JSON.stringify(s)), state);

  // Banks rendered with stable data-attrs.
  await expect(page.locator('#banks .bank')).toHaveCount(2);
  await expect(page.locator('.bank[data-id="fore"]')).toHaveAttribute('data-fire-ready', 'true');
  await expect(page.locator('.bank[data-id="aft"]')).toHaveAttribute('data-on-cooldown', 'true');

  // Per-bank cooldown rows: one row per bank, ready/cooling state visible
  // in the row class and value text.
  await expect(page.locator('#phaser-cooldowns .cooldown-row')).toHaveCount(2);
  await expect(page.locator('.cooldown-row[data-id="fore"]')).toHaveClass(/is-ready/);
  await expect(page.locator('.cooldown-row[data-id="fore"] .value')).toHaveText('READY');
  await expect(page.locator('.cooldown-row[data-id="aft"]')).toHaveClass(/is-cooling/);
  await expect(page.locator('.cooldown-row[data-id="aft"] .value')).toHaveText('1.5s');

  // Tubes rendered.
  await expect(page.locator('#tubes .tube')).toHaveCount(2);
  await expect(page.locator('.tube[data-id="fore"]')).toHaveAttribute('data-loaded', 'true');
  await expect(page.locator('.tube[data-id="aft"]')).toHaveAttribute('data-loaded', 'false');

  // Torpedo count.
  await expect(page.locator('#torpedo-count')).toHaveAttribute('data-count', '7');
  await expect(page.locator('#torpedo-count')).toHaveText('7');
});

test('weapons console: Low complexity preset hides the gated controls, Std shows them', async ({ page }) => {
  // Issue #461 — console-core applies gui/hideable-elements.js after render,
  // toggling .cpx-hidden on [data-hideable] elements per the active preset.
  await page.goto(CONSOLE_URL);

  const base = {
    target_uuid: null, banks: [], tubes: [], torpedo_count: 0, phaser_mode: 'Auto',
  };
  const hideable = [
    '[data-hideable="phaser_mode_selector"]',
    '[data-hideable="torpedo_tube_selector"]',
    '[data-hideable="target_lock_button"]',
  ];

  // Low preset → all three hideable controls carry .cpx-hidden (display:none).
  // (tube-chips is empty in this state, so assert on the class — the hiding
  // contract — rather than computed visibility of a zero-size container.)
  await page.evaluate((s) => (window as any).__updateConsole('Tactical', JSON.stringify(s)),
    { ...base, complexityPreset: 'Low' });
  for (const sel of hideable) {
    await expect(page.locator(sel)).toHaveClass(/cpx-hidden/);
    await expect(page.locator(sel)).toBeHidden();
  }

  // Std preset → .cpx-hidden removed from every control.
  await page.evaluate((s) => (window as any).__updateConsole('Tactical', JSON.stringify(s)),
    { ...base, complexityPreset: 'Std' });
  for (const sel of hideable) {
    await expect(page.locator(sel)).not.toHaveClass(/cpx-hidden/);
  }
  // The non-empty readout strip is genuinely visible again under Std.
  await expect(page.locator('[data-hideable="target_lock_button"]')).toBeVisible();
});

test('weapons console: FIRE buttons call __sendAction with correct envelopes', async ({ page }) => {
  // The landscape layout targets 2160×1080; at the default 1280×720 viewport
  // the fixed-width radar-col crowds out the ctrl-col, causing the fire buttons
  // to overflow their clip-path'd inset-body and become unreachable for clicks.
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto(CONSOLE_URL);

  // Stub __sendAction to capture every call's argument.
  await page.evaluate(() => {
    (window as any).__sent = [];
    (window as any).__sendAction = (json: string) => (window as any).__sent.push(json);
  });

  // Render state so per-tube fire buttons are created.
  await page.evaluate((s) => (window as any).__updateConsole('Tactical', JSON.stringify(s)), {
    target_uuid: null,
    banks: [{ id: 'fore', fire_ready: true, on_cooldown: false, cooldown_remaining: 0.0 }],
    tubes: [{ id: 'fore', loaded: true, reload_secs: 0.0 }],
    torpedo_count: 3,
    phaser_mode: 'Auto',
  });

  await page.locator('#fire-phaser').click();
  await page.locator('#tube-list .tube-row:first-child .fire-btn').click();

  const sent: string[] = await page.evaluate(() => (window as any).__sent);
  expect(sent).toHaveLength(2);

  expect(JSON.parse(sent[0])).toEqual({ action: 'fire_phaser', console: 'Tactical', bank: 'fore' });
  expect(JSON.parse(sent[1])).toEqual({
    action: 'fire_torpedo',
    console: 'Tactical',
    tube: 'fore',
    target_uuid: null,
  });
});

test('weapons console: short landscape keeps action buttons on screen', async ({ page }) => {
  await page.setViewportSize({ width: 844, height: 390 });
  await page.goto(CONSOLE_URL);

  await page.evaluate((s) => (window as any).__updateConsole('Tactical', JSON.stringify(s)), {
    target_uuid: 'tgt-1',
    target_name: 'Harrow Patrol',
    banks: [
      { id: 'fore', fire_ready: true, on_cooldown: false, cooldown_remaining: 0.0 },
      { id: 'aft', fire_ready: false, on_cooldown: false, cooldown_remaining: 0.0 },
    ],
    tubes: [
      { id: 'fore', loaded: true, reload_secs: 0.0 },
      { id: 'port', loaded: true, reload_secs: 0.0 },
      { id: 'aft', loaded: false, reload_secs: 3.5 },
    ],
    torpedo_count: 7,
    phaser_mode: 'Auto',
  });

  const buttons = ['#tube-list .tube-row:first-child .fire-btn', '#fire-phaser', '#phaser-mode-toggle'];
  for (const selector of buttons) {
    const box = await page.locator(selector).boundingBox();
    expect(box, `${selector} should have layout bounds`).not.toBeNull();
    expect(box!.y, `${selector} top`).toBeGreaterThanOrEqual(0);
    expect(box!.y + box!.height, `${selector} bottom`).toBeLessThanOrEqual(390);
  }

  const torpedoLayout = await page.locator('.torpedo-bay').evaluate((bay) => {
    const summary = bay.querySelector('.torpedo-summary')!.getBoundingClientRect();
    const tubes = bay.querySelector('#tube-list')!.getBoundingClientRect();
    return {
      summaryRight: summary.right,
      summaryTop: summary.top,
      tubesLeft: tubes.left,
      tubesTop: tubes.top,
    };
  });
  expect(torpedoLayout.tubesLeft, 'tube list should sit to the right of the torpedo summary').toBeGreaterThan(torpedoLayout.summaryRight);
  expect(Math.abs(torpedoLayout.tubesTop - torpedoLayout.summaryTop), 'summary and tube list should share the same row').toBeLessThanOrEqual(2);

  const bodySizes = await page.locator('body').evaluate((body) => ({
    clientHeight: body.clientHeight,
    scrollHeight: body.scrollHeight,
  }));
  expect(bodySizes.scrollHeight).toBeLessThanOrEqual(bodySizes.clientHeight);
});

test('viewscreen HUD: __updateHud updates the status strip and red-alert class', async ({ context }) => {
  // Boot the real server page (it carries the #hud-overlay markup + __updateHud).
  // createServerPage already waits for __wasmReady.
  const serverPage = await createServerPage(context);

  // Inject HUD state directly — exercises the DOM-update logic without needing
  // the simulation to drive a real push.
  await serverPage.evaluate(() =>
    (window as any).__updateHud(
      JSON.stringify({ heading: 7, hull_pct: 64, condition: 'ALERT', red_alert: true }),
    ),
  );

  await expect(serverPage.locator('#hud-heading')).toHaveText('007'); // zero-padded to 3
  await expect(serverPage.locator('#hud-hull')).toHaveText('64');
  await expect(serverPage.locator('#hud-condition')).toHaveText('ALERT');
  await expect(serverPage.locator('#hud-overlay')).toHaveClass(/alert-on/);
  await expect(serverPage.locator('#hud-strip')).toHaveClass(/shown/);

  // Clearing red alert removes the class.
  await serverPage.evaluate(() =>
    (window as any).__updateHud(
      JSON.stringify({ heading: 359, hull_pct: 100, condition: 'NOMINAL', red_alert: false }),
    ),
  );
  await expect(serverPage.locator('#hud-overlay')).not.toHaveClass(/alert-on/);
  await expect(serverPage.locator('#hud-heading')).toHaveText('359');

  await serverPage.close();
});
