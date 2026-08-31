// Issues #1274/#1275 tracer: a real client refuses a reserved chord, resolves
// an overlapping Captain binding, resets the profile, then remaps Red Alert.
// The remapped key traverses the existing action-map, transport,
// command-admission and Captain server path. The component is not painted
// optimistically, so its eventual active state is authoritative read-back.

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

  const alertButton = captain.frameLocator('#captain-iframe')
    .locator('ph-red-alert').locator('#alert-btn');
  const alertFeedback = captain.frameLocator('#captain-iframe')
    .locator('ph-red-alert').locator('#feedback-status');
  await expect(alertButton).toBeEnabled();
  await expect(alertButton).toHaveText(ts('component.red_alert.standby'));

  await captain.click('#settings-btn');
  await captain.click('.settings-tab[data-tab="controls"]');
  const binding = captain.locator(
    '[data-control="semantic-binding-captain.red-alert-0"]',
  );
  const holdBinding = captain.locator(
    '[data-control="semantic-binding-captain.weapons-hold-0"]',
  );
  await expect(binding).toHaveValue('R');
  await expect(holdBinding).toHaveValue('H');

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
  await holdBinding.click();
  await holdBinding.press('KeyR');
  await expect(captain.locator('[data-control="semantic-binding-conflict-cancel"]'))
    .toBeFocused();
  // Conflict Escape is modal-wide: move focus out of the prompt, then cancel
  // without letting the shared Settings trap close the modal. Do not encode a
  // fixed Tab count: the registry and private-profile sections grow as actions
  // and portable settings are delivered.
  const resetAll = captain.locator('[data-control="semantic-binding-reset-all"]');
  await resetAll.focus();
  await expect(resetAll).toBeFocused();
  await captain.keyboard.press('Escape');
  await expect(captain.locator('#settings-overlay')).toBeVisible();
  await expect(captain.locator('.settings-binding-conflict')).toHaveCount(0);
  await expect(holdBinding).toBeFocused();
  await expect(holdBinding).toHaveValue(ts('settings.controls.press_key'));
  await holdBinding.blur();
  await expect(holdBinding).toHaveValue('H');

  await holdBinding.click();
  await holdBinding.press('KeyR');
  await expect(captain.locator('[data-control="semantic-binding-conflict-cancel"]'))
    .toBeFocused();
  await captain.click('[data-control="semantic-binding-conflict-replace"]');
  await expect(binding).toHaveValue(ts('input.binding.unassigned'));
  await expect(holdBinding).toBeFocused();
  await expect(holdBinding).toHaveValue(ts('settings.controls.press_key'));
  await holdBinding.blur();
  await expect(holdBinding).toHaveValue('R');
  await captain.click('[data-control="semantic-binding-reset-captain.red-alert"]');
  await expect(binding).toHaveValue('R');
  await expect(holdBinding).toHaveValue(ts('input.binding.unassigned'));
  await captain.click('[data-control="semantic-binding-reset-all"]');
  await expect(binding).toHaveValue('R');
  await expect(holdBinding).toHaveValue('H');

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
  await captain.keyboard.press('KeyU');

  await expect(alertButton).toHaveText(ts('component.red_alert.active'), {
    timeout: 10_000,
  });
  await expect(alertButton).toHaveClass(/active/);
  await expect(alertFeedback).toHaveText(ts('action_feedback.applied'));

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
