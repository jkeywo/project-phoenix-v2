import { test, expect } from '@playwright/test';

// Exercise the real document listener and initConsole contract, not merely
// the semantic adapter: initConsole exposes activation on window, not its handle.
for (const hull of ['cruiser', 'destroyer']) {
  test(`${hull} Dock button emits contextual named Dock and Undock actions`, async ({ page }) => {
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`/gui/${hull}/helm.html`);
    await page.waitForFunction(() => typeof window.__updateConsole === 'function');
    await page.evaluate(() => {
      window.__sent = [];
      window.__sendAction = json => window.__sent.push(JSON.parse(json));
    });
    const publish = docked => page.evaluate(docked => window.__updateConsole('helm', JSON.stringify({
      system_ids: ['berthing-clamps'],
      system_families: { 'berthing-clamps': 'helm' },
      helm_auto: false,
      dock: { system_id: 'berthing-clamps', available: !docked, engaged: docked, docked },
    })), docked);
    await publish(false);
    await page.locator('#dock-btn').click();
    await publish(true);
    await page.locator('#dock-btn').click();
    const sent = await page.evaluate(() => window.__sent);
    expect(sent).toHaveLength(2);
    for (const [index, action] of ['dock', 'undock'].entries()) {
      expect(sent[index]).toMatchObject({ action, target: 'berthing-clamps',
        control_system_id: 'berthing-clamps', semantic_action: 'helm.dock' });
      expect(sent[index].correlation).toEqual(expect.any(String));
    }
    expect(errors).toEqual([]);
  });
}
