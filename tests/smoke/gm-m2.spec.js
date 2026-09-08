// #1316: deliberately opt-in milestone run against the unmodified Combat Test
// asset and a combined build. All GM mutations below originate in real controls.
import { readFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';
import { parse as parseToml } from 'smol-toml';
import { test, expect, createTestClient, captureServerPageErrors,
  readHostPeerId, waitForJoinCode, waitForWasmReady } from './fixtures';
import { observeGm, observeCrew, readGmEvidence, assertGmOnlyCrewWitness,
  retainEvidence } from './gm-m2-evidence.js';
import { ts } from './strings';

const execute = promisify(execFile);
const WORLD = 'assets/worlds/combat_test.toml';
const HULL = 'assets/entities/alliance_cruiser.toml';
const URL = `/?scenario=${WORLD}&ship=${HULL}`;
const REPOSITORY = path.resolve(__dirname, '../..');

async function settings(page, tab) {
  await page.locator('#server-settings-btn').click();
  await page.locator(`.server-settings-tab[data-tab="${tab}"]`).click();
}

async function joinGm(page, code) {
  await page.evaluate(() => localStorage.removeItem('phoenix.fleet.gm-identity.v1'));
  await settings(page, 'gameplay');
  await Promise.all([
    page.waitForURL(url => url.searchParams.get('gm') === '1'),
    page.locator('[data-control="fleet-role-gm"]').click(),
  ]);
  await waitForWasmReady(page);
  await settings(page, 'gameplay');
  await page.locator('[data-control="fleet-code"]').fill(code);
  await page.locator('[data-control="fleet-join"]').click();
  await page.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    return state?.admitted && state.presentationReady && state.localValidation;
  });
  await page.locator('#server-settings-btn').click();
}

async function selectEntity(page, uuid) {
  const map = page.locator('#gm-entity-map');
  await map.scrollIntoViewIfNeeded(); await map.focus();
  const count = await map.evaluate(element =>
    element.state.blips.length + (element.state.regions?.length || 0));
  // The map's ordinary accessible selection ring, including its stable UUID
  // semantics, remains under test; this helper never dispatches a fake payload.
  await map.press('Home');
  for (let i = 0; i <= count; i++) {
    if (await page.locator('#gm-entity-card').getAttribute('data-entity-id') === uuid) return;
    await map.press('ArrowRight');
  }
  throw new Error(`Entity ${uuid} is absent from the real map selection ring`);
}

async function requestCount(page) {
  return page.evaluate(() => window.__m2Evidence.requests.length);
}

async function settleControl(page, click, { outcome = 'applied', cancel = false, afterSubmit } = {}) {
  const before = await requestCount(page);
  await click();
  await page.waitForFunction(before => !document.getElementById('gm-action-confirmation').hidden
    || window.__m2Evidence.requests.length > before, before);
  if (await page.locator('#gm-action-confirmation').isVisible()) {
    await page.locator(cancel ? '[data-confirmation-cancel]' : '[data-confirmation-accept]').click();
  } else if (cancel) throw new Error('Cancellation requires an actual confirmation');
  if (cancel) {
    expect(await requestCount(page)).toBe(before);
    return null;
  }
  await expect.poll(() => requestCount(page)).toBeGreaterThan(before);
  const request = await page.evaluate(index => window.__m2Evidence.requests[index].request, before);
  const correlation = request.correlation;
  expect(correlation).toBeTruthy();
  if (afterSubmit) await afterSubmit();
  await page.waitForFunction(({ correlation, operator, outcome }) => {
    const activity = window.__hostGmActivityState().entries.some(entry => entry.category === 'gm_action'
      && entry.detail.data.correlation === correlation && entry.detail.data.operator.id === operator
      && entry.detail.data.outcome === outcome);
    const station = window.__hostGmStationState().projection.results.some(result =>
      result.correlation === correlation && result.operator_id === operator && result.outcome === outcome);
    return activity || station || !!document.querySelector(
      `[data-correlation="${correlation}"][data-outcome="${outcome}"]`);
  }, { correlation, operator: request.operator_id, outcome });
  return request;
}

