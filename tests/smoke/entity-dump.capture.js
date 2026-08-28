// Capture aid (not a spec — `npx playwright test` ignores it): boots the patrol
// world with one crew client attached and prints the host page's entity/sim
// diagnostics. Run it by hand when a spawn or render pipeline question needs
// the host's own log rather than an assertion.
//
// It stands up its own browser context rather than using fixtures.js, so it
// carries its own copy of the wiring: the transport stand-in, the registry
// route, the QR stub and a join client. Keep it in step with
// tests/smoke/transport-fixture.js — it is the same handshake, written out.

import { chromium } from '@playwright/test';
import fs from 'fs';
import path from 'path';

// The origin dist/ is served on — see playwright.config.js's PHOENIX_SMOKE_PORT.
// This aid is run by hand against whichever server is up, so it reads the same
// variable rather than assuming 3000.
const ORIGIN = `http://localhost:${process.env.PHOENIX_SMOKE_PORT || 3000}`;

const PATROL_TOML = fs.readFileSync(path.join(__dirname, '../../assets/worlds/patrol.toml'), 'utf-8');
const SHIM = fs.readFileSync(path.join(__dirname, 'rendezvous-shim.js'), 'utf-8');
const REGISTRY_JS = fs.readFileSync(
  path.join(__dirname, '..', '..', 'worker-rendezvous', 'src', 'registry.js'), 'utf-8');
// The registry's own `import './relay.js'` resolves against the path it is
// served at (`/__rendezvous-registry.js` below), which puts it at `/relay.js`
// at the site root — a URL dist/ has no file for. Unrouted, that import 404s,
// the registry promise never resolves, and every host socket call silently
// drops with nothing in the logs connecting it back to a missing sibling
// module (see fixtures.js's RENDEZVOUS_RELAY_JS note).
const RELAY_JS = fs.readFileSync(
  path.join(__dirname, '..', '..', 'worker-rendezvous', 'src', 'relay.js'), 'utf-8');
const STUB_QRCODE = `'use strict'; window.QRCode = { toCanvas: function () { return Promise.resolve(); } };`;
const CLIENT_STAMP = (() => {
  const html = fs.readFileSync(path.join(__dirname, '../../dist/client/index.html'), 'utf-8');
  const m = /<meta\s+name="phoenix-client-stamp"\s+content="([^"]*)"/.exec(html);
  return m ? m[1] : '';
})();

async function readJoinCode(page) {
  await page.waitForFunction(() => {
    const el = document.getElementById('qr-link');
    return el?.href?.includes('#');
  }, { timeout: 30_000 });
  return page.evaluate(() => document.getElementById('qr-link').href.split('#')[1]);
}

async function main() {
  const browser = await chromium.launch({ headless: true });
  const ctx = await browser.newContext();
  await ctx.addInitScript({ content: SHIM });
  await ctx.addInitScript({ content: STUB_QRCODE });
  await ctx.route('**/__rendezvous-registry.js', r =>
    r.fulfill({ contentType: 'application/javascript', body: REGISTRY_JS }));
  await ctx.route('**/relay.js', r =>
    r.fulfill({ contentType: 'application/javascript', body: RELAY_JS }));
  await ctx.route('**/qrcode*.js', r => r.fulfill({ contentType: 'application/javascript', body: STUB_QRCODE }));
  await ctx.route('**/assets/worlds/default.toml', r => r.fulfill({ contentType: 'text/plain', body: PATROL_TOML }));

  const serverPage = await ctx.newPage();
  const logs = [];
  serverPage.on('console', msg => logs.push(`[${msg.type()}] ${msg.text()}`));
  serverPage.on('pageerror', err => logs.push(`[PAGE_ERROR] ${err.message}`));

  await serverPage.goto(`${ORIGIN}/?scenario=assets/worlds/default.toml`);
  await serverPage.waitForFunction(() => !!window.__wasmReady, { timeout: 60_000 });

  const joinCode = await readJoinCode(serverPage);

  // Create helm client
  const helmToken = 'helm-' + Math.random().toString(16).slice(2, 10);
  const helmPage = await ctx.newPage();
  const rk = Math.random().toString(16).slice(2, 10);
  await helmPage.route(`**/blank-${rk}`, r => r.fulfill({ contentType: 'text/html', body: '<html><body></body></html>' }));
  await helmPage.goto(`${ORIGIN}/blank-${rk}`);
  await helmPage.evaluate(({ joinCode, token, stamp }) => new Promise((resolve, reject) => {
    window.__messages = [];
    const factories = window.PhoenixTransportFactories;
    const socket = factories.socket('https://rendezvous.test/v1/join');
    let pc = null;
    socket.onmessage = async (e) => {
      const msg = JSON.parse(e.data);
      if (msg.type === 'ready') {
        socket.send(JSON.stringify({ v: 1, type: 'join', code: joinCode }));
      } else if (msg.type === 'joined') {
        pc = factories.peer({ iceServers: [] });
        const conn = pc.createDataChannel('reliable', { ordered: true });
        pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 });
        window.__conn = conn;
        conn.onopen = () => conn.send(JSON.stringify({ type: 'JoinHandshake', data: { stamp } }));
        conn.onmessage = (ev) => {
          const m = JSON.parse(ev.data);
          if (m.type === 'JoinAccepted') {
            conn.send(JSON.stringify({ type: 'Identify', data: { token, name: 'Helm' } }));
            return;
          }
          if (m.type === 'JoinRefused') { reject(new Error(`refused: ${m.data?.code}`)); return; }
          window.__messages.push(m);
        };
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        socket.send(JSON.stringify({ v: 1, type: 'signal', payload: { sdp: pc.localDescription } }));
      } else if (msg.type === 'signal' && msg.payload?.sdp) {
        await pc.setRemoteDescription(msg.payload.sdp);
      } else if (msg.type === 'error' || msg.type === 'closed') {
        reject(new Error(`rendezvous refused the join: ${msg.reason}`));
      }
    };
    const t = setInterval(() => { if (window.__messages?.some((m) => m.type === 'Welcome')) { clearInterval(t); resolve(); } }, 50);
    setTimeout(() => { clearInterval(t); reject(new Error('Welcome timeout')); }, 15_000);
  }), { joinCode, token: helmToken, stamp: CLIENT_STAMP });

  await helmPage.evaluate(({ station }) => window.__conn.send(JSON.stringify({ type: 'SelectStation', data: { station } })), { station: 'Helm' });
  await helmPage.waitForFunction((t) => window.__messages?.some((m) => m.type === 'StationAssigned' && m.data.token === t), helmToken, { timeout: 5_000 });

  await helmPage.evaluate(() => window.__conn.send(JSON.stringify({ type: 'SetReady', data: { ready: true } })));
  await helmPage.waitForFunction(() => window.__messages?.some((m) => m.type === 'GameStarted'), { timeout: 10_000 });

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
