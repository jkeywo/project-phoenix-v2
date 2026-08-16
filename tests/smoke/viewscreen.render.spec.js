// The viewscreen actually draws — a standing check on the one defect class the
// rest of this suite is blind to by construction.
//
// # Why this file needs its own Playwright project
//
// Every other spec here asserts on wire messages and DOM, so the suite runs
// without a GL backend and `src/server/bridge.rs` skips `RenderPlugin`
// entirely under `navigator.webdriver` (Bevy's wgpu init panics on a GPU-less
// runner). That is the right trade for 135 message tests — and it is exactly
// why a render-graph break shipped: nothing in CI ever drew a frame.
//
// So this spec opts back in on both counts, and the `render` project in
// `playwright.config.js` is where the first half lives:
//
//   * SwiftShader supplies a software WebGL2 context (the same launch args
//     `playwright.capture.config.js` uses for the capture aids);
//   * the `navigator.webdriver` flag is hidden below, so bridge.rs takes the
//     real render path instead of the automation one.
//
// # What it asserts, and why that shape
//
// A render-graph break does not have to be loud. The regression that prompted
// this file (`Hdr` on the 3D camera while the 2D UI camera sharing the same
// window stayed LDR) produced a completely clean console: two cameras with
// different `hdr` get different main textures out of Bevy's texture cache
// (`prepare_view_targets` keys on `view.hdr`), so the UI camera's `Upscaling`
// node blitted its own untouched black texture over the 3D scene. Perfectly
// valid wgpu; nothing to log; black viewscreen.
//
// The assertion is therefore about UNIFORMITY rather than brightness. A wiped
// canvas is exactly one colour across the scene area; a drawn one is not,
// whatever the art direction does next. Pinning a brightness range instead
// would fail every time somebody tunes the skybox, and pass the day the
// compositing breaks again.
//
// Both halves of the `[render]` retreat are covered: the shipped default
// (`hdr = true`) and the documented one-line escape (`hdr = false`), because a
// retreat valve that was never tested is not a retreat valve.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import fs from 'fs';
import path from 'path';

/** Worlds this spec boots. Both are shipped scenarios, deliberately: the thing
 *  under test is whether the REAL viewscreen draws, and a fixture world with no
 *  skybox, dust or hull would be a weaker check than the one players see. */
const COMBAT_TEST = 'assets/worlds/combat_test.toml';
const FALLING_SKYWAY = 'assets/worlds/falling_skyway.toml';

function worldToml(rel) {
  return fs.readFileSync(path.join(__dirname, '../..', rel), 'utf-8');
}

/** The shipped world with an authored `[render]` block appended.
 *
 *  No world ships one today (every one takes `RenderConfig::default()`), so
 *  this only ever ADDS the block — but it drops any existing `[render]` /
 *  `[render.*]` section first, so the helper stays correct, and the TOML stays
 *  parseable, the day a designer authors one.
 */
