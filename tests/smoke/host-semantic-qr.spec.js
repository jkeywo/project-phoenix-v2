// Issue #1281 — one real host-chrome action crosses the shared semantic input
// and feedback path. This browser proof uses server.html's actual QR closure,
// not a stub: visible Enter/Space and a remapped key must reach the same adapter,
// while persistent on/off and aria-pressed continue to follow host readback.

import { test, expect, waitForWasmReady } from './fixtures';
import { ts } from './strings';

const settings = (page) => page.locator('#server-settings-btn');
const tab = (page, id) => page.locator(`.server-settings-tab[data-tab="${id}"]`);
const control = (page, id) => page.locator(`[data-control="${id}"]`);

async function qrReadback(page) {
  return page.evaluate(() => window.__hostIsQrVisible());
}

test('host QR button and closed-Settings remap share local lifecycle and readback', async ({ context }) => {
  test.setTimeout(120_000);
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page, 90_000);
  await settings(page).waitFor({ state: 'visible' });
  // Lobby deliberately keeps the join code shown. During a mission the
  // shared QR law yields to operator toggles, which is this test's contract.
  await page.locator('#ai-launch-btn').click();
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

  await page.evaluate(() => {
    window.__hostQrFeedback = [];
    window.addEventListener('phoenix-action-feedback', (event) => {
      if (event.detail && event.detail.actionId === 'host.qr-code') {
        window.__hostQrFeedback.push({
          state: event.detail.state,
          correlation: event.detail.correlation,
        });
      }
    });
  });

  const initial = await qrReadback(page);
  await settings(page).click();
  await tab(page, 'gameplay').click();
  const qr = control(page, 'qr-code');
  await expect(qr).toHaveAttribute('aria-pressed', initial ? 'true' : 'false');

  // Native keyboard activation of the visible button uses its click handler,
  // which is the same semantic adapter the remapped document shortcut uses.
  await qr.focus();
  await qr.press('Enter');
  await expect.poll(() => qrReadback(page)).toBe(!initial);
  await expect(qr).toHaveAttribute('aria-pressed', initial ? 'false' : 'true');
  await expect(page.locator('#host-action-feedback')).toHaveAttribute('data-state', 'Applied');
  await expect(page.locator('#host-action-feedback')).toContainText(ts('action_feedback.applied'));

  const firstTransitions = await page.evaluate(() => window.__hostQrFeedback.slice());
  expect(firstTransitions.map(({ state }) => state)).toEqual(['Pressed', 'Pending', 'Applied']);
  expect(new Set(firstTransitions.map(({ correlation }) => correlation)).size).toBe(1);

  await qr.press('Space');
  await expect.poll(() => qrReadback(page)).toBe(initial);
  await expect(qr).toHaveAttribute('aria-pressed', initial ? 'true' : 'false');

  // Remap KeyQ -> KeyY through the host's shared Controls presenter.
  await tab(page, 'controls').click();
  const capture = control(page, 'semantic-binding-host.qr-code-0');
  await expect(capture).toHaveValue('Q');
  await capture.focus();
  await capture.press('y');
  await expect(control(page, 'semantic-binding-host.qr-code-0')).toHaveValue('Y');
  await settings(page).click();
  await expect(page.locator('#server-settings-overlay')).toBeHidden();

  const beforeRemappedKey = await qrReadback(page);
  await page.keyboard.press('q');
  await expect.poll(() => qrReadback(page)).toBe(beforeRemappedKey);
  await page.keyboard.press('y');
  await expect.poll(() => qrReadback(page)).toBe(!beforeRemappedKey);
  await expect(page.locator('#host-action-feedback')).toBeVisible();
  await expect(page.locator('#host-action-feedback')).toHaveAttribute('data-state', 'Applied');

  // A non-registry host path can still change the QR. Reopening Gameplay must
  // read that existing truth; no semantic-action-owned on/off state may win.
  await page.evaluate(() => window.__hostToggleQrCode());
  await settings(page).click();
  await tab(page, 'gameplay').click();
  await expect.poll(async () => {
    const [ariaPressed, visible] = await Promise.all([
      qr.getAttribute('aria-pressed'),
      qrReadback(page),
    ]);
    return ariaPressed === (visible ? 'true' : 'false');
  }).toBe(true);
});
