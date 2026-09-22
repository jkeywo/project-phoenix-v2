import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';

const root = path.resolve(__dirname, '../..');
// Use the native document's real script-free host markup and private boot.
// No WASM simulation or fleet peer is needed to exercise this presentation.
const html = readFileSync(path.join(root, 'server.html'), 'utf8')
  .replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, '')
  .replace('</body>', `<script>${readFileSync(path.join(root, 'src/native_host/native_gm/queue.js'), 'utf8')}</script>
    <script>${readFileSync(path.join(root, 'src/native_host/panes/operator_storage.js'), 'utf8')}</script>
    <script>${readFileSync(path.join(root, 'src/native_host/audio/private_boot.js'), 'utf8')}</script>
    <script type="module">${readFileSync(path.join(root, 'src/native_host/native_gm/boot.js'), 'utf8')}</script></body>`);

for (const viewport of [{ width: 1280, height: 720 }, { width: 1920, height: 1080 }]) {
  test(`native GM lobby Ready is reachable at ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.route('**/*', async route => {
      const url = new URL(route.request().url());
      if (url.hostname !== 'localhost') return route.abort();
      if (url.pathname === '/native-gm-lobby.html') {
        return route.fulfill({ contentType: 'text/html', body: html });
      }
      const file = path.resolve(root, `.${decodeURIComponent(url.pathname)}`);
      if (!file.startsWith(`${root}${path.sep}`)) return route.abort();
      try { await route.fulfill({ path: file }); } catch { await route.abort(); }
    });
    await page.goto('/native-gm-lobby.html');
    await page.waitForFunction(() => typeof window.__phoenixNativeGmChannels === 'object');
    const metadata = { phase: 'Lobby', local_operator_id: 'native-gm',
      gms: [{ id: 'native-gm', name: 'Local GM', connected: true, ready: false }],
      start_policy: { ready_total: 0, connected_total: 1 } };
    await page.evaluate(value => window.__phoenixNativeGmChannels.metadata(JSON.stringify(value)), metadata);
    await expect(page.locator('#gm-header-ready')).toBeVisible();
    await expect(page.locator('#gm-header-ready')).toBeEnabled();
    await expect(page.locator('#gm-session-pause')).toBeHidden();
    await expect(page.locator('#gm-session-resume')).toBeHidden();
    await expect(page.locator('#gm-role-preset-select')).toBeHidden();
    for (const control of await page.locator('.gm-segment button[aria-pressed="true"]').all()) {
      expect(await control.evaluate(node => getComputedStyle(node).color !== getComputedStyle(node).backgroundColor)).toBe(true);
    }
    await page.locator('#gm-header-ready').click();
    expect(await page.evaluate(() => window.__phoenixNativeGmOutDrain().split('\n').filter(Boolean).map(JSON.parse)))
      .toContainEqual({ kind: 'ready', ready: true });
    const selected = page.locator('#gm-live-layout [role="tab"][aria-selected="true"]').first();
    expect(await selected.evaluate(node => getComputedStyle(node).color !== getComputedStyle(node).backgroundColor)).toBe(true);
    await expect(page.locator('#gm-live-layout [data-layout-control="float"]')).toHaveCount(0);
    expect(await page.locator('#gm-live-layout .workshop-split-resize').count()).toBeGreaterThan(0);
    const readiness = page.locator('#gm-live-layout .workshop-panel-switcher [data-layout-panel="readiness"]');
    await readiness.locator('xpath=ancestor::details/summary').click();
    await expect(readiness).toBeInViewport({ ratio: 1 });
    await readiness.click();
    const ready = page.locator('#gm-ready-btn');
    await expect(ready).toBeEnabled();
    // Centred rather than nearest-edge: a nearest scroll leaves the control
    // flush with its frame's fractional clip edge, a third of a pixel short of
    // the whole button, which is reachable but not "ratio 1".
    await ready.evaluate(node => node.scrollIntoView({ block: 'center' }));
    await expect(ready).toBeInViewport({ ratio: 1 });
    await ready.click({ timeout: 5000 });
    expect(await page.evaluate(() => window.__phoenixNativeGmOutDrain().split('\n').filter(Boolean).map(JSON.parse)))
      .toContainEqual({ kind: 'ready', ready: true });
    await page.evaluate(value => window.__phoenixNativeGmChannels.metadata(JSON.stringify(value)),
      { ...metadata, gms: [{ ...metadata.gms[0], ready: true }] });
    await expect(ready).toContainText('Unready');
    await ready.click({ timeout: 5000 });
    expect(await page.evaluate(() => window.__phoenixNativeGmOutDrain().split('\n').filter(Boolean).map(JSON.parse)))
      .toContainEqual({ kind: 'ready', ready: false });
    await page.evaluate(value => window.__phoenixNativeGmChannels.metadata(JSON.stringify(value)),
      { ...metadata, phase: 'InProgress' });
    await expect(ready).toBeDisabled();
    await expect(page.locator('#gm-header-ready')).toBeHidden();
    await expect(page.locator('#gm-session-pause')).toBeVisible();
    await page.evaluate(() => window.__phoenixNativeGmChannels.gm_session(JSON.stringify({ paused: true, results: [] })));
    await expect(page.locator('#gm-session-pause')).toBeHidden();
    await expect(page.locator('#gm-session-resume')).toBeVisible();

    const menu = page.locator('#gm-live-layout details').first();
    await menu.locator('summary').click();
    await page.locator('#gm-console-title').click();
    await expect(menu).not.toHaveAttribute('open', '');

    const mapTab = page.locator('#gm-live-layout [role="tab"][data-layout-panel="map"]');
    await mapTab.click();
    await expect(page.locator('.workshop-dock-targets:visible')).toHaveCount(0);
    const tabBox = await mapTab.boundingBox();
    await page.mouse.move(tabBox.x + 20, tabBox.y + 12);
    await page.mouse.down();
    await page.mouse.move(tabBox.x + 50, tabBox.y + 80, { steps: 5 });
    expect(await page.locator('.workshop-tab-stack .workshop-dock-targets:visible').count()).toBeGreaterThan(0);
    await page.mouse.up();
    const floating = page.locator('[data-panel="map"].is-floating');
    await expect(floating).toBeVisible();
    const before = await floating.boundingBox();
    const grip = await floating.locator('.workshop-float-resize').boundingBox();
    await page.mouse.move(grip.x + 5, grip.y + 5);
    await page.mouse.down();
    await page.mouse.move(grip.x + 65, grip.y + 35, { steps: 5 });
    await page.mouse.up();
    expect((await floating.boundingBox()).width).toBeGreaterThan(before.width);

    const handle = await floating.locator('.workshop-panel-tab').boundingBox();
    await page.mouse.move(handle.x + 20, handle.y + 12);
    await page.mouse.down();
    await page.mouse.move(handle.x + 35, handle.y + 15, { steps: 3 });
    const target = page.locator('.workshop-tab-stack .workshop-dock-target.is-tab:visible').first();
    const targetBox = await target.boundingBox();
    await page.mouse.move(targetBox.x + targetBox.width / 2, targetBox.y + targetBox.height / 2, { steps: 10 });
    await page.mouse.up();
    await expect(floating).toHaveCount(0);
    const splitter = page.locator('.workshop-split-resize:visible').first();
    const split = splitter.locator('..');
    const horizontal = await split.evaluate(node => node.classList.contains('is-horizontal'));
    const trackStyle = () => split.evaluate((node, horizontal) => horizontal
      ? node.style.gridTemplateColumns : node.style.gridTemplateRows, horizontal);
    const previousTracks = await trackStyle();
    const splitterBox = await splitter.boundingBox();
    const x = splitterBox.x + splitterBox.width / 2, y = splitterBox.y + splitterBox.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x + (horizontal ? 60 : 0), y + (horizontal ? 0 : 60), { steps: 5 });
    await page.mouse.up();
    expect(await trackStyle()).not.toBe(previousTracks);
  });
}
