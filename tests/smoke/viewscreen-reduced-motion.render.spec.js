// Issue #1173 — the viewscreen honours reduced motion (PRD #1168).
//
// The server viewscreen renders hull-damage screen shake (the WASM whole-page
// CSS `transform: translate()` path) and the red-alert vignette pulse (CSS).
// Under a reduced-motion preference the shake must produce NO page transform
// and the vignette pulse must be capped/disabled; without it, both behave as
// before. The intensity/zeroing MATH is unit-tested without a GPU in
// `src/server/viewscreen_border.rs` (`shake_magnitude` / `capped_flash_intensity`);
// what only a real browser can prove is the WIRING those unit tests cannot
// reach: that the host's `prefers-reduced-motion` preference actually crosses
// into the WASM render path, that the live shake host-channel emits only zeros
// under it (so the page never translates), and that the CSS vignette rule the
// same preference drives collapses the pulse.
//
// # Why this needs the `render` project
//
// `src/server/bridge.rs` skips `RenderPlugin`/`ViewscreenBorderPlugin` entirely
// under `navigator.webdriver`, so the message suite never runs `apply_camera_shake`
// at all. This spec hides that flag and runs under SwiftShader — the same opt-in
// `viewscreen.render.spec.js` uses — so the real shake system runs and the real
// `wasm_set_reduced_motion` seam is exercised end to end.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

const COMBAT_TEST = 'assets/worlds/combat_test.toml';

/** Boot combat_test to a live, drawing viewscreen with the OS motion
 *  preference emulated, and a Helm client readied so the game is InProgress
 *  (so `apply_camera_shake` — gated on InProgress — actually runs). Mirrors
 *  `viewscreen.render.spec.js`'s SwiftShader + hidden-webdriver recipe. */
async function bootViewscreen(context, { reducedMotion }) {
  const page = await context.newPage();

  // The viewscreen reads the host machine's OS preference; emulate it BEFORE
  // load so both `matchMedia` (forwarded to Rust in startServer) and the CSS
  // `@media (prefers-reduced-motion: reduce)` rule reflect it on the first frame.
  await page.emulateMedia({ reducedMotion });

  // bridge.rs skips the render stack under WebDriver; SwiftShader supplies the
  // GL context, so hide the flag and take the real render path.
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
  });

  await page.goto(`/?scenario=${COMBAT_TEST}`);

  // combat_test is multi-hull, so the ship picker shows. The first card is a
  // crewed hull; pick it so wasm_init runs.
  const firstCard = page.locator('#scenario-panel ph-ship-picker .ship-card').first();
  await firstCard.waitFor({ state: 'visible', timeout: 60_000 });
  await firstCard.click();

  await waitForWasmReady(page, 120_000);

  const gl = await page.evaluate(() => !!document.createElement('canvas').getContext('webgl2'));
  expect(gl, 'SwiftShader supplied a WebGL2 context').toBe(true);

  // Ready a single Helm client so the collective SetReady auto-start fires and
  // the ship reaches InProgress (empty stations backfill to AI).
  const hostId = await readHostPeerId(page);
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  await helm.send('SelectStation', { station: 'Helm' });
  await helm.page.waitForFunction(
    (t) => window.__messages?.some((m) => m.type === 'StationAssigned' && m.data.token === t),
    helm.token,
    { timeout: 30_000 },
  );
  await helm.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 60_000);
  await page.bringToFront();

  // Wait until the viewscreen HUD is actually drawing (InProgress).
  await page.waitForFunction(
    () => !/Preparing scenario|Loading…/.test(document.body.innerText),
    undefined,
    { timeout: 180_000 },
  );
  await page.waitForFunction(() => /HEADING \d{3}/.test(document.body.innerText), undefined, {
    timeout: 90_000,
  });

  return { page, helm };
}

/** Install a spy over the shake host-channel sink so we can read back the
 *  largest offset Rust ever pushed and how many frames it fired on. The
 *  host-channel dispatcher looks `window.__applyShake` up dynamically on every
 *  call, so wrapping it after boot intercepts every subsequent frame. */
