// Verification: NPC ship GLB meshes actually load (regression for the
// "ship meshes invisible" bug). The raider in patrol.toml carries
// `model = "assets/models/dynasty_destroyer.glb"`. render_spawned_entities
// strips the leading `assets/` before handing the path to the Bevy asset
// server (root = assets/), so the GLB must be fetched at
// `assets/models/dynasty_destroyer.glb` and return 200 — NOT the broken
// `assets/assets/models/...` (404) that left entities unrendered.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady, stripHeavyEntities } from './fixtures';
import fs from 'fs';
import path from 'path';

const PATROL_TOML = fs.readFileSync(
  path.join(__dirname, '../../assets/worlds/patrol.toml'),
  'utf-8',
);

test('NPC ship GLB model loads (200) after game start', async ({ context }) => {
  // Strip the asteroid_field block from patrol.toml so the lobby preload
  // gate clears in CI. The raider (the entity this test inspects) is
  // preserved.
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(PATROL_TOML) }),
  );

  const serverPage = await context.newPage();

  // Record every .glb response and its status code.
  const glbResponses: { url: string; status: number }[] = [];
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
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t),
    helm.token, { timeout: 5_000 });

  await tactical.send('SelectStation', { station: 'Tactical' });
  await tactical.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t),
    tactical.token, { timeout: 5_000 });

  await helm.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 5_000);
  await helm.waitForMessage('WorldSetup', 5_000);

  // Point the viewscreen at the raider (it sits fore-starboard of spawn) and
  // give the asset server time to fetch + spawn the SceneRoot.
  await helm.send('SetView', { mode: { kind: 'Camera', data: 'Starboard' } });

  await serverPage.bringToFront();
  // Poll until the GLB request has been observed (or time out).
  await expect.poll(
    () => glbResponses.some((r) => r.url.includes('models/dynasty_destroyer.glb')),
    { timeout: 20_000, message: 'dynasty_destroyer.glb was never requested' },
  ).toBe(true);

  const shipGlb = glbResponses.find((r) => r.url.includes('models/dynasty_destroyer.glb'))!;
  console.log('GLB responses observed:', JSON.stringify(glbResponses, null, 2));

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