function withRenderBlock(toml, block) {
  if (!block) return toml;
  const kept = [];
  let dropping = false;
  for (const line of toml.split('\n')) {
    if (/^\s*\[render(\.[a-z_]+)*\]\s*$/.test(line)) {
      dropping = true;
      continue;
    }
    if (dropping && /^\s*\[/.test(line)) dropping = false;
    if (!dropping) kept.push(line);
  }
  return `${kept.join('\n')}\n\n${block}\n`;
}

/** Pixel statistics for the centre of the Bevy canvas.
 *
 *  Reads back through a screenshot rather than `gl.readPixels`: Bevy does not
 *  set `preserveDrawingBuffer`, so the drawing buffer is empty by the time a
 *  test could reach it, while a screenshot captures what was composited.
 *  The PNG is decoded back inside the page through a 2D canvas, which keeps
 *  this dependency-free.
 *
 *  The sampled region is the middle 40% of the canvas — inside the viewscreen
 *  border, away from the HUD chrome the 2D camera draws round the edges, so a
 *  live HUD over a dead 3D scene cannot pass this.
 */
async function canvasStats(page) {
  const box = await page.locator('#canvas').boundingBox();
  expect(box, 'the Bevy canvas is laid out').toBeTruthy();
  const clip = {
    x: Math.round(box.x + box.width * 0.3),
    y: Math.round(box.y + box.height * 0.3),
    width: Math.round(box.width * 0.4),
    height: Math.round(box.height * 0.4),
  };
  const png = await page.screenshot({ clip });
  return page.evaluate(async (b64) => {
    const img = new Image();
    img.src = 'data:image/png;base64,' + b64;
    await img.decode();
    const c = document.createElement('canvas');
    c.width = img.width;
    c.height = img.height;
    const ctx = c.getContext('2d');
    ctx.drawImage(img, 0, 0);
    const d = ctx.getImageData(0, 0, c.width, c.height).data;
    let max = 0;
    let sum = 0;
    let nonBlack = 0;
    const seen = new Set();
    for (let i = 0; i < d.length; i += 4) {
      const r = d[i];
      const g = d[i + 1];
      const b = d[i + 2];
      const lum = Math.max(r, g, b);
      if (lum > max) max = lum;
      sum += lum;
      if (lum > 8) nonBlack += 1;
      if (seen.size < 512) seen.add((r << 16) | (g << 8) | b);
    }
    const n = d.length / 4;
    return {
      maxChannel: max,
      meanLuma: +(sum / n).toFixed(2),
      nonBlackFraction: +(nonBlack / n).toFixed(4),
      distinctColours: seen.size,
    };
  }, png.toString('base64'));
}

/** Console lines that mean the render stack failed rather than merely warned.
 *
 *  Deliberately narrower than "every console error": the deployed page also
 *  logs LOD-sidecar 404s, which are a content gap tracked elsewhere and not a
 *  reason to fail a render check. Anything naming a pipeline, shader or wgpu
 *  validation failure is not in that category.
 */
function captureRenderErrors(page) {
  const errors = [];
  page.on('pageerror', (e) => {
    if (e.message === 'unreachable') return;
    errors.push(`pageerror: ${e.message}`);
  });
  page.on('console', (m) => {
    if (m.type() !== 'error') return;
    const t = m.text();
    if (/404|Failed to load resource/i.test(t)) return;
    if (
      /wgpu|naga|wgsl|shader|pipeline|render graph|validation|panicked at|RuntimeError/i.test(t)
    ) {
      errors.push(t.slice(0, 400));
    }
  });
  return errors;
}

/** Boot a world to the point the viewscreen is live, and measure it. */
async function bootAndMeasure(context, { world, renderBlock }) {
  await context.route(`**/${world}`, (route) =>
    route.fulfill({ contentType: 'text/plain', body: withRenderBlock(worldToml(world), renderBlock) }),
  );

  const page = await context.newPage();
  const errors = captureRenderErrors(page);

  // bridge.rs skips the render stack under WebDriver; this project supplies a
  // SwiftShader GPU, so hide the flag and take the real path.
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'webdriver', { get: () => false });
  });

  await page.goto(`/?scenario=${world}`);

  // A multi-hull world shows the picker; a single-hull one auto-selects and the
  // card never appears, so a timeout here is an expected outcome rather than a
  // failure.
  const card = page.locator('#scenario-panel ph-ship-picker .ship-card').first();
  const picked = await card
    .waitFor({ state: 'visible', timeout: 45_000 })
    .then(() => true)
    .catch(() => false);
  if (picked) await card.click();
  await waitForWasmReady(page, 120_000);

  // Without a GL context the canvas never draws and every assertion below
  // would "pass" the black check for the wrong reason.
  const gl = await page.evaluate(() => {
    const ctx = document.createElement('canvas').getContext('webgl2');
    return !!ctx;
  });
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

  // Let a few frames composite before reading the canvas back.
  for (let i = 0; i < 12; i += 1) {
    await page.bringToFront();
    await page.waitForTimeout(150);
  }

  const stats = await canvasStats(page);
  return { page, helm, errors, stats };
}

/** The shared assertion: the scene drew, and the render stack said nothing. */
function expectViewscreenDrawn(stats, errors, label) {
  // The defect signature: one flat colour where the 3D scene should be.
  expect(stats.distinctColours, `${label}: the scene area is not one flat colour`).toBeGreaterThan(1);
  // And it is not merely two shades of black — something is actually lit.
  expect(stats.maxChannel, `${label}: the scene area has lit pixels`).toBeGreaterThan(16);
  expect(errors, `${label}: no render-stack errors`).toEqual([]);
}

test.describe('viewscreen renders', () => {
  test.describe.configure({ timeout: 420_000 });

  test('combat_test draws the scene on the shipped [render] defaults', async ({ context }) => {
    const { errors, stats } = await bootAndMeasure(context, { world: COMBAT_TEST });
    console.log(`combat_test/default pixels: ${JSON.stringify(stats)}`);
    expectViewscreenDrawn(stats, errors, 'combat_test default');
  });

  // The documented retreat from PRD #1023's HDR calibration. It is a one-line
  // change a designer is told they can make, so it is a path that has to boot
  // and draw, not just parse.
  test('combat_test draws with the [render] hdr = false retreat', async ({ context }) => {
    const { errors, stats } = await bootAndMeasure(context, {
      world: COMBAT_TEST,
      renderBlock: '[render]\nhdr = false\n',
    });
    console.log(`combat_test/hdr-off pixels: ${JSON.stringify(stats)}`);
    expectViewscreenDrawn(stats, errors, 'combat_test hdr=false');
  });

  test('falling_skyway draws the scene on the shipped [render] defaults', async ({ context }) => {
    const { errors, stats } = await bootAndMeasure(context, { world: FALLING_SKYWAY });
    console.log(`falling_skyway/default pixels: ${JSON.stringify(stats)}`);
    expectViewscreenDrawn(stats, errors, 'falling_skyway default');
  });
});
