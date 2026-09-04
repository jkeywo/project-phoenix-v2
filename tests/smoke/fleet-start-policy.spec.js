// Issue #1290 — collective crew/GM readiness and attributed force-start in the
// real browser host. The pure host-mesh/fleet-session suites exhaustively pin
// policy permutations; this file proves that the shipped server page carries
// Rust's crew tally into that policy, confines controls to admitted GM peers,
// and hands the resulting grant back through the real WASM boundary.

import {
  test,
  expect,
  waitForWasmReady,
  readHostPeerId,
  createTestClient,
  waitForJoinCode,
  JOIN_CODE_PATTERN_SOURCE,
} from './fixtures';

async function bootRunningHost(context) {
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await waitForJoinCode(page, 'join-code', 30_000);
  return page;
}

// Hold only the exact terminal Leave status after Rust has decided it. Join
// status remains live, so this exposes the narrow adoption-accepted / leave-
// not-yet-acknowledged window without faking either Rust decision.
async function bootHostWithHeldLeaveStatus(context) {
  const page = await context.newPage();
  await page.addInitScript(() => {
    window.addEventListener('TrunkApplicationStarted', () => {
      const real = window.wasmBindings;
      let leaveGeneration = null;
      let terminal = null;
      let released = false;
      window.__fleetLeaveStatusHeld = () => terminal !== null;
      window.__releaseFleetLeaveStatus = () => { released = true; };
      window.wasmBindings = new Proxy(real, {
        get(target, key, receiver) {
          if (key === 'wasm_leave_fleet') {
            return (...args) => {
              const queued = Reflect.apply(target.wasm_leave_fleet, target, args);
              leaveGeneration = Number(queued);
              return queued;
            };
          }
          if (key === 'wasm_fleet_join_status') {
            return (...args) => {
              const raw = Reflect.apply(target.wasm_fleet_join_status, target, args);
              let status = null;
              try { status = JSON.parse(raw); } catch (_) {}
              if (leaveGeneration !== null && status?.generation === leaveGeneration
                  && status.status !== 'pending' && status.status !== 'idle') {
                terminal = raw;
              }
              if (terminal !== null && !released) {
                return JSON.stringify({
                  generation: leaveGeneration, status: 'pending', reason: null,
                });
              }
              return terminal ?? raw;
            };
          }
          return Reflect.get(target, key, receiver);
        },
      });
    }, { once: true });
  });
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await waitForJoinCode(page, 'join-code', 30_000);
  return page;
}

// A host before scenario/hull selection. Trunk and the rendezvous transport are
// live, so it may form a fleet, but its independent start-validation answer is
// false until content is selected. That is the browser-real refusal fixture.
async function bootSelectingHost(context) {
  const page = await context.newPage();
  await page.goto('/');
  await page.waitForFunction(
    (src) => typeof window.__hostFleetOpen === 'function'
      && !!window.wasmBindings?.wasm_delivery_stamp_field
      && new RegExp(src).test(document.getElementById('join-code')?.textContent ?? ''),
    JOIN_CODE_PATTERN_SOURCE,
    { timeout: 60_000 },
  );
  return page;
}

// A ship whose world and hull selection load normally, but whose final Rust
// station gate rejects the candidate. The fleet stamp remains the real content
// stamp, so a valid GM peer can still join and observe the independent false
// validation answer rather than being stopped earlier as content-incompatible.
async function bootRejectedHullHost(context) {
  const page = await context.newPage();
  await page.addInitScript(() => {
    window.addEventListener('TrunkApplicationStarted', () => {
      const real = window.wasmBindings;
      window.wasmBindings = new Proxy(real, {
        get(target, key, receiver) {
          if (key === 'wasm_validate_stations') {
            return () => { throw new Error('smoke station validation refusal'); };
          }
          return Reflect.get(target, key, receiver);
        },
      });
    }, { once: true });
  });
  await page.goto('/?scenario=assets/worlds/default.toml');
  await page.waitForFunction(
    (src) => !!document.querySelector('#wasm-spinner strong')
      && new RegExp(src).test(document.getElementById('join-code')?.textContent ?? ''),
    JOIN_CODE_PATTERN_SOURCE,
    { timeout: 60_000 },
  );
  return page;
}

