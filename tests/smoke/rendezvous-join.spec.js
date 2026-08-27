// Issues #1111/#1112 — the typed five-letter join, end to end in a browser.
//
// One host page registers with a rendezvous service and is issued a code; a
// separate client page types those five letters, resolves the host, opens a
// direct reliable channel and completes the ordinary Identify→Welcome flow.
// The rendezvous service is tests/smoke/rendezvous-shim.js, which runs the
// REAL worker-rendezvous registry in the host page over a fake socket, and
// pairs the two pages' DataChannels over a BroadcastChannel — CI has no real
// WebRTC and no deployed worker.
//
// Nothing here opts in any more. #1112 made this THE route: the host registers
// on an ordinary page load and the client page asks for five letters with no
// parameter set, which is exactly what these specs now exercise.

import { test, expect, waitForWasmReady } from './fixtures';
import { ts } from './strings';

async function bootHost(context) {
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await page.waitForFunction(
    () => /^[A-Z]{5}$/.test(document.getElementById('join-code')?.textContent ?? ''),
    { timeout: 30_000 },
  );
  return page;
}

const joinCodeOn = (page) => page.evaluate(() => document.getElementById('join-code').textContent);
const joinLinkOn = (page) => page.evaluate(() => document.getElementById('qr-link').href);

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

const errorText = (page) =>
  page.evaluate(() => document.getElementById('join-entry-error')?.textContent ?? '');

test('the host shows five letters a guest can read across the room', async ({ context }) => {
  const host = await bootHost(context);
  const code = await joinCodeOn(host);
  expect(code).toMatch(/^[A-Z]{5}$/);
  // The link the QR encodes carries the whole structured identifier, so a
  // camera scan and a typed suffix are the same join by two routes.
  expect(await joinLinkOn(host)).toContain(`#`);
  expect((await joinLinkOn(host)).split('#')[1]).toMatch(new RegExp(`_${code}$`));
});

test('typing the five letters reaches Welcome over a direct channel', async ({ context }) => {
  const host = await bootHost(context);
  const code = await joinCodeOn(host);

  const client = await openClient(context);
  await expect(client.locator('#join-entry')).toBeVisible();

  // Typed the way a guest would: lower case, because the code on screen is
  // upper case and nobody switches their phone keyboard for it.
  await client.fill('#join-code-input', code.toLowerCase());
  await client.click('#join-submit-btn');

  await waitForConnected(client);
  await expect(client.locator('#join-entry')).toBeHidden();

  // The host really did admit this phone as a player, not merely answer it.
  const token = await client.evaluate(() => sessionStorage.getItem('session-token'));
  expect(token).toBeTruthy();
  await host.waitForFunction(
    (t) => {
      try {
        // eslint-disable-next-line no-eval
        return (0, eval)('tokenConns').has(t);
      } catch { return false; }
    },
    token,
    { timeout: 10_000 },
  );
});

test('a QR link joins without anything being typed', async ({ context }) => {
  const host = await bootHost(context);
  const link = await joinLinkOn(host);

  // What the phone's own camera app opens — there is no scanner in the
  // product, so "QR entry" is exactly this URL arriving in the address bar.
  const client = await context.newPage();
  await client.goto(link);

  await waitForConnected(client);
  await expect(client.locator('#join-entry')).toBeHidden();
});

test('an unknown code says so and leaves the guest in front of the field', async ({ context }) => {
  await bootHost(context);
  const client = await openClient(context);

  await client.fill('#join-code-input', 'ZZZZZ');
  await client.click('#join-submit-btn');

  await expect(client.locator('#join-entry-error')).toHaveText(ts('client.join.error_unknown'), {
    timeout: 15_000,
  });
  await expect(client.locator('#join-entry')).toBeVisible();
});

test('a refused word is refused with its own message, not as an unknown code', async ({ context }) => {
  await bootHost(context);
  const client = await openClient(context);

  await client.fill('#join-code-input', 'ADMIN');
  await client.click('#join-submit-btn');

  await expect(client.locator('#join-entry-error')).toHaveText(ts('client.join.error_denied'));
  expect(await errorText(client)).not.toBe(ts('client.join.error_unknown'));
});

test('too few letters is its own message too', async ({ context }) => {
  await bootHost(context);
  const client = await openClient(context);

  await client.fill('#join-code-input', 'ABC');
  await client.click('#join-submit-btn');

  await expect(client.locator('#join-entry-error')).toHaveText(ts('client.join.error_length'));
});
