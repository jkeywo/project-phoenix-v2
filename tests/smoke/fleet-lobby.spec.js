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

test('a second ship host joins the fleet and appears as its own slot', async ({ context }) => {
  // Two WASM host pages plus a fleet handshake. The suite's 60s default is a
  // budget for ONE host booting; these specs boot two and then talk between
  // them, and the whole file runs after ~40 other specs on one worker.
  test.setTimeout(180_000);
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
});

test('each crew star stays attached to its own host', async ({ context }) => {
  // Two WASM host pages plus a fleet handshake. The suite's 60s default is a
  // budget for ONE host booting; these specs boot two and then talk between
  // them, and the whole file runs after ~40 other specs on one worker.
  test.setTimeout(180_000);
  const lead = await bootHost(context);
  const fleet = await openFleet(lead);
  const second = await bootHost(context, `#${fleet.url.split('#')[1]}`);
  await waitForSlots(second, 2);

  // A phone joins the LEAD's ship with the lead's own crew code.
  const crew = await createTestClient(context, await readHostPeerId(lead), {
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
    crew.token,
    { timeout: 15_000 },
  );

  // The second host is in the same fleet and knows nothing about that phone:
  // the fleet link is a different socket in a different namespace carrying a
  // different protocol, so there is no filtering for this to depend on.
  expect(await tokensOn(second)).toEqual([]);
  await crew.close();
});

test('a crew code typed into the fleet field is refused by type', async ({ context }) => {
  // Two WASM host pages plus a fleet handshake. The suite's 60s default is a
  // budget for ONE host booting; these specs boot two and then talk between
  // them, and the whole file runs after ~40 other specs on one worker.
  test.setTimeout(180_000);
  const lead = await bootHost(context);
  const second = await bootHost(context);

  // The lead's CREW code — five perfectly good letters, in the wrong namespace.
  const crewCode = (await readHostPeerId(lead)).split('_').pop();
  await typeFleetCode(second, crewCode);

  await waitForFleetError(second);
  const panel = await fleetPanel(second);
  expect(panel.error).toBe(
    ts('server.fleet.error_joining', { reason: ts('client.join.error_wrong_type') }),
  );
  expect(panel.slots).toEqual([]);
});

test('a fleet code typed into the crew field is refused by type', async ({ context }) => {
  const lead = await bootHost(context);
  const fleet = await openFleet(lead);

  const phone = await context.newPage();
  await phone.goto('/client/');
  await phone.fill('#join-code-input', fleet.suffix.toLowerCase());
  await phone.click('#join-submit-btn');

  await expect(phone.locator('#join-entry-error'))
    .toHaveText(ts('client.join.error_wrong_type'), { timeout: 15_000 });
  await expect(phone.locator('#join-entry')).toBeVisible();
});

test('closing admission refuses a new host without disturbing an admitted one', async ({ context }) => {
  // Two WASM host pages plus a fleet handshake. The suite's 60s default is a
  // budget for ONE host booting; these specs boot two and then talk between
  // them, and the whole file runs after ~40 other specs on one worker.
  test.setTimeout(180_000);
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

  // A host arriving now is refused, and the fleet is unchanged.
  await second.evaluate(() => window.__hostFleetLeave());
  await waitForSlots(lead, 1);
  await second.evaluate((code) => window.__hostFleetJoin(code), fleet.suffix);
  await waitForFleetError(second);
  expect((await fleetPanel(second)).error).toBe(
    ts('server.fleet.error_joining', { reason: ts('client.join.error_closed') }),
  );
  expect((await fleetPanel(lead)).slots).toHaveLength(1);

  // Reopen, and the same host is admitted again.
  await openFleetTab(lead);
  await lead.click('[data-control="fleet-admission"]');
  await closeCog(lead);
  await second.evaluate((code) => window.__hostFleetJoin(code), fleet.suffix);
  await second.waitForFunction(
    () => (document.getElementById('fleet-error')?.textContent ?? '').length > 0
      || document.querySelectorAll('#fleet-slots li').length === 2,
    { timeout: 30_000 },
  );
  const readmitted = await fleetPanel(second);
  expect(readmitted.error, JSON.stringify(readmitted)).toBe('');
  await waitForSlots(lead, 2);
  await waitForSlots(second, 2);
});

test('mission start freezes the slot roster', async ({ context }) => {
  // Two WASM host pages plus a fleet handshake. The suite's 60s default is a
  // budget for ONE host booting; these specs boot two and then talk between
  // them, and the whole file runs after ~40 other specs on one worker.
  test.setTimeout(180_000);
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
  // resolvable — it is the recovery capability #1120 will honour — so the
  // refusal comes from the fleet lead, and it is its own reason rather than
  // "admission closed".
  await second.evaluate(() => window.__hostFleetLeave());
  await second.evaluate((code) => window.__hostFleetJoin(code), fleet.suffix);
  await waitForFleetError(second);
  expect((await fleetPanel(second)).error).toBe(
    ts('server.fleet.error_joining', { reason: ts('server.fleet.error_frozen') }),
  );

  // And the frozen slot is kept, marked disconnected, rather than deleted.
  const onLead = await fleetPanel(lead);
  expect(onLead.slots).toHaveLength(2);
  expect(onLead.slots[1].text).toContain(ts('server.fleet.disconnected'));
  await crew.close();
});