async function openFleetTab(page) {
  await page.bringToFront();
  await page.click('#server-settings-btn');
  await page.click('.server-settings-tab[data-tab="gameplay"]');
  await page.waitForSelector('[data-control="fleet-code"]', { state: 'attached' });
}

async function selectGmProfile(page) {
  await Promise.all([
    page.waitForURL((url) => url.searchParams.get('gm') === '1'),
    page.click('[data-control="fleet-role-gm"]'),
  ]);
  if (new URL(page.url()).searchParams.has('scenario')) {
    await waitForWasmReady(page);
  } else {
    await page.waitForFunction(
      () => typeof window.__hostFleetOpen === 'function'
        && document.documentElement.dataset.phoenixBootRequest === 'browser-game-master',
      { timeout: 60_000 },
    );
  }
  await openFleetTab(page);
}

async function openFleet(page, role = 'ship') {
  await openFleetTab(page);
  if (role === 'gm') await selectGmProfile(page);
  await page.click('[data-control="fleet-open"]');
  await waitForJoinCode(page, 'fleet-code', 30_000);
  return page.evaluate(() => document.getElementById('fleet-code').textContent);
}

async function openFleetDirect(page) {
  await page.evaluate(() => window.__hostFleetOpen());
  await waitForJoinCode(page, 'fleet-code', 30_000);
  return page.evaluate(() => document.getElementById('fleet-code').textContent);
}

async function joinFleetAsGm(page, code) {
  // Every smoke page shares one BrowserContext/localStorage, while real GM
  // machines have independent stores. Clear the previous page's reconnect
  // capability so this page exercises a distinct equal operator admission
  // rather than intentionally reclaiming the first GM identity.
  await page.evaluate(() => localStorage.removeItem('phoenix.fleet.gm-identity.v1'));
  await openFleetTab(page);
  await selectGmProfile(page);
  await page.fill('[data-control="fleet-code"]', code);
  await page.click('[data-control="fleet-join"]');
  await page.waitForFunction(
    () => {
      const state = window.__hostGmStartState?.();
      return state?.admitted === true
        && state.presentationReady === true
        && state.localValidation === true;
    },
    { timeout: 30_000 },
  );
  await expect(page.locator('#gm-start-controls')).toHaveAttribute('aria-hidden', 'false');
}

const gmStart = (page) => page.evaluate(() => window.__hostGmStartState());
const clickGmControl = (page, controlId) => page.evaluate((id) => {
  const control = document.getElementById(id);
  if (!control) throw new Error(`missing GM control: ${id}`);
  control.click();
}, controlId);

