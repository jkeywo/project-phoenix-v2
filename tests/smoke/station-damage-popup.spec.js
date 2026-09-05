import { test, expect, createServerPage, readHostPeerId, waitForWasmReady } from './fixtures';

/**
 * The per-system damage popup on the Station Bar (PRD #1371, issue #1374).
 *
 * Every console used to end in a footer strip carrying a summed hull bar you
 * tapped for that Station's individual systems. Issue #1374 took the footers
 * off all twenty-two documents and moved the popup onto the bar: tapping the
 * tab that is ALREADY SELECTED opens it.
 *
 * Driven end to end here because the seam crosses three realms that every
 * unit test stubs one side of — the console's own `own_hull` rows, the
 * `console_hull` postMessage that carries them out of the iframe, and the
 * shell's bar deciding a tap on the selected tab means "show me the damage".
 * `tests/client/station-damage-popup.test.js` pins what the popup says;
 * `tests/client/console-chrome.test.js` pins that no console draws a footer.
 * Only here do the rows actually travel.
 *
 * The destroyer, for the same reason console-tabs.spec.js uses it: its
 * Tactical seat carries overlay tabs, so this also covers the one case the
 * rule has to get right — while an overlay IS open, the bar's selection is the
 * overlay, and a tap on the Station tab means "come back to the console"
 * rather than "open the popup".
 */

const DESTROYER_WORLD = `
[global]
seed = 42
title = "Station Damage Popup Smoke World"
description = "Minimal destroyer-hulled world for tests/smoke/station-damage-popup.spec.js."

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[[available_ships]]
template_path = "assets/entities/alliance_destroyer.toml"

[[entity]]
template_path = "assets/entities/alliance_destroyer.toml"
id            = "player-ship"
transform     = { position = [0.0, 0.0, 0.0] }
spawn_on      = "game_start"
`;

test('damage popup: the selected Station tab opens it', { tag: '@core' }, async ({ context }) => {
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: DESTROYER_WORLD }));
  const serverPage = await createServerPage(context);
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const phone = await context.newPage();
  await phone.setViewportSize({ width: 1280, height: 800 });
  await phone.goto(`/client/#${hostId}`);
  await phone.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await phone.click('#station-list .station-row:has-text("Tactical") button.claim-btn');
  await phone.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await phone.click('#ready-btn');
  await phone.waitForSelector('#station-hero[aria-hidden="false"]', { timeout: 10_000 });

  const popup = phone.locator('#station-damage-popup');
  const selectedTab = phone.locator('#station-hero-tabs [data-tab-id="tactical"]');

  // ── The console draws no footer bar of its own any more ───────────────────
  const tactical = phone.frameLocator('#weapons-iframe');
  await expect(tactical.locator('.console-footer')).toHaveCount(0);
  await expect(tactical.locator('#station-damage')).toHaveCount(0);
  await expect(popup).toBeHidden();

  // ── Tapping the already-selected tab opens it ─────────────────────────────
  await expect(selectedTab).toHaveAttribute('aria-selected', 'true');
  await selectedTab.click();
  await expect(popup).toBeVisible();
  // It names the Station it is reporting on, resolved through the string
  // table rather than showing a raw station id.
  const title = await phone.locator('#station-damage-popup-title').textContent();
  expect(title).not.toBe('');
  expect(title).not.toMatch(/^(component|station)\./);
  // The rows travelled: Tactical owns damageable systems on this hull, so the
  // detail is what shows, not the no-damage-model line.
  await expect(phone.locator('#station-damage-popup-detail')).toBeVisible();
  await expect(phone.locator('#station-damage-popup-empty')).toBeHidden();
  // And the seat did not move.
  await expect(phone.locator('#weapons-ui')).toHaveClass(/\bactive\b/);

  // ── Escape closes it, so a keyboard is never stranded inside ──────────────
  await phone.keyboard.press('Escape');
  await expect(popup).toBeHidden();

  // ── The close button closes it too ────────────────────────────────────────
  await selectedTab.click();
  await expect(popup).toBeVisible();
  await phone.click('#station-damage-popup-close');
  await expect(popup).toBeHidden();

  // ── While an overlay is open, the same tap means "come back" ──────────────
  await phone.click('#station-hero-tabs [data-tab-id="intel-overlay"]');
  await expect(tactical.locator('#intel-overlay')).toHaveClass(/\bopen\b/);
  await selectedTab.click();
  await expect(tactical.locator('#intel-overlay')).not.toHaveClass(/\bopen\b/);
  await expect(popup).toBeHidden();

  // ── A tap on a DIFFERENT Station's tab is a seat change, not a popup ──────
  await phone.click('#station-hero-tabs [data-tab-id="navigation"]');
  await expect(popup).toBeHidden();
  await expect(phone.locator('#navigation-ui')).toHaveClass(/\bactive\b/);

  await phone.close();
});
