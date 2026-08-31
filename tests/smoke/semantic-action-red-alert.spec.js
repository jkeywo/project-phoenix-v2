// Issues #1274/#1275 tracer: a real client refuses a reserved chord, resolves
// an overlapping Captain binding, resets the profile, then remaps Red Alert.
// The remapped key traverses the existing action-map, transport,
// command-admission and Captain server path. The component is not painted
// optimistically, so its eventual active state is authoritative read-back.

import { test, expect, readHostPeerId, waitForWasmReady } from './fixtures';
import { ts } from './strings';

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
  // Conflict Escape is modal-wide: move backward out of the prompt to Reset
  // All, then cancel without letting the shared Settings trap close the modal.
  await captain.keyboard.press('Shift+Tab');
  await captain.keyboard.press('Shift+Tab');
  await expect(captain.locator('[data-control="semantic-binding-reset-all"]'))
    .toBeFocused();
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

  // Close Settings so the key relay may hand the host-page event to the
  // active Captain iframe. The remap capture itself stops propagation, so the
  // capture key above cannot also fire the action.
  await captain.keyboard.press('Escape');
  await expect(captain.locator('#settings-overlay')).toBeHidden();
  await captain.keyboard.press('KeyY');

  await expect(alertButton).toHaveText(ts('component.red_alert.active'), {
    timeout: 10_000,
  });
  await expect(alertButton).toHaveClass(/active/);
  await expect(alertFeedback).toHaveText(ts('action_feedback.applied'));

  await captain.close();
  await serverPage.close();
});
