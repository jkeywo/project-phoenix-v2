// Verification: NPC ship GLB meshes actually load (regression for the
// "ship meshes invisible" bug). `render_spawned_entities` strips the leading
// `assets/` before handing the path to the Bevy asset server (root = assets/),
// so a hull's `model = "assets/models/<x>.glb"` must be fetched at
// `assets/models/<x>.glb` and return 200 — NOT the broken
// `assets/assets/models/...` (404) that left entities unrendered.
//
// # Why this file needs the `render` Playwright project
//
// GLTF loading is disabled in automation mode: `src/server/bridge.rs` skips the
// render/audio/gltf/gizmo plugins under `navigator.webdriver`, because Bevy's
// wgpu init panics on a GPU-less runner. So this test was `test.skip`'d — the
// asset server never fetched the GLB, and there was nothing to assert. The
// `render` project (playwright.config.js) removes that blocker exactly as it did
// for viewscreen.render.spec.js: it boots a SwiftShader software GL context, and
// the spec below hides the `navigator.webdriver` flag so bridge.rs takes the
// real render path and the GLTF loader runs. `*.render.spec.js` is matched only
// by the `render` project and ignored by the message-suite `chromium` project.
//
// Issue #941: this used to serve the production `assets/worlds/patrol.toml` and
// look for `dynasty_destroyer.glb` — a filename that had ALREADY moved
// (`ship_harrow_patrol.toml` now carries `dynasty_cruiser.glb`) and went
// unnoticed because the test is skipped. The world is now a self-contained
// fixture and the expected GLB is read out of the hull TOML being spawned, so
// the assertion is about the path *transform* rather than about which model a
// designer happens to have attached. See the header of `fixtures.js` for the
// fixture convention.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';
import fs from 'fs';
import path from 'path';

const NPC_TEMPLATE = 'assets/entities/ship_harrow_patrol.toml';

/** The GLB the spawned hull declares, e.g. `assets/models/dynasty_cruiser.glb`. */
function npcModelPath() {
  const toml = fs.readFileSync(path.join(__dirname, '../..', NPC_TEMPLATE), 'utf-8');
  const m = toml.match(/^\s*model\s*=\s*"([^"]+)"/m);
  if (!m) throw new Error(`${NPC_TEMPLATE} declares no model — nothing to load`);
  return m[1];
}

const MESH_TEST_WORLD = `
[global]
seed = 42
title = "Ship Mesh Fixture"
description = "Player ship + one NPC with a GLB; see tests/smoke/ship-mesh-load.render.spec.js."

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id            = "player-ship"
transform     = { position = [0.0, 0.0, 0.0] }
spawn_on      = "game_start"
overrides     = { tags = ["ship"] }

[[entity]]
template_path = "${NPC_TEMPLATE}"
name          = "raider_alpha"
transform     = { position = [120.0, 0.0, 0.0] }
spawn_on      = "game_start"
`;

test.describe('ship meshes load', () => {
  test.describe.configure({ timeout: 300_000 });

  test('NPC ship GLB model loads (200) after game start', async ({ context }) => {
    const modelPath = npcModelPath();
    const modelFile = modelPath.split('/').pop();

    await context.route('**/assets/worlds/default.toml', (route) =>
      route.fulfill({ contentType: 'text/plain', body: MESH_TEST_WORLD }),
    );

    const serverPage = await context.newPage();

    // Record every .glb response and its status code.
    const glbResponses = [];
    serverPage.on('response', (resp) => {
      const url = resp.url();
      if (url.endsWith('.glb')) glbResponses.push({ url, status: resp.status() });
    });

    // bridge.rs skips the render/gltf stack under WebDriver; the `render`
    // project supplies a SwiftShader GPU, so hide the flag and take the real
    // path. Without this the GLB is never requested and the test is vacuous.
    await serverPage.addInitScript(() => {
      Object.defineProperty(navigator, 'webdriver', { get: () => false });
    });

    await serverPage.goto('/?scenario=assets/worlds/default.toml');

    // A single-hull fixture auto-selects and shows no picker; guard for one
    // anyway so the boot is robust either way.
    const card = serverPage.locator('#scenario-panel ph-ship-picker .ship-card').first();
    const picked = await card
      .waitFor({ state: 'visible', timeout: 20_000 })
      .then(() => true)
      .catch(() => false);
    if (picked) await card.click();

    await waitForWasmReady(serverPage, 120_000);

    // If SwiftShader failed to supply a context the GLTF loader never runs, and
    // the GLB assertions below would "pass" for the wrong reason.
    const gl = await serverPage.evaluate(() => {
      const ctx = document.createElement('canvas').getContext('webgl2');
      return !!ctx;
    });
    expect(gl, 'SwiftShader supplied a WebGL2 context').toBe(true);

    const hostId = await readHostPeerId(serverPage);
    const helm = await createTestClient(context, hostId, { name: 'Helm' });

    await helm.send('SelectStation', { station: 'Helm' });
    await helm.page.waitForFunction(
      (t) => window.__messages?.some((m) => m.type === 'StationAssigned' && m.data.token === t),
      helm.token,
      { timeout: 30_000 },
    );

    await helm.send('SetReady', { ready: true });
    await helm.waitForMessage('GameStarted', 60_000);

    // The GLB assertions below are about the fixture's NPC, so confirm the
    // fixture is what got parsed rather than production `default.toml` (which
    // spawns its own GLB-carrying hulls and would satisfy them incidentally).
    expectFixtureWorld(await helm.waitForMessage('WorldSetup', 5_000), MESH_TEST_WORLD);

    await serverPage.bringToFront();

    // Poll until the NPC's GLB request has been observed (or time out). The NPC
    // spawns on game_start and `render_spawned_entities` fetches its model once
    // it is processed — independent of viewscreen direction.
    await expect
      .poll(() => glbResponses.some((r) => r.url.endsWith(`/${modelPath}`)), {
        timeout: 60_000,
        message: `${modelFile} was never requested`,
      })
      .toBe(true);

    const shipGlb = glbResponses.find((r) => r.url.endsWith(`/${modelPath}`));

    // The request must NOT have gone to the doubled `assets/assets/...` path.
    expect(shipGlb.url, 'GLB must load from assets/models/, not assets/assets/models/').not.toContain(
      'assets/assets/',
    );
    // And it must succeed.
    expect(shipGlb.status, `GLB fetch returned ${shipGlb.status}`).toBe(200);

    // Capture the 3D canvas as supplementary proof the scene renders.
    await serverPage.screenshot({ path: path.join(__dirname, '../../target/ship-mesh-canvas.png') });

    await helm.close();
  });
});
