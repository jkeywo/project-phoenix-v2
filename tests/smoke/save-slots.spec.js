// Browser lifecycle proof for issue #865. This drives the real WASM catalogue
// exports and localStorage backend: named fixed-tick capture in a running App,
// pre-session list/export/confirmed-delete, a real compatibility verdict, and
// a compatible row reloading through the fresh-App snapshot path.

import fs from 'node:fs';
import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  MINIMAL_DEFAULT_WORLD,
} from './fixtures';
import { ts } from './strings';

const WORLD = 'assets/worlds/default.toml';
const CRUISER = 'assets/entities/alliance_cruiser.toml';
const DESTROYER = 'assets/entities/alliance_destroyer.toml';
const MULTI_HULL_WORLD = `${MINIMAL_DEFAULT_WORLD}

[[available_ships]]
template_path = "${CRUISER}"

[[available_ships]]
template_path = "${DESTROYER}"
`;

async function installMultiHullWorld(context) {
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: MULTI_HULL_WORLD }));
}

async function selectDestroyer(page) {
  // finishInit mounts the web component directly in #world-list. Playwright's
  // CSS engine pierces its open shadow root, so clicking the real option button
  // exercises ph-ship-picker's supported `ship-selected` event (rather than
  // dispatching the host action from the test). The card count also proves the
  // per-test two-hull route won over fixtures.js's one-world default.
  const picker = page.locator('#world-list > ph-ship-picker');
  await expect(picker).toBeVisible({ timeout: 60_000 });
  const cards = picker.locator('.ship-card');
  await expect(cards).toHaveCount(2);
  await expect(cards.nth(0)).toHaveAttribute('data-template', CRUISER);
  const destroyer = picker.locator(`.ship-card[data-template="${DESTROYER}"]`);
  await expect(destroyer).toBeVisible();
  await destroyer.click();
}

async function startedHost(context, name = 'Save Tester', { denySaveLocks = false } = {}) {
  const page = await context.newPage();
  if (denySaveLocks) {
    await page.addInitScript(() => {
      const manager = navigator.locks;
      const request = manager.request.bind(manager);
      Object.defineProperty(manager, 'request', {
        configurable: true,
        value(name, ...args) {
          if (String(name).startsWith('phoenix-save-peer:')) {
            window.__saveLockDenials = (window.__saveLockDenials || 0) + 1;
            throw new DOMException('save lock denied by test', 'SecurityError');
          }
          return request(name, ...args);
        },
      });
    });
  }
  await page.goto(`/?scenario=${WORLD}`);
  await selectDestroyer(page);
  await waitForWasmReady(page);

  const hostId = await readHostPeerId(page);
  const captain = await createTestClient(context, hostId, { name });
  await captain.send('SelectStation', { station: 'Captain' });
  await captain.page.waitForFunction(
    (token) => window.__messages?.some(
      (message) => message.type === 'StationAssigned' && message.data.token === token,
    ),
    captain.token,
    { timeout: 10_000 },
  );
  await captain.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 20_000);
  const createSave = page.locator('#manual-save-panel [data-save-action="create"]');
  await expect(createSave).toBeVisible({ timeout: 20_000 });
  await expect(createSave).toBeEnabled();
  return { page, captain };
}

async function waitForRedAlert(client, timeout = 10_000) {
  await client.page.waitForFunction(
    () => window.__messages?.some(
      (message) => message.type === 'BlackboardUpdate' && message.data.updates?.some(
        ([id, blackboard]) => id === 'captain' && blackboard?.data?.red_alert === true,
      ),
    ),
    null,
    { timeout },
  );
}

