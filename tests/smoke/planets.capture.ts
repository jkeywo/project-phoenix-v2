// Manual capture: boots a purpose-built world with all six textured planet
// types ringed around the player spawn, then spins the ship 360° taking
// screenshots so the planet shaders can be eyeballed (terminator direction,
// nightside city lights, always-on lava glow, cloud shells, rim atmosphere).
//
// Not part of the smoke suite (AGENTS.md: renderer visual output is not
// tested) — this is a verification aid. Run it explicitly:
//
//   npx playwright test -c playwright.capture.config.ts planets.capture.ts
//
// Screenshots land in tests/smoke/.captures/.

import { test, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import fs from 'fs';
import path from 'path';

const OUT = path.join(__dirname, '.captures');

// Star at the origin; player at [400, 0, 200]. The ecumenopolis sits on the
// player→star line so its camera-facing side is the night side (city lights).
// Earth+Luna, the gas giant+ice moon, and the lava planet ring the player at
// 200–350u so a full rotation sweeps past every body.
const CAPTURE_WORLD = `
[global]
seed = 42
title = "Planet Capture"
description = "All six textured planets around the player."

[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/planet_ecumenopolis.toml"
id = "ecumenopolis"
name = "ecumenopolis"
transform = { position = [200.0, 0.0, 100.0] }

[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "earth"
name = "earth"
transform = { position = [400.0, 0.0, 420.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "luna"
name = "luna"
transform = { relative_to = "earth", offset = [40.0, 0.0, 20.0] }

[[entity]]
template_path = "assets/entities/planet_gas_giant.toml"
id = "gas-giant"
name = "gas-giant"
transform = { position = [700.0, 0.0, 350.0] }

[[entity]]
template_path = "assets/entities/moon_ice.toml"
id = "ice-moon"
name = "ice-moon"
transform = { relative_to = "gas-giant", offset = [125.0, 0.0, 40.0] }

[[entity]]
template_path = "assets/entities/planet_lava.toml"
id = "lava-planet"
name = "lava-planet"
transform = { position = [400.0, 0.0, -50.0] }

[[available_ships]]
template_path = "assets/entities/alliance_cruiser.toml"
label = "Alliance Cruiser"

[player_spawn]
position = [400.0, 0.0, 200.0]

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [400.0, 0.0, 200.0] }
spawn_on = "game_start"
`;

test('capture all six planet shaders through a full rotation', async ({ context }) => {
  test.setTimeout(240_000);
  fs.mkdirSync(OUT, { recursive: true });

  await context.route('**/assets/worlds/combat_test.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: CAPTURE_WORLD }),
  );

  const serverPage = await context.newPage();
  serverPage.on('pageerror', (e) => console.log(`PAGEERROR: ${e.message.slice(0, 200)}`));
  serverPage.on('console', (m) => {
    if (m.type() === 'error' || /wgsl|pipeline|shader/i.test(m.text())) {
      console.log(`CONSOLE[${m.type()}]: ${m.text().slice(0, 300)}`);
    }
  });

  // bridge.rs skips RenderPlugin when navigator.webdriver is true; this
  // config supplies a SwiftShader GPU, so hide the flag (see dust-pfx).
  await serverPage.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
  });

  await serverPage.goto('/?scenario=assets/worlds/combat_test.toml');

  // A single [[available_ships]] entry skips the lobby ship picker and
  // auto-selects, so wasm init proceeds directly.
  await waitForWasmReady(serverPage, 120_000);

  const gl = await serverPage.evaluate(() => {
    const c = document.createElement('canvas');
    const ctx = c.getContext('webgl2') as WebGL2RenderingContext | null;
    if (!ctx) return { ok: false, renderer: null };
    const dbg = ctx.getExtension('WEBGL_debug_renderer_info');
    return {
      ok: true,
      renderer: dbg ? ctx.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : 'unknown',
    };
  });
  console.log(`webgl2: ${JSON.stringify(gl)}`);
  if (!gl.ok) throw new Error('no WebGL2 context — captures would be blank');

  const hostId = await readHostPeerId(serverPage);
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  await helm.send('SelectStation', { station: 'Helm' });
  await helm.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    helm.token,
    { timeout: 10_000 },
  );
  await helm.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 30_000);
  await serverPage.bringToFront();

  await serverPage.waitForFunction(
    () => !/Preparing scenario|Loading…/.test(document.body.innerText),
    undefined,
    { timeout: 120_000 },
  );
  await serverPage.waitForFunction(
    () => /HEADING \d{3}/.test(document.body.innerText),
    undefined,
    { timeout: 60_000 },
  );

  // Hold steering while keeping the server page fronted (backgrounded pages
  // get timer-throttled), screenshotting as the ship turns through 360°.
  const holdSteer = async (ms: number) => {
    const until = Date.now() + ms;
    while (Date.now() < until) {
      // Per-axis helm wire messages (issue #801). Steering only bites while
      // under way, so keep some thrust on.
      await helm.send('ControlSystem', {
        target: 'helm-thrust',
        payload: { type: 'SetThrust', data: { value: 0.5 } },
      });
      await helm.send('ControlSystem', {
        target: 'helm-steering',
        payload: { type: 'SetSteering', data: { value: 1.0 } },
      });
      await serverPage.bringToFront();
      await serverPage.waitForTimeout(100);
    }
  };

  for (let i = 0; i < 20; i++) {
    await holdSteer(2000);
    await serverPage.screenshot({
      path: path.join(OUT, `planets-${String(i).padStart(2, '0')}.png`),
    });
    console.log(`captured planets-${String(i).padStart(2, '0')}`);
  }
});