async function exportOrigin(ship) {
  const pending = ship.waitForEvent('download');
  await settings(ship, 'debug');
  await ship.locator('[data-action="export-snapshot"]').click();
  const download = await pending;
  await ship.locator('#server-settings-btn').click();
  return readFile(await download.path(), 'utf8');
}

async function exportFinalAutosave(ship) {
  // Export the existing GameOver capture. Asking to capture a new manual save
  // after GameOver is correctly refused and would not prove the ending tick.
  return ship.evaluate(() => {
    const api = window.wasmBindings;
    const rows = Array.from(api.wasm_list_save_slots()).map(row =>
      typeof row === 'string' ? JSON.parse(row) : row);
    const row = rows.find(row => row.kind === 'autosave');
    if (!row) throw new Error('The ordinary GameOver autosave is missing');
    const refusal = api.wasm_export_save_slot(row.slot_id);
    if (refusal) throw new Error(refusal);
    const text = api.wasm_take_exported_snapshot();
    if (!text) throw new Error('The ordinary autosave export returned no artifact');
    return { text, catalogue: rows };
  });
}

test('M2 Combat Test directing produces an identical replay of its browser record',
  { tag: '@m2' }, async ({ context }, testInfo) => {
  test.skip(process.env.PHOENIX_M2_EXIT !== '1', 'explicit combined milestone gate');
  test.setTimeout(900_000);
  context.setDefaultTimeout(30_000);
  expect(process.env.PHOENIX_GM_REPLAY_EXE, 'build and freeze the native recorded_gm_exports binary').toBeTruthy();
  const ship = await context.newPage();
  const authored = parseToml(await readFile(`${REPOSITORY}/${WORLD}`, 'utf8'));
  const errors = [captureServerPageErrors(ship)];
  const crew = [], gms = [], checkpoints = [];
  let originPath, finalPath;
  try {
    await ship.goto(URL); await waitForWasmReady(ship);
    const host = await readHostPeerId(ship);
    // Settle crew-count layout before seating. No ordinary crew command, rating
    // or seat changes are permitted after the origin; outgoing traffic is kept.
    for (const station of ['Captain', 'Helm', 'Engineering', 'Science']) {
      crew.push(await createTestClient(context, host, { name: `M2 ${station}` }));
    }
    for (const [index, station] of ['Captain', 'Helm', 'Engineering', 'Science'].entries()) {
      await crew[index].send('SelectStation', { station });
      await crew[index].page.waitForFunction(({ token, station }) => window.__messages.some(message =>
        message.type === 'StationAssigned' && message.data.token === token
        && message.data.station?.toLowerCase() === station.toLowerCase()),
      { token: crew[index].token, station });
      if (station === 'Captain') await crew[index].send('SetStationRating', { rating_name: 'Std' });
      await observeCrew(crew[index]);
    }
    await ship.evaluate(() => window.__hostFleetOpen());
    await waitForJoinCode(ship, 'fleet-code');
    const code = await ship.locator('#fleet-code').textContent();
    for (let i = 0; i < 2; i++) {
      const page = await context.newPage(); errors.push(captureServerPageErrors(page));
      await page.goto(URL); await waitForWasmReady(page); await joinGm(page, code);
      await observeGm(page, `GM ${i + 1}`); gms.push(page);
    }
    for (const client of crew) await client.send('SetReady', { ready: true });
    for (const page of gms) await page.locator('#gm-ready-btn').click();
    await Promise.all([ship, ...gms].map(page => page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress')));
    const [gm, second] = gms;
    const operators = await Promise.all(gms.map(page => page.evaluate(() => window.__hostLocalGm().id)));
    expect(new Set(operators).size).toBe(2);
    originPath = await retainEvidence(testInfo, 'origin.ron', await exportOrigin(ship), 'text/plain');

    const checkpoint = async name => {
      const row = { name, gms: await Promise.all(gms.map(readGmEvidence)) };
      checkpoints.push(row); return row;
    };
    const session = (page, verb) => settleControl(page, () => page.locator(`#gm-session-${verb}`).click());
    const pulse = async () => {
      const before = await gm.evaluate(() => window.wasm_sim_tick());
      await session(second, 'resume');
      await gm.waitForFunction(before => window.wasm_sim_tick() >= before + 3, before);
      await session(second, 'pause');
    };
    const event = (page, id, verb, options) => settleControl(page, () => page.locator(
      `#gm-mission-events [data-event-id="base-world::${id}"] button[data-role="${verb}"]`).click(), options);
    await session(gm, 'pause');
    for (let wave = 2; wave <= 8; wave++) await event(gm, `release_wave_${wave}`, 'pause');
    await event(gm, 'report_wave_2', 'skip', { cancel: true });
    await event(gm, 'report_wave_2', 'skip');
    await event(second, 'release_wave_2', 'pause');
    await event(gm, 'release_wave_2', 'pause');
    await checkpoint('session and event pause, resume, skip, cancellation');

    const objective = (page, id, verb) => settleControl(page, () => page.locator(
      `#gm-objective-list .gm-objective-row[data-objective="${id}"] button[data-verb="${verb}"]`).click());
    await Promise.all([
      objective(gm, 'gm-relief-rendezvous', 'activate'),
      objective(second, 'gm-relief-cover', 'activate'),
    ]);
    await objective(gm, 'gm-relief-rendezvous', 'complete');
    await objective(second, 'gm-relief-cover', 'fail');
    for (const page of gms) {
      await expect(page.locator('#gm-objective-list .gm-objective-row[data-objective="gm-relief-rendezvous"]')).toHaveAttribute('data-status', 'Completed');
      await expect(page.locator('#gm-objective-list .gm-objective-row[data-objective="gm-relief-cover"]')).toHaveAttribute('data-status', 'Failed');
    }
    await pulse();
    await crew[0].page.waitForFunction(() => window.__messages.some(message => message.type === 'ObjectiveSummary'
      && message.data.objectives.some(row => row.id === 'gm-relief-rendezvous' && row.status === 'Completed')));
    await checkpoint('simultaneous equal GM Objective ownership');

    const player = await gm.evaluate(() => window.__hostGmStationState().projection.ships
      .find(row => row.stations.some(station => station.station_id === 'captain' && station.rating === 'Std'))?.ship_id);
    expect(player, 'frozen lobby ratings arrive without a corrective crew command').toBeTruthy();
    await selectEntity(gm, player);
    await gm.locator('#gm-system-select').selectOption('red-alert');
    const hullBefore = await gm.locator('#gm-entity-hull').getAttribute('value');
    await settleControl(gm, () => gm.locator('#gm-system-disable').click());
    await pulse();
    await gm.waitForFunction(id => window.__hostGmSystemState().controls[id]
      .find(row => row.system_id === 'red-alert').gm_disabled, player);
    await settleControl(gm, () => gm.locator('#gm-system-restore').click());
    await pulse();
    await gm.waitForFunction(id => !window.__hostGmSystemState().controls[id]
      .find(row => row.system_id === 'red-alert').gm_disabled, player);
    await expect(gm.locator('#gm-entity-hull')).toHaveAttribute('value', hullBefore);

    const puppet = async (page, shipId, stationId) => {
      const choice = await page.evaluate(({ shipId, stationId }) => {
        const row = window.__hostGmStationState().projection.ships.find(row => row.ship_id === shipId);
        const station = row.stations.find(row => row.station_id === stationId);
        return [...document.getElementById('gm-station-select').options]
          .find(option => option.textContent === `${row.name} — ${station.name}`)?.value;
      }, { shipId, stationId });
      expect(choice).toBeTruthy(); await page.locator('#gm-station-select').selectOption(choice);
      await settleControl(page, () => page.locator('#gm-station-toggle').click());
    };
    await puppet(gm, player, 'captain'); await puppet(second, player, 'captain');
    await pulse();
    await gm.waitForFunction(ids => window.__hostGmStationState().selectedRow.station.operators
      .filter(id => ids.includes(id)).length === 2, operators);
    const alertBefore = await gm.frameLocator('#gm-station-frame').locator('ph-red-alert #alert-btn')
      .evaluate(button => button.classList.contains('active'));
    await settleControl(gm, () => gm.frameLocator('#gm-station-frame').locator('ph-red-alert #alert-btn').click(),
      { afterSubmit: () => session(second, 'resume') });
    await gm.waitForFunction(before => window.__hostGmStationState().selectedRow.ship.blackboards
      .some(([, board]) => board.kind === 'Captain' && board.data.red_alert === !before), alertBefore);
    await session(second, 'pause');
    await settleControl(gm, () => gm.locator('#gm-station-toggle').click());
    await settleControl(second, () => second.locator('#gm-station-toggle').click());
    await checkpoint('System isolation and two equal puppets of a human-held Captain');

    const npc = await gm.evaluate(() => Object.keys(window.__hostGmNpcState().profiles)[0]);
    expect(npc).toBeTruthy(); await selectEntity(gm, npc);
    await gm.locator('#gm-npc-choice').selectOption('raider-regroup');
    await settleControl(gm, () => gm.locator('#gm-npc-apply').click());
    await session(second, 'resume');
    await gm.waitForFunction(id => window.__hostGmNpcState().profiles[id]?.current === 'raider-regroup', npc);
    const regroupBefore = await gm.evaluate(id => window.__hostGmStationState().projection.ships
      .find(row => row.ship_id === id).ship_pose, npc);
    await expect(gm.locator('#gm-npc-intent')).toHaveText(ts('world.combat_test.gm.doctrine.regroup'));
    await gm.waitForFunction(({ id, before, anchor }) => {
      const pose = window.__hostGmStationState().projection.ships.find(row => row.ship_id === id).ship_pose;
      const distance = value => Math.hypot(value.x - anchor[0], value.z - anchor[2]);
      return distance(pose) < distance(before) - 1;
    }, { id: npc, before: regroupBefore, anchor: authored.anchors.enemy_e_far });
    await session(second, 'pause');
    await gm.locator('#gm-npc-choice').selectOption('raider-assault');
    await settleControl(gm, () => gm.locator('#gm-npc-apply').click());
    await pulse();
    await gm.waitForFunction(id => window.__hostGmNpcState().profiles[id]?.current === 'raider-assault', npc);
    await expect(gm.locator('#gm-npc-intent')).toHaveText(ts('world.combat_test.gm.doctrine.assault'));
    await puppet(gm, npc, 'helm');
    await session(second, 'resume');
    const poseBefore = await gm.evaluate(id => window.__hostGmStationState().projection.ships
      .find(row => row.ship_id === id).ship_pose, npc);
    const joystick = gm.frameLocator('#gm-station-frame').locator('ph-helm-joystick');
    await joystick.focus(); await gm.keyboard.down('ArrowUp');
    try {
      await gm.waitForFunction(id => window.__hostGmStationState().projection.activity
        .some(entry => entry.ship === id && entry.action === 'SetThrust'), npc);
    } finally { await gm.keyboard.up('ArrowUp'); }
    await gm.waitForFunction(({ id, before }) => JSON.stringify(window.__hostGmStationState().projection.ships
      .find(row => row.ship_id === id).ship_pose) !== JSON.stringify(before), { id: npc, before: poseBefore });
    await session(second, 'pause');
    await settleControl(gm, () => gm.locator('#gm-station-toggle').click());
    await checkpoint('authored NPC doctrine and compatible Helm input');

    await selectEntity(gm, npc);
    await gm.locator('#gm-contact-observer').selectOption(player);
    await gm.locator('#gm-knowledge-select').selectOption(player);
    for (const mode of ['reveal', 'conceal', 'normal']) {
      await settleControl(gm, () => gm.locator(`#gm-contact-${mode}`).click());
      await pulse();
      await gm.waitForFunction(({ player, npc, mode }) =>
        (window.__hostGmContactState().overrides[player]?.[npc] || 'normal') === mode, { player, npc, mode });
      const picture = () => crew[0].page.evaluate(async id => {
        const { ClientSimState } = await import('/gui/sim-state.js');
        const { buildSensorsConsoleState } = await import('/gui/console-state.js');
        const state = new ClientSimState();
        for (const message of window.__messages) state.apply(message);
        return JSON.parse(buildSensorsConsoleState(state)).blips.some(row => row.uuid === id);
      }, npc);
      if (mode === 'reveal') await expect.poll(picture).toBe(true);
      if (mode === 'conceal') await expect.poll(picture).toBe(false);
      await expect(gm.locator('#gm-knowledge-contacts-rows')).toContainText(
        await gm.locator('#gm-entity-name').textContent());
      await checkpoint(`Truth and Crew Knowledge: ${mode}`);
    }
    const effect = async (page, target, scope, verb, amount) => {
      await selectEntity(page, target); await page.locator('#gm-effect-scope').selectOption(scope);
      await page.locator('#gm-effect-amount').fill(String(amount));
      return settleControl(page, () => page.locator(`#gm-effect-${verb}`).click());
    };
    for (const scope of ['entity', 'station:helm', 'system:helm-engine-port']) {
      await selectEntity(gm, player); await gm.locator('#gm-effect-scope').selectOption(scope);
      const before = await gm.locator('#gm-effect-hull').textContent();
      await effect(gm, player, scope, 'damage', 5);
      await pulse();
      await expect(gm.locator('#gm-effect-hull')).not.toHaveText(before);
      await effect(gm, player, scope, 'heal', 5);
      await pulse();
      await expect(gm.locator('#gm-effect-hull')).toHaveText(before);
    }
    await checkpoint('Entity, Station and System damage and repair');

    const palette = gm.locator('#gm-spawn-palette [data-palette-id="relief-cruiser"].gm-spawn-entry');
    await palette.locator('select').selectOption('removable');
    const existing = await gm.evaluate(() => document.getElementById('gm-entity-map').state.blips.map(row => row.uuid));
    await settleControl(gm, async () => {
      await palette.locator('button[data-role="place"]').click();
      await gm.keyboard.press('ArrowRight'); await gm.keyboard.press('Enter');
    });
    await session(gm, 'resume');
    await gm.waitForFunction(before => document.getElementById('gm-entity-map').state.blips
      .some(row => row.kind === 'npc_ship' && !before.includes(row.uuid)), existing);
    await session(gm, 'pause');
    const spawned = await gm.evaluate(before => document.getElementById('gm-entity-map').state.blips
      .find(row => row.kind === 'npc_ship' && !before.includes(row.uuid)).uuid, existing);
    await selectEntity(gm, spawned); await selectEntity(second, spawned);
    await settleControl(gm, () => gm.locator('#gm-despawn-preview').click(), { cancel: true });
    // One operator holds a captured lethal intent while the equally privileged
    // peer removes its target. Acceptance must return canonical Refused.
    await gm.locator('#gm-effect-amount').fill('999999'); await gm.locator('#gm-effect-damage').click();
    await expect(gm.locator('#gm-action-confirmation')).toBeVisible();
    await settleControl(second, () => second.locator('#gm-despawn-preview').click());
    await session(second, 'resume');
    await second.waitForFunction(id => !document.getElementById('gm-entity-map').state.blips.some(row => row.uuid === id), spawned);
    await session(second, 'pause');
    await settleControl(gm, () => gm.locator('[data-confirmation-accept]').click(), { outcome: 'refused' });
    await checkpoint('map placement, cancelled removal and stale canonical refusal');

    // The report-only trigger is skipped at its original deadline; all future
    // wave timers are paused, so this wait does not skip or rewrite a wave.
    await effect(gm, npc, 'entity', 'damage', 999999);
    await session(gm, 'resume');
    await gm.waitForFunction(() => document.querySelector('[data-event-id="base-world::report_wave_2"]')?.dataset.spent === 'true',
      undefined, { timeout: 90_000 });
    await session(gm, 'pause');
    for (let wave = 2; wave <= 8; wave++) await event(gm, `release_wave_${wave}`, 'fire');
    await session(gm, 'resume');
    await gm.waitForFunction(() => document.querySelector('[data-event-id="base-world::release_wave_8"]')?.dataset.spent === 'true');
    await session(gm, 'pause');
    const raiders = await gm.evaluate(() => document.getElementById('gm-entity-map').state.blips
      .filter(row => row.kind === 'npc_ship' && !row.destroyed).map(row => row.uuid));
    expect(raiders.length).toBeGreaterThan(0);
    for (const target of raiders) await effect(gm, target, 'entity', 'damage', 999999);
    await checkpoint('all eight authored waves released and cleared through GM effects');
    await session(gm, 'resume');
    await Promise.all([ship, ...gms].map(page => page.waitForFunction(() => window.__saveSlotsPhase === 'GameOver',
      undefined, { timeout: 90_000 })));
    const final = await exportFinalAutosave(ship);
    finalPath = await retainEvidence(testInfo, 'final.ron', final.text, 'text/plain');
    await retainEvidence(testInfo, 'final-catalogue.json', final.catalogue);
    await checkpoint('ordinary Combat Test ending');
    const witnesses = await Promise.all(crew.map(client => assertGmOnlyCrewWitness(client, expect)));
    const crewViews = await Promise.all(crew.map(client => client.page.evaluate(() => {
      const last = new Map();
      for (const message of window.__messages) last.set(message.type, message);
      return [...last.values()];
    })));
    for (const views of crewViews) {
      const fiction = JSON.stringify(views.filter(message => ['ObjectiveSummary', 'CommsMessage', 'GameOver'].includes(message.type)));
      for (const operator of operators) expect(fiction).not.toContain(operator);
      expect(views.find(message => message.type === 'GameOver')?.data.outcome).toBe('victory');
    }
    // These observations are required to interpret the ordinary exports even
    // when native replay refuses them. Preserve them before spawning it.
    await retainEvidence(testInfo, 'crew-input-witnesses.json', {
      world: WORLD, seed: authored.global.seed, operators, witnesses, crewViews,
    });
    const reportPath = testInfo.outputPath('native-replay.json');
    let replay;
    try {
      replay = await execute(process.env.PHOENIX_GM_REPLAY_EXE,
        ['verify_browser_gm_exports', '--exact', '--ignored', '--nocapture', '--test-threads=1'],
        { cwd: REPOSITORY, timeout: 180_000, maxBuffer: 4 * 1024 * 1024,
          env: { ...process.env, PHOENIX_GM_RECORDING_INITIAL: originPath,
            PHOENIX_GM_RECORDING_FINAL: finalPath, PHOENIX_GM_RECORDING_REPORT: reportPath } });
    } catch (error) {
      await retainEvidence(testInfo, 'native-replay.log',
        String(error.stdout ?? '') + String(error.stderr ?? ''), 'text/plain');
      await retainEvidence(testInfo, 'native-process.json', {
        completed: false, exitCode: typeof error.code === 'number' ? error.code : null,
        errorCode: typeof error.code === 'string' ? error.code : null,
        signal: error.signal ?? null, killed: error.killed ?? null, message: error.message,
      });
      // A refused replay may still have written its structured report. Keep it
      // with the failure without substituting it for the original process error.
      const refusal = await readFile(reportPath, 'utf8').catch(() => null);
      if (refusal !== null) await retainEvidence(testInfo, 'native-replay.json', refusal);
      throw error;
    }
    await retainEvidence(testInfo, 'native-replay.log', replay.stdout + replay.stderr, 'text/plain');
    await retainEvidence(testInfo, 'native-process.json', { completed: true, exitCode: 0 });
    const proof = JSON.parse(await readFile(reportPath, 'utf8'));
    expect(proof.pass).toBe(true); expect(proof.report.results_match).toBe(true);
    expect(proof.report.actual_final_digest).toBe(proof.report.expected_final_digest);
    expect(errors.flat()).toEqual([]);
    await retainEvidence(testInfo, 'm2-report.json', { pass: true, world: WORLD, seed: authored.global.seed,
      operators, witnesses, crewViews, checkpoints, replay: proof, errors });
  } finally {
    // Preserve partial observations on a failure too, without labelling them PASS.
    await retainEvidence(testInfo, 'browser-observations.json', { originPath, finalPath, checkpoints,
      gms: await Promise.all(gms.map(page => readGmEvidence(page).catch(error => ({ error: String(error) })))), errors });
    for (const client of crew) await client.close();
  }
});