test.describe('fleet lobby start policy', () => {
  test('pending Leave withholds a just-adopted frozen roster and grant until its exact result', async ({ context }) => {
    const owner = await bootHostWithHeldLeaveStatus(context);
    const code = await openFleet(owner, 'gm');
    const member = await bootRunningHost(context);
    await joinFleetAsGm(member, code);

    await clickGmControl(member, 'gm-ready-btn');
    await owner.waitForFunction(
      () => window.__hostGmStartState?.().policy?.ready_gms === 1,
      { timeout: 30_000 },
    );

    const request = await owner.evaluate(() => {
      document.getElementById('gm-ready-btn').click();
      const leaveQueued = window.__hostFleetLeave();
      return { leaveQueued, fleet: window.__hostFleetState() };
    });
    expect(request.leaveQueued).toBe(true);
    expect(request.fleet.open).toBe(true);
    expect(request.fleet.canLeave).toBe(false);

    // Rust has accepted the join and definitively decided the queued Leave;
    // only the browser's observation of that exact generation is held here.
    await owner.waitForFunction(() => window.__fleetLeaveStatusHeld?.() === true, {
      timeout: 30_000,
    });
    expect(await owner.evaluate(() => window.__hostFleetState().open)).toBe(true);
    expect(await member.evaluate(() => window.__hostGmStartState().policy)).toMatchObject({
      ready_gms: 2,
      started: false,
    });
    expect(await member.evaluate(() => window.__hostMeshStatus().in_fleet)).toBe(false);
    expect(await member.evaluate(() => window.__saveSlotsPhase)).toBe('Lobby');

    await owner.evaluate(() => window.__releaseFleetLeaveStatus());
    await owner.waitForFunction(() => window.__hostFleetState().open === false, {
      timeout: 30_000,
    });
    // The adoption Promise resolves `fleet-left`, so its continuation cannot
    // publish the frozen topology or deliver the grant after teardown either.
    await member.waitForTimeout(250);
    expect(await member.evaluate(() => window.__hostMeshStatus().in_fleet)).toBe(false);
    expect(await member.evaluate(() => window.__saveSlotsPhase)).toBe('Lobby');
    expect(await member.evaluate(() => window.__hostGmStartState().fixedResult)).toBeNull();
  });

  test('equal GMs auto-start after a spectator and an unready disconnected GM are excluded', async ({ context }) => {
    const ship = await bootRunningHost(context);
    const code = await openFleet(ship);

    // A real crew peer explicitly chooses Spectator. It remains visible in the
    // lobby but Rust's readiness tally must publish zero connected players.
    const crewCode = await readHostPeerId(ship);
    const spectator = await createTestClient(context, crewCode, { name: 'Observer' });
    const spectatorChanged = spectator.waitForMessage('SpectatorChanged');
    await spectator.send('SetSpectator', { spectator: true });
    await spectatorChanged;

    const gmA = await bootRunningHost(context);
    const gmB = await bootRunningHost(context);
    await joinFleetAsGm(gmA, code);
    await joinFleetAsGm(gmB, code);
    for (const page of [gmA, gmB]) {
      await page.waitForFunction(
        () => {
          const state = window.__hostGmStartState?.();
          return state?.presentationReady === true
            && state.localValidation === true
            && state.policy?.validation_passed === true;
        },
        { timeout: 30_000 },
      );
    }

    // Both admitted operators have the same controls; neither page is a fleet
    // leader surface. Public text states not-ready rather than relying on colour.
    for (const page of [gmA, gmB]) {
      await expect(page.locator('#gm-ready-btn')).toBeVisible();
      await expect(page.locator('#gm-force-start-btn')).toBeVisible();
      await expect(page.locator('#lobby-gm-list')).toContainText(/not ready/i);
    }

    await clickGmControl(gmA, 'gm-ready-btn');
    await gmA.waitForFunction(
      () => {
        const p = window.__hostGmStartState?.().policy;
        return p && p.connected_players === 0 && p.connected_gms === 2
          && p.ready_gms === 1 && p.started === false;
      },
      { timeout: 30_000 },
    );
    expect((await gmStart(gmA)).policy).toMatchObject({
      connected_players: 0,
      connected_gms: 2,
      ready_gms: 1,
      all_ready: false,
      started: false,
    });

    // The second operator vanishes without consenting. It was already unready;
    // this browser case proves disconnected-GM exclusion and explicit public
    // not-ready text. The pure policy suite separately pins ready→disconnect
    // clearing. The one connected ready GM is now the complete participant set.
    await gmB.close();
    await gmA.waitForFunction(
      () => window.__hostGmStartState?.().policy?.started === true,
      { timeout: 30_000 },
    );
    await expect(ship.locator('#fleet-gms [data-gm-id="gm-2"]')).toContainText(/not ready/i);
    await ship.waitForFunction(
      () => window.__saveSlotsPhase === 'InProgress',
      { timeout: 30_000 },
    );
    await gmA.waitForFunction(
      () => window.__saveSlotsPhase === 'InProgress'
        && window.__hostGmStartState?.().fixedResult?.status === 'applied',
      { timeout: 30_000 },
    );
    // The grant is not merely after JS boot/selection. Every surviving
    // simulation host must have observed Rust's terminal render-preload fact.
    for (const page of [ship, gmA]) {
      expect(await page.evaluate(() => window.__hostGmStartState())).toMatchObject({
        presentationReady: true,
        localValidation: true,
      });
      await page.waitForFunction(
        () => {
          const mesh = window.__hostMeshStatus?.();
          return mesh?.in_fleet === true && mesh.peers?.length === 1
            && mesh.tick > 1 && mesh.disagreement == null;
        },
        undefined,
        { timeout: 30_000 },
      );
    }
    const [shipMesh, gmMesh] = await Promise.all([
      ship.evaluate(() => window.__hostMeshStatus()),
      gmA.evaluate(() => window.__hostMeshStatus()),
    ]);
    expect(shipMesh.peers).toEqual([gmMesh.slot]);
    expect(gmMesh.peers).toEqual([shipMesh.slot]);
    expect((await gmStart(gmA)).policy).toMatchObject({
      connected_players: 0,
      connected_gms: 1,
      ready_gms: 1,
      all_ready: true,
      started: true,
    });
  });

  test('a GM-only roster can collectively ready and apply automatic start', async ({ context }) => {
    const gm = await bootRunningHost(context);
    await openFleet(gm, 'gm');
    await gm.waitForFunction(
      () => {
        const state = window.__hostGmStartState?.();
        return state?.admitted === true
          && state.presentationReady === true
          && state.localValidation === true
          && state.policy?.validation_passed === true;
      },
      { timeout: 30_000 },
    );

    // Product roster, not technical topology: one GM, zero player-ship rows,
    // zero crew. The owner is merely the star centre and gets no extra control.
    await expect(gm.locator('#fleet-slots li')).toHaveCount(0);
    await expect(gm.locator('#fleet-gms [data-gm-id="gm-1"]')).toContainText(/not ready/i);
    await clickGmControl(gm, 'gm-ready-btn');

    await gm.waitForFunction(
      () => window.__saveSlotsPhase === 'InProgress'
        && window.__hostGmStartState?.().policy?.started === true
        && window.__hostGmStartState?.().fixedResult?.status === 'applied',
      { timeout: 30_000 },
    );
    expect(await gmStart(gm)).toMatchObject({
      admitted: true,
      ready: true,
      presentationReady: true,
      localValidation: true,
      policy: {
        connected_players: 0,
        connected_gms: 1,
        ready_gms: 1,
        all_ready: true,
        started: true,
      },
      fixedResult: {
        status: 'applied',
        operator_id: null,
      },
    });
  });

  test('an admitted GM-only host cannot force start before selecting valid content', async ({ context }) => {
    const gm = await bootSelectingHost(context);
    await openFleet(gm, 'gm');
    await gm.waitForFunction(
      () => window.__hostGmStartState?.().admitted === true
        && window.__hostGmStartState?.().policy?.validation_passed === false,
      { timeout: 30_000 },
    );

    await clickGmControl(gm, 'gm-force-start-btn');
    await expect(gm.locator('#gm-start-result')).toContainText(/validation failed/i, {
      timeout: 30_000,
    });
    expect(await gmStart(gm)).toMatchObject({
      admitted: true,
      policy: { validation_passed: false, started: false },
    });
  });

  test('a selected hull that fails Rust validation remains a force-start refusal', async ({ context }) => {
    const invalidShip = await bootRejectedHullHost(context);
    const code = await openFleetDirect(invalidShip);
    await expect(invalidShip.locator('#fleet-slots li')).toHaveCount(1);
    await expect(invalidShip.locator('#fleet-slots .fleet-ship'))
      .toContainText(/alliance[ _]cruiser/i);

    const gm = await bootRunningHost(context);
    await joinFleetAsGm(gm, code);
    await gm.waitForFunction(
      () => window.__hostGmStartState?.().policy?.validation_passed === false,
      { timeout: 30_000 },
    );

    await clickGmControl(gm, 'gm-force-start-btn');
    await expect(gm.locator('#gm-start-result')).toContainText(/validation failed/i, {
      timeout: 30_000,
    });
    expect((await gmStart(gm)).policy).toMatchObject({
      validation_passed: false,
      started: false,
    });
  });

  test('force start remains refused when a ship host has not validated content', async ({ context }) => {
    const unselectedShip = await bootSelectingHost(context);
    const code = await openFleet(unselectedShip);
    const gm = await bootRunningHost(context);
    await joinFleetAsGm(gm, code);

    await gm.waitForFunction(
      () => window.__hostGmStartState?.().policy?.validation_passed === false,
      { timeout: 30_000 },
    );
    await clickGmControl(gm, 'gm-force-start-btn');
    await expect(gm.locator('#gm-start-result')).toContainText(/validation failed/i, {
      timeout: 30_000,
    });
    expect((await gmStart(gm)).policy.started).toBe(false);
    expect(await gm.evaluate(() => window.__saveSlotsPhase)).toBe('Lobby');
  });
});