async function spyOnShake(page) {
  await page.evaluate(() => {
    window.__shakeCalls = 0;
    window.__maxShake = 0;
    const orig = window.__applyShake;
    window.__applyShake = function (x, y) {
      window.__shakeCalls += 1;
      const m = Math.max(Math.abs(x || 0), Math.abs(y || 0));
      if (m > window.__maxShake) window.__maxShake = m;
      return orig ? orig.call(this, x, y) : undefined;
    };
  });
}

/** Let a run of frames composite so the per-frame shake channel fires, then
 *  read back the spy plus the live shell transform. */
async function sampleShake(page, frames = 30) {
  for (let i = 0; i < frames; i += 1) {
    await page.bringToFront();
    await page.waitForTimeout(100);
  }
  return page.evaluate(() => ({
    calls: window.__shakeCalls,
    max: window.__maxShake,
    shellTransform: getComputedStyle(document.getElementById('viewscreen-shell')).transform,
  }));
}

/** Read the rendered vignette-pulse state with the alert class forced on, so
 *  the check is on the reduced-motion CSS rule rather than on red-alert
 *  plumbing. `animation-name` is `none` when the pulse is disabled and
 *  `hud-pulse` when it loops. */
async function vignetteAnimation(page) {
  return page.evaluate(() => {
    const overlay = document.getElementById('hud-overlay');
    overlay.classList.add('alert-on');
    const vignette = document.getElementById('hud-vignette');
    return getComputedStyle(vignette).animationName;
  });
}

test.describe('viewscreen honours reduced motion', () => {
  test.describe.configure({ timeout: 420_000 });

  test('reduced motion: no page transform, pulse disabled, flag reaches the render path', async ({
    context,
  }) => {
    const { page, helm } = await bootViewscreen(context, { reducedMotion: 'reduce' });

    // AC3 (WASM path): the OS preference crossed into the WASM renderer.
    const flag = await page.evaluate(() => window.wasm_is_reduced_motion());
    expect(flag, 'the reduced-motion preference reached the WASM render path').toBe(true);

    // AC1: the live shake channel is firing, and every frame's offset is zero —
    // so the whole page never translates.
    await spyOnShake(page);
    const shake = await sampleShake(page);
    expect(shake.calls, 'the per-frame shake channel is live in InProgress').toBeGreaterThan(0);
    expect(shake.max, 'reduced motion: Rust pushes only zero shake offsets').toBe(0);
    expect(shake.shellTransform, 'reduced motion: no whole-page transform').toBe('none');

    // AC2: the red-alert vignette pulse is capped/disabled.
    expect(await vignetteAnimation(page), 'reduced motion: vignette pulse is off').toBe('none');

    await helm.close();
  });

  test('normal motion: pulse loops, the translate path is intact, flag is off', async ({
    context,
  }) => {
    const { page, helm } = await bootViewscreen(context, { reducedMotion: 'no-preference' });

    // AC4: without the preference the flag is off and behaviour is unchanged.
    const flag = await page.evaluate(() => window.wasm_is_reduced_motion());
    expect(flag, 'no reduced-motion preference reported').toBe(false);

    // AC4: the vignette pulse loops as before.
    expect(await vignetteAnimation(page), 'normal motion: vignette pulse loops').toBe('hud-pulse');

    // AC4: the whole-page translate path still applies a nonzero offset — proof
    // the shake would move the page when motion is allowed (the zeroing under
    // reduced motion happens in Rust, not by disabling this path). Driven and
    // read in one evaluate so no Rust frame clears it between.
    const translate = await page.evaluate(() => {
      const shell = document.getElementById('viewscreen-shell');
      window.__applyShake(4, -4);
      const applied = getComputedStyle(shell).transform;
      window.__applyShake(0, 0);
      const cleared = getComputedStyle(shell).transform;
      return { applied, cleared };
    });
    expect(translate.applied, 'normal motion: a nonzero offset translates the page').not.toBe('none');
    expect(translate.cleared, 'a zero offset clears the transform').toBe('none');

    await helm.close();
  });
});
