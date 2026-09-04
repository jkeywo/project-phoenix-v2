// Issues #1111/#1112 — the typed join, end to end in a browser.
//
// One host page registers with a rendezvous service and is issued a code; a
// separate client page types that code, resolves the host, opens a
// direct reliable channel and completes the ordinary Identify→Welcome flow.
// The rendezvous service is tests/smoke/rendezvous-shim.js, which runs the
// REAL worker-rendezvous registry in the host page over a fake socket, and
// pairs the two pages' DataChannels over a BroadcastChannel — CI has no real
// WebRTC and no deployed worker.
//
// Nothing here opts in any more. #1112 made this THE route: the host registers
// on an ordinary page load and the client page asks for the code with no
// parameter set, which is exactly what these specs now exercise.

import { test, expect, waitForWasmReady, waitForJoinCode, JOIN_CODE_PATTERN, JOIN_CODE_LENGTH } from './fixtures';
import { ts } from './strings';

async function bootHost(context) {
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await waitForJoinCode(page, 'join-code', 30_000);
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

test('the host shows a code a guest can read across the room', async ({ context }) => {
  const host = await bootHost(context);
  const code = await joinCodeOn(host);
  expect(code).toMatch(JOIN_CODE_PATTERN);
  // The link the QR encodes carries the whole structured identifier, so a
  // camera scan and a typed suffix are the same join by two routes.
  const link = await joinLinkOn(host);
  expect(link).toContain(`#`);
  expect(link.split('#')[1]).toMatch(new RegExp(`_${code}$`));
  // …and the draw was actually REACHED, with that URL. The stub encoder in
  // fixtures.js records every draw it is asked for, so this pins the pixels a
  // phone points a camera at to the link the page prints beneath them — issue
  // #1329's AC5, where the draw used to sit in a PeerJS callback #1112 deleted
  // and nothing since had checked its replacement was on the boot path.
  expect(await host.evaluate(() => window.__qrDraws)).toContain(link);
});

test('typing the code reaches Welcome over a direct channel', { tag: '@core' }, async ({ context }) => {
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

// Both codes below are built from the authored length, not typed out: at five
// letters they were well-formed codes, but the suffix is eight now and a short
// entry is refused for its LENGTH before the registry ever looks it up — which
// silently turned both of these into a different test than the one they name.
const UNKNOWN_CODE = 'Z'.repeat(JOIN_CODE_LENGTH);
// A deny-list word is matched ANYWHERE INSIDE the canonicalised suffix
// (assets/join/join-codes.toml says so), so padding keeps this a DENIED code
// rather than an unknown one.
const DENIED_CODE = ('ADMIN' + 'BCDEFGHIJK').slice(0, JOIN_CODE_LENGTH);

test('an unknown code says so and leaves the guest in front of the field', async ({ context }) => {
  await bootHost(context);
  const client = await openClient(context);

  await client.fill('#join-code-input', UNKNOWN_CODE);
  await client.click('#join-submit-btn');

  await expect(client.locator('#join-entry-error')).toHaveText(ts('client.join.error_unknown'), {
    timeout: 15_000,
  });
  await expect(client.locator('#join-entry')).toBeVisible();
});

test('a refused word is refused with its own message, not as an unknown code', async ({ context }) => {
  await bootHost(context);
  const client = await openClient(context);

  await client.fill('#join-code-input', DENIED_CODE);
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

// ── Reclaim across a transient signalling loss (issue #1115) ────────────────

test('a host that loses its socket reclaims the SAME code, and a phone still joins on it', async ({ context }) => {
  const host = await bootHost(context);
  const before = await joinCodeOn(host);

  // The whole page goes offline — every socket and channel it holds dies at
  // once, without either end calling close() — the closest a browser test can
  // get to the transient losses issue #1115 is written for: a Durable Object
  // hiccup, a phone radio killing a backgrounded tab's WS. The join panel
  // blanks while the host is out of reach…
  await host.evaluate(() => window.__transportShim.sever());
  await expect(host.locator('#join-code-row')).toBeHidden();
  await host.evaluate(() => window.__transportShim.revive());

  // …and comes back with the SAME letters, reclaimed with the secret the
  // host's own earlier registration was issued — not a fresh set nobody in
  // the room has read yet.
  await host.waitForFunction(
    (expected) => document.getElementById('join-code')?.textContent === expected,
    before,
    { timeout: 30_000 },
  );

  const client = await openClient(context);
  await client.fill('#join-code-input', before.toLowerCase());
  await client.click('#join-submit-btn');
  await waitForConnected(client);
  await expect(client.locator('#join-entry')).toBeHidden();
});
