// Issues #1278/#1288: the selected standard gamepad reaches the real Helm
// continuous and discrete routes. Authoritative feedback proves accepted and
// refused commands; disconnect emits one stop, never transfers to a second
// pad, and an explicitly reselected displaced replacement stays gated.

import {
  test, expect, createTestClient, readHostPeerId, waitForWasmReady,
} from './fixtures';
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
        const buttons = Array.from(
          { length: 17 },
          (_unused, index) => ({
            pressed: (spec.pressed || []).includes(index),
            value: (spec.pressed || []).includes(index) ? 1 : 0,
          }),
        );
        pads[spec.index] = {
          index: spec.index,
          mapping: spec.mapping === undefined ? 'standard' : spec.mapping,
          buttons,
          axes: spec.axes || [0, 0, 0, 0],
          id: spec.id || `fabricated-hardware-${spec.index}`,
        };
      }
    };
  });
}

async function setPads(page, specs) {
  await page.evaluate((next) => window.__setFabricatedGamepads(next), specs);
}

async function installHelmFeedbackProbe(frameBody) {
  await frameBody.evaluate(() => {
    window.__helmFeedbackTransitions = [];
    window.addEventListener('phoenix-action-feedback', (event) => {
      const value = event && event.detail;
      if (!value || value.lifecycleTransition !== true) return;
      window.__helmFeedbackTransitions.push({
        actionId: value.actionId,
        correlation: value.correlation,
        state: value.state,
      });
    });
  });
}

async function expectAppliedLifecycles(frameBody, actionId, count = 1) {
  const expected = Array.from(
    { length: count },
    () => ['Pressed', 'Pending', 'Applied'],
  ).flat();
  await expect.poll(
    () => frameBody.evaluate((_body, id) => (
      window.__helmFeedbackTransitions
        .filter((event) => event.actionId === id)
        .map((event) => event.state)
    ), actionId),
    { timeout: 10_000 },
  ).toEqual(expected);
  const events = await frameBody.evaluate((_body, id) => (
    window.__helmFeedbackTransitions.filter((event) => event.actionId === id)
  ), actionId);
  for (let offset = 0; offset < events.length; offset += 3) {
    expect(new Set(events.slice(offset, offset + 3).map((event) => event.correlation)).size).toBe(1);
  }
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

  const helmFrame = helm.frameLocator('#helm-iframe');
  const frameBody = helmFrame.locator('body');
  await installHelmFeedbackProbe(frameBody);

  // Three independently bindable discrete Helm actions take their real
  // gamepad routes. Boost is a hold: both the rising and falling edge receive
  // their own terminal authoritative outcome.
  await setPads(helm, [{ index: 0, pressed: [3] }, { index: 1 }]);
  await expectAppliedLifecycles(frameBody, 'helm.viewscreen');
  await setPads(helm, [{ index: 0 }, { index: 1 }]);
  await setPads(helm, [{ index: 0, pressed: [1] }, { index: 1 }]);
  await expectAppliedLifecycles(frameBody, 'helm.impulse');
  await setPads(helm, [{ index: 0 }, { index: 1 }]);
  await setPads(helm, [{ index: 0, pressed: [0] }, { index: 1 }]);
  await expectAppliedLifecycles(frameBody, 'helm.boost');
  await setPads(helm, [{ index: 0 }, { index: 1 }]);
  await expectAppliedLifecycles(frameBody, 'helm.boost', 2);

  // The same well-formed correlated Helm command from a peer without Helm
  // tenure is refused by the real authority boundary and cannot start a jump.
  const nonHelm = await createTestClient(context, hostId, { name: 'No Helm' });
  const refusedCorrelation = 'smoke-non-helm-impulse';
  await nonHelm.send('ControlSystemCorrelated', {
    correlation: refusedCorrelation,
    target: 'helm-impulse',
    payload: { type: 'StartImpulseCharge' },
  });
  expect((await nonHelm.waitForMessage('ActionFeedback', 10_000)).data).toEqual({
    correlation: refusedCorrelation,
    outcome: 'Refused',
  });
  await nonHelm.close();

  const radar = helmFrame.locator('ph-helm-radar');
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

test('@core saved controller restores at startup, updates help and hides only usable Helm controls', async ({ context }) => {
  test.setTimeout(180_000);
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml', { waitUntil: 'domcontentloaded' });
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);
  const helm = await context.newPage();
  await installFabricatedGamepads(helm);
  await helm.addInitScript(() => {
    if (window !== window.parent) return;
    window.__setFabricatedGamepads([{ index: 2, id: 'saved controller' }]);
    localStorage.setItem('phoenix-operator-profile-v1', JSON.stringify({
      kind: 'project-phoenix/operator-profile', version: 1,
      gamepad: { preferredSlot: 0, preferredDevice: { id: 'saved controller', mapping: 'standard' } },
    }));
  });
  await helm.goto(`/client/#${hostId}`, { waitUntil: 'domcontentloaded' });
  await helm.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await helm.click('#station-list .station-row:has-text("Helm") button.claim-btn');
  await helm.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await helm.click('#ready-btn');
  await expect(helm.locator('#helm-ui')).toHaveClass(/active/, { timeout: 10_000 });
  const frame = helm.frameLocator('#helm-iframe');
  await expect(frame.locator('ph-helm-joystick')).toBeHidden();
  await expect(frame.locator('ph-lateral-thrust-joystick')).toBeHidden();

  await helm.click('#settings-btn');
  await helm.click('.settings-tab[data-tab="controls"]');
  await expect(helm.locator('[data-control="semantic-gamepad-select"]')).toHaveValue('2');
  await expect(helm.locator('[data-control="gamepad-hide-touch"]')).toBeChecked();
  await helm.locator('[data-control="gamepad-hide-touch"]').uncheck();
  await expect(frame.locator('ph-helm-joystick')).toBeVisible();
  await helm.locator('[data-control="gamepad-hide-touch"]').check();
  await helm.click('.settings-tab[data-tab="station-help"]');
  await expect(helm.locator('.settings-documentation')).toContainText(ts('input.gamepad.left_stick_x'));
  await setPads(helm, []);
  await expect(helm.locator('.settings-documentation')).not.toContainText(ts('input.gamepad.left_stick_x'));
  await helm.keyboard.press('Escape');
  await expect(frame.locator('ph-helm-joystick')).toBeVisible();
  await expect(frame.locator('ph-lateral-thrust-joystick')).toBeVisible();
  await setPads(helm, [{ index: 1, id: 'saved controller' }]);
  await expect(frame.locator('ph-helm-joystick')).toBeHidden();
  await expect(frame.locator('ph-lateral-thrust-joystick')).toBeHidden();
  await helm.close();
  await serverPage.close();
});
