// Issue #1116 — two ship hosts holding one tick clock, in two real browser pages.
//
// The browser half of the two-host proof. `tests/lockstep_mesh.rs` is the other
// half and the deeper one: it runs two complete simulations through a whole
// mission with a crew driving each, and folds both after every tick. What it
// cannot do is cross a real wire, and what this file cannot do is run a mission.
// Between them they cover the thing: the ORDER and the AGREEMENT are proved
// natively, and the WIRE — the vocabulary, the encode/decode seam, the wasm
// boundary and the barrier that reads it — is proved here, in the browser the
// product actually ships in.
//
// Everything below the page is shipped code: the real worker-rendezvous
// registry, the real join-code table, the real host-mesh vocabulary, the real
// `core::codec` encoder and the real `lockstep` barrier. Only the socket and the
// RTCPeerConnection are faked (tests/smoke/rendezvous-shim.js), because CI has
// no WebRTC and no deployed worker.
//
// # AC6's "matching state", and where this file stops (issue #1116 review)
//
// AC6 asks that the multi-instance and browser tests "finish with matching
// state". The cross-instance STATE-MATCHING is the native test's: it folds two
// differently-crewed hosts after every tick and asserts the digests agree — the
// strong claim. This file does NOT fold and compare the two pages' digests, and
// that is a deliberate, recorded split rather than an oversight. Two reasons:
// per-slot crewing is not published yet (see `joinLockstep` below — both pages
// adopt the identical empty-crew roster, so a browser compare would be identical
// inputs agreeing, a weaker claim than the native one), and the digest exchange
// runs on a 300-tick diagnostic interval that the barrier gates to the throttled
// background tab's pace, which is not reliably reachable inside a smoke timeout.
// So the browser proves the WIRE and the native test proves the AGREEMENT; the
// pasm slice `host-mesh-lockstep.yaml`'s fleet-digest-exchange records the same.
//
// # What makes the claim non-vacuous
//
// The barrier is its own control. A host in a fleet may run `delay` ticks past
// the last watermark it heard and no further, so if the tick frames were not
// arriving — a broken encoder, a mis-routed frame, a wasm export that never
// fired — both hosts would stall within a handful of ticks and say so. The
// positive test asserts the clock keeps running; the negative one severs a host
// and asserts the other one stops and names the peer it is waiting for. A test
// that only asserted "the clock runs" would pass on a page that never joined a
// fleet at all.

import { test, expect, waitForWasmReady, waitForJoinCode } from './fixtures';

/** The two slots this fleet's hosts fly. Slot 1 is the lead's, as #1114 mints. */
const SLOT_LEAD = 1;
const SLOT_MEMBER = 2;

/** The hull both slots fly here — the same one `?scenario=` boots. */
const HULL = 'assets/entities/alliance_cruiser.toml';

/** The fleet delay, stated rather than read: what is under test is that the
 *  barrier uses it, and a test that took its bound from the same place the code
 *  does could not tell a wrong bound from a right one. */
const DELAY_TICKS = 6;

/** A ship host, booted straight into its lobby with its crew code on screen. */
async function bootHost(context) {
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await waitForJoinCode(page, 'join-code', 30_000);
  return page;
}

/** Open the cog on its Gameplay tab, where the fleet controls live. */
async function openFleetTab(page) {
  await page.click('#server-settings-btn');
  await page.click('.server-settings-tab[data-tab="gameplay"]');
  await page.waitForSelector('[data-control="fleet-code"]', { state: 'attached' });
}

const closeCog = (page) => page.keyboard.press('Escape');

/** Mint this session's fleet code through the operator's own control. */
async function openFleet(page) {
  await openFleetTab(page);
  await page.click('[data-control="fleet-open"]');
  await waitForJoinCode(page, 'fleet-code', 30_000);
  await closeCog(page);
  return page.evaluate(() => document.getElementById('fleet-code').textContent);
}

/** Type a fleet code into the cog's field and submit it. */
async function joinFleet(page, code) {
  await openFleetTab(page);
  await page.fill('[data-control="fleet-code"]', code);
  await page.click('[data-control="fleet-join"]');
  await closeCog(page);
}

/**
 * Hand both hosts the identical frozen roster and let them start waiting for
 * each other.
 *
 * The roster is built HERE rather than gathered from the lobby because gathering
 * it — every host publishing which of its own stations are crewed, and at what
 * Rating — is a further slot-patch field the fleet lobby does not carry yet.
 * What that would change is which ships boot crewed; what is under test in this
 * file is the wire, and both hosts adopting the same roster is exactly the
 * precondition the wire has to work under.
 */
