// Issue #1294 — a known frozen-session GM returns through the real browser
// WASM paused restore. Pure JS and native integration suites pin every refusal;
// this smoke proves the shipped page keeps the returning identity private until
// Rust's matching-digest Commit and still requires an explicit typed Resume.

import {
  test,
  expect,
  waitForWasmReady,
  waitForJoinCode,
} from './fixtures';

const GM_IDENTITY_KEY = 'phoenix.fleet.gm-identity.v1';

async function bootRunningHost(context) {
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await waitForJoinCode(page, 'join-code', 30_000);
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
  await waitForWasmReady(page);
  await openFleetTab(page);
}

async function openFleet(owner) {
  await openFleetTab(owner);
  await owner.click('[data-control="fleet-open"]');
  await waitForJoinCode(owner, 'fleet-code', 30_000);
  return owner.evaluate(() => document.getElementById('fleet-code').textContent);
}

async function joinFreshGm(page, code) {
  await page.evaluate((key) => localStorage.removeItem(key), GM_IDENTITY_KEY);
  await openFleetTab(page);
  await selectGmProfile(page);
  await page.fill('[data-control="fleet-code"]', code);
  await page.click('[data-control="fleet-join"]');
  await page.waitForFunction(
    () => window.__hostGmStartState?.().admitted === true,
    undefined,
    { timeout: 30_000 },
  );
}

async function joinKnownGm(page, code) {
  await openFleetTab(page);
  await selectGmProfile(page);
  await page.evaluate(() => {
    const receive = window.wasm_receive_mesh_frame;
    const prepare = window.wasm_prepare_gm_join_candidate;
    const ingress = {};
    const snapshots = [];
    const bootstraps = [];
    window.__gmReconnectIngress = () => ({ counts: { ...ingress }, snapshots: [...snapshots] });
    window.__gmReconnectBootstraps = () => [...bootstraps];
    window.wasm_receive_mesh_frame = (slot, raw) => {
      let type = 'unreadable';
      try {
        const frame = JSON.parse(raw);
        type = frame?.t || type;
        if (type === 'snapshot') {
          snapshots.push({
            seq: frame.d?.seq,
            total: frame.d?.total,
            bytes: frame.d?.text?.length,
            text: frame.d?.text,
          });
        }
      } catch (_) {}
      ingress[type] = (ingress[type] || 0) + 1;
      return receive(slot, raw);
    };
    window.wasm_prepare_gm_join_candidate = (id, roster) => {
      const accepted = prepare(id, roster);
      bootstraps.push({ id: String(id), accepted });
      return accepted;
    };
  });
  await page.fill('[data-control="fleet-code"]', code);
  await page.click('[data-control="fleet-join"]');
}

const clickGmControl = (page, controlId) => page.evaluate((id) => {
  const control = document.getElementById(id);
  if (!control) throw new Error(`missing GM control: ${id}`);
  control.click();
}, controlId);

