import { test, expect, createServerPage, readHostPeerId, waitForWasmReady } from './fixtures';

/**
 * The overlay-tab seam (PRD #1371, issue #1373): a console declares its
 * overlay panels, the shell's Station Bar renders them as tabs between the
 * direct Station and its visitors, and selecting one opens that panel INSIDE
 * the console's own iframe.
 *
 * This drives the whole seam in a real browser — a real client page, a real
 * console iframe, real postMessage — because every unit test on either side
 * of it stubs the other. `tests/client/console-core-tabs.test.js` pins what a
 * console declares, `tests/client/hero-bar.test.js` pins what the bar draws
 * from a declaration; only here do the two actually meet.
 *
 * The destroyer, because it is the hull whose Tactical seat has the two
 * overlays (Intel, issue #1030; Security, issue #1346) — and, with only
 * Tactical seated, it also hosts Navigation and Comms as visiting tabs
 * (`host_order` starts with `tactical` for both), which is what makes
 * "between the direct Station and the visiting Stations" a real position
 * rather than "at the end".
 */

// Minimal destroyer-hulled world, the same shape comms-visiting-station.spec.js
// serves: this spec needs the destroyer's `[[station]]` topology, not the
// cruiser the shared default world spawns.
const DESTROYER_WORLD = `
[global]
seed = 42
title = "Console Tabs Smoke World"
description = "Minimal destroyer-hulled world for tests/smoke/console-tabs.spec.js."

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

test('overlay tabs: Tactical declares them, the bar selects them', { tag: '@core' }, async ({ context }) => {
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: DESTROYER_WORLD }));
  const serverPage = await createServerPage(context);
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const phone = await context.newPage();
  // Wide enough that the tabs show full names; the codes are the same tabs.
  await phone.setViewportSize({ width: 1280, height: 800 });
  await phone.goto(`/client/#${hostId}`);
  await phone.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await phone.click('#station-list .station-row:has-text("Tactical") button.claim-btn');
  await phone.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await phone.click('#ready-btn');
  await phone.waitForSelector('#station-hero[aria-hidden="false"]', { timeout: 10_000 });

  const tabIds = () => phone.evaluate(() =>
    [...document.querySelectorAll('#station-hero-tabs button[data-tab-id]')]
      .map((b) => b.dataset.tabId));

  // ── The declaration reaches the bar, in the right place ───────────────────
  // Overlay tabs sit between the seat and the Stations visiting it. They came
  // from the Tactical document's own `.overlay-panel[data-tab-code]` markup.
  await expect.poll(tabIds, { timeout: 10_000 })
    .toEqual(['tactical', 'security-overlay', 'intel-overlay', 'navigation', 'comms']);

  const shape = await phone.evaluate(() => {
    const button = (id) => document.querySelector(`#station-hero-tabs [data-tab-id="${id}"]`);
    return {
      kinds: ['tactical', 'intel-overlay', 'navigation'].map((id) => button(id).dataset.tabKind),
      // An overlay tab names its panel; only a Station tab is a Station.
      overlayAttr: button('intel-overlay').dataset.overlay,
      overlayIsNotAStation: button('intel-overlay').dataset.station === undefined,
      // Resolved through the string table, not the raw ids the console posted.
      labels: ['security-overlay', 'intel-overlay'].map((id) => button(id).children[0].textContent),
      // No badge until Intel actually has something unread on file.
      badge: button('intel-overlay').dataset.badge,
      selected: button('tactical').getAttribute('aria-selected'),
    };
  });
  expect(shape.kinds).toEqual(['station', 'overlay', 'station']);
  expect(shape.overlayAttr).toBe('intel-overlay');
  expect(shape.overlayIsNotAStation).toBe(true);
  for (const label of shape.labels) expect(label).not.toMatch(/^console\./);
  expect(shape.badge).toBe('0');
  expect(shape.selected).toBe('true');

  // ── Selecting an overlay tab opens that panel inside the iframe ───────────
  const tactical = phone.frameLocator('#weapons-iframe');
  await expect(tactical.locator('#intel-overlay')).not.toHaveClass(/\bopen\b/);

  await phone.click('#station-hero-tabs [data-tab-id="intel-overlay"]');
  await expect(tactical.locator('#intel-overlay')).toHaveClass(/\bopen\b/);
  await expect(phone.locator('#station-hero-tabs [data-tab-id="intel-overlay"]'))
    .toHaveAttribute('aria-selected', 'true');
  // The seat did not move: the Tactical console is still the one on screen.
  await expect(phone.locator('#weapons-ui')).toHaveClass(/\bactive\b/);

  // One panel at a time — selecting the other overlay swaps them.
  await phone.click('#station-hero-tabs [data-tab-id="security-overlay"]');
  await expect(tactical.locator('#security-overlay')).toHaveClass(/\bopen\b/);
  await expect(tactical.locator('#intel-overlay')).not.toHaveClass(/\bopen\b/);

  // ── Selecting a Station tab closes it ─────────────────────────────────────
  await phone.click('#station-hero-tabs [data-tab-id="tactical"]');
  await expect(tactical.locator('#security-overlay')).not.toHaveClass(/\bopen\b/);
  await expect(phone.locator('#station-hero-tabs [data-tab-id="tactical"]'))
    .toHaveAttribute('aria-selected', 'true');

  // ── Closing from inside the console un-selects the tab ────────────────────
  // The panel still draws its own Back button (the console-side chrome goes in
  // issue #1374), and the document is the truth about what is covering the
  // console — so the bar must follow it, not the other way round.
  await phone.click('#station-hero-tabs [data-tab-id="intel-overlay"]');
  await expect(tactical.locator('#intel-overlay')).toHaveClass(/\bopen\b/);
  await tactical.locator('#intel-overlay [data-overlay-back]').click();
  await expect(phone.locator('#station-hero-tabs [data-tab-id="intel-overlay"]'))
    .toHaveAttribute('aria-selected', 'false');
  await expect(phone.locator('#station-hero-tabs [data-tab-id="tactical"]'))
    .toHaveAttribute('aria-selected', 'true');

  // ── Only the ACTIVE console's declaration is rendered ─────────────────────
  // Every mounted console posts its own tabs; Navigation declares none, so
  // moving to it must empty the overlay track rather than leave Tactical's
  // tabs behind on somebody else's console.
  await phone.click('#station-hero-tabs [data-tab-id="navigation"]');
  await expect.poll(tabIds, { timeout: 5_000 }).toEqual(['tactical', 'navigation', 'comms']);
  await phone.click('#station-hero-tabs [data-tab-id="tactical"]');
  await expect.poll(tabIds, { timeout: 5_000 })
    .toEqual(['tactical', 'security-overlay', 'intel-overlay', 'navigation', 'comms']);

  // ── An iframe reload re-posts, and the bar recovers ───────────────────────
  await phone.click('#station-hero-tabs [data-tab-id="intel-overlay"]');
  await expect(tactical.locator('#intel-overlay')).toHaveClass(/\bopen\b/);
  await phone.evaluate(() => {
    document.getElementById('weapons-iframe').contentWindow.location.reload();
  });
  // The reloaded document has every panel closed, and says so by re-declaring:
  // the tabs come back and the shell has dropped the selection it was holding.
  await expect.poll(tabIds, { timeout: 10_000 })
    .toEqual(['tactical', 'security-overlay', 'intel-overlay', 'navigation', 'comms']);
  await expect(phone.locator('#station-hero-tabs [data-tab-id="tactical"]'))
    .toHaveAttribute('aria-selected', 'true');
  await expect(tactical.locator('#intel-overlay')).not.toHaveClass(/\bopen\b/);

  await phone.close();
});