async function joinLockstep(page, local) {
  return page.evaluate(({ local: slot, hull, delay, lead, member }) => window.__hostMeshJoin({
    local: slot,
    owner: lead,
    participants: [lead, member],
    delay,
    ships: [
      { host: lead, ship_path: hull, crew: [] },
      { host: member, ship_path: hull, crew: [] },
    ],
  }), { local, hull: HULL, delay: DELAY_TICKS, lead: SLOT_LEAD, member: SLOT_MEMBER });
}

/** One-player topology for testing the fresh-Lobby adoption lifecycle itself. */
async function joinSoloLockstep(page) {
  return page.evaluate(({ hull, delay, lead }) => window.__hostMeshJoin({
    local: lead,
    owner: lead,
    participants: [lead],
    delay,
    ships: [{ host: lead, ship_path: hull, crew: [] }],
  }), { hull: HULL, delay: DELAY_TICKS, lead: SLOT_LEAD });
}

const meshStatus = (page) => page.evaluate(() => window.__hostMeshStatus());

/** Wait until `predicate` holds of this host's mesh status, or fail saying what it was. */
async function waitForMesh(page, predicate, what) {
  await page
    .waitForFunction(
      // `new Function('s', 'return (' + source + ')')(status)` (the previous
      // shape here) never calls the reconstructed predicate at all: binding
      // `status` to an unused parameter named `s` and then evaluating the
      // arrow-function EXPRESSION as the return value hands back a function
      // OBJECT — always truthy — so `waitForFunction` resolved on its first
      // poll no matter what the predicate said. That raced the slower
      // (throttled, backgrounded) host's own join every time this file ran,
      // which is why `status.slot` sometimes came back null downstream: the
      // wait had already returned before the join was applied. Rebuilding the
      // predicate as a value first and then calling it is what makes this an
      // actual wait.
      // eslint-disable-next-line no-new-func
      (source) => new Function(`return (${source})`)()(window.__hostMeshStatus()),
      predicate.toString(),
      { timeout: 30_000 },
    )
    .catch(async (e) => {
      throw new Error(`${what}: last status was ${JSON.stringify(await meshStatus(page))}\n${e}`);
    });
}

/** A fleet of two, both hosts in lockstep and both told the same roster. */
async function fleetOfTwo(context) {
  const lead = await bootHost(context);
  const code = await openFleet(lead);
  const member = await bootHost(context);
  await joinFleet(member, code);
  // The roster reaching both panels is #1114's own proof that the link is up;
  // this file starts from there rather than re-testing it.
  await member.waitForFunction(
    () => document.querySelectorAll('#fleet-slots li').length === 2,
    { timeout: 30_000 },
  );

  expect(await joinLockstep(lead, SLOT_LEAD)).toBe('');
  expect(await joinLockstep(member, SLOT_MEMBER)).toBe('');
  return { lead, member };
}

