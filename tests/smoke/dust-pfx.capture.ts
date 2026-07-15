// Manual capture: drives the ship through each speed regime and screenshots
// the viewscreen so the dust field can be eyeballed.
//
// Not part of the smoke suite (AGENTS.md: renderer visual output is not
// tested) — this is a verification aid. Run it explicitly:
//
//   npx playwright test dust-pfx.capture.ts --headed
//
// Screenshots land in tests/smoke/.captures/.

import { test, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import fs from 'fs';
import path from 'path';

const OUT = path.join(__dirname, '.captures');

test('capture dust field across speed regimes', async ({ context }) => {
  test.setTimeout(240_000);
  fs.mkdirSync(OUT, { recursive: true });

  const serverPage = await context.newPage();
  serverPage.on('pageerror', (e) => console.log(`PAGEERROR: ${e.message.slice(0, 200)}`));

  // bridge.rs:317-327 skips RenderPlugin entirely when `navigator.webdriver`
  // is true, because Bevy's wgpu init panics on a GPU-less CI runner. That is
  // the right call for the smoke suite (which only asserts on messages and
  // DOM) but it means a Playwright page can never draw the viewscreen. Since
  // this config supplies a SwiftShader GPU, hide the webdriver flag so the
  // real render path runs.
  await serverPage.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
  });

  await serverPage.goto('/?scenario=assets/worlds/combat_test.toml');

  await serverPage.waitForSelector('#scenario-panel ph-ship-picker .ship-card', {
    state: 'visible',
    timeout: 60_000,
  });
  await serverPage.click('#scenario-panel ph-ship-picker .ship-card:first-child');
  await waitForWasmReady(serverPage);

  // Fail loudly rather than silently capturing eight identical black frames:
  // without a WebGL2 context Bevy draws nothing and the canvas stays at its
  // default 300x150.
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

  // GameStarted fires before the loading overlay clears, so without this the
  // captures are just screenshots of "Preparing scenario 70%".
  await serverPage.waitForFunction(
    () => !/Preparing scenario|Loading…/.test(document.body.innerText),
    undefined,
    { timeout: 120_000 },
  );
  // And wait for the HUD to report live telemetry, i.e. the sim is rendering.
  await serverPage.waitForFunction(
    () => /HEADING \d{3}/.test(document.body.innerText),
    undefined,
    { timeout: 60_000 },
  );

  // Hold an input for `ms` while keeping the server page fronted — Chromium
  // throttles timers on backgrounded pages, and the sim reads helm at 10Hz.
  const hold = async (
    payload: Record<string, unknown>,
    ms: number,
    target = 'helm',
  ) => {
    const until = Date.now() + ms;
    while (Date.now() < until) {
      await helm.send('ControlSystem', { target, payload });
      await serverPage.bringToFront();
      await serverPage.waitForTimeout(100);
    }
  };

  const shot = async (name: string) => {
    await serverPage.screenshot({ path: path.join(OUT, `${name}.png`) });
    console.log(`captured ${name}`);
  };

  const thrust = (t: number, steering = 0.0) => ({
    type: 'HelmInput',
    data: { thrust: t, steering },
  });

  // 1. Station-keeping — the field should be empty, not snowing.
  await hold(thrust(0.0), 1500);
  await shot('01-stationary');

  // 2. Quarter throttle — sparse dim points.
  await hold(thrust(0.25), 6000);
  await shot('02-quarter-throttle');

  // 3. Full throttle — brighter, longer streaks.
  await hold(thrust(1.0), 8000);
  await shot('03-full-throttle');

  // 4. Hard turn while under way — streaks follow velocity, not facing.
  await hold(thrust(1.0, 1.0), 5000);
  await shot('04-hard-turn');

  // 5. Pure strafe, no forward thrust. This is the regression: dust used to
  //    ignore lateral_speed entirely and sit frozen here.
  await hold(thrust(0.0), 2500);
  await hold({ type: 'LateralThrustInput', data: { lateral: 1.0 } }, 6000, 'helm-lateral-thrust');
  await shot('05-pure-strafe');
  await hold({ type: 'LateralThrustInput', data: { lateral: 0.0 } }, 500, 'helm-lateral-thrust');

  // 6. Impulse — the warp field takes over.
  await hold(thrust(1.0), 4000);
  await helm.send('ControlSystem', { target: 'helm', payload: { type: 'StartImpulseCharge' } });
  await hold(thrust(1.0), 2000);
  await shot('06-impulse-charging');
  await hold(thrust(1.0), 6000);
  await shot('07-impulse-active');

  // 7. Disengage — warp field spins down, ordinary motes repopulate.
  await helm.send('ControlSystem', { target: 'helm', payload: { type: 'CancelImpulse' } });
  await hold(thrust(1.0), 1500);
  await shot('08-impulse-exit');

  await hold(thrust(0.0), 500);
});
