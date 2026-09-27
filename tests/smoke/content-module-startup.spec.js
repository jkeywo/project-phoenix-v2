// Regression for Trunk completing before the independent content module.
import { test, expect, waitForWasmReady } from './fixtures';

test('world preload awaits the delayed content module after WASM arrives', async ({ context }) => {
  const page = await context.newPage();
  const errors = [];
  let rejectFatal;
  const fatal = new Promise((_, reject) => { rejectFatal = reject; });
  page.on('console', message => { if (message.type() === 'error') { errors.push(message.text()); if (message.text().includes('[Phoenix] Fatal')) rejectFatal(new Error(message.text())); } });
  let releaseModule;
  const wasmArrived = new Promise(resolve => { releaseModule = resolve; });
  await page.exposeFunction('__testWasmArrived', releaseModule);
  await page.addInitScript(() => window.addEventListener('TrunkApplicationStarted', () => window.__testWasmArrived(), { once: true }));
  await page.route('**/gui/host-content-fetch.js', async route => {
    await wasmArrived;
    // Give the pre-change direct global call a deterministic opportunity to fail.
    await new Promise(resolve => setTimeout(resolve, 100));
    await route.continue();
  });
  await Promise.race([(async () => {
    await page.goto('/?scenario=assets/worlds/default.toml');
    await waitForWasmReady(page);
  })(), fatal]);
  await expect.poll(() => page.evaluate(() => window.__hostMeshStatus?.()?.tick || 0)).toBeGreaterThan(0);
  expect(errors.filter(text => text.includes('[Phoenix] Fatal'))).toEqual([]);
});
