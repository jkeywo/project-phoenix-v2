// #1044: one full, unmodified authored timeline in the browser. Human tuning,
// manual storm navigation and allocation choices remain separate acceptance.
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { test, expect, createTestClient, readHostPeerId,
  waitForWasmReady, expectFixtureWorld, captureServerPageErrors } from './fixtures';

test('Falling Skyway major flow reaches spectators and its ending', async ({ context }, testInfo) => {
  test.skip(process.env.PHOENIX_SKYWAY_FLOW !== '1', 'opt-in full authored timeline');
  test.setTimeout(40 * 60_000);
  const scenario = 'assets/worlds/falling_skyway.toml';
  const world = readFileSync(path.resolve(__dirname, '../../', scenario), 'utf8');
  const host = await context.newPage();
  const errors = captureServerPageErrors(host);
  await host.goto(`/?scenario=${scenario}&ship=assets/entities/alliance_destroyer.toml`);
  await waitForWasmReady(host);
  const code = await readHostPeerId(host);
  const witness = await createTestClient(context, code, { name: 'Skyway wire observer' });
  const seated = await createTestClient(context, code, { name: 'Skyway screen observer' });
  for (const client of [witness, seated]) {
    await client.send('SetSpectator', { spectator: true });
    await client.page.waitForFunction(token => window.__messages.some(message =>
      message.type === 'SpectatorChanged' && message.data.token === token
        && message.data.spectator === true), client.token, { timeout: 15_000 });
  }
  const token = seated.token;
  await seated.close();
  const crew = await context.newPage();
  await crew.addInitScript(token => sessionStorage.setItem('session-token', token), token);
  await crew.goto(`/client/index.html#${code}`);
  await host.evaluate(() => window.wasm_set_debug_surface('ScenarioState', true));
  await witness.page.evaluate(() => {
    window.__skywayObjectives = {};
    window.__skywayEnding = null;
    window.__skywayCollector = setInterval(() => {
      for (const message of window.__messages) {
        if (message.type === 'ObjectiveSummary') for (const objective of message.data.objectives) {
          const states = window.__skywayObjectives[objective.id] ??= [];
          if (!states.includes(objective.status)) states.push(objective.status);
        }
        if (message.type === 'GameOver') window.__skywayEnding = message;
      }
      window.__messages = window.__messages.filter(message =>
        message.type === 'WorldSetup' || message.type === 'GameStarted');
      window.__reliableMessages = [];
      window.__snapshotMessages = [];
    }, 100);
  });
  const milestones = [];
  const checkpoint = async label => {
    const tick = await host.evaluate(() => window.wasm_sim_tick());
    milestones.push({ label, tick });
    console.log(`Skyway: ${label} at tick ${tick}`);
  };
  try {
    // The chrome hides its AI-only button once spectators join. Use that
    // button's existing host entry point so observers receive initial state;
    // this is a production launch request, never a scenario-state mutation.
    await host.evaluate(() => window.wasm_force_start());
    await witness.waitForMessage('GameStarted', 30_000);
    expectFixtureWorld(await witness.waitForMessage('WorldSetup'), world);
    await expect(crew.locator('#spectator-ui')).toBeVisible({ timeout: 30_000 });
    await host.bringToFront();
    const objective = (id, status, timeout) => witness.page.waitForFunction(
      ({ id, status }) => window.__skywayObjectives[id]?.includes(status), { id, status }, { timeout });
    const flag = (name, timeout) => host.waitForFunction(name =>
      JSON.parse(window.wasm_get_scenario_state()).flags.some(flag => flag.name === name && flag.value > 0),
    name, { timeout });
    const deadline = (id, timeout) => host.waitForFunction(id =>
      JSON.parse(window.wasm_get_scenario_state()).deadlines.some(row => row.id === id && row.state === 'fired'),
    id, { timeout });
    await objective('obj-a1-corridor', 'Active', 30_000);
    await objective('obj-a1-triage', 'Active', 30_000);
    await checkpoint('load and Act 1 triage');
    await objective('obj-a1-corridor', 'Completed', 4 * 60_000);
    await flag('skyway_settled_by_negotiation', 5 * 60_000);
    await flag('act1_complete', 5 * 60_000);
    await checkpoint('Act 1 resolved');
    for (const id of ['storm_front_due', 'storm_band_one_due', 'storm_band_two_due', 'storm_band_three_due']) {
      await deadline(id, 5 * 60_000);
      await checkpoint(id);
    }
    await flag('act2_complete', 5 * 60_000);
    await deadline('skyway_transfer_window', 12 * 60_000);
    await checkpoint('transfer window opened');
    // Backfill's choices are observed, not prescribed. Verify the authored
    // consequence of this run's actual allocation after the window closes.
    await deadline('skyway_window_closes', 5 * 60_000);
    await checkpoint('transfer window closed');
    await witness.page.waitForFunction(() => window.__skywayEnding !== null, undefined, { timeout: 3 * 60_000 });
    const ending = await witness.page.evaluate(() => window.__skywayEnding);
    expect(ending.data.report.length).toBeGreaterThan(0);
    const scenarioState = await host.evaluate(() => JSON.parse(window.wasm_get_scenario_state()));
    const flags = Object.fromEntries(scenarioState.flags.map(({ name, value }) => [name, value]));
    expect(flags.skyway_window_open ?? 0).toBe(0);
    expect(flags.skyway_window_closed).toBe(1);
    expect(flags.skyway_mission_finalized).toBe(1);
    expect(flags.skyway_mission_resolved).toBe(1);
    const windowStatus = (flags.skyway_window_lifts_started ?? 0) > 0 ? 'Completed' : 'Failed';
    await objective('obj-a3-window', windowStatus, 30_000);
    const carried = ['committee', 'havelock', 'convoy'].filter(claimant =>
      (flags[`skyway_window_served_${claimant}`] ?? 0) > 0
      || (flags[`skyway_berth_${claimant}`] ?? 0) > 0).length;
    expect(ending.data.report.find(row => row.id === 'lifts')).toMatchObject({
      state: carried >= 2 ? 'saved' : carried === 1 ? 'partial' : 'lost',
      outcome: `world.falling_skyway.report.lifts.${carried >= 2 ? 'full' : carried === 1 ? 'some' : 'none'}`,
    });
    await expect(crew.locator('#game-over-overlay')).toBeVisible();
    await expect(crew.locator('#game-over-report dt')).toHaveCount(ending.data.report.length);
    await expect(crew.locator('#game-over-message')).not.toBeEmpty();
    await checkpoint('visible crew ending');
    expect(errors).toEqual([]);
    await crew.screenshot({ path: testInfo.outputPath('skyway-ending.png') });
  } finally {
    const receipt = { scenario, worldSha256: createHash('sha256').update(world).digest('hex'), milestones, errors,
      ending: await witness.page.evaluate(() => window.__skywayEnding),
      objectives: await witness.page.evaluate(() => window.__skywayObjectives),
      scenarioState: await host.evaluate(() => {
        const raw = window.wasm_get_scenario_state();
        return raw ? JSON.parse(raw) : null;
      }) };
    const receiptPath = testInfo.outputPath('skyway-browser-flow.json');
    writeFileSync(receiptPath, JSON.stringify(receipt, null, 2));
    await testInfo.attach('skyway-browser-flow', { contentType: 'application/json', path: receiptPath });
  }
});
