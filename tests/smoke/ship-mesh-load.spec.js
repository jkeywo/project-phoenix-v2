// Verification: NPC ship GLB meshes actually load (regression for the
// "ship meshes invisible" bug). `render_spawned_entities` strips the leading
// `assets/` before handing the path to the Bevy asset server (root = assets/),
// so a hull's `model = "assets/models/<x>.glb"` must be fetched at
// `assets/models/<x>.glb` and return 200 — NOT the broken
// `assets/assets/models/...` (404) that left entities unrendered.
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
description = "Player ship + one NPC with a GLB; see tests/smoke/ship-mesh-load.spec.js."

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

// GLTF loading is intentionally disabled in automation mode
// (bridge.rs: "skip render/audio/gltf/gizmo plugins"), so this test
// cannot pass in CI. The asset server never fetches the GLB.
test.skip('NPC ship GLB model loads (200) after game start', async ({ context }) => {
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

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });

  await helm.send('SelectStation', { station: 'Helm' });
  await helm.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t),
    helm.token, { timeout: 5_000 });

  await tactical.send('SelectStation', { station: 'Tactical' });
  await tactical.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t),
    tactical.token, { timeout: 5_000 });

  await helm.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 10_000);

  // The GLB assertions below are about the fixture's NPC, so confirm the
  // fixture is what got parsed rather than production `default.toml` (which
  // spawns its own GLB-carrying hulls and would satisfy them incidentally).
  expectFixtureWorld(await helm.waitForMessage('WorldSetup', 5_000), MESH_TEST_WORLD);

  // Point the viewscreen at the raider (it sits to starboard of spawn) and
  // give the asset server time to fetch + spawn the SceneRoot.
  await helm.send('ControlSystem', { target: 'viewscreen', payload: { type: 'SetView', data: { mode: { kind: 'Camera', data: 'Starboard' } } } });

  await serverPage.bringToFront();

  // Poll until the GLB request has been observed (or time out).
  await expect.poll(
    () => glbResponses.some((r) => r.url.endsWith(`/${modelPath}`)),
    { timeout: 20_000, message: `${modelFile} was never requested` },
  ).toBe(true);

  const shipGlb = glbResponses.find((r) => r.url.endsWith(`/${modelPath}`));

  // The request must NOT have gone to the doubled `assets/assets/...` path.
  expect(shipGlb.url, 'GLB must load from assets/models/, not assets/assets/models/')
    .not.toContain('assets/assets/');
  // And it must succeed.
  expect(shipGlb.status, `GLB fetch returned ${shipGlb.status}`).toBe(200);

  // Capture the 3D canvas as supplementary proof the scene renders.
  await serverPage.screenshot({ path: path.join(__dirname, '../../target/ship-mesh-canvas.png') });

  await helm.close();
  await tactical.close();
});
