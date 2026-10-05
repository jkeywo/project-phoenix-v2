// Real browser -> native and browser -> WASM host over the shared rendezvous stack.
import { chromium, expect } from '../smoke/node_modules/@playwright/test/index.mjs';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, dirname, extname, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const port = Number(process.env.GRID_SMOKE_PORT || 18180);
const relayPort = port + 1;
const origin = `http://127.0.0.1:${port}`;
const base = `http://127.0.0.1:${relayPort}`;
const children = [];
function start(command, args) {
  const child = spawn(command, args, { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  children.push(child); child.log = '';
  for (const stream of [child.stdout, child.stderr]) stream.on('data', bytes => { child.log += bytes.toString(); });
  child.on('error', error => { child.log += String(error); });
  return child;
}
async function waitFor(child, pattern) {
  await expect.poll(() => child.log, { timeout: 20000 }).toMatch(pattern);
  return child.log.match(pattern);
}
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, origin);
    const path = resolve(root, '.' + decodeURIComponent(url.pathname));
    const rel = relative(root, path);
    if (isAbsolute(rel) || rel.startsWith('..')) { response.writeHead(403).end(); return; }
    const file = url.pathname.endsWith('/') ? resolve(path, 'index.html') : path;
    const types = { '.html': 'text/html', '.js': 'text/javascript', '.json': 'application/json', '.wasm': 'application/wasm' };
    response.setHeader('Content-Type', types[extname(file)] || 'application/octet-stream');
    response.end(await readFile(file));
  } catch { response.writeHead(404).end(); }
});
let browser;
try {
  await new Promise(done => server.listen(port, '127.0.0.1', done));
  const relay = start(process.execPath, ['scripts/rendezvous-dev-server.mjs', '--port', String(relayPort), '--codes', 'examples/grid/join-codes.json']);
  await waitFor(relay, /listening on/);
  const native = start(resolve(root, `target/debug/phoenix-grid${process.platform === 'win32' ? '.exe' : ''}`), [base, origin]);
  const nativeCode = (await waitFor(native, /Grid join code: (\S+)/))[1];
  browser = await chromium.launch({ headless: true });
  const client = await browser.newPage();
  const errors = []; const clientWasm = [];
  client.on('pageerror', error => errors.push(String(error)));
  client.on('request', request => { if (request.url().endsWith('.wasm')) clientWasm.push(request.url()); });
  async function open(page) {
    await page.goto(`${origin}/examples/grid/`);
    await page.locator('#service').fill(base);
  }
  async function join(page, code) {
    await page.locator('#code').fill(code); await page.locator('#join').click();
    await expect(page.locator('#digest')).toContainText('grid/1/', { timeout: 20000 });
  }
  await open(client); await join(client, nativeCode);
  await client.getByRole('button', { name: 'Right →', exact: true }).click();
  await expect(client.locator('#digest')).toHaveText(/grid\/1\/\d+\/1\/0/, { timeout: 10000 });
  await client.locator('#recover').click();
  await expect(client.locator('#status')).toContainText('Checkpoint received');
  await client.reload(); await client.locator('#service').fill(base); await join(client, nativeCode);
  await expect(client.locator('#digest')).toHaveText(/grid\/1\/\d+\/1\/0/);
  expect(clientWasm).toEqual([]);
  console.log('PASS browser console -> native host: movement, reliable checkpoint, reconnect; no client WASM');

  const host = await browser.newPage(); host.on('pageerror', error => errors.push(String(error)));
  await open(host); await host.locator('#host').click();
  await expect(host.locator('#status')).toContainText('Hosting ', { timeout: 20000 });
  const browserCode = await host.locator('#code').inputValue();
  await client.locator('#disconnect').click(); await join(client, browserCode);
  await client.getByRole('button', { name: 'Down ↓', exact: true }).click();
  await expect(client.locator('#digest')).toHaveText(/grid\/1\/\d+\/0\/1/, { timeout: 10000 });
  await expect(host.locator('#digest')).toHaveText(/grid\/1\/\d+\/0\/1/);
  await host.reload(); await host.locator('#service').fill(base); await host.locator('#host').click();
  await expect(host.locator('#status')).toContainText('Hosting ');
  await expect(host.locator('#digest')).toHaveText(/grid\/1\/\d+\/0\/1/);
  await mkdir(resolve(root, '.scratch'), { recursive: true });
  await host.screenshot({ path: resolve(root, '.scratch/grid-browser.png') });
  expect(errors).toEqual([]);
  console.log('PASS browser console -> WASM host: movement, shared transport, checkpoint restored on reload');
} finally {
  await browser?.close(); server.close();
  for (const child of children) child.kill();
}
