// Issues #1274/#1275 tracer: a real client refuses a reserved chord, resolves
// an overlapping Captain binding, resets the profile, then remaps Red Alert.
// The remapped key traverses the existing action-map, transport,
// command-admission and Captain server path. The component is not painted
// optimistically, so its eventual active state is authoritative read-back.

import {
  test,
  expect,
  createTestClient,
  readHostPeerId,
  waitForWasmReady,
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
        const buttons = Array.from({ length: 17 }, () => ({ pressed: false, value: 0 }));
        for (const index of spec.pressed || []) buttons[index] = { pressed: true, value: 1 };
        pads[spec.index] = {
          index: spec.index,
          mapping: spec.mapping === undefined ? 'standard' : spec.mapping,
          buttons,
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

async function installCaptainFeedbackProbe(frameBody) {
  await frameBody.evaluate(() => {
    window.__captainFeedbackTransitions = [];
    window.addEventListener('phoenix-action-feedback', (event) => {
      const value = event && event.detail;
      if (!value || value.lifecycleTransition !== true) return;
      const alertRoot = document.querySelector('ph-red-alert')?.shadowRoot;
      const cameraRoot = document.querySelector('ph-camera-select')?.shadowRoot;
      const alertButton = alertRoot?.getElementById('alert-btn');
      const activeView = cameraRoot?.querySelector('.cam-btn.active');
      window.__captainFeedbackTransitions.push({
        actionId: value.actionId,
        correlation: value.correlation,
        state: value.state,
        alertActive: alertButton?.classList.contains('active') ?? false,
        alertBusy: alertButton?.getAttribute('aria-busy') ?? null,
        activeView: activeView?.dataset.view ?? null,
      });
    });
  });
}

async function expectAppliedLifecycle(frameBody, actionId) {
  await expect.poll(
    () => frameBody.evaluate((_body, id) => (
      window.__captainFeedbackTransitions
        .filter((event) => event.actionId === id)
        .map((event) => event.state)
    ), actionId),
    { timeout: 10_000 },
  ).toEqual(['Pressed', 'Pending', 'Applied']);

  const events = await frameBody.evaluate((_body, id) => (
    window.__captainFeedbackTransitions.filter((event) => event.actionId === id)
  ), actionId);
  expect(new Set(events.map((event) => event.correlation)).size).toBe(1);
  return events;
}

test('remapped Captain Red Alert binding reaches the authoritative command path', async ({ context }) => {
  test.setTimeout(180_000);

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml', {
    waitUntil: 'domcontentloaded',
  });
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);
  const captain = await context.newPage();
  // The client shell is interactive at DOMContentLoaded. Waiting for the full
  // load event also waits on non-critical presentation assets and can exhaust
  // the navigation timeout before the authoritative lobby flow even begins.
  await captain.goto(`/client/#${hostId}`, { waitUntil: 'domcontentloaded' });
  await captain.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await captain.click('#station-list .station-row:has-text("Captain") button.claim-btn');
  await captain.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await captain.click('#ready-btn');
  await expect(captain.locator('#captain-ui')).toHaveClass(/active/, { timeout: 10_000 });

  const captainFrame = captain.frameLocator('#captain-iframe');
  const frameBody = captainFrame.locator('body');
  const alertComponent = captainFrame.locator('ph-red-alert');
  const alertButton = alertComponent.locator('#alert-btn');
  const alertFeedback = alertComponent.locator('#feedback-status');
  // Issue #1398: `ph-red-alert` carries ONE button. Restraint moved to Power,
  // so the Captain's second discoverable action here is the viewscreen.
  await expect(alertComponent.locator('#hold-btn')).toHaveCount(0);
  const activeViewButton = captainFrame.locator('ph-camera-select .cam-btn.active');
  await expect(alertButton).toBeEnabled();
  await expect(alertButton).toHaveText(ts('component.red_alert.standby'));

  await captain.click('#settings-btn');
  await captain.click('.settings-tab[data-tab="controls"]');
  const binding = captain.locator(
    '[data-control="semantic-binding-captain.red-alert-0"]',
  );
  const viewBinding = captain.locator(
    '[data-control="semantic-binding-captain.view-0"]',
  );
  await expect(binding).toHaveValue('R');
  await expect(viewBinding).toHaveValue('V');

  // Browser-delivered proof: even with Shift optional in the reserved policy,
  // Ctrl+R cannot become a Station binding and the authored R survives.
  await binding.click();
  await binding.press('Control+KeyR');
  await expect(captain.locator('.settings-binding-feedback[role="alert"]'))
    .toContainText('Ctrl + R');
  await expect(binding).toBeFocused();
  await expect(binding).toHaveValue(ts('settings.controls.press_key'));
  // The replacement capture is immediately usable; plain R is harmless and
  // restores the same authored value without escaping to browser chrome.
  await binding.press('KeyR');
  await expect(binding).toHaveValue('R');

  // Both actions share Captain context. Replace clears the previous slot;
  // resetting Red Alert then clears that colliding remap before restoring R.
  await viewBinding.click();
  await viewBinding.press('KeyR');
  await expect(captain.locator('[data-control="semantic-binding-conflict-cancel"]'))
    .toBeFocused();
  // Conflict Escape is modal-wide: move backward out of the prompt to Reset
  // All, then cancel without letting the shared Settings trap close the modal.
  // Derive the distance from the live trap ring: adding another discoverable
  // semantic action legitimately adds another binding input to that ring.
  const resetAll = captain.locator('[data-control="semantic-binding-reset-all"]');
  const reverseTabsToResetAll = await captain.evaluate(() => {
    const overlay = document.querySelector('#settings-overlay');
    const target = overlay.querySelector('[data-control="semantic-binding-reset-all"]');
    const focusable = window.focusableWithin(overlay);
    const from = focusable.indexOf(document.activeElement);
    const to = focusable.indexOf(target);
    return from >= 0 && to >= 0
      ? (from - to + focusable.length) % focusable.length
      : -1;
  });
  expect(reverseTabsToResetAll).toBeGreaterThan(0);
  for (let index = 0; index < reverseTabsToResetAll; index += 1) {
    await captain.keyboard.press('Shift+Tab');
  }
  await expect(resetAll).toBeFocused();
  await captain.keyboard.press('Escape');
  await expect(captain.locator('#settings-overlay')).toBeVisible();
  await expect(captain.locator('.settings-binding-conflict')).toHaveCount(0);
  await expect(viewBinding).toBeFocused();
  await expect(viewBinding).toHaveValue(ts('settings.controls.press_key'));
  await viewBinding.blur();
  await expect(viewBinding).toHaveValue('V');

  await viewBinding.click();
  await viewBinding.press('KeyR');
  await expect(captain.locator('[data-control="semantic-binding-conflict-cancel"]'))
    .toBeFocused();
  await captain.click('[data-control="semantic-binding-conflict-replace"]');
  await expect(binding).toHaveValue(ts('input.binding.unassigned'));
  await expect(viewBinding).toBeFocused();
  await expect(viewBinding).toHaveValue(ts('settings.controls.press_key'));
  await viewBinding.blur();
  await expect(viewBinding).toHaveValue('R');
  await captain.click('[data-control="semantic-binding-reset-captain.red-alert"]');
  await expect(binding).toHaveValue('R');
  await expect(viewBinding).toHaveValue(ts('input.binding.unassigned'));
  await captain.click('[data-control="semantic-binding-reset-all"]');
  await expect(binding).toHaveValue('R');
  await expect(viewBinding).toHaveValue('V');

  // Modified navigation keys are chords, not modal navigation. Both remain
  // inside capture, are refused by the registry, and leave the field ready
  // for an immediate harmless retry.
  for (const [chord, display] of [
    ['Control+Tab', 'Ctrl + Tab'],
    ['Control+Escape', 'Ctrl + Escape'],
  ]) {
    await binding.focus();
    await binding.press(chord);
    await expect(captain.locator('.settings-binding-feedback[role="alert"]'))
      .toContainText(display);
    await expect(binding).toBeFocused();
    await expect(binding).toHaveValue(ts('settings.controls.press_key'));
    await expect(captain.locator('#settings-overlay')).toBeVisible();
    await binding.press('KeyR');
    await expect(binding).toHaveValue('R');
  }

  // Capture must yield modal navigation keys instead of treating them as
  // reserved binding proposals. Tab and Shift+Tab move on inside the open
  // modal; plain Escape then bubbles to the shared trap and closes Settings.
  await binding.focus();
  await binding.press('Tab');
  await expect(binding).not.toBeFocused();
  await expect(captain.locator('#settings-overlay')).toBeVisible();
  await binding.focus();
  await binding.press('Shift+Tab');
  await expect(binding).not.toBeFocused();
  await expect(captain.locator('#settings-overlay')).toBeVisible();
  await binding.focus();
  await binding.press('Escape');
  await expect(captain.locator('#settings-overlay')).toBeHidden();

  await captain.click('#settings-btn');
  await captain.click('.settings-tab[data-tab="controls"]');
  await expect(binding).toHaveValue('R');
  await binding.click();
  await binding.press('KeyY');
  await expect(captain.locator(
    '[data-control="semantic-binding-captain.red-alert-0"]',
  )).toHaveValue('Y');

  // #1279: the same live choices form a private, portable JSON profile. The
  // download contains no session/Station/save state, and a valid import is
  // applied as one profile before the next action dispatch.
  const storedProfile = await captain.evaluate(() => JSON.parse(
    localStorage.getItem('phoenix-operator-profile-v1'),
  ));
  expect(Object.keys(storedProfile).sort()).toEqual([
    'accessibility', 'bindings', 'feedback', 'gamepad', 'gmConfirmations',
    'kind', 'version',
  ]);
  expect(JSON.stringify(storedProfile)).not.toMatch(/session-token|player-name|station|saveCatalogue/i);
  const downloadPromise = captain.waitForEvent('download');
  await captain.click('[data-control="operator-profile-export"]');
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe('phoenix-operator-profile.json');
  await expect(captain.locator('[data-control="operator-profile-status"]'))
    .toContainText(ts('settings.controls.profile.status_exported'));

  storedProfile.bindings['captain.red-alert'][0].code = 'KeyU';
  await captain.locator('[data-control="operator-profile-file"]').setInputFiles({
    name: 'operator-profile.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(storedProfile)),
  });
  await expect(captain.locator('[data-control="operator-profile-status"]'))
    .toContainText(ts('settings.controls.profile.status_imported'));
  await expect(captain.locator(
    '[data-control="semantic-binding-captain.red-alert-0"]',
  )).toHaveValue('U');

  // Close Settings so the key relay may hand the host-page event to the
  // active Captain iframe. The remap capture itself stops propagation, so the
  // capture key above cannot also fire the action.
  await captain.keyboard.press('Escape');
  await expect(captain.locator('#settings-overlay')).toBeHidden();
  await installCaptainFeedbackProbe(frameBody);
  await captain.keyboard.press('KeyU');

  const alertEvents = await expectAppliedLifecycle(frameBody, 'captain.red-alert');
  const alertPending = alertEvents.find((event) => event.state === 'Pending');
  expect(alertPending).toMatchObject({ alertActive: false, alertBusy: 'true' });
  await expect(alertButton).toHaveText(ts('component.red_alert.active'), {
    timeout: 10_000,
  });
  await expect(alertButton).toHaveClass(/active/);
  await expect(alertFeedback).toHaveText(ts('action_feedback.applied'));

  // The same shipped registry/transport/authority lifecycle covers the
  // parameterised View action. Pending is observed before the ordinary Captain
  // blackboard changes the control's rendered state.
  const viewBefore = await activeViewButton.count() === 1
    ? await activeViewButton.getAttribute('data-view')
    : null;
  await captain.keyboard.press('KeyV');
  const viewEvents = await expectAppliedLifecycle(frameBody, 'captain.view');
  const viewPending = viewEvents.find((event) => event.state === 'Pending');
  expect(viewPending.activeView).toBe(viewBefore);
  await expect.poll(
    async () => await activeViewButton.count() === 1
      ? activeViewButton.getAttribute('data-view')
      : null,
    { timeout: 10_000 },
  ).not.toBe(viewBefore);
  const viewAfter = await activeViewButton.getAttribute('data-view');

  // A real peer with no Captain tenure sends the same well-formed correlated
  // state-setting command. Admission targets Refused back to that token, and
  // advancing the authoritative clock proves the rejected request never
  // changes Red Alert (or the already-applied sibling Captain state).
  const nonCaptain = await createTestClient(context, hostId, {
    name: 'Unassigned Crew',
  });
  const refusedCorrelation = 'smoke-non-captain-red-alert';
  await nonCaptain.send('ControlSystemCorrelated', {
    correlation: refusedCorrelation,
    target: 'red-alert',
    payload: { type: 'SetRedAlert', data: { active: false } },
  });
  const refusal = await nonCaptain.waitForMessage('ActionFeedback', 10_000);
  expect(refusal.data).toEqual({
    correlation: refusedCorrelation,
    outcome: 'Refused',
  });
  const refusedAtTick = await serverPage.evaluate(() => window.wasm_sim_tick());
  await expect.poll(
    () => serverPage.evaluate(() => window.wasm_sim_tick()),
    { timeout: 10_000 },
  ).toBeGreaterThan(refusedAtTick + 2);
  await expect(alertButton).toHaveText(ts('component.red_alert.active'));
  await expect(alertButton).toHaveClass(/active/);
  await expect(activeViewButton).toHaveAttribute('data-view', viewAfter);

  await nonCaptain.close();
  await captain.close();
  await serverPage.close();
});

