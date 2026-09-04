// Issue #1115 — the settings cog's rotate lever, end to end in a real browser.
//
// Everything below the page is the shipped code: the REAL
// worker-rendezvous/src/registry.js and the real join-code table. Only the
// socket and the RTCPeerConnection are faked (tests/smoke/rendezvous-shim.js),
// because CI has no WebRTC and no deployed worker.

import { test, expect, waitForWasmReady, waitForJoinCode } from './fixtures';
import { ts } from './strings';

/** A ship host, booted straight into its lobby, with its crew code on screen. */
async function bootHost(context) {
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await waitForJoinCode(page, 'join-code', 30_000);
  return page;
}

/** Open the cog on its Gameplay tab, where both rotate levers live. */
async function openGameplayTab(page) {
  await page.click('#server-settings-btn');
  await page.click('.server-settings-tab[data-tab="gameplay"]');
  await page.waitForSelector('[data-control="rotate-join-code"]', { state: 'attached' });
}

const closeCog = (page) => page.keyboard.press('Escape');

async function openClient(context, search = '') {
  const page = await context.newPage();
  await page.goto(`/client/${search}`);
  return page;
}

const waitForConnected = (page) =>
  page.waitForFunction(
    (expected) => document.getElementById('status')?.textContent === expected,
    ts('client.status_connected'),
    { timeout: 30_000 },
  );

test('rotating the crew code invalidates the old one and a phone joins on the new one', async ({ context }) => {
  const host = await bootHost(context);
  const before = await host.evaluate(() => document.getElementById('join-code').textContent);

  await openGameplayTab(host);
  await expect(host.locator('[data-control="join-code-readout"]')).toHaveText(before);
  await expect(host.locator('[data-control="rotate-join-code"]')).toBeEnabled();
  await host.click('[data-control="rotate-join-code"]');

  // The cog's own readout repaints with the new suffix…
  await host.waitForFunction(
    (old) => {
      const el = document.querySelector('[data-control="join-code-readout"]');
      return !!el && el.textContent && el.textContent !== old;
    },
    before,
    { timeout: 30_000 },
  );
  const after = await host.locator('[data-control="join-code-readout"]').textContent();
  expect(after).not.toBe(before);
  await closeCog(host);

  // …and so does the viewscreen panel, the same code by the other route.
  await expect(host.locator('#join-code')).toHaveText(after);

  // The OLD code is dead immediately — not a grace hold, a real drop.
  const staleClient = await openClient(context);
  await staleClient.fill('#join-code-input', before.toLowerCase());
  await staleClient.click('#join-submit-btn');
  await expect(staleClient.locator('#join-entry-error')).toHaveText(ts('client.join.error_unknown'));

  // The NEW code joins normally.
  const client = await openClient(context);
  await client.fill('#join-code-input', after.toLowerCase());
  await client.click('#join-submit-btn');
  await waitForConnected(client);
  await expect(client.locator('#join-entry')).toBeHidden();
});

test('rotating the fleet code leaves the crew code untouched, and vice versa (AC4)', async ({ context }) => {
  const host = await bootHost(context);
  const crewBefore = await host.evaluate(() => document.getElementById('join-code').textContent);

  await openGameplayTab(host);
  await host.click('[data-control="fleet-open"]');
  await waitForJoinCode(host, 'fleet-code', 30_000);
  const fleetBefore = await host.evaluate(() => document.getElementById('fleet-code').textContent);
  await expect(host.locator('[data-control="fleet-rotate"]')).toBeEnabled();

  // Rotating the FLEET code…
  await host.click('[data-control="fleet-rotate"]');
  await host.waitForFunction(
    (old) => document.getElementById('fleet-code')?.textContent !== old,
    fleetBefore,
    { timeout: 30_000 },
  );
  const fleetAfter = await host.evaluate(() => document.getElementById('fleet-code').textContent);
  expect(fleetAfter).not.toBe(fleetBefore);
  // …never touches the crew code sitting right beside it.
  expect(await host.evaluate(() => document.getElementById('join-code').textContent)).toBe(crewBefore);

  // And the reverse: rotating the CREW code never touches the fleet code.
  await host.click('[data-control="rotate-join-code"]');
  await host.waitForFunction(
    (old) => document.getElementById('join-code')?.textContent !== old,
    crewBefore,
    { timeout: 30_000 },
  );
  expect(await host.evaluate(() => document.getElementById('fleet-code').textContent)).toBe(fleetAfter);
});
