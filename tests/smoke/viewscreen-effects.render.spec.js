// Issue #1428 — the viewscreen's three visual effects are set separately
// (PRD #1418 stories 13-15).
//
// The sibling of `viewscreen-reduced-motion.render.spec.js`, and it exists for
// the same reason: the intensity MATH is unit-tested without a GPU in
// `src/server/viewscreen_border.rs` (`shake_magnitude` / `scaled_flash_intensity`)
// and the settings behaviour in `tests/client/visual-effects.test.js`, but what
// only a real browser can prove is the WIRING between them — that a choice made
// on the Display tab crosses into the WASM render path, that the live shake
// host-channel then emits only zeros so the page never translates, and that the
// red-alert vignette the same choice governs stops pulsing while STAYING fully
// lit.
//
// The two states this file captures are the two the acceptance criteria name:
// the DEFAULT combination (everything at full, both effects present and
// overlapping) and the REDUCED one (this endpoint's own settings turned down),
// on the same scenario, so the pair is comparable rather than two anecdotes.
//
// # Why this needs the `render` project
//
// `src/server/bridge.rs` skips `RenderPlugin`/`ViewscreenBorderPlugin` entirely
// under `navigator.webdriver`, so the message suite never runs
// `apply_camera_shake` at all. This spec hides that flag and runs under
// SwiftShader — the same opt-in `viewscreen.render.spec.js` uses — so the real
// shake system runs and the real intensity seams are exercised end to end.
//
// # What this file deliberately does NOT claim
//
// It does not claim the DEFAULT flashing is comfortable. PRD #1418 is explicit
// that "a Reduce effects toggle is not evidence that default flashing is
// acceptable", so the default capture below is evidence for a human to judge
// against, recorded with its measured pulse period — not a pass mark.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

const COMBAT_TEST = 'assets/worlds/combat_test.toml';

/** Boot combat_test to a live, drawing viewscreen with a Helm client readied so
 *  the game is InProgress (`apply_camera_shake` is gated on it). Mirrors the
 *  SwiftShader + hidden-webdriver recipe `viewscreen.render.spec.js` uses. */
async function bootViewscreen(context) {
  const page = await context.newPage();

  // The machine says nothing about motion: every effect below is then decided
  // by THIS ENDPOINT's own record, which is the thing under test.
  await page.emulateMedia({ reducedMotion: 'no-preference' });

  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
  });

  await page.goto(`/?scenario=${COMBAT_TEST}`);

  const firstCard = page.locator('#scenario-panel ph-ship-picker .ship-card').first();
  await firstCard.waitFor({ state: 'visible', timeout: 60_000 });
  await firstCard.click();

  await waitForWasmReady(page, 120_000);

  const gl = await page.evaluate(() => !!document.createElement('canvas').getContext('webgl2'));
  expect(gl, 'SwiftShader supplied a WebGL2 context').toBe(true);

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

/** Press one stop on one effect's control row through the real settings cog,
 *  the way an operator standing at the display does. */
async function chooseEffect(page, effect, stop) {
  const slug = effect.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
  await page.evaluate(() => {
    const settings = window.__serverSettings;
    settings.open();
    settings.selectTab('presentation');
  });
  const control = page.locator(`[data-control="viewscreen-${slug}-${stop}"]`);
  await control.waitFor({ state: 'visible', timeout: 30_000 });
  await control.click();
  await page.evaluate(() => window.__serverSettings.close());
}

/** Spy over the shake host-channel sink, so the largest offset Rust ever pushed
 *  and the number of frames it fired on can be read back. The dispatcher looks
 *  `window.__applyShake` up on every call, so this intercepts every later frame. */
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

/** Let a run of frames composite, then read the spy plus the live transform. */
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

/** The rendered red-alert vignette with the alert class forced on, so the check
 *  is on the effect rule rather than on red-alert plumbing. Returns what is
 *  drawn AND what the state is still saying, which is the pair story 15 is
 *  about: the pulse may stop, the alert may not disappear. */
