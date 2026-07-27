// Manual capture: boots a self-contained minimal world (cruiser player +
// stationary raider dead ahead, mirroring tactical-fire-flow.spec.ts), locks
// the raider, boosts phaser DPS so one beam tick kills it, and screenshots
// the viewscreen so the ship death-explosion PFX (src/server/pfx.rs
// spawn_ship_explosions, driven by ShipDestroyedVfx from
// src/console/weapons/server.rs) can be eyeballed.
//
// Not part of the smoke suite (AGENTS.md: renderer visual output is not
// tested) — this is a verification aid, modeled on blaster-pfx.capture.ts.
// Run it explicitly:
//
//   npx playwright test --config=playwright.capture.config.ts explosion-pfx.capture.ts --headed
//
// Screenshots land in tests/smoke/.captures/.

import { test, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import fs from 'fs';
import path from 'path';

const OUT = path.join(__dirname, '.captures');

// Player (cruiser) at the origin facing -Z; raider 15 units dead ahead,
// well within phaser range (matches tactical-fire-flow.spec.ts's layout).
const MINIMAL_TEST_WORLD = `
[global]
seed = 42

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[anchors]
patrol_alpha = [600.0, 0.0, -600.0]

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
tags = ["ship"]

[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
name          = "raider_alpha"
transform     = { position = [0.0, 0.0, -15.0] }
spawn_on      = "game_start"
`;

test('capture ship death explosion on a one-shot phaser kill', async ({ context }) => {
  test.setTimeout(120_000);
  fs.mkdirSync(OUT, { recursive: true });

  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: MINIMAL_TEST_WORLD }),
  );
  // Keep the raider stationary and passive.
  await context.route('**/assets/entities/ship_harrow_patrol.toml', async (route) => {
    const response = await route.fetch();
    const text = await response.text();
    const patched = text
      .replace(/initial_state\s*=\s*"patrol"/, 'initial_state = "idle"')
      .replace(/target_speed\s*=\s*[\d.]+/g, 'target_speed = 0.0')
      .replace(/condition = "enemy_in_range"/, 'condition = "never_matches"');
    await route.fulfill({ contentType: 'text/plain', body: patched });
  });
  // Boost phaser DPS so one beam tick kills the 30 HP raider outright —
  // same technique as tactical-fire-flow.spec.ts's NPC-hull-decreases test.
  await context.route('**/assets/entities/alliance_cruiser.toml', async (route) => {
    const response = await route.fetch();
    const text = await response.text();
    const patched = text.replace(
      /beam_damage_per_sec\s*=\s*[\d.]+/,
      'beam_damage_per_sec = 100.0',
    );
    await route.fulfill({ contentType: 'text/plain', body: patched });
  });

  const serverPage = await context.newPage();
  serverPage.on('pageerror', (e) => console.log(`PAGEERROR: ${e.message.slice(0, 200)}`));
  await serverPage.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
  });

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const gl = await serverPage.evaluate(() => {
    const c = document.createElement('canvas');
    const ctx = c.getContext('webgl2') as WebGL2RenderingContext | null;
    if (!ctx) return { ok: false, renderer: null };
    const dbg = ctx.getExtension('WEBGL_debug_renderer_info');
    return { ok: true, renderer: dbg ? ctx.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : 'unknown' };
  });
  console.log(`webgl2: ${JSON.stringify(gl)}`);
  if (!gl.ok) throw new Error('no WebGL2 context — captures would be blank');

  const hostId = await readHostPeerId(serverPage);
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });

  await helm.send('SelectStation', { station: 'Helm' });
  await helm.page.waitForFunction(
    (t) => (window as any).__messages?.some((m: any) => m.type === 'StationAssigned' && m.data.token === t),
    helm.token,
    { timeout: 10_000 },
  );
  await tactical.send('SelectStation', { station: 'Tactical' });
  await tactical.page.waitForFunction(
    (t) => (window as any).__messages?.some((m: any) => m.type === 'StationAssigned' && m.data.token === t),
    tactical.token,
    { timeout: 10_000 },
  );

  await helm.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });

  await serverPage.bringToFront();
  const until = Date.now() + 30_000;
  let helmStarted = false;
  let tacticalStarted = false;
  while (Date.now() < until && !(helmStarted && tacticalStarted)) {
    await serverPage.bringToFront();
    await serverPage.waitForTimeout(200);
    helmStarted ||= await helm.page.evaluate(
      () => (window as any).__messages?.some((m: any) => m.type === 'GameStarted'),
    );
    tacticalStarted ||= await tactical.page.evaluate(
      () => (window as any).__messages?.some((m: any) => m.type === 'GameStarted'),
    );
  }
  if (!helmStarted || !tacticalStarted) {
    throw new Error(`GameStarted not received (helm=${helmStarted}, tactical=${tacticalStarted})`);
  }

  await serverPage.waitForFunction(
    () => !/Preparing scenario|Loading…/.test(document.body.innerText),
    undefined,
    { timeout: 60_000 },
  );
  await serverPage.waitForFunction(
    () => /HEADING \d{3}/.test(document.body.innerText),
    undefined,
    { timeout: 60_000 },
  );

  const shot = async (name: string) => {
    await serverPage.screenshot({ path: path.join(OUT, `${name}.png`) });
    console.log(`captured ${name}`);
  };

  await shot('00-pre-kill');

  // Lock the raider, then fire — one 100 DPS beam tick one-shots its 30 HP.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetup?.data?.world?.entities ?? [];
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc') && e.tags.includes('enemy'),
  );
  if (!raider) throw new Error(`raider entity not found in WorldSetup: ${JSON.stringify(entities)}`);
  await tactical.send('ControlSystem', {
    target: 'tactical-radar',
    payload: { type: 'SetTarget', data: { uuid: raider.uuid } },
  });
  await serverPage.bringToFront();
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate' && m.data?.target_uuid,
    ),
    undefined,
    { timeout: 10_000 },
  );
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate'
        && Array.isArray(m.data?.banks)
        && m.data.banks.some((b: any) => b.fire_ready === true),
    ),
    undefined,
    { timeout: 15_000 },
  );

  await tactical.send('ControlSystem', {
    target: 'phaser-fore',
    payload: { type: 'FirePhaser' },
  });
  await serverPage.bringToFront();

  for (let i = 0; i < 16; i++) {
    await serverPage.waitForTimeout(150);
    await shot(`explosion-${String(i).padStart(2, '0')}`);
  }
});