test('known departed GM restores through matching digest before rejoin and explicit Resume', async ({ context }) => {
  test.setTimeout(240_000);
  const owner = await bootRunningHost(context);
  const code = await openFleet(owner);
  const original = await bootRunningHost(context);
  await joinFreshGm(original, code);

  await clickGmControl(original, 'gm-ready-btn');
  for (const page of [owner, original]) {
    await page.waitForFunction(
      () => window.__saveSlotsPhase === 'InProgress'
        && window.__hostMeshStatus?.().in_fleet === true,
      undefined,
      { timeout: 30_000 },
    );
  }
  const identity = await original.evaluate((key) => JSON.parse(localStorage.getItem(key)), GM_IDENTITY_KEY);
  expect(identity).toMatchObject({
    operatorId: 'gm-1',
    rolePreset: null,
  });
  expect(typeof identity.reconnectCredential).toBe('string');
  expect(identity.reconnectCredential.length).toBeGreaterThan(10);

  await original.close();
  await expect(owner.locator('#fleet-gms [data-gm-id="gm-1"]')).toContainText(/disconnected/i);
  const departedTick = await owner.evaluate(() => window.wasm_sim_tick());
  await owner.waitForFunction((tick) => window.wasm_sim_tick() > tick + 10, departedTick, {
    timeout: 30_000,
  });

  // Hold only the browser's observation of Rust's real terminal result. The
  // transaction, snapshot restore, digest proof, and Commit frame still run;
  // this exposes the exact Commit/public-roster boundary deterministically.
  await owner.evaluate(() => {
    const real = window.wasm_gm_join_status;
    let terminal = null;
    let released = false;
    window.__gmReconnectCommitHeld = () => terminal !== null;
    window.__releaseGmReconnectCommit = () => { released = true; };
    window.wasm_gm_join_status = () => {
      const raw = real();
      let progress = null;
      try { progress = JSON.parse(raw); } catch (_) {}
      if (progress?.status === 'committed') terminal = raw;
      if (terminal && !released) {
        const committed = JSON.parse(terminal);
        return JSON.stringify({
          ...committed,
          status: 'transferring',
          commit: null,
        });
      }
      return terminal || raw;
    };
  });

  const returning = await bootRunningHost(context);
  const privateTick = await returning.evaluate(() => window.wasm_sim_tick());
  expect(privateTick).toBeLessThan(await owner.evaluate(() => window.wasm_sim_tick()));
  await joinKnownGm(returning, code);

  await owner.waitForFunction(() => {
    if (window.__gmReconnectCommitHeld?.() === true) return true;
    try { return JSON.parse(window.wasm_gm_join_status()).status === 'refused'; }
    catch (_) { return false; }
  }, undefined, { timeout: 30_000 });
  if (!await owner.evaluate(() => window.__gmReconnectCommitHeld?.() === true)) {
    const [ownerStatus, candidateStatus, ingress, bootstraps] = await Promise.all([
      owner.evaluate(() => window.wasm_gm_join_status()),
      returning.evaluate(() => window.wasm_gm_join_status()),
      returning.evaluate(() => window.__gmReconnectIngress()),
      returning.evaluate(() => window.__gmReconnectBootstraps()),
    ]);
    const importGate = await returning.evaluate(() => {
      const transfer = window.__gmReconnectIngress().snapshots
        .sort((left, right) => left.seq - right.seq)
        .map((chunk) => chunk.text)
        .join('');
      return window.wasm_prepare_import(transfer);
    });
    for (const snapshot of ingress.snapshots) delete snapshot.text;
    throw new Error(`GM reconnect refused: ${JSON.stringify({
      ownerStatus, candidateStatus, importGate, ingress, bootstraps,
    })}`);
  }
  await expect(owner.locator('#gm-join-controls')).toBeVisible();
  await expect(owner.locator('#gm-join-accept')).toBeHidden();
  await expect(owner.locator('#gm-join-reject')).toBeHidden();
  await expect(owner.locator('#fleet-gms [data-gm-id="gm-1"]')).toContainText(/disconnected/i);
  await expect(owner.locator('#gm-join-pause-state')).toContainText(/paused/i);
  expect(await returning.evaluate(() => window.__hostGmStartState().admitted)).toBe(false);

  await owner.evaluate(() => window.__releaseGmReconnectCommit());
  await returning.waitForFunction(
    () => window.__hostGmStartState?.().admitted === true
      && window.__hostMeshStatus?.().in_fleet === true,
    undefined,
    { timeout: 30_000 },
  );
  await expect(owner.locator('#fleet-gms [data-gm-id="gm-1"]')).not.toContainText(/disconnected/i);
  await expect(owner.locator('#fleet-gms [data-gm-id]')).toHaveCount(1);
  await expect(returning.locator('#gm-join-accept')).toBeHidden();
  await expect(returning.locator('#gm-join-reject')).toBeHidden();
  expect(await returning.evaluate(() => window.wasm_is_paused())).toBe(true);
  expect(await returning.evaluate((key) => JSON.parse(localStorage.getItem(key)), GM_IDENTITY_KEY))
    .toMatchObject(identity);

  await clickGmControl(returning, 'gm-session-resume');
  for (const page of [owner, returning]) {
    await page.waitForFunction(
      () => window.wasm_is_paused() === false,
      undefined,
      { timeout: 30_000 },
    );
  }
  const resumedAt = await owner.evaluate(() => window.wasm_sim_tick());
  await owner.waitForFunction((tick) => window.wasm_sim_tick() > tick + 5, resumedAt, {
    timeout: 30_000,
  });
  const [ownerMesh, returningMesh] = await Promise.all([
    owner.evaluate(() => window.__hostMeshStatus()),
    returning.evaluate(() => window.__hostMeshStatus()),
  ]);
  expect(ownerMesh.disagreement).toBeNull();
  expect(returningMesh.disagreement).toBeNull();
  expect(ownerMesh.peers).toEqual([returningMesh.slot]);
  expect(returningMesh.peers).toEqual([ownerMesh.slot]);
});
