import { test, expect, createTestClient, expectFixtureWorld, readHostPeerId,
  waitForJoinCode, waitForWasmReady } from './fixtures';
import { ts } from './strings';

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
  await crew.send('SetReady', { ready: true }); await gm.locator('#gm-ready-btn').click();
  await Promise.all([host, gm].map(page => page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress')));
  expectFixtureWorld(await crew.waitForMessage('WorldSetup'), WORLD);
  const panel = gm.locator('#gm-presentation-panel');
  await expect(panel.locator('#gm-presentation-ship option')).toHaveCount(1);
  await gm.locator('#gm-session-pause').click();
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
  await gm.locator('#gm-session-resume').click();
  await gm.waitForFunction(() => !window.__hostGmSessionState().paused);
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