async function vignetteState(page) {
  return page.evaluate(() => {
    const overlay = document.getElementById('hud-overlay');
    overlay.classList.add('alert-on');
    const vignette = document.getElementById('hud-vignette');
    const style = getComputedStyle(vignette);
    return {
      animationName: style.animationName,
      animationDuration: style.animationDuration,
      opacity: style.opacity,
      boxShadow: style.boxShadow,
    };
  });
}

test.describe('the viewscreen’s three effects are set separately', () => {
  test.describe.configure({ timeout: 480_000 });

  test('default: every effect at full, and the combination is captured', async ({ context }, testInfo) => {
    const { page, helm } = await bootViewscreen(context);

    // Nothing chosen on this endpoint and nothing asked for by the machine:
    // both intensities resolve to full and are published as such.
    const published = await page.evaluate(() => ({
      shake: window.wasm_shake_intensity(),
      flash: window.wasm_flash_intensity(),
      reduced: window.wasm_is_reduced_motion(),
    }));
    expect(published.reduced, 'the machine asked for nothing').toBe(false);
    expect(published.shake, 'full shake reached the render path').toBeCloseTo(1, 3);
    expect(published.flash, 'full flash reached the render path').toBeCloseTo(1, 3);

    // The whole-page translate path is intact: a nonzero offset really moves
    // the page, so a zero one later is Rust's decision and not a dead wire.
    const translate = await page.evaluate(() => {
      const shell = document.getElementById('viewscreen-shell');
      window.__applyShake(4, -4);
      const applied = getComputedStyle(shell).transform;
      window.__applyShake(0, 0);
      return { applied, cleared: getComputedStyle(shell).transform };
    });
    expect(translate.applied, 'a nonzero offset translates the page').not.toBe('none');
    expect(translate.cleared, 'a zero offset clears the transform').toBe('none');

    // The vignette loops at its authored period. Recorded rather than judged:
    // this capture is the flash evidence a human assesses, and a passing test
    // here is NOT a claim that the default is comfortable.
    const vignette = await vignetteState(page);
    expect(vignette.animationName, 'the alert vignette loops by default').toBe('hud-pulse');
    expect(vignette.animationDuration).toBe('1.3s');

    // The two effects overlapping, in one frame, for the human assessment the
    // PRD requires: red alert lit and the page mid-shake.
    await page.evaluate(() => window.__applyShake(4, -4));
    await testInfo.attach('viewscreen-effects-default.png', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    await page.evaluate(() => window.__applyShake(0, 0));

    await helm.close();
  });

  test('reduced: shake off and flash off, with the alert still readable', async ({ context }, testInfo) => {
    const { page, helm } = await bootViewscreen(context);

    // One press each, through the real cog — not a poked global.
    await chooseEffect(page, 'shake', 'off');
    await chooseEffect(page, 'flash', 'off');

    const published = await page.evaluate(() => ({
      shake: window.wasm_shake_intensity(),
      flash: window.wasm_flash_intensity(),
    }));
    expect(published.shake, 'the shake choice reached the render path').toBe(0);
    expect(published.flash, 'the flash choice reached the render path').toBe(0);

    // The live channel is still firing, and every frame's offset is zero — so
    // the whole page never translates, whatever the hull takes.
    await spyOnShake(page);
    const shake = await sampleShake(page);
    expect(shake.calls, 'the per-frame shake channel is live in InProgress').toBeGreaterThan(0);
    expect(shake.max, 'shake off: Rust pushes only zero offsets').toBe(0);
    expect(shake.shellTransform, 'shake off: no whole-page transform').toBe('none');

    // The pulse stops — and the alert does NOT. Full opacity and the inset red
    // glow are still drawn, so a room reading a ship at red alert off this
    // screen reads it exactly as before (story 15).
    const vignette = await vignetteState(page);
    expect(vignette.animationName, 'flash off: the vignette pulse stops').toBe('none');
    expect(Number(vignette.opacity), 'flash off: the alert stays fully lit').toBe(1);
    expect(vignette.boxShadow, 'flash off: the alert glow is still drawn').not.toBe('none');

    // The HUD text has not moved either: hull and condition are still readable
    // with no shake at all, which is the non-motion equivalent of the cue.
    expect(await page.evaluate(() => document.body.innerText)).toMatch(/HULL/i);

    await testInfo.attach('viewscreen-effects-reduced.png', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });

    await helm.close();
  });

  test('one effect down leaves the others alone', async ({ context }) => {
    const { page, helm } = await bootViewscreen(context);

    // The whole point of three controls: turning the camera shake off must not
    // cost the operator the red-alert cue or the interface's animation.
    await chooseEffect(page, 'shake', 'off');

    const published = await page.evaluate(() => ({
      shake: window.wasm_shake_intensity(),
      flash: window.wasm_flash_intensity(),
    }));
    expect(published.shake).toBe(0);
    expect(published.flash, 'the flash was not touched').toBeCloseTo(1, 3);

    const vignette = await vignetteState(page);
    expect(vignette.animationName, 'the alert vignette still loops').toBe('hud-pulse');
    expect(await page.evaluate(() => document.documentElement.getAttribute('data-shake')))
      .toBe('off');
    expect(await page.evaluate(() => document.documentElement.getAttribute('data-flash')))
      .toBe('full');

    // …and it is still there after a reload, because it is this endpoint's.
    await page.reload();
    await waitForWasmReady(page, 120_000);
    expect(await page.evaluate(() => document.documentElement.getAttribute('data-shake')))
      .toBe('off');

    await helm.close();
  });

  test('interface animation off does not take the red-alert flash with it', async ({ context }) => {
    // The one combination a source-reading test cannot judge, because it is
    // decided by the CASCADE: `gui/tokens.css`'s decorative band sweeps `*` with
    // `!important`, and the vignette's pulse is an ordinary element caught by
    // that `*`. Only a real engine can say which declaration wins, so this is
    // the test that would catch the band silently killing a flash the operator
    // explicitly kept (story 13).
    const { page, helm } = await bootViewscreen(context);

    await chooseEffect(page, 'decorativeMotion', 'off');

    expect(await page.evaluate(() => document.documentElement.getAttribute('data-decorative-motion')))
      .toBe('off');
    expect(await page.evaluate(() => document.documentElement.getAttribute('data-flash')))
      .toBe('full');

    const vignette = await page.evaluate(() => {
      document.getElementById('hud-overlay').classList.add('alert-on');
      const style = getComputedStyle(document.getElementById('hud-vignette'));
      return {
        animationName: style.animationName,
        animationDuration: style.animationDuration,
        animationIterationCount: style.animationIterationCount,
      };
    });
    expect(vignette.animationName, 'the alert pulse survives a stilled interface')
      .toBe('hud-pulse');
    expect(vignette.animationIterationCount, 'and it still LOOPS rather than settling')
      .toBe('infinite');
    expect(vignette.animationDuration, 'at its own full-intensity period')
      .toBe('1.3s');

    // …while the decorative loop it shares the page with really did stop, so
    // holding the flash did not quietly buy back the setting beside it.
    const spinner = await page.evaluate(() => {
      const ring = document.querySelector('.spinner-ring');
      if (!ring) return null;
      const style = getComputedStyle(ring);
      return { name: style.animationName, duration: style.animationDuration };
    });
    if (spinner) {
      expect(
        spinner.name === 'none' || parseFloat(spinner.duration) < 0.01,
        `the decorative spinner is stilled: ${JSON.stringify(spinner)}`,
      ).toBe(true);
    }

    await helm.close();
  });
});
