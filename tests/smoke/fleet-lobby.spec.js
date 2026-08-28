// Issue #1114 — two ship hosts assembling a fleet, in two real browser pages.
//
// Everything below the page is the shipped code: the REAL
// worker-rendezvous/src/registry.js, the real join-code table, the real
// host-mesh vocabulary and the real compatibility handshake. Only the socket
// and the RTCPeerConnection are faked (tests/smoke/rendezvous-shim.js), because
// CI has no WebRTC and no deployed worker.
//
// That shim used to hand registry ownership to "the first page that opens a
// /v1/host socket", which was enough while the product was one host and N
// phones. Two SHIP HOSTS both took it and each ran a registry the other could
// not see — so the election it grew for this spec is part of what is under
// test here: if it regressed, the fleet code minted on one page would resolve
// to nothing on the other and the first test would fail on the roster.

import { readFileSync } from 'node:fs';
import * as path from 'node:path';
import { test, expect, waitForWasmReady, createTestClient, readHostPeerId } from './fixtures';
import { ts } from './strings';

/** The authored fleet capacity, read rather than pinned. */
const MAX_FLEET_HOSTS = JSON.parse(
  readFileSync(path.resolve(__dirname, '../../assets/join/join-codes.json'), 'utf8'),
).limits.max_fleet_hosts;

/** A ship host, booted straight into its lobby, with its crew code on screen. */
async function bootHost(context, fragment = '') {
  const page = await context.newPage();
  await page.goto(`/?scenario=assets/worlds/default.toml${fragment}`);
  await waitForWasmReady(page);
  await page.waitForFunction(
    () => /^[A-Z]{5}$/.test(document.getElementById('join-code')?.textContent ?? ''),
    { timeout: 30_000 },
  );
  return page;
}

/** Open the cog on its Gameplay tab, where the fleet controls live. */
async function openFleetTab(page) {
  await page.click('#server-settings-btn');
  await page.click('.server-settings-tab[data-tab="gameplay"]');
  // Attached, not visible: which of these controls is on screen is the whole
  // point of the section, and a host already in a fleet hides the open button.
  await page.waitForSelector('[data-control="fleet-code"]', { state: 'attached' });
}

const closeCog = (page) => page.keyboard.press('Escape');

/** Mint this session's fleet code through the operator's own control. */
async function openFleet(page) {
  await openFleetTab(page);
  await page.click('[data-control="fleet-open"]');
  await page.waitForFunction(
    () => /^[A-Z]{5}$/.test(document.getElementById('fleet-code')?.textContent ?? ''),
    { timeout: 30_000 },
  );
  await closeCog(page);
  return page.evaluate(() => ({
    suffix: document.getElementById('fleet-code').textContent,
    url: document.getElementById('fleet-url').textContent,
  }));
}

/** Type a code into the cog's fleet field and submit it. */
async function typeFleetCode(page, code) {
  await openFleetTab(page);
  await page.fill('[data-control="fleet-code"]', code);
  await page.click('[data-control="fleet-join"]');
  await closeCog(page);
}

/** The fleet panel as the operator reads it. */
const fleetPanel = (page) =>
  page.evaluate(() => ({
    code: document.getElementById('fleet-code')?.textContent ?? '',
    url: document.getElementById('fleet-url')?.textContent ?? '',
    status: document.getElementById('fleet-status')?.textContent ?? '',
    error: document.getElementById('fleet-error')?.textContent ?? '',
    slots: [...document.querySelectorAll('#fleet-slots li')].map((li) => ({
      text: li.textContent,
      mine: li.classList.contains('mine'),
    })),
  }));

const waitForSlots = (page, n) =>
  page.waitForFunction(
    (count) => document.querySelectorAll('#fleet-slots li').length === count,
    n,
    { timeout: 30_000 },
  );

const waitForFleetError = (page) =>
  page.waitForFunction(
    () => (document.getElementById('fleet-error')?.textContent ?? '').length > 0,
    { timeout: 30_000 },
  );

// Every test here boots at least two WASM host pages and then talks between
// them. The suite's 60s default is a budget for ONE host booting, and this file
// runs after ~40 other specs on a single worker — so each test buys its own
// budget rather than relying on the default. Assertions that belong to one
// scenario are kept in ONE test for the same reason: a second test that has to
// stand up the same fleet again is two more WASM boots for one more assertion.
const FLEET_TIMEOUT = 240_000;