test('one selected standard gamepad owns Red Alert without transfer on disconnect', async ({ context }) => {
  test.setTimeout(180_000);

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml', {
    waitUntil: 'domcontentloaded',
  });
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);
  const captain = await context.newPage();
  await installFabricatedGamepads(captain);
  await captain.goto(`/client/#${hostId}`, { waitUntil: 'domcontentloaded' });
  await captain.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await captain.click('#station-list .station-row:has-text("Captain") button.claim-btn');
  await captain.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await captain.click('#ready-btn');
  await expect(captain.locator('#captain-ui')).toHaveClass(/active/, { timeout: 10_000 });

  const alertButton = captain.frameLocator('#captain-iframe')
    .locator('ph-red-alert').locator('#alert-btn');
  await expect(alertButton).toHaveText(ts('component.red_alert.standby'));

  await setPads(captain, [{ index: 0 }, { index: 1 }]);
  await captain.click('#settings-btn');
  await captain.click('.settings-tab[data-tab="controls"]');
  const selector = captain.locator('[data-control="semantic-gamepad-select"]');
  await expect(selector.locator('option')).toHaveCount(3);
  await expect(captain.locator('[data-control="semantic-binding-captain.red-alert-1"]'))
    .toHaveValue(ts('input.gamepad.face_bottom'));
  await selector.selectOption('0');
  await expect(captain.locator('[data-control="semantic-gamepad-status"]'))
    .toContainText(ts('settings.controls.gamepad.status_ready'));
  await captain.keyboard.press('Escape');

  // Pad 1 is connected and active but unowned.
  await setPads(captain, [{ index: 0 }, { index: 1, pressed: [0] }]);
  await captain.waitForTimeout(150);
  await expect(alertButton).toHaveText(ts('component.red_alert.standby'));
  await setPads(captain, [{ index: 0 }, { index: 1 }]);
  await captain.waitForTimeout(50);

  // The selected pad's rising edge uses the real Captain authority route.
  await setPads(captain, [{ index: 0, pressed: [0] }, { index: 1 }]);
  await expect(alertButton).toHaveText(ts('component.red_alert.active'), { timeout: 10_000 });

  // Disconnect retains the dead ownership generation and never transfers to pad 1.
  await setPads(captain, [null, { index: 1, pressed: [0] }]);
  const liveWarning = captain.locator('#gamepad-input-alert');
  await expect(liveWarning).toBeVisible();
  await expect(liveWarning).toHaveAttribute('role', 'alert');
  await expect(liveWarning).toContainText(ts('client.gamepad.disconnect_warning'));
  await captain.waitForTimeout(150);
  await expect(alertButton).toHaveText(ts('component.red_alert.active'));

  // Keyboard is independent of the disconnected gamepad owner.
  await captain.keyboard.press('KeyR');
  await expect(alertButton).toHaveText(ts('component.red_alert.standby'), { timeout: 10_000 });
  await expect(liveWarning).toBeVisible();

  // Settings mirrors the same dead ownership without being needed for the
  // warning to appear.
  await captain.click('#settings-btn');
  await captain.click('.settings-tab[data-tab="controls"]');
  const warning = captain.locator('[data-control="semantic-gamepad-status"]');
  await expect(warning).toHaveAttribute('role', 'alert');
  await expect(warning).toContainText(ts('settings.controls.gamepad.status_disconnected'));
  await captain.keyboard.press('Escape');

  // A new pad at index 0 is a new generation. Explicit reselection while its
  // button is held stays neutral-gated until release and a fresh edge.
  await setPads(captain, [{ index: 0, pressed: [0] }, { index: 1 }]);
  await captain.click('#settings-btn');
  await captain.click('.settings-tab[data-tab="controls"]');
  await selector.selectOption('');
  await expect(liveWarning).toBeHidden();
  await selector.selectOption('0');
  await expect(captain.locator('[data-control="semantic-gamepad-status"]'))
    .toContainText(ts('settings.controls.gamepad.status_neutral'));
  await expect(liveWarning).toBeHidden();
  await captain.keyboard.press('Escape');
  await captain.waitForTimeout(150);
  await expect(alertButton).toHaveText(ts('component.red_alert.standby'));
  await setPads(captain, [{ index: 0 }, { index: 1 }]);
  await captain.waitForTimeout(100);
  await setPads(captain, [{ index: 0, pressed: [0] }, { index: 1 }]);
  await expect(alertButton).toHaveText(ts('component.red_alert.active'), { timeout: 10_000 });

  await captain.close();
  await serverPage.close();
});
