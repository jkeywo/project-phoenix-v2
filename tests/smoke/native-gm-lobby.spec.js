import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';

const root = path.resolve(__dirname, '../..');
// Use the native document's real script-free host markup and private boot.
// No WASM simulation or fleet peer is needed to exercise this presentation.
const html = readFileSync(path.join(root, 'server.html'), 'utf8')
  .replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, '')
  .replace('</body>', `<script>${readFileSync(path.join(root, 'src/native_host/native_gm/queue.js'), 'utf8')}</script>
    <script type="module">${readFileSync(path.join(root, 'src/native_host/native_gm/boot.js'), 'utf8')}</script></body>`);

for (const viewport of [{ width: 1280, height: 720 }, { width: 1920, height: 1080 }]) {
  test(`native GM lobby Ready is immediately visible at ${viewport.width}x${viewport.height}`, async ({ page }) => {
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
    const metadata = { phase: 'Lobby', gms: [{ id: 'native-gm', name: 'Local GM', connected: true, ready: false }],
      start_policy: { ready_total: 0, connected_total: 1 } };
    await page.evaluate(value => window.__phoenixNativeGmChannels.metadata(JSON.stringify(value)), metadata);
    const ready = page.locator('#gm-ready-btn');
    await expect(ready).toBeEnabled();
    // Check clipping before Playwright's click can scroll a buried control into view.
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
  });
}