test('two ship hosts assemble a fleet, and each crew star stays on its own host', async ({ context }) => {
  test.setTimeout(FLEET_TIMEOUT);
  const lead = await bootHost(context);
  const fleet = await openFleet(lead);
  expect(fleet.suffix).toMatch(/^[A-Z]{5}$/);
  // The fleet code is NOT the crew code: two namespaces, two records, and the
  // link the second operator opens is this page, not client/index.html.
  const crew = await readHostPeerId(lead);
  expect(fleet.url).toContain('#');
  expect(fleet.url).not.toContain('client/index.html');
  expect(fleet.url.split('#')[1]).toMatch(new RegExp(`_${fleet.suffix}$`));
  expect(crew).not.toContain(fleet.suffix);

  // The second machine opens the link off the first one's viewscreen.
  const second = await bootHost(context, `#${fleet.url.split('#')[1]}`);

  await waitForSlots(lead, 2);
  await waitForSlots(second, 2);

  const onLead = await fleetPanel(lead);
  const onSecond = await fleetPanel(second);
  expect(onLead.slots.map((s) => s.mine)).toEqual([true, false]);
  expect(onSecond.slots.map((s) => s.mine)).toEqual([false, true]);
  expect(onLead.slots[0].text).toContain(ts('server.fleet.slot_owner', { n: '1' }));
  expect(onLead.slots[1].text).toContain(ts('server.fleet.slot_member', { n: '2' }));
  expect(onLead.status).toBe(
    ts('server.fleet.open', { n: '2', max: String(MAX_FLEET_HOSTS) }),
  );
  // Both hosts read out the same five letters — it is the fleet's code, and a
  // member reading it aloud invites a third ship to the same lead. What only
  // the ISSUING host carries is the invitation link and its QR: a member
  // repainting one would be handing out a record it does not hold.
  expect(onSecond.code).toBe(fleet.suffix);
  expect(onSecond.url).toBe('');
  expect(onLead.url).toBe(fleet.url);
  expect(onLead.error).toBe('');

  // ── and a phone joins the LEAD's ship with the lead's own crew code ──────
  const phone = await createTestClient(context, crew, {
    token: 'fleet-crew-a',
    name: 'Crewman',
  });

  const tokensOn = (page) =>
    page.evaluate(() => {
      try {
        // eslint-disable-next-line no-eval
        return [...(0, eval)('tokenConns').keys()];
      } catch { return null; }
    });

  await lead.waitForFunction(
    (t) => {
      try {
        // eslint-disable-next-line no-eval
        return (0, eval)('tokenConns').has(t);
      } catch { return false; }
    },
    phone.token,
    { timeout: 15_000 },
  );

  // The second host is in the same fleet and knows nothing about that phone:
  // the fleet link is a different socket in a different namespace carrying a
  // different protocol, so there is no filtering for this to depend on.
  expect(await tokensOn(second)).toEqual([]);
  await phone.close();
});

test('a code entered into the wrong typed field is refused as wrong-type', async ({ context }) => {
  test.setTimeout(FLEET_TIMEOUT);
  const lead = await bootHost(context);
  const fleet = await openFleet(lead);

  // A phone typing the FLEET code into the crew field.
  const phone = await context.newPage();
  await phone.goto('/client/');
  await phone.fill('#join-code-input', fleet.suffix.toLowerCase());
  await phone.click('#join-submit-btn');
  await expect(phone.locator('#join-entry-error'))
    .toHaveText(ts('client.join.error_wrong_type'), { timeout: 15_000 });
  await expect(phone.locator('#join-entry')).toBeVisible();

  // A ship host typing the lead's CREW code into the fleet field — five
  // perfectly good letters, in the wrong namespace.
  const second = await bootHost(context);
  await typeFleetCode(second, (await readHostPeerId(lead)).split('_').pop());
  await waitForFleetError(second);
  const panel = await fleetPanel(second);
  expect(panel.error).toBe(
    ts('server.fleet.error_joining', { reason: ts('client.join.error_wrong_type') }),
  );
  expect(panel.slots).toEqual([]);
});

