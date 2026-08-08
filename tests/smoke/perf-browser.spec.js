// Issue #868 — browser-host measurement harness.
//
// Loads the server page, lets it run for a fixed number of animation frames,
// and writes out the capture the wasm collector accumulated. The comparison
// against the committed baseline is not done here: `phoenix-perf report` owns
// that, so the measurement contract has exactly one implementation rather than
// a second opinion written in JS.
//
// WHAT THIS MEASURES, AND WHAT IT DOES NOT
//
// Under Playwright `navigator.webdriver` is true, and `wasm_init` responds by
// skipping the render, audio, glTF and gizmo plugins — a headless runner has
// no GPU for wgpu to initialise. So `browser.frame` here is the ECS schedule
// per animation frame, NOT rendering, and the capture's provenance records the
// runtime as `wasm-automation` to keep it from ever being compared against a
// real browser session. Measuring the real render path would mean the
// SwiftShader route the *.capture.js aids use, which measures a software
// rasteriser — a different thing again, and not obviously a better proxy for a
// player's GPU.
//
// This test asserts on the shape of the measurement, never on a duration. A
// slow runner must not fail the build.

import fs from 'fs';
import path from 'path';
import { test, expect, waitForWasmReady } from './fixtures';

const SCENARIO = 'browser-automation';
const OUT_DIR = path.resolve(__dirname, '../../target/perf');
const OUT_FILE = path.join(OUT_DIR, 'browser-capture.json');

// Enough frames for the summary percentiles to mean something, short enough
// that the smoke suite does not grow a minute.
const FRAMES = 240;

test('browser host: boot, preload and frame time are measured', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  // Wait for real frames rather than a wall-clock sleep: a slow runner should
  // produce the same number of samples, just later. `evaluate` rather than
  // `waitForFunction` because this is one awaited promise, not a predicate to
  // poll — polling would restart the frame count on every attempt.
  await serverPage.evaluate(
    (frames) =>
      new Promise((resolve) => {
        let seen = 0;
        const tick = () => {
          if (++seen >= frames) return resolve();
          requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }),
    FRAMES,
  );

  const json = await serverPage.evaluate(
    (scenario) => window.wasm_perf_capture(scenario),
    SCENARIO,
  );

  // An empty string is the collector saying "nothing sampled" — treat it as a
  // failure of the harness, not as a zeroed measurement.
  expect(json, 'the wasm collector produced no capture').not.toBe('');

  const capture = JSON.parse(json);
  expect(capture.scenario).toBe(SCENARIO);
  expect(capture.profile.runtime).toBe('wasm-automation');

  // Boot always samples; the frame metric must have accumulated real samples.
  expect(capture.summaries['browser.boot']).toBeTruthy();
  expect(capture.summaries['browser.frame']).toBeTruthy();
  expect(capture.summaries['browser.frame'].summary.count).toBeGreaterThan(30);

  fs.mkdirSync(OUT_DIR, { recursive: true });
  fs.writeFileSync(OUT_FILE, `${JSON.stringify(capture, null, 2)}\n`);
});
