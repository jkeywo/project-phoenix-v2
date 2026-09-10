import { test, expect, waitForWasmReady } from './fixtures';
import fs from 'node:fs';
import path from 'node:path';
test('GM desktop layout is usable at both host viewport sizes', async ({ context }, testInfo) => {
  test.setTimeout(90000);
  const world = fs.readFileSync(path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
  await context.route('**/assets/worlds/default.toml', route => route.fulfill({contentType:'text/plain',body:world}));
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await page.evaluate(() => window.__hostFleetOpen());
  await page.waitForFunction(() => window.__hostGmStartState?.().localValidation);
  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');
  await expect(page.locator('#gm-roster-ships button').first()).toBeVisible();
  await page.locator('#gm-roster-ships button').first().click();
  for (const [width,height] of [[1440,900],[1280,720]]) {
    await page.setViewportSize({width,height});
    await page.locator('#gm-console').evaluate(el=>el.scrollTop=0);
    await expect(page.locator('#gm-workspace')).toBeVisible();
    const geometry = await page.locator('#gm-workspace').evaluate(el=>({width:el.clientWidth,scroll:el.scrollWidth}));
    expect(geometry.scroll).toBeLessThanOrEqual(geometry.width);
    const screenshot = testInfo.outputPath(`gm-screen-${width}.png`);
    await page.screenshot({path:screenshot});
    await testInfo.attach(`GM ${width}×${height}`, {path:screenshot,contentType:'image/png'});
  }
  // PRD #1418: the desk stays usable at 200% text on the smaller supported
  // landscape viewport. The variable is set DIRECTLY here rather than through
  // the settings slider, whose exposed ceiling is issue #1422's to lift; this
  // asserts the layout, not the control. The attention queue (issue #1433) is
  // included by name because it is the newest region and the one whose rows
  // wrap most: panels must scroll their own overflow rather than push the desk
  // sideways.
  await page.setViewportSize({width:1280,height:720});
  await page.evaluate(() => document.documentElement.style.setProperty('--a11y-text-scale', '2'));
  await expect(page.locator('#gm-attention-panel')).toBeVisible();
  await expect(page.locator('#gm-attention-heading')).toBeVisible();
  await expect(page.locator('#gm-attention-filter-band')).toBeVisible();
  const scaled = await page.locator('#gm-workspace').evaluate(el => ({
    width: el.clientWidth,
    scroll: el.scrollWidth,
    body: document.documentElement.scrollWidth,
    viewport: document.documentElement.clientWidth,
    fontPx: parseFloat(getComputedStyle(document.getElementById('gm-console')).fontSize),
  }));
  expect(scaled.scroll).toBeLessThanOrEqual(scaled.width);
  expect(scaled.body).toBeLessThanOrEqual(scaled.viewport);
  expect(scaled.fontPx).toBeGreaterThanOrEqual(28);
  // An eligible authored beat (issue #1434) is a row in that same queue, so it
  // meets the same contract: fed through the REAL host channel, its band reads
  // as a word, its reason is a sentence, and both of its verbs are still on
  // screen and pressable at 200% without the desk scrolling sideways.
  await page.evaluate(() => window.__hostChannel('gm_attention', JSON.stringify({
    occurrences: [{
      id: 'event:base-world::smoke#1',
      category: 'eligible_beat',
      band: 'background',
      first_seen_tick: 1,
      age_ms: 1000,
      reason: {
        id: 'server.gm.attention.reason.eligible_beat_manual',
        params: { beat: 'server.gm.mission.heading' },
      },
      target: {
        event: {
          id: 'base-world::smoke', label: 'server.gm.mission.heading',
          fire: true, pause: false, skip: false,
        },
      },
    }],
  })));
  const beatRow = page.locator('#gm-attention-list li[data-occurrence-id="event:base-world::smoke#1"]');
  await expect(beatRow).toBeVisible();
  await expect(beatRow.locator('.gm-attention-band')).toHaveText(/\S/);
  await expect(beatRow.locator('.gm-attention-reason')).toHaveText(/\S/);
  await expect(beatRow.locator('button[data-action="open"]')).toBeVisible();
  await expect(beatRow.locator('button[data-action="snooze"]')).toBeVisible();
  const withBeat = await page.locator('#gm-workspace').evaluate(el => ({
    scroll: el.scrollWidth,
    width: el.clientWidth,
    body: document.documentElement.scrollWidth,
    viewport: document.documentElement.clientWidth,
  }));
  expect(withBeat.scroll).toBeLessThanOrEqual(withBeat.width);
  expect(withBeat.body).toBeLessThanOrEqual(withBeat.viewport);
  // The keyboard reaches the row's verbs, and reading one holds the list.
  await beatRow.locator('button[data-action="open"]').focus();
  expect(await page.evaluate(() => document.activeElement?.dataset?.action)).toBe('open');
  const doubled = testInfo.outputPath('gm-screen-1280-200pc.png');
  await page.screenshot({path:doubled});
  await testInfo.attach('GM 1280×720 at 200% text', {path:doubled,contentType:'image/png'});
  await page.evaluate(() => document.documentElement.style.removeProperty('--a11y-text-scale'));
  expect(errors).toEqual([]);
});