test('closing admission refuses a new host without disturbing an admitted one', async ({ context }) => {
  test.setTimeout(FLEET_TIMEOUT);
  const lead = await bootHost(context);
  const fleet = await openFleet(lead);
  const second = await bootHost(context, `#${fleet.url.split('#')[1]}`);
  await waitForSlots(second, 2);

  // Close.
  await openFleetTab(lead);
  await lead.click('[data-control="fleet-admission"]');
  await lead.waitForFunction(
    (closed) => document.getElementById('fleet-status')?.textContent === closed,
    ts('server.fleet.closed'),
    { timeout: 20_000 },
  );
  await closeCog(lead);

  // The admitted host was told, not dropped: same two slots on both panels.
  await second.waitForFunction(
    (closed) => document.getElementById('fleet-status')?.textContent === closed,
    ts('server.fleet.closed'),
    { timeout: 20_000 },
  );
  expect((await fleetPanel(second)).slots).toHaveLength(2);
  expect((await fleetPanel(lead)).slots).toHaveLength(2);

  // A THIRD machine arriving now is refused — and `second` stays up throughout,
  // which is the half of this criterion that would otherwise be argued rather
  // than shown.
  const third = await bootHost(context, `#${fleet.url.split('#')[1]}`);
  await waitForFleetError(third);
  expect((await fleetPanel(third)).error).toBe(
    ts('server.fleet.error_joining', { reason: ts('client.join.error_closed') }),
  );
  expect((await fleetPanel(third)).slots).toEqual([]);
  expect((await fleetPanel(lead)).slots).toHaveLength(2);
  expect((await fleetPanel(second)).slots).toHaveLength(2);

  // Reopen, and the same machine gets in.
  await openFleetTab(lead);
  await lead.click('[data-control="fleet-admission"]');
  await closeCog(lead);
  await third.evaluate((code) => window.__hostFleetJoin(code), fleet.suffix);
  // Settle on EITHER outcome before asserting, so a still-refusing fleet fails
  // with the sentence on screen rather than as a bare roster timeout.
  await third.waitForFunction(
    () => (document.getElementById('fleet-error')?.textContent ?? '').length > 0
      || document.querySelectorAll('#fleet-slots li').length === 3,
    { timeout: 30_000 },
  );
  const readmitted = await fleetPanel(third);
  expect(readmitted.error, JSON.stringify(readmitted)).toBe('');
  await waitForSlots(lead, 3);
  await waitForSlots(second, 3);

  // And the ordinary operator round trip: leave a fleet, then come back to it.
  // Its slot goes while the roster is still mutable, and it is issued a new one
  // — an id is spent once, because #1116 will put these in a shared stream.
  await second.evaluate(() => window.__hostFleetLeave());
  await waitForSlots(lead, 2);
  await second.evaluate((code) => window.__hostFleetJoin(code), fleet.suffix);
  await waitForSlots(lead, 3);
  const rejoined = await fleetPanel(second);
  expect(rejoined.error).toBe('');
  expect(rejoined.slots.filter((r) => r.mine)).toHaveLength(1);
});

test('mission start freezes the slot roster', async ({ context }) => {
  test.setTimeout(FLEET_TIMEOUT);
  const lead = await bootHost(context);
  const fleet = await openFleet(lead);
  const second = await bootHost(context, `#${fleet.url.split('#')[1]}`);
  await waitForSlots(second, 2);

  // Start the mission the ordinary way: a crew member on the lead's own ship
  // readies up, and the collective auto-start fires.
  const crew = await createTestClient(context, await readHostPeerId(lead), {
    token: 'fleet-crew-b',
    name: 'Crewman',
  });
  await crew.send('SetReady', { ready: true });
  await crew.waitForMessage('GameStarted', 20_000);

  const frozen = ts('server.fleet.frozen');
  for (const page of [lead, second]) {
    await page.waitForFunction(
      (want) => document.getElementById('fleet-status')?.textContent === want,
      frozen,
      { timeout: 20_000 },
    );
  }

  // Past the freeze the server code can no longer create a slot. It is still
  // RESOLVABLE — it is the recovery capability #1120 will honour, so a claim has
  // to reach the lead to be judged — and the refusal is its own reason rather
  // than "admission closed".
  const third = await bootHost(context, `#${fleet.url.split('#')[1]}`);
  await waitForFleetError(third);
  expect((await fleetPanel(third)).error).toBe(
    ts('server.fleet.error_joining', { reason: ts('server.fleet.error_frozen') }),
  );
  expect((await fleetPanel(lead)).slots).toHaveLength(2);

  // A frozen slot whose host goes is KEPT and marked disconnected, because it
  // is the object a recovery claim will name.
  await second.close();
  await lead.waitForFunction(
    (gone) => {
      const rows = [...document.querySelectorAll('#fleet-slots li')];
      return rows.length === 2 && rows[1].textContent.includes(gone);
    },
    ts('server.fleet.disconnected'),
    { timeout: 20_000 },
  );
  await crew.close();
});
