import { chromium, Page } from '@playwright/test';
import fs from 'fs';
import path from 'path';

const PATROL_TOML = fs.readFileSync(path.join(__dirname, '../../assets/worlds/patrol.toml'), 'utf-8');
const SHIM = fs.readFileSync(path.join(__dirname, 'peerjs-shim.js'), 'utf-8');
const STUB_PEER_JS = `'use strict'; if (typeof window.Peer === 'undefined') { window.Peer = function Peer() {}; };`;
const STUB_QRCODE = `'use strict'; window.QRCode = { toCanvas: function () { return Promise.resolve(); } };`;

async function readHostPeerId(page: Page): Promise<string> {
  await page.waitForFunction(() => { const el = document.getElementById('qr-link'); return el?.href?.includes('#'); }, { timeout: 20_000 });
  return page.evaluate(() => (document.getElementById('qr-link') as HTMLAnchorElement).href.split('#')[1]);
}

async function main() {
  const browser = await chromium.launch({ headless: true });
  const ctx = await browser.newContext();
  await ctx.addInitScript({ content: STUB_PEER_JS });
  await ctx.addInitScript({ content: STUB_QRCODE });
  await ctx.addInitScript({ content: SHIM });
  await ctx.route('**/peerjs*.js', r => r.fulfill({ contentType: 'application/javascript', body: STUB_PEER_JS }));
  await ctx.route('**/qrcode*.js', r => r.fulfill({ contentType: 'application/javascript', body: STUB_QRCODE }));
  await ctx.route('**/assets/worlds/default.toml', r => r.fulfill({ contentType: 'text/plain', body: PATROL_TOML }));

  const serverPage = await ctx.newPage();
  const logs: string[] = [];
  serverPage.on('console', msg => logs.push(`[${msg.type()}] ${msg.text()}`));
  serverPage.on('pageerror', err => logs.push(`[PAGE_ERROR] ${err.message}`));

  await serverPage.goto('http://localhost:3000/?scenario=assets/worlds/default.toml');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 60_000 });

  const hostId = await readHostPeerId(serverPage);

  // Create helm client
  const helmToken = 'helm-' + Math.random().toString(16).slice(2, 10);
  const helmPage = await ctx.newPage();
  const rk = Math.random().toString(16).slice(2, 10);
  await helmPage.route(`**/blank-${rk}`, r => r.fulfill({ contentType: 'text/html', body: '<html><body></body></html>' }));
  await helmPage.goto(`http://localhost:3000/blank-${rk}`);
  await helmPage.evaluate(({ hostId, token }) => new Promise<void>((resolve, reject) => {
    (window as any).__messages = [];
    const peer = new (window as any).Peer();
    peer.on('open', () => {
      const conn = peer.connect(hostId);
      (window as any).__conn = conn;
      conn.on('open', () => conn.send(JSON.stringify({ type: 'Identify', data: { token, name: 'Helm' } })));
      conn.on('data', (raw: string) => { try { (window as any).__messages.push(JSON.parse(raw)); } catch {} });
    });
    const t = setInterval(() => { if ((window as any).__messages?.some((m: any) => m.type === 'Welcome')) { clearInterval(t); resolve(); } }, 50);
    setTimeout(() => { clearInterval(t); reject(new Error('Welcome timeout')); }, 15_000);
  }), { hostId, token: helmToken });

  await helmPage.evaluate(({ station }) => (window as any).__conn.send(JSON.stringify({ type: 'SelectStation', data: { station } })), { station: 'Helm' });
  await helmPage.waitForFunction((t: any) => (window as any).__messages?.some((m: any) => m.type === 'StationAssigned' && m.data.token === t), helmToken, { timeout: 5_000 });

  await helmPage.evaluate(() => (window as any).__conn.send(JSON.stringify({ type: 'StartGame' })));
  await helmPage.waitForFunction(() => (window as any).__messages?.some((m: any) => m.type === 'GameStarted'), { timeout: 10_000 });

  // Wait for several sim ticks so render_spawned_entities and sim_state run
  await new Promise(r => setTimeout(r, 3000));

  // Filter and print diagnostic logs
  for (const line of logs) {
    if (line.includes('render_spawned_entities') || line.includes('sim_state npc') || line.includes('ENTITY DUMP') || line.includes('ENTTY') || line.includes('===')) {
      console.log(line);
    }
  }

  // Print sim_state lines
  for (const line of logs) {
    if (line.includes('sim_state npc')) {
      console.log('SIM:', line.substring(0, 200));
    }
  }

  await browser.close();
}

main().catch(err => { console.error(err); process.exit(1); });
