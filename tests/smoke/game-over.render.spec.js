// Exercise the real WASM phase transition, both transport classes and the
// Viewscreen HUD. The rendererless smoke profile does not install the HUD owner.
import {
  test,
  expect,
  createServerPage,
  createTestClient,
  readHostPeerId,
  expectFixtureWorld,
} from './fixtures';

const ENDING_REASON = 'Smoke mission completed.';
const ENDING_WORLD = `
[global]
seed = 42
title = "Terminal delivery smoke world"
description = "A bounded scripted ending for lifecycle regression coverage."

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[script]
setup = """
on_timer(2, "end_smoke_mission");

fn end_smoke_mission(ctx) {
    ctx.effects.game_over("${ENDING_REASON}", "victory");
}
"""
`;

test('mission ending reaches consoles and HUD before returning to the lobby', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(90_000);
  await context.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
  });
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: ENDING_WORLD }),
  );
  const host = await createServerPage(context);
  const captain = await createTestClient(context, await readHostPeerId(host), { name: 'Captain' });
  await captain.send('SelectStation', { station: 'Captain' });
  await captain.page.waitForFunction(
    (token) => window.__messages?.some(
      (message) => message.type === 'StationAssigned' && message.data.token === token,
    ),
    captain.token,
    { timeout: 8_000 },
  );
  await captain.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 15_000);
  expectFixtureWorld(await captain.waitForMessage('WorldSetup'), ENDING_WORLD);
  await host.bringToFront();

  // Independently report both symptoms: the terminal wire event used to wait
  // in the stopped simulation queue, while an unordered HUD reader lost its text.
  await expect.soft(host.locator(':light(#game-over-message)'))
    .toHaveText(ENDING_REASON, { timeout: 15_000 });
  await expect.soft(host.locator(':light(#game-over-overlay)')).toBeVisible();
  const ending = await captain.waitForMessage('GameOver', 15_000);
  expect(ending.data).toEqual({ reason: ENDING_REASON, outcome: 'victory', report: [] });
  expect(await captain.page.evaluate(() => ({
    reliable: window.__reliableMessages.filter((message) => message.type === 'GameOver'),
    snapshot: window.__snapshotMessages.filter((message) => message.type === 'GameOver'),
  }))).toEqual({ reliable: [ending], snapshot: [] });

  // This is an ordinary admitted participant command, received while fixed
  // simulation publication is stopped. It must bring both surfaces back.
  await captain.send('ReturnToLobby');
  await captain.waitForMessage('ReturnedToLobby', 10_000);
  await expect(host.locator(':light(#game-over-overlay)')).toBeHidden();
  await expect(host.locator('#lobby-panel')).toBeVisible();
  expect(await captain.page.evaluate(
    () => window.__reliableMessages.filter((message) => message.type === 'GameOver').length,
  )).toBe(1);
  await captain.close();
});