function snapshotIdentity(artifact) {
  const found = artifact.match(
    /snapshot:\s*Some\s*\(\s*\(\s*tick:\s*(\d+),\s*digest:\s*(\d+)/s,
  );
  if (!found) throw new Error('exported save carried no readable snapshot tick/digest');
  return { tick: Number(found[1]), digest: BigInt(found[2]) };
}

async function createNamedSave(page, displayName) {
  await page.bringToFront();
  const panel = page.locator('#manual-save-panel');
  await panel.locator('input').fill(displayName);
  await panel.locator('[data-save-action="create"]').click();
  await page.waitForFunction(
    (name) => Array.from(window.wasm_list_save_slots?.() || [])
      .some((row) => row.display_name === name),
    displayName,
    { timeout: 20_000 },
  );
  return page.evaluate(
    (name) => Array.from(window.wasm_list_save_slots()).find((row) => row.display_name === name),
    displayName,
  );
}

async function cataloguePage(page) {
  // This helper's direct Playwright navigation stands in for the host choosing
  // to leave its running App and open the pre-session catalogue. Production's
  // Start action arms the same synchronous handoff before assigning href.
  await page.evaluate(() => window.phSaveSlots.armBrowserSaveIdentityHandoff(window));
  await page.goto('/');
  await expect(page.locator('#save-slots-panel .save-slots-heading')).toBeVisible({
    timeout: 30_000,
  });
  return page;
}

async function saveRows(page) {
  return page.evaluate(() => Array.from(window.wasm_list_save_slots?.() || []));
}

async function exportSlot(page, slotId) {
  return page.evaluate((id) => {
    const refusal = window.wasm_export_save_slot(id);
    const artifact = window.wasm_take_exported_snapshot();
    return { refusal, artifact };
  }, slotId);
}

test('local save slots complete their browser lifecycle and restore only in a fresh App', { tag: '@core' }, async ({
  context,
}) => {
  test.setTimeout(240_000);
  await installMultiHullWorld(context);
  const { page: running, captain } = await startedHost(
    context,
    'Save Tester',
    { denySaveLocks: true },
  );
  const fallbackIdentity = await running.evaluate(() => ({
    denials: window.__saveLockDenials || 0,
    identity: sessionStorage.getItem('phoenix-save-peer-id'),
  }));
  expect(fallbackIdentity.denials).toBeGreaterThan(0);
  expect(fallbackIdentity.identity).toMatch(/^[0-9a-f]{32}$/);

  // Put the authoritative World into a non-default, stable state before the
  // capture. A fresh run starts with Red Alert down, so seeing it up after the
  // restore proves state was applied rather than merely staged and abandoned.
  await captain.send('ControlSystem', {
    target: 'red-alert',
    payload: { type: 'SetRedAlert', data: { active: true } },
  });
  await waitForRedAlert(captain);

  // The manual affordance is persistent running-session chrome. It has a name
  // field, is enabled from the authoritative phase, and offers no live restore.
  const capture = running.locator('#manual-save-panel');
  await expect(capture.locator('[data-save-action="create"]')).toBeEnabled();
  await expect(capture.locator('[data-save-action="start"]')).toHaveCount(0);
  const restore = await createNamedSave(running, 'Restore point');
  const moved = await createNamedSave(running, 'Old format');
  expect(restore.slot_id).not.toBe(moved.slot_id);
  expect(restore.selected_ship).toBe(DESTROYER);

  // Make one REAL stored record incompatible by moving vellum-save's envelope
  // version in localStorage. The catalogue API, rather than test fabrication,
  // computes the verdict that the UI renders below.
  await running.evaluate((slotId) => {
    const identity = sessionStorage.getItem('phoenix-save-peer-id');
    const key = `phoenix:${identity}:${slotId}`;
    const stored = localStorage.getItem(key);
    if (!stored) throw new Error(`missing stored slot ${slotId}`);
    const older = stored.replace(/format:\s*(\d+)/, (_, value) => `format:${Number(value) - 1}`);
    if (older === stored) throw new Error('stored run carried no format field');
    localStorage.setItem(key, older);
  }, moved.slot_id);

  // Reuse the same top-level page for the fresh App. Save catalogues are peer
  // local, so a simultaneous new page is a different simulation peer and must
  // not inherit these rows. This page has no usable Web Locks: its one-shot,
  // peer-private handoff must preserve the namespace across both navigations.
  const catalogue = await cataloguePage(running);
  expect(await catalogue.evaluate(() => sessionStorage.getItem('phoenix-save-peer-id')))
    .toBe(fallbackIdentity.identity);
  const restoreRow = catalogue.locator(`[data-slot-id="${restore.slot_id}"]`);
  const movedRow = catalogue.locator(`[data-slot-id="${moved.slot_id}"]`);
  await expect(restoreRow).toContainText('Restore point');
  await expect(movedRow).toContainText(ts('server.save_slots.incompatible_format'));
  const catalogueLayout = await catalogue.evaluate(() => {
    const join = document.getElementById('overlay');
    const saves = document.getElementById('save-slots-panel');
    const joinBox = join.getBoundingClientRect();
    const savesBox = saves.getBoundingClientRect();
    const overlaps = !(
      joinBox.right <= savesBox.left || joinBox.left >= savesBox.right ||
      joinBox.bottom <= savesBox.top || joinBox.top >= savesBox.bottom
    );
    return { parentId: join.parentElement?.id || '', overlaps };
  });
  expect(catalogueLayout).toEqual({ parentId: 'scenario-panel', overlaps: false });

  // The reserved internal Store key is never player-facing text.
  const autosave = catalogue.locator('[data-slot-id="autosave"] .save-slot-name');
  await expect(autosave).toHaveText(ts('server.save_slots.autosave'));

  // Export is for the exact selected row and yields the real RON artifact.
  await restoreRow.click();
  const pendingDownload = catalogue.waitForEvent('download', { timeout: 20_000 });
  await catalogue.locator('[data-save-action="export"]').click();
  const download = await pendingDownload;
  const downloadedAt = await download.path();
  const artifact = fs.readFileSync(downloadedAt, 'utf-8');
  expect(download.suggestedFilename()).toBe('phoenix-save.ron');
  expect(artifact).toContain(WORLD);
  const saved = snapshotIdentity(artifact);
  expect(saved.tick).toBe(Number(restore.capture_tick));
  expect(typeof saved.digest).toBe('bigint');

  // Incompatible saves cannot start, but remain exportable/deletable. Delete
  // is a two-click operation and the Store remains untouched before confirm.
  await movedRow.click();
  await expect(catalogue.locator('[data-save-action="start"]')).toBeDisabled();
  await catalogue.locator('[data-save-action="delete"]').click();
  await expect(catalogue.locator('[role="alertdialog"]')).toBeVisible();
  expect(await catalogue.evaluate(
    (slotId) => Array.from(window.wasmBindings.wasm_list_save_slots())
      .some((row) => row.slot_id === slotId),
    moved.slot_id,
  )).toBe(true);
  await catalogue.locator('[data-save-action="confirm-delete"]').click();
  await expect(movedRow).toHaveCount(0);
  expect(await catalogue.evaluate(
    (slotId) => Array.from(window.wasmBindings.wasm_list_save_slots())
      .some((row) => row.slot_id === slotId),
    moved.slot_id,
  )).toBe(false);
  expect(await catalogue.evaluate(
    (slotId) => Array.from(window.wasmBindings.wasm_list_save_slots())
      .some((row) => row.slot_id === slotId),
    restore.slot_id,
  )).toBe(true);

  // Starting the compatible row reloads this page. First prove the pre-init
  // gate staged it in the NEW App; this is deliberately only an intermediate
  // assertion, because pending=true alone also describes a restore that will
  // later be abandoned.
  await restoreRow.click();
  await expect.poll(() => catalogue.evaluate(() =>
    window.phSaveSlotsController.model().selected?.selectedShip)).toBe(DESTROYER);
  const startNavigations = [];
  const startConsole = [];
  const rememberNavigation = (frame) => {
    if (frame === catalogue.mainFrame()) startNavigations.push(frame.url());
  };
  const rememberConsole = (message) => startConsole.push(`${message.type()}: ${message.text()}`);
  catalogue.on('framenavigated', rememberNavigation);
  catalogue.on('console', rememberConsole);
  await catalogue.locator('[data-save-action="start"]').click();
  await waitForWasmReady(catalogue);
  expect(await catalogue.evaluate(() => sessionStorage.getItem('phoenix-save-peer-id')))
    .toBe(fallbackIdentity.identity);
  await expect.poll(() => catalogue.evaluate(() => ({
    parentId: document.getElementById('overlay')?.parentElement?.id || '',
    nextId: document.getElementById('overlay')?.nextElementSibling?.id || '',
    preScenario: document.getElementById('overlay')?.classList.contains('pre-scenario') || false,
  }))).toEqual({ parentId: 'viewscreen-shell', nextId: 'top-bar', preScenario: false });
  try {
    await catalogue.waitForFunction(
      () => !!(window.wasm_resume_pending && window.wasm_resume_pending()),
      null,
      { timeout: 60_000 },
    );
  } catch (error) {
    const diagnostic = await catalogue.evaluate((slotId) => ({
      href: location.href,
      status: document.getElementById('snapshot-status')?.textContent || '',
      identity: sessionStorage.getItem('phoenix-save-peer-id'),
      pending: !!(window.wasm_resume_pending && window.wasm_resume_pending()),
      row: Array.from(window.wasmBindings?.wasm_list_save_slots?.() || [])
        .find((entry) => entry.slot_id === slotId) || null,
    }), restore.slot_id);
    throw new Error(`${error.message}\n${JSON.stringify({ diagnostic, startNavigations, startConsole })}`);
  }
  await expect.poll(() => catalogue.url()).not.toContain('resume=');
  await expect.poll(() => catalogue.url()).not.toContain('scenario=');
  await expect.poll(() => catalogue.url()).not.toContain('ship=');
  expect(startNavigations.some((href) => new URL(href).searchParams.get('ship') === DESTROYER))
    .toBe(true);
  catalogue.off('framenavigated', rememberNavigation);
  catalogue.off('console', rememberConsole);

  // Preserve every status paint across the four-second visual fade. The Rust
  // bridge emits `ok/resumed at tick N` only after applying the snapshot,
  // recomputing the authoritative world digest, matching the recorded digest,
  // and finding no restore gaps. Refusal, corruption and abandonment all emit
  // `failed` instead, so none can satisfy the assertion below.
  await catalogue.evaluate(() => {
    const status = document.getElementById('snapshot-status');
    window.__restoreStatusEvents = [];
    const remember = () => {
      if (!status?.textContent) return;
      window.__restoreStatusEvents.push({
        text: status.textContent,
        className: status.className,
      });
    };
    new MutationObserver(remember).observe(status, {
      attributes: true,
      childList: true,
      characterData: true,
      subtree: true,
    });
    remember();
  });

  // Rebuild the roster and start the fresh App. Reusing the same participant
  // token is intentional: it is the same local player joining a different host
  // process, not a live-session restore route.
  const restoredHostId = await readHostPeerId(catalogue);
  const restoredCaptain = await createTestClient(context, restoredHostId, {
    name: 'Restored Save Tester',
    token: captain.token,
  });
  await restoredCaptain.send('SelectStation', { station: 'Captain' });
  await restoredCaptain.page.waitForFunction(
    (token) => window.__messages?.some(
      (message) => message.type === 'StationAssigned' && message.data.token === token,
    ),
    restoredCaptain.token,
    { timeout: 10_000 },
  );
  await restoredCaptain.send('SetReady', { ready: true });
  await catalogue.bringToFront();
  await restoredCaptain.waitForMessage('GameStarted', 20_000);

  await catalogue.waitForFunction(
    () => typeof window.wasm_resume_pending === 'function' && !window.wasm_resume_pending(),
    null,
    { timeout: 60_000 },
  );
  const restoreOutcomeHandle = await catalogue.waitForFunction(
    () => window.__restoreStatusEvents?.find((event) => {
      const classes = event.className.split(/\s+/);
      return classes.includes('ok') || classes.includes('failed');
    }),
    null,
    { timeout: 10_000 },
  );
  const restoreOutcome = await restoreOutcomeHandle.jsonValue();
  expect(restoreOutcome.className.split(/\s+/)).toContain('ok');
  expect(restoreOutcome.text).toBe(`resumed at tick ${saved.tick}`);

  // The saved non-default state came back through an authoritative blackboard,
  // while the mirrored logical tick resumed at (or just after) the exact tick
  // named by both the catalogue row and exported digest-bearing snapshot.
  await waitForRedAlert(restoredCaptain);
  const restoredTick = await catalogue.evaluate(() => window.wasm_sim_tick());
  expect(restoredTick).toBeGreaterThanOrEqual(saved.tick);

  // Continuation is live rather than frozen at the restored boundary: advance
  // beyond the observed restore tick and ensure the restored state remains the
  // authoritative value published afterwards.
  await catalogue.bringToFront();
  await expect.poll(
    () => catalogue.evaluate(() => window.wasm_sim_tick()),
    { timeout: 10_000, intervals: [50, 100, 200] },
  ).toBeGreaterThan(restoredTick);
  await waitForRedAlert(restoredCaptain);

  await restoredCaptain.close();
  await captain.close();
});

test('two same-origin browser peers keep autosave, manual, export, delete, and failures private', async ({
  context,
}) => {
  test.setTimeout(300_000);
  await installMultiHullWorld(context);
  const first = await startedHost(context, 'First Save Peer');
  const second = await startedHost(context, 'Second Save Peer');

  const identities = await Promise.all([first.page, second.page].map((page) =>
    page.evaluate(() => sessionStorage.getItem('phoenix-save-peer-id'))));
  expect(identities[0]).toMatch(/^[0-9a-f]{32}$/);
  expect(identities[1]).toMatch(/^[0-9a-f]{32}$/);
  expect(identities[1]).not.toBe(identities[0]);

  // Each independent session produces its own deterministic RunStarted
  // autosave. Their absolute ticks need not match because their lobbies began
  // and became ready at different times; the native two-peer/frame-batching
  // contract test covers equal decisions for peers in the same simulation.
  // Here each peer can see only its own Store row despite sharing this browser
  // context's origin-wide localStorage object.
  const [firstInitial, secondInitial] = await Promise.all([
    saveRows(first.page),
    saveRows(second.page),
  ]);
  const firstAutosave = firstInitial.find((row) => row.slot_id === 'autosave');
  const secondAutosave = secondInitial.find((row) => row.slot_id === 'autosave');
  expect(firstAutosave).toBeTruthy();
  expect(secondAutosave).toBeTruthy();
  expect(Number.isSafeInteger(Number(firstAutosave.capture_tick))).toBe(true);
  expect(Number.isSafeInteger(Number(secondAutosave.capture_tick))).toBe(true);
  expect(Number(firstAutosave.capture_tick)).toBeGreaterThanOrEqual(0);
  expect(Number(secondAutosave.capture_tick)).toBeGreaterThanOrEqual(0);

  const firstManual = await createNamedSave(first.page, 'First peer only');
  const secondManual = await createNamedSave(second.page, 'Second peer only');
  expect(firstManual.slot_id).not.toBe(secondManual.slot_id);

  const [firstRows, secondRows] = await Promise.all([
    saveRows(first.page),
    saveRows(second.page),
  ]);
  expect(firstRows.map((row) => row.display_name)).toContain('First peer only');
  expect(firstRows.map((row) => row.display_name)).not.toContain('Second peer only');
  expect(secondRows.map((row) => row.display_name)).toContain('Second peer only');
  expect(secondRows.map((row) => row.display_name)).not.toContain('First peer only');

  // Export resolves in the selected peer's namespace. Asking the other peer
  // for that opaque id is an ordinary local Empty refusal, never data access.
  const firstOwnExport = await exportSlot(first.page, firstManual.slot_id);
  expect(firstOwnExport.refusal).toBe('');
  expect(firstOwnExport.artifact).toContain(WORLD);
  const firstWrongExport = await exportSlot(first.page, secondManual.slot_id);
  expect(firstWrongExport.refusal).not.toBe('');
  expect(firstWrongExport.artifact).toBe('');
  const secondOwnExport = await exportSlot(second.page, secondManual.slot_id);
  expect(secondOwnExport.refusal).toBe('');
  expect(secondOwnExport.artifact).toContain(WORLD);

  // Refuse Storage writes in the first page's JavaScript realm only. The
  // second peer continues to capture while the first reports its local failure.
  await first.page.evaluate((identity) => {
    const storagePrototype = Object.getPrototypeOf(localStorage);
    window.__saveSlotsOriginalSetItem = storagePrototype.setItem;
    storagePrototype.setItem = function (key, value) {
      if (String(key).startsWith(`phoenix:${identity}:`)) {
        throw new DOMException('peer-local quota refusal', 'QuotaExceededError');
      }
      return window.__saveSlotsOriginalSetItem.call(this, key, value);
    };
  }, identities[0]);
  await first.page.bringToFront();
  const refusedSlot = await first.page.evaluate(() =>
    window.wasm_create_save_slot('First peer refused'));
  expect(refusedSlot).toMatch(/^[0-9a-f-]{36}$/);
  const secondAfterFailure = await createNamedSave(second.page, 'Second peer after failure');
  await expect(first.page.locator('#manual-save-panel .save-slots-status.failed')).toBeVisible({
    timeout: 20_000,
  });
  expect((await saveRows(first.page)).map((row) => row.display_name))
    .not.toContain('First peer refused');
  expect((await saveRows(second.page)).map((row) => row.slot_id))
    .toContain(secondAfterFailure.slot_id);
  await first.page.evaluate(() => {
    Object.getPrototypeOf(localStorage).setItem = window.__saveSlotsOriginalSetItem;
    delete window.__saveSlotsOriginalSetItem;
  });

  // Deleting the first peer's rolling/manual rows changes only its catalogue.
  // Even an id copied from the second peer cannot cross that namespace.
  expect(await first.page.evaluate((id) => window.wasm_delete_save_slot(id, true), firstManual.slot_id))
    .toBe('');
  expect(await first.page.evaluate(() => window.wasm_delete_save_slot('autosave', true))).toBe('');
  expect(await first.page.evaluate((id) => window.wasm_delete_save_slot(id, true), secondManual.slot_id))
    .toBe('');
  const firstAfterDelete = await saveRows(first.page);
  const secondAfterDelete = await saveRows(second.page);
  expect(firstAfterDelete.map((row) => row.slot_id)).not.toContain(firstManual.slot_id);
  expect(firstAfterDelete.map((row) => row.slot_id)).not.toContain('autosave');
  expect(secondAfterDelete.map((row) => row.slot_id)).toContain(secondManual.slot_id);
  expect(secondAfterDelete.map((row) => row.slot_id)).toContain('autosave');

  await first.captain.close();
  await second.captain.close();
});
