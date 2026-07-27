// Manual capture: boots a self-contained minimal world (destroyer player +
// stationary raider dead ahead, mirroring tactical-fire-flow.spec.ts's
// MINIMAL_TEST_WORLD), locks the raider and fires the port blaster bank via
// `ControlSystem { target: "blaster-port", payload: FireBlaster }`, and
// screenshots the viewscreen so the textured blaster-bolt PFX
// (src/server/pfx.rs sync_blaster_pfx) can be eyeballed in flight and on
// impact.
//
// Not part of the smoke suite (AGENTS.md: renderer visual output is not
// tested) — this is a verification aid, modeled on dust-pfx.capture.ts. Run
// it explicitly:
//
//   npx playwright test --config=playwright.capture.config.ts blaster-pfx.capture.ts --headed
//
// Screenshots land in tests/smoke/.captures/. Note: the viewscreen is a
// chase camera behind the player ship looking forward, so a dead-ahead bolt
// travels almost directly away from the camera and reads as a small glow
// near the bow rather than a broadside streak — reposition the raider
// off-axis (still within the port bank's 45° arc) for a clearer silhouette.

import { test, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import fs from 'fs';
import path from 'path';

const OUT = path.join(__dirname, '.captures');

// Player (destroyer) at the origin facing -Z; raider 15 units dead ahead,
// well within the port/starboard banks' 35-unit range and 45° fire arc.
const MINIMAL_TEST_WORLD = `
[global]
seed = 42

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[anchors]
patrol_alpha = [600.0, 0.0, -600.0]

[[entity]]
template_path = "assets/entities/alliance_destroyer.toml"
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

test('capture blaster bolts firing on a stationary target', async ({ context }) => {
  test.setTimeout(120_000);
  fs.mkdirSync(OUT, { recursive: true });

  // Self-contained world — same rationale as tactical-fire-flow.spec.ts.
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: MINIMAL_TEST_WORLD }),
  );
  // Keep the raider stationary and passive so it doesn't fire back or drift
  // out of frame mid-capture.
  await context.route('**/assets/entities/ship_harrow_patrol.toml', async (route) => {
    const response = await route.fetch();
    const text = await response.text();
    const patched = text
      .replace(/initial_state\s*=\s*"patrol"/, 'initial_state = "idle"')
      .replace(/target_speed\s*=\s*[\d.]+/g, 'target_speed = 0.0')
      .replace(/condition = "enemy_in_range"/, 'condition = "never_matches"');
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

  // The Bevy sim on serverPage ticks via requestAnimationFrame, which
  // Chromium throttles hard on backgrounded tabs (see dust-pfx.capture.ts) —
  // the lobby countdown / GameStarted broadcast would otherwise crawl at a
  // fraction of wall-clock speed. Keep it fronted while polling for both.
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

  await shot('00-pre-fire');

  // Human blaster fire requires a locked WeaponsTarget (arc check in
  // handle_fire_blaster, src/console/weapons/server.rs:2667) — same
  // requirement as phasers/torpedoes. Lock the raider first.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetup?.data?.world?.entities ?? [];
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc') && e.tags.includes('enemy'),
  );
  if (!raider) throw new Error(`raider entity not found in WorldSetup: ${JSON.stringify(entities)}`);
  await tactical.send('ControlSystem', {
    target: 'tactical',
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

  // Wait for WeaponsUpdate confirming the port bank is fire_ready, mirroring
  // tactical-fire-flow.spec.ts, then fire (volley_count=3,
  // volley_interval_secs=0.5) and capture frames through the volley and its
  // impacts.
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate'
        && Array.isArray(m.data?.blasters)
        && m.data.blasters.some((b: any) => b.id === 'port' && b.fire_ready === true),
    ),
    undefined,
    { timeout: 15_000 },
  );

  await tactical.send('ControlSystem', {
    target: 'blaster-port',
    payload: { type: 'FireBlaster' },
  });
  await serverPage.bringToFront();

  for (let i = 0; i < 20; i++) {
    await serverPage.waitForTimeout(150);
    await shot(`blaster-${String(i).padStart(2, '0')}`);
  }
});
