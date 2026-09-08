// #1449 A2: the real render path must expose LoadingProgress in the DOM.
// Rendererless automation skips preload by design, so this belongs to the
// existing SwiftShader render project. Wire values come from the actual host.
import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

test('loading bar updates during a real slow load', async ({ context }) => {
  test.setTimeout(120_000);
  await context.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
  });
  const waiting = [];
  let released = false;
  await context.route('**/assets/models/**/*.glb', async route => {
    if (!released) await new Promise(resolve => waiting.push(resolve));
    await route.continue();
  });
  const host = await context.newPage();
  let client;
  try {
    await host.goto('/?scenario=assets/worlds/default.toml');
    await waitForWasmReady(host);
    client = await createTestClient(context, await readHostPeerId(host), { name: 'Loading check' });
    await client.send('SelectStation', { station: 'Helm' });
    await client.waitForMessage('StationAssigned');
    await client.send('SetReady', { ready: true });
    await expect(host.locator('#asset-loading')).toBeVisible();
    await client.waitForMessage('LoadingProgress', 30_000);
    // Release individual assets while holding the remainder, so the DOM has
    // an observable intermediate state rather than racing directly to 100%.
    for (let attempt = 0; attempt < 40; attempt++) {
      const percentage = await host.locator('#asset-loading-pct').textContent();
      if (Number.parseFloat(percentage) > 0 && Number.parseFloat(percentage) < 100) break;
      waiting.shift()?.();
      await host.waitForTimeout(200);
    }
    const percentage = Number.parseFloat(await host.locator('#asset-loading-pct').textContent());
    await expect(host.locator('#asset-loading')).toBeVisible();
    expect(percentage).toBeGreaterThan(0);
    expect(percentage).toBeLessThan(100);
    await client.page.waitForFunction(() => window.__messages.some(message =>
      message.type === 'LoadingProgress' && message.data.fraction > 0 && message.data.fraction < 1));
    const messages = await client.page.evaluate(() => window.__messages.filter(message => message.type === 'LoadingProgress'));
    expect(messages.length).toBeGreaterThan(0);
    expect(messages.some(message => message.data.fraction > 0 && message.data.fraction < 1)).toBe(true);
    for (const message of messages) {
      expect(typeof message.data.fraction).toBe('number');
      expect(message.data.fraction).toBeGreaterThanOrEqual(0);
      expect(message.data.fraction).toBeLessThanOrEqual(1);
    }
    released = true;
    waiting.splice(0).forEach(resolve => resolve());
    await client.waitForMessage('GameStarted', 60_000);
    await expect(host.locator('#asset-loading')).toBeHidden();
  } finally {
    released = true;
    waiting.splice(0).forEach(resolve => resolve());
    await client?.close();
  }
});

