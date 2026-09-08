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
  expect(errors).toEqual([]);
});
