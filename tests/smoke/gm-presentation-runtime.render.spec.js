import { test, expect, createTestClient, expectFixtureWorld, readHostPeerId,
  waitForJoinCode, waitForWasmReady } from './fixtures';
import { ts } from './strings';
import { clickGmControl, revealGmPanel } from './dock-helpers.js';

const WORLD = `
[global]
seed = 1468
title = "Presentation admission fixture"
description = "The real GM lane stages the crew's shared Viewscreen."
[[available_ships]]
template_path = "assets/entities/alliance_cruiser.toml"
[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
spawn_on = "game_start"
`;

test('real GM presentation admission reaches the host card and crew view while rejecting ship impersonation', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(180_000);
  await context.route('**/assets/worlds/default.toml', route => route.fulfill({ contentType: 'text/plain', body: WORLD }));
  const host = await context.newPage(), gm = await context.newPage();
  // The ordinary automation boot deliberately omits ViewscreenBorderPlugin,
  // which owns the HUD/card producer. Exercise the real Viewscreen with this
  // project's SwiftShader backend; the independent GM keeps its ordinary boot.
  await host.addInitScript(() => Object.defineProperty(navigator, 'webdriver', { get: () => false }));
  await host.goto('/?scenario=assets/worlds/default.toml'); await waitForWasmReady(host);
  const crew = await createTestClient(context, await readHostPeerId(host), { name: 'Presentation witness' });
  await crew.send('SelectStation', { station: 'Captain' });
  await host.evaluate(() => window.__hostFleetOpen()); await waitForJoinCode(host, 'fleet-code');
  const code = await host.locator('#fleet-code').textContent();
  await gm.addInitScript(() => localStorage.removeItem('phoenix.fleet.gm-identity.v1'));
  await gm.goto('/?gm=1&scenario=assets/worlds/default.toml'); await waitForWasmReady(gm);
  await gm.locator('#server-settings-btn').click();
  await gm.locator('.server-settings-tab[data-tab="gameplay"]').click();
  await gm.locator('[data-control="fleet-code"]').fill(code);
  await gm.locator('[data-control="fleet-join"]').click();
  await gm.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    return state?.admitted && state.presentationReady && state.localValidation;
  });
  await gm.locator('#server-settings-btn').click();
  await crew.send('SetReady', { ready: true }); await clickGmControl(gm, 'gm-ready-btn');
  await Promise.all([host, gm].map(page => page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress')));
  expectFixtureWorld(await crew.waitForMessage('WorldSetup'), WORLD);
  // Presentation is a dock panel since issue #1505 — the shown tab of its group
  // by default, but a spec should not depend on that.
  await revealGmPanel(gm, 'presentation');
  const panel = gm.locator('#gm-presentation-panel');
  await expect(panel.locator('#gm-presentation-ship option')).toHaveCount(1);
  await clickGmControl(gm, 'gm-session-pause');
  await gm.waitForFunction(() => window.__hostGmSessionState().paused);
  const heldTick = await gm.evaluate(() => window.wasm_sim_tick());
  await panel.locator('#gm-presentation-duration').fill('12000');
  await panel.locator('#gm-presentation-heading').fill('Arrival at Lyra');
  await panel.locator('#gm-presentation-body').fill('Awaiting the crew');
  const button = key => panel.getByRole('button', { name: ts(`server.gm.presentation.${key}`), exact: true });
  await button('show_title').click();
  await expect(panel.locator('[role=status]')).toHaveAttribute('data-state', 'applied');
  const card = host.locator('.vs-presentation-card');
  await expect(card).toBeVisible(); await expect(card).toContainText('Arrival at Lyra');
  expect(await gm.evaluate(() => window.wasm_sim_tick())).toBe(heldTick);
  await clickGmControl(gm, 'gm-session-resume');
  await gm.waitForFunction(() => !window.__hostGmSessionState().paused);
  // A12: keep the actual remote GM admission and host channel in this proof.
  // Silence unrelated background buses through the recipient's real mixer;
  // the fixture has no world alert to produce its own Alerts output.
  await host.locator('#server-settings-btn').click();
  await host.locator('.server-settings-tab[data-tab="audio"]').click();
  for (const bus of ['music', 'ambience', 'effects']) {
    const mute = host.locator(`#server-settings-overlay [data-audio-bus="${bus}"] button`);
    if (await mute.isEnabled() && await mute.getAttribute('aria-pressed') !== 'true') await mute.click();
  }
  await host.locator('#server-settings-overlay [data-audio-enable]').click();
  await host.waitForFunction(() => window.__audioDebug().output.ready.includes('authored_red-alert'));
  await expect.poll(() => host.evaluate(() => window.__audioDebug().outputPeak)).toBeLessThan(0.00001);
  await host.evaluate(() => {
    // Observe and forward the real event; never synthesize acceptance or PCM.
    const forward = window.__audioCue;
    window.__authoredSoundProof = { count: 0, peak: 0 };
    window.__audioCue = payload => {
      if (JSON.parse(payload).kind === 'authored') window.__authoredSoundProof.count++;
      forward(payload);
    };
    window.__authoredSoundSampler = setInterval(() => {
      window.__authoredSoundProof.peak = Math.max(window.__authoredSoundProof.peak, window.__audioDebug().outputPeak);
    }, 10);
  });
  await panel.locator('#gm-presentation-sound').selectOption('red-alert');
  await button('play_sound').click();
  await expect(panel.locator('[role=status]')).toHaveAttribute('data-state', 'applied');
  // The accessibility equivalent is deliberately a two-second live cue. Read
  // it before the independent analyser poll, which can be descheduled beyond
  // that window on a contended CI renderer even though the sound did play.
  await expect(host.locator('[data-audio-equivalent="authored"]')).toContainText('Ship computer');
  await expect.poll(() => host.evaluate(() => window.__authoredSoundProof.peak)).toBeGreaterThan(0.00001);
  expect(await host.evaluate(() => window.__authoredSoundProof.count)).toBe(1);
  expect(await gm.evaluate(() => window.__audioDebug().output.active.some(voice => voice.id === 'authored_red-alert'))).toBe(false);
  await host.evaluate(() => clearInterval(window.__authoredSoundSampler));
  await host.locator('#server-settings-btn').click();
  const ship = await panel.locator('#gm-presentation-ship').inputValue();
  const operator = await gm.evaluate(() => window.__hostGmStartState().operatorId);
  const ingress = await host.evaluate(request => ({ tick: window.wasm_sim_tick(),
    queued: window.wasm_submit_gm_action(JSON.stringify(request)),
  }), { action: 'presentation', operator_id: operator, correlation: 'ship-presentation-impersonation', ship, cue: 'clear_card' });
  expect(ingress.queued).toBe(true);
  await host.waitForFunction(tick => window.wasm_sim_tick() >= tick + 20, ingress.tick);
  await expect(card).toContainText('Arrival at Lyra');
  await panel.locator('#gm-presentation-view').selectOption('radar');
  await button('force').click();
  await crew.page.waitForFunction(() => window.__messages.some(message => message.type === 'BlackboardUpdate'
    && message.data.updates?.some(([id, bb]) => id === 'captain' && bb?.data?.view_mode?.kind === 'Radar')));
  await expect(panel.locator('[role=status]')).toHaveAttribute('data-state', 'applied');
  await button('release').click();
  await gm.waitForFunction(ship => !window.__hostGmPresentationState().presentation[ship]?.forced_view, ship);
  await expect(panel.locator('[role=status]')).toHaveAttribute('data-state', 'applied');
  await button('clear').click(); await expect(card).toBeHidden();
  await crew.close();
});
