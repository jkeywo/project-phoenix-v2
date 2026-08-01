// Issue #895 — the fixed logical tick in the browser host.
//
// The Rust suite proves the schedule shape natively; what it cannot see is the
// real browser frame loop, so this is the browser-side half of the acceptance:
// the simulation advances on `Time<Fixed>` at the world's authored
// `[global] sim_tick_hz` (default 60), not once per rendered frame.
//
// `wasm_sim_tick()` reads back the `SimTick` counter (mirrored per frame by
// `publish_sim_tick` in `src/server/bridge.rs`). Two properties are pinned:
//
// 1. RATE — over a window of real animation frames the tick advances at
//    roughly the authored rate per WALL second, with deliberately wide bounds
//    so a slow CI runner cannot fail the build on timing.
// 2. DECOUPLING — the debug sim pause (F9 / `wasm_toggle_debug_pause`) pauses
//    `Time<Virtual>`, which is what feeds the fixed accumulator. Frames keep
//    rendering while the tick FREEZES, and resume advances it again. A
//    frame-driven sim cannot pass this: its "tick" would follow the frames.

import { test, expect, waitForWasmReady } from './fixtures';

/** Wait for `frames` real animation frames on the page. */
async function waitFrames(page: any, frames: number): Promise<void> {
  await page.evaluate(
    (n: number) =>
      new Promise<void>((resolve) => {
        let seen = 0;
        const tick = () => {
          if (++seen >= n) return resolve();
          requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }),
    frames,
  );
}

test('sim advances on the fixed logical tick, not the rendered frame', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  // The export exists and the counter is live even in the lobby — the fixed
  // loop runs from boot; only the SimSet chain inside it gates on InProgress.
  const t0 = await serverPage.evaluate(() => ({
    tick: (window as any).wasm_sim_tick() as number,
    now: performance.now(),
  }));
  expect(typeof t0.tick).toBe('number');

  // RATE. 120 real frames (~2 s at 60 Hz rAF); the default world authors no
  // sim_tick_hz, so the serde-default 60 Hz applies. Bounds are wide on both
  // sides: a throttled runner under-runs (Bevy credits at most 250 ms of
  // virtual time per frame), and nothing should ever overshoot 3x.
  await waitFrames(serverPage, 120);
  const t1 = await serverPage.evaluate(() => ({
    tick: (window as any).wasm_sim_tick() as number,
    now: performance.now(),
  }));
  expect(t1.tick).toBeGreaterThan(t0.tick);
  const wallSecs = (t1.now - t0.now) / 1000;
  const rate = (t1.tick - t0.tick) / wallSecs;
  expect(
    rate,
    `the logical tick advanced at ${rate.toFixed(1)} Hz over ${wallSecs.toFixed(2)} s — ` +
      'expected the authored 60 Hz (wide bounds for slow runners)',
  ).toBeGreaterThan(15);
  expect(rate).toBeLessThan(180);

  // DECOUPLING. Pause the sim clock; frames keep coming, the tick must not.
  await serverPage.evaluate(() => (window as any).wasm_toggle_debug_pause());
  // One frame for the PreUpdate drain to apply the toggle, then measure.
  await waitFrames(serverPage, 5);
  const pausedStart = await serverPage.evaluate(() => (window as any).wasm_sim_tick() as number);
  await waitFrames(serverPage, 60);
  const pausedEnd = await serverPage.evaluate(() => (window as any).wasm_sim_tick() as number);
  expect(
    pausedEnd,
    'the sim clock is paused but the logical tick kept counting — the sim is ' +
      'advancing per rendered frame, not on the fixed tick',
  ).toBe(pausedStart);

  // Resume: the tick advances again.
  await serverPage.evaluate(() => (window as any).wasm_toggle_debug_pause());
  await waitFrames(serverPage, 30);
  const resumed = await serverPage.evaluate(() => (window as any).wasm_sim_tick() as number);
  expect(resumed).toBeGreaterThan(pausedEnd);
});
