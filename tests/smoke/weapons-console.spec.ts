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
  };

  await page.evaluate((s) => (window as any).__updateConsole('Tactical', JSON.stringify(s)), state);

  // Banks rendered with stable data-attrs.
  await expect(page.locator('#banks .bank')).toHaveCount(2);
  await expect(page.locator('.bank[data-id="fore"]')).toHaveAttribute('data-fire-ready', 'true');
  await expect(page.locator('.bank[data-id="aft"]')).toHaveAttribute('data-on-cooldown', 'true');

  // Tubes rendered.
  await expect(page.locator('#tubes .tube')).toHaveCount(2);
  await expect(page.locator('.tube[data-id="fore"]')).toHaveAttribute('data-loaded', 'true');
  await expect(page.locator('.tube[data-id="aft"]')).toHaveAttribute('data-loaded', 'false');

  // Torpedo count.
  await expect(page.locator('#torpedo-count')).toHaveAttribute('data-count', '7');
  await expect(page.locator('#torpedo-count')).toHaveText('7');
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

  await page.locator('#fire-phaser').click();
  await page.locator('#fire-torpedo').click();

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
