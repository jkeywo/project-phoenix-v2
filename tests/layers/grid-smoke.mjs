// Real browser -> native and browser -> WASM host over the shared rendezvous stack.
import { chromium, expect } from '../smoke/node_modules/@playwright/test/index.mjs';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { once } from 'node:events';
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
const temporary = await mkdtemp(resolve(tmpdir(), 'phoenix-grid-smoke-'));
const checkpointPath = resolve(temporary, 'grid.json');
const executable = resolve(root, `target/debug/phoenix-grid${process.platform === 'win32' ? '.exe' : ''}`);
let browser;
try {
  await new Promise(done => server.listen(port, '127.0.0.1', done));
  const relay = start(process.execPath, ['scripts/rendezvous-dev-server.mjs', '--port', String(relayPort), '--codes', 'examples/grid/join-codes.json']);
  await waitFor(relay, /listening on/);
  let native = start(executable, [base, origin, checkpointPath]);
  let nativeCode = (await waitFor(native, /Grid join code: (\S+)/))[1];
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
  // Exercise the real optional atomic-file adapter and a fresh native process.
  await expect.poll(async () => JSON.parse(await readFile(checkpointPath, 'utf8')).state.x).toBe(1);
  const nativeExit = once(native, 'exit'); native.kill(); await nativeExit;
  const savedCheckpoint = JSON.parse(await readFile(checkpointPath, 'utf8'));
  // A known pending command proves queue and issuer restoration as well as state.
  savedCheckpoint.pending.push({ tick: savedCheckpoint.state.tick + 2, order: { origin: 1, seq: savedCheckpoint.next_sequence }, movement: { dx: 1, dy: 0 } });
  savedCheckpoint.next_sequence++;
  const nativeCheckpoint = JSON.stringify(savedCheckpoint);
  const continuationPath = resolve(temporary, 'continuation.json');
  await writeFile(continuationPath, nativeCheckpoint);
  const verify = start(executable, ['--verify-continuation', continuationPath]);
  const [verifyExit] = await once(verify, 'exit');
  expect(verifyExit).toBe(0);
  const expectedContinuation = JSON.parse(verify.log.trim());
  native = start(executable, [base, origin, checkpointPath]);
  nativeCode = (await waitFor(native, /Grid join code: (\S+)/))[1];
  await client.locator('#disconnect').click(); await join(client, nativeCode);
  await expect(client.locator('#digest')).toHaveText(/grid\/1\/\d+\/1\/0/);
  console.log('PASS native atomic persistence and fresh-process restore');


  const host = await browser.newPage(); host.on('pageerror', error => errors.push(String(error)));
  await open(host);
  const browserContinuation = await host.evaluate(async checkpoint => {
    const module = await import('./pkg/phoenix_grid.js'); await module.default();
    const grid = new module.GridHost();
    try {
      grid.restore(checkpoint); grid.move_piece(1, 0); grid.move_piece(0, 1);
      for (let index = 0; index < 5; index++) grid.tick();
      return { checkpoint: grid.checkpoint(), state: JSON.parse(grid.state()) };
    } finally { grid.free(); }
  }, nativeCheckpoint);
  expect(browserContinuation).toEqual(expectedContinuation);
  console.log('PASS native/WASM checkpoint continuation: state, digest, pending moves and issuer sequence');
  await host.locator('#host').click();
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
  expect(clientWasm).toEqual([]);
  expect(errors).toEqual([]);
  console.log('PASS browser console -> WASM host: movement, shared transport, checkpoint restored on reload');
} finally {
  await browser?.close(); server.close();
  for (const child of children) {
    if (child.exitCode !== null || child.signalCode !== null) continue;
    const exited = once(child, 'exit'); child.kill(); await exited;
  }
  await rm(temporary, { recursive: true, force: true });
}
