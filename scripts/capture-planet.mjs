// Repeatable captures through Phoenix's real WASM viewer. Run from the repo root:
// node scripts/capture-planet.mjs <built-viewer-dir> <output-dir>
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import sharp from 'sharp';
import { parse } from 'smol-toml';
import path from 'node:path';
import { createRequire } from 'node:module';
const require = createRequire(path.resolve(process.env.PLAYWRIGHT_PACKAGE_ROOT ?? 'tests/smoke', 'package.json'));
const { chromium } = require('playwright');

const [bundle, output, assetRoot = '.'] = process.argv.slice(2);
const entity = process.env.PLANET_ENTITY ?? 'assets/entities/planet_ecumenopolis.toml';
if (!bundle || !output) throw new Error('Expected built viewer directory and output directory');
await mkdir(output, { recursive: true });
const planet = parse(await readFile(path.join(assetRoot, entity), 'utf8')).planet;
const radiusScale = planet.radius / 33;
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.wgsl': 'text/plain', '.toml': 'text/plain', '.json': 'application/json', '.png': 'image/png', '.webp': 'image/webp' };
const server = createServer(async (req, res) => {
  const name = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
  const root = path.resolve(name.startsWith('/assets/') ? assetRoot : bundle);
  const file = path.resolve(root, '.' + (name === '/' ? '/index.html' : name));
  if (!file.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
  try { res.setHeader('Content-Type', mime[path.extname(file)] ?? 'application/octet-stream'); res.end(await readFile(file)); }
  catch { res.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await chromium.launch({ headless: true, args: ['--enable-webgl', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1 });
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  page.on('console', msg => {
    const text = msg.text();
    if ((msg.type() === 'error' || /error.*shader|validation error/i.test(text)) && !/Failed to load resource|WebSocket connection/.test(text)) errors.push(text);
  });
  page.on('response', response => {
    if (response.status() >= 400 && !/\.meta$|\/api\/lod\/|favicon/.test(response.url())) errors.push(`HTTP ${response.status()}: ${response.url()}`);
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/?entity=${encodeURIComponent(entity)}&lighting=directional`);
  await page.waitForFunction(() => window.wasmBindings?.viewer_load_entity, null, { timeout: 180000 });
  await page.evaluate(entity => window.wasmBindings.viewer_load_entity(entity), entity);
  await page.waitForTimeout(12000);
  await page.waitForFunction(() => {
    const s = JSON.parse(window.wasmBindings.viewer_stats());
    return s.meshes > 0 && s.extent > 0 && s.textures === s.measuredTextures;
  }, null, { timeout: 120000 });
  await page.evaluate(() => { document.querySelectorAll('.panel, #status').forEach(e => e.style.display = 'none'); window.wasmBindings.viewer_set_skybox_brightness(0); });
  const poses = [
    ['day', 0.6, 0.2, 105, 0.9, 0.35],
    ['terminator', 0.6, 0.2, 105, 2.1, 0.2],
    ['night', 0.6, 0.2, 105, 3.7, 0.2],
    ['close', 0.6, 0.2, 66, 1.5, 0.3],
    ['pole', 0.6, 1.5, 105, 1.8, 0.4],
  ];
  if (process.env.PLANET_MOTION === '1') {
    const sunYaw = planet.surface.natural?.kind === 'ice' ? 3.7 : 0.9;
    poses.push(['motion_a', 0.6, 0.2, 105, sunYaw, 0.2], ['motion_b', 0.6, 0.2, 105, sunYaw, 0.2]);
  }
  let motionFirst;
  let motionMeanDifference;
  for (const [name, yaw, pitch, radius, sunYaw, sunPitch] of poses) {
    await page.evaluate(({ yaw, pitch, radius, sunYaw, sunPitch }) => {
      const w = window.wasmBindings;
      w.viewer_set_camera(0, 0, 0, radius, yaw, pitch);
      w.viewer_set_directional(10000, sunYaw, sunPitch);
    }, { yaw, pitch, radius: radius * radiusScale, sunYaw, sunPitch });
    await page.waitForTimeout(name === 'motion_b' ? 6000 : 2500);
    const png = await page.screenshot({ path: path.join(output, name + '.png') });
    const pixels = await sharp(png).extract({ left: 320, top: 225, width: 640, height: 450 }).stats();
    if (pixels.channels.slice(0, 3).every(c => c.stdev < 2)) errors.push(`${name}: scene is blank`);
    if (name.startsWith('motion_')) {
      const rgb = await sharp(png).removeAlpha().raw().toBuffer();
      if (name === 'motion_a') motionFirst = rgb;
      else {
        motionMeanDifference = rgb.reduce((sum, value, index) => sum + Math.abs(value - motionFirst[index]), 0) / rgb.length;
        if (motionMeanDifference < 0.001) errors.push('motion: fixed-camera frames did not change');
      }
    }
  }
  const report = { errors, motionMeanDifference, stats: JSON.parse(await page.evaluate(() => window.wasmBindings.viewer_stats())) };
  await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
  if (errors.length) process.exitCode = 1;
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
