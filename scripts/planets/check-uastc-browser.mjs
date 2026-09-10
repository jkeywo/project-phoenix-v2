// Real production viewer build, no prototype bridge. Also tests a site prefix.
// Usage: node scripts/planets/check-uastc-browser.mjs [dist-viewer] [--serve]
import fs from 'node:fs/promises';
import path from 'node:path';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import assert from 'node:assert/strict';
const require = createRequire(path.resolve('tests/smoke/package.json'));
const { chromium } = require('playwright');
const bundle = path.resolve(process.argv[2] ?? 'dist-viewer');
const server = createServer(async (req, res) => {
  try {
    let url = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
    url = url.replace(/^\/subdir(?=\/)/, '');
    if (url === '/api/lod/index') {
      res.setHeader('content-type', 'application/json');
      return res.end('{"models":[]}');
    }
    const assets = url.startsWith('/assets/');
    const root = assets ? path.resolve('assets') : bundle;
    const file = path.resolve(root, assets ? url.slice(8) : url === '/' ? 'index.html' : url.slice(1));
    if (!file.startsWith(root + path.sep)) { res.writeHead(403); return res.end(); }
    const mime = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm',
      '.json': 'application/json', '.ktx2': 'image/ktx2' };
    res.setHeader('content-type', mime[path.extname(file)] ?? 'application/octet-stream');
    res.end(await fs.readFile(file));
  } catch { res.writeHead(404); res.end(); }
});
const serveOnly = process.argv.includes('--serve');
await new Promise(resolve => server.listen(serveOnly ? 8085 : 0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
if (serveOnly) {
  console.log(`Production planet viewer: ${origin}/?entity=assets/entities/planet_gas_giant.toml&lighting=directional`);
  await new Promise(() => {});
}
const browser = await chromium.launch({ headless: true,
  args: ['--enable-webgl', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
await fs.mkdir('target/uastc-game-check', { recursive: true });
try {
  const requestedModes = process.argv.slice(3);
  for (const mode of requestedModes.length ? requestedModes : ['auto', 'rgba', 'missing-worker', 'missing-texture', 'bad-template', 'bad-output', 'subdir']) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    const logs = [], errors = [], requests = [];
    page.on('console', message => {
      if (/Planet UASTC/.test(message.text())) logs.push(message.text());
      if (/panicked at|validation error|shader.*error/i.test(message.text())) errors.push(message.text());
    });
    page.on('pageerror', error => errors.push(String(error)));
    page.on('request', request => requests.push(request.url()));
    await page.addInitScript(({ rgba }) => {
      window.textureAllocations = [];
      const get = HTMLCanvasElement.prototype.getContext;
      HTMLCanvasElement.prototype.getContext = function(kind, ...args) {
        const gl = get.call(this, kind, ...args);
        if (kind === 'webgl2' && gl && !gl.uastcObserved) {
          gl.uastcObserved = true;
          if (rgba) {
            const extensions = gl.getSupportedExtensions.bind(gl);
            gl.getSupportedExtensions = () => extensions().filter(name => !/compressed_texture|texture_compression/i.test(name));
            const extension = gl.getExtension.bind(gl);
            gl.getExtension = name => /compressed_texture|texture_compression/i.test(name) ? null : extension(name);
          }
          const allocate = gl.texStorage2D.bind(gl);
          gl.texStorage2D = (target, levels, format, width, height) => {
            if (width === 4096 && height === 2048) window.textureAllocations.push({ levels, format });
            return allocate(target, levels, format, width, height);
          };
        }
        return gl;
      };
    }, { rgba: mode === 'rgba' });
    if (mode === 'missing-worker') await page.route('**/uastc-worker.js', route => route.abort());
    if (mode === 'missing-texture') await page.route('**/surface_colour.uastc.ktx2', route => route.fulfill({ status: 404, body: '' }));
    if (mode === 'bad-template') await page.route('**/gas-base-templates.json', route => route.fulfill({ contentType: 'application/json', body: '{}' }));
    if (mode === 'bad-output') await page.route('**/uastc-worker.js', route => route.fulfill({
      contentType: 'text/javascript', body: 'self.onmessage = () => self.postMessage({buffer:new ArrayBuffer(80)});',
    }));
    const prefix = mode === 'subdir' ? '/subdir' : '';
    await page.goto(`${origin}${prefix}/?entity=assets/entities/planet_gas_giant.toml&lighting=directional`);
    await page.waitForFunction(() => {
      const raw = window.wasmBindings?.viewer_stats?.();
      const stats = raw && JSON.parse(raw);
      return stats?.settled && stats.textures === 11 && stats.measuredTextures === 11;
    }, null, { timeout: 180000 });
    const allocations = await page.evaluate(() => window.textureAllocations);
    const fallback = mode.startsWith('missing') || mode.startsWith('bad-');
    assert.ok(logs.some(line => line.includes(fallback ? 'fallback' : 'loaded')), `${mode}: ${logs}`);
    const expected = fallback || mode === 'rgba' ? [0x8c43] : [0x93d0, 0x8e8d, 0x9279];
    assert.ok(allocations.some(a => a.levels === 13 && expected.includes(a.format)), `${mode}: ${JSON.stringify(allocations)}`);
    assert.equal(requests.some(url => /\/surface_colour\.ktx2$/.test(url)), fallback, `${mode}: original download`);
    assert.deepEqual(errors, []);
    await page.screenshot({ path: `target/uastc-game-check/${mode}.png` });
    console.log(JSON.stringify({ mode, allocations, logs, pass: true }));
    await page.close();
  }
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
