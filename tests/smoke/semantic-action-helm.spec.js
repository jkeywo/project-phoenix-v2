// Issue #1278: the selected standard gamepad's horizontal stick reaches the
// real Helm SetSteering route. Disconnect emits one stop, never transfers to a
// second pad, and an explicitly reselected displaced replacement stays gated.

import { test, expect, readHostPeerId, waitForWasmReady } from './fixtures';
import { ts } from './strings';

async function installFabricatedGamepads(page) {
  await page.addInitScript(() => {
    let pads = [];
    Object.defineProperty(navigator, 'getGamepads', {
      configurable: true,
      value: () => pads,
    });
    window.__setFabricatedGamepads = (specs) => {
      pads = [];
      for (const spec of specs) {
        if (!spec) continue;
        pads[spec.index] = {
          index: spec.index,
          mapping: spec.mapping === undefined ? 'standard' : spec.mapping,
          buttons: Array.from({ length: 17 }, () => ({ pressed: false, value: 0 })),
          axes: spec.axes || [0, 0, 0, 0],
          id: `fabricated-hardware-${spec.index}`,
        };
      }
    };
  });
}

async function setPads(page, specs) {
  await page.evaluate((next) => window.__setFabricatedGamepads(next), specs);
}

test('selected continuous Helm axis steers authoritatively and reconnects neutral-gated', async ({ context }) => {
  test.setTimeout(180_000);

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml', {
    waitUntil: 'domcontentloaded',
  });
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);
  const helm = await context.newPage();
  await installFabricatedGamepads(helm);
  await helm.goto(`/client/#${hostId}`, { waitUntil: 'domcontentloaded' });
  await helm.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await helm.click('#station-list .station-row:has-text("Helm") button.claim-btn');
  await helm.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await helm.click('#ready-btn');
  await expect(helm.locator('#helm-ui')).toHaveClass(/active/, { timeout: 10_000 });

  // Observe the real iframe -> parent action seam. The authoritative proof is
  // the heading change below; this log pins cadence/neutral behavior without
  // inventing any test-only wire or server API.
  await helm.evaluate(() => {
    window.__observedHelmSteering = [];
    window.addEventListener('message', (event) => {
      if (!event.data || event.data.type !== 'console_action') return;
      try {
        const action = JSON.parse(event.data.payload);
        if (action.action === 'set_helm_steering') {
          window.__observedHelmSteering.push(action.value);
        }
      } catch (_) { /* ignore unrelated frames */ }
    });
  });

  await setPads(helm, [{ index: 0 }, { index: 1 }]);
  await helm.click('#settings-btn');
  await helm.click('.settings-tab[data-tab="controls"]');
  const selector = helm.locator('[data-control="semantic-gamepad-select"]');
  await selector.selectOption('0');
  await expect(helm.locator('[data-control="semantic-binding-helm.steering-0"]'))
    .toHaveValue(ts('input.gamepad.left_stick_x'));
  await helm.keyboard.press('Escape');

  const radar = helm.frameLocator('#helm-iframe').locator('ph-helm-radar');
  const initialHeading = await radar.evaluate((element) => Number(element.state.ship_heading));
  // Pad 1 is displaced but unowned and cannot steer.
  await setPads(helm, [{ index: 0 }, { index: 1, axes: [1, 0, 0, 0] }]);
  await helm.waitForTimeout(250);
  expect(await helm.evaluate(() => window.__observedHelmSteering)).toEqual([]);

  await setPads(helm, [{ index: 0, axes: [0.8, 0, 0, 0] }, { index: 1 }]);
  await expect.poll(async () => {
    const heading = await radar.evaluate((element) => Number(element.state.ship_heading));
    return Math.abs(heading - initialHeading);
  }, { timeout: 10_000 }).toBeGreaterThan(0.25);
  await expect.poll(async () => (await helm.evaluate(
    () => window.__observedHelmSteering.filter((value) => value > 0).length,
  )), { timeout: 5_000 }).toBeGreaterThan(0);

  // Disconnect stops the active command exactly once and leaves pad 1 inert.
  await setPads(helm, [null, { index: 1, axes: [-1, 0, 0, 0] }]);
  await expect(helm.locator('#gamepad-input-alert')).toBeVisible();
  await expect(helm.locator('#gamepad-input-alert'))
    .toContainText(ts('client.gamepad.disconnect_warning'));
  await expect.poll(async () => (await helm.evaluate(
    () => window.__observedHelmSteering.filter((value) => value === 0).length,
  )), { timeout: 5_000 }).toBe(1);
  await helm.waitForTimeout(250);
  expect(await helm.evaluate(
    () => window.__observedHelmSteering.filter((value) => value === 0).length,
  )).toBe(1);

  // Index reuse is a new connection. Held displacement after explicit
  // reselection remains inert until neutral, then a fresh deflection operates.
  await setPads(helm, [{ index: 0, axes: [0.8, 0, 0, 0] }, { index: 1 }]);
  await helm.click('#settings-btn');
  await helm.click('.settings-tab[data-tab="controls"]');
  await selector.selectOption('');
  await selector.selectOption('0');
  await expect(helm.locator('[data-control="semantic-gamepad-status"]'))
    .toContainText(ts('settings.controls.gamepad.status_neutral'));
  await helm.evaluate(() => { window.__observedHelmSteering = []; });
  await helm.keyboard.press('Escape');
  await helm.waitForTimeout(250);
  expect(await helm.evaluate(() => window.__observedHelmSteering)).toEqual([]);
  await setPads(helm, [{ index: 0 }, { index: 1 }]);
  await helm.waitForTimeout(100);
  await setPads(helm, [{ index: 0, axes: [-0.6, 0, 0, 0] }, { index: 1 }]);
  await expect.poll(async () => (await helm.evaluate(
    () => window.__observedHelmSteering.some((value) => value < 0),
  )), { timeout: 5_000 }).toBe(true);

  await helm.close();
  await serverPage.close();
});
