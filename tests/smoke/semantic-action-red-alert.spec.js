// Issue #1274 tracer: a real client remaps the Captain Red Alert semantic
// action, then the remapped key traverses the existing action-map, transport,
// command-admission and Captain server path. The component is not painted
// optimistically, so its eventual active state is authoritative read-back.

import { test, expect, readHostPeerId, createServerPage } from './fixtures';
import { ts } from './strings';

test('remapped Captain Red Alert binding reaches the authoritative command path', async ({ context }) => {
  test.setTimeout(120_000);

  const serverPage = await createServerPage(context);
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
  await expect(alertButton).toBeEnabled();
  await expect(alertButton).toHaveText(ts('component.red_alert.standby'));

  await captain.click('#settings-btn');
  await captain.click('.settings-tab[data-tab="controls"]');
  const binding = captain.locator(
    '[data-control="semantic-binding-captain.red-alert-0"]',
  );
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

  await captain.close();
  await serverPage.close();
});