test.describe('two ship hosts hold one tick clock', () => {
  test('each host runs on because it keeps hearing from the other', { tag: '@core' }, async ({ context }) => {
    const { lead, member } = await fleetOfTwo(context);

    // Both are in a fleet, know their own slot, and took the agreed delay.
    for (const [page, slot] of [[lead, SLOT_LEAD], [member, SLOT_MEMBER]]) {
      await waitForMesh(page, (s) => s && s.in_fleet, 'the host never joined the fleet');
      const status = await meshStatus(page);
      expect(status.slot, 'each host flies its own slot').toBe(slot);
      expect(status.delay, 'and takes the fleet\'s agreed input delay').toBe(DELAY_TICKS);
      expect(status.peers, 'and waits for exactly the other one').toEqual([
        slot === SLOT_LEAD ? SLOT_MEMBER : SLOT_LEAD,
      ]);
    }

    // Now the claim. A host may run `delay` ticks past the last watermark it
    // heard and no further, so a clock that reaches well BEYOND that window can
    // only have done so on frames that crossed the wire, were encoded by
    // `core::codec`, arrived through `wasm_receive_mesh_frame`, and moved the
    // barrier's watermark.
    //
    // Each page is measured against its OWN start, because only one browser tab
    // is foregrounded and Chromium throttles the other's animation frames hard.
    // Two hosts advancing at different WALL rates is exactly what lockstep is
    // for and is not a failure; what would be one is either of them running out
    // of input.
    for (const page of [lead, member]) {
      await waitForMesh(
        page,
        (s) => s && s.tick > 0 && !s.stalled,
        'the host stalled instead of hearing its peer',
      );
      const from = (await meshStatus(page)).tick;
      await page.waitForFunction(
        (t) => (window.__hostMeshStatus() || {}).tick > t,
        from + DELAY_TICKS + 30,
        { timeout: 60_000 },
      );
    }

    for (const page of [lead, member]) {
      const status = await meshStatus(page);
      expect(status.stalled, 'a fleet that is hearing from itself does not wait').toBe(false);
      expect(status.waiting_on, 'and has nobody outstanding').toEqual([]);
    }
  });

  test('a host that stops hearing its peer withholds the tick and says whose', async ({ context }) => {
    const { lead, member } = await fleetOfTwo(context);
    await waitForMesh(lead, (s) => s && s.in_fleet && !s.stalled, 'the lead never got going');
    const before = (await meshStatus(lead)).tick;
    expect(before, 'precondition: the lead was running').toBeGreaterThan(0);

    // The member falls off the network entirely — no close, no goodbye, exactly
    // the shape a lost host has.
    await member.evaluate(() => window.__transportShim.sever());

    await waitForMesh(
      lead,
      (s) => s && s.stalled,
      'the lead ran on with a silent peer — it speculated past input it does not have',
    );
    const stalled = await meshStatus(lead);
    expect(stalled.waiting_on, 'the stall names the peer holding it up')
      .toEqual([SLOT_MEMBER]);
    // …and it stopped within the window the delay bought, not eventually.
    expect(stalled.tick).toBeLessThanOrEqual(before + DELAY_TICKS + 30);

    // What is NOT asserted here, deliberately: that the lead resumes when the
    // link comes back. Reviving a severed page brings the SERVICE back, and
    // re-establishing the DataChannel behind it is host recovery — #1120's, and
    // not a claim this issue's code can make. The barrier's own resume is
    // proved where it belongs, on the barrier: `tests/lockstep_mesh.rs`'s
    // `a_host_that_has_not_heard_from_its_peer_waits` stalls a host, feeds it
    // the missing watermark, and steps it on past the tick it withheld.
  });

  test('a host with no fleet has no barrier at all', async ({ context }) => {
    // The solo path must not acquire a stall it can never clear. This is the
    // shipped single-player case, and it is asserted here rather than assumed
    // because everything above is inert without a fleet — a bug that made the
    // barrier fire unconditionally would show up as a hung game, not a failure.
    const page = await bootHost(context);
    const status = await meshStatus(page);
    expect(status.in_fleet).toBe(false);
    expect(status.delay, 'and takes no input delay').toBe(0);
    expect(status.stalled_frames, 'and never withholds a tick').toBe(0);
    await page.waitForFunction(
      () => (window.__hostMeshStatus() || {}).tick > 30,
      undefined,
      { timeout: 30_000 },
    );
  });

  test('a fresh-Lobby leave is acknowledged before teardown and permits a new topology', async ({ context }) => {
    const page = await bootHost(context);
    await openFleet(page);
    expect(await joinSoloLockstep(page)).toBe('');
    await waitForMesh(page, (s) => s && s.in_fleet, 'the first topology was not adopted');

    const requested = await page.evaluate(() => {
      const queued = window.__hostFleetLeave();
      return { queued, state: window.__hostFleetState() };
    });
    expect(requested.queued).toBe(true);
    expect(requested.state.open, 'the transport stays live until Rust accepts the leave').toBe(true);
    expect(requested.state.canLeave, 'a duplicate leave is disabled while acknowledgement is pending')
      .toBe(false);

    await page.waitForFunction(() => {
      const fleet = window.__hostFleetState();
      const mesh = window.__hostMeshStatus();
      return fleet && !fleet.open && mesh && !mesh.in_fleet;
    }, undefined, { timeout: 30_000 });

    await openFleet(page);
    expect(await joinSoloLockstep(page)).toBe('');
    await waitForMesh(page, (s) => s && s.in_fleet, 'the replacement topology was not adopted');
    const replacement = await page.evaluate(() => window.__hostFleetState());
    expect(replacement.open).toBe(true);
    expect(replacement.canLeave).toBe(true);
  });
});
