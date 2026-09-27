#!/usr/bin/env node
// #1530 real sockets and browser documents. No smoke transport fixtures.
import { createServer } from 'node:http';
import { spawn, execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { readFile, writeFile, mkdir, readdir, stat, realpath } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { cpus, totalmem, platform, release } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { impairDataChannel } from './fleet-channel-impairment.mjs';
import { readVerifiedWasmReceipt } from './fleet-wasm-build-receipt.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const ROUTES = ['direct', 'ws-relay', 'automatic-fallback'];
export function optionsFrom(args) {
  const options = { dist: path.join(root, 'dist'), dependencies: path.join(root, 'tests/smoke'),
    routes: [...ROUTES], seconds: 10, timeout: 90, port: 18430, rendezvousPort: 18431, delayMs: 0, lossPercent: 0, seed: 1530, render: false };
  for (let i = 0; i < args.length; i++) {
    const key = args[i];
    if (key === '--render') { options.render = true; continue; }
    const value = args[++i];
    if (!value || value.startsWith('--')) throw new Error(`Missing value for ${key}`);
    if (['--out', '--dist', '--dependencies', '--service-script', '--wasm-build-receipt'].includes(key)) options[key.slice(2)] = path.resolve(value);
    else if (key === '--routes') options.routes = value.split(',');
    else if (['--seconds', '--timeout', '--port', '--rendezvous-port', '--delay-ms', '--loss-percent', '--seed'].includes(key)) {
      const name = ({ '--rendezvous-port': 'rendezvousPort', '--delay-ms': 'delayMs', '--loss-percent': 'lossPercent' })[key] || key.slice(2);
      options[name] = Number(value);
    } else throw new Error(`Unknown option ${key}`);
  }
  if (!options.out) throw new Error('Choose --out <fresh directory>');
  if (!options.routes.length || options.routes.some(route => !ROUTES.includes(route)) || new Set(options.routes).size !== options.routes.length) throw new Error('Choose distinct supported --routes');
  for (const [name, min, max] of [['seconds', 1, 300], ['timeout', 1, 180], ['port', 1024, 65535], ['rendezvousPort', 1024, 65535], ['delayMs', 0, 2000], ['lossPercent', 0, 100], ['seed', 0, 4294967295]]) {
    if (!Number.isInteger(options[name]) || options[name] < min || options[name] > max) throw new Error(`Invalid ${name}`);
  }
  if (options.port === options.rendezvousPort) throw new Error('HTTP and rendezvous ports must differ');
  return options;
}
export function verifyEvidence(result) {
  if (result.peers.length !== 6 || result.clients.length !== 12) throw new Error('Expected six real peers and twelve client documents');
  if (result.peers.filter(peer => peer.state?.fleet?.role === 'ship').length !== 4 || result.peers.filter(peer => peer.state?.fleet?.role === 'gm').length !== 2) throw new Error('Actual peer roles differ from four ships and two GMs');
  if (new Set(result.peers.map(peer => peer.state?.mesh?.slot)).size !== 6) throw new Error('Actual fleet slots are not distinct');
  if (!Number.isInteger(result.commandWaves) || result.commandWaves < 1) throw new Error('No active command waves');
  for (const endpoint of [...result.peers, ...result.clients]) {
    if (endpoint.errors?.length) throw new Error(`${endpoint.label}: browser exception observed`);
  }
  for (const peer of result.peers) {
    if (!peer.state?.mesh?.in_fleet || peer.state.mesh.peers.length !== 5 || peer.state.phase !== 'InProgress') throw new Error(`${peer.label}: incomplete actual fleet launch`);
    if (!peer.state.mesh.samples || peer.state.mesh.peers_heard?.length !== 5) throw new Error(`${peer.label}: no complete digest exchange`);
    if (!peer.state.mesh.agreed || peer.state.mesh.disagreement) throw new Error(`${peer.label}: observed simulation disagreement`);
  }
  for (const client of result.clients) {
    if (client.state?.phase !== 'InProgress') throw new Error(`${client.label}: client did not enter play`);
    const receipts = client.state?.outcomes || [];
    if (receipts.filter(row => row.outcome === 'Applied' && row.correlation?.startsWith('matrix-')).length < 1) throw new Error(`${client.label}: no applied command receipt`);
    if (receipts.some(row => row.outcome === 'Refused' && row.correlation?.startsWith('matrix-'))) throw new Error(`${client.label}: workload command refused`);
    if (receipts.filter(row => row.outcome === 'Applied' && row.correlation?.startsWith('matrix-')).length < Math.min(result.commandWaves, 64)) throw new Error(`${client.label}: workload receipts incomplete`);
  }
  for (const endpoint of [...result.peers, ...result.clients]) {
    const direct = endpoint.state?.rtc?.some(peer => peer.connectionState === 'connected' && peer.selected.some(pair => pair.state === 'succeeded' && ['host', 'srflx', 'prflx'].includes(pair.localType) && ['host', 'srflx', 'prflx'].includes(pair.remoteType) && pair.bytesReceived > 0));
    const connected = endpoint.state?.rtc?.filter(peer => peer.connectionState === 'connected') || [];
    const expectedLinks = endpoint.label === 'ship-1' ? 8 : /^ship-\d$/.test(endpoint.label) ? 4 : 1;
    if (result.route === 'direct' && (endpoint.state?.relayReady || endpoint.state?.counts?.['relay-peer'] || connected.length !== expectedLinks || connected.some(peer => !peer.selected.some(pair => pair.state === 'succeeded' && ['host', 'srflx', 'prflx'].includes(pair.localType) && ['host', 'srflx', 'prflx'].includes(pair.remoteType) && pair.bytesReceived > 0)) || !direct)) throw new Error(`${endpoint.label}: no observed direct WebRTC traffic`);
    if (result.route !== 'direct' && ((endpoint.label === 'ship-1' ? endpoint.state?.counts?.['relay-peer'] !== 8 : !endpoint.state?.relayReady) || !endpoint.state?.relayFrames || direct)) throw new Error(`${endpoint.label}: no observed relay-ready handshake`);
    if (result.route === 'ws-relay' && endpoint.state?.signalOffersSent) throw new Error(`${endpoint.label}: forced relay unexpectedly attempted RTC`);
    if (result.route === 'automatic-fallback' && endpoint.label !== 'ship-1' && !endpoint.state?.signalOffersSent) throw new Error(`${endpoint.label}: automatic fallback never attempted RTC`);
  }
  if (result.gmActions?.length !== 2) throw new Error('Both GM action outcomes must be observed');
  return true;
}
export function verifyImpairment(result, options) {
  if (result.route === 'automatic-fallback' && (!result.impairment?.profile?.block_rtc_offers || result.impairment.counters.signal_offers_dropped < 1)) throw new Error('Automatic fallback lacks observed suppressed Phoenix offers');
  if (!options.delayMs && !options.lossPercent) return true;
  let reliableWritten = 0, snapshotWritten = 0, snapshotDropped = 0, delayed = 0;
  const observedDelays = [];
  const requireDelay = value => { if (value?.count > 0) observedDelays.push(value); };
  if (result.route === 'direct') {
    for (const endpoint of [...result.peers, ...result.clients]) {
      const counters = endpoint.state?.directImpairment || {};
      if (counters.failures || counters.overflows) throw new Error('Direct impairment queue failed');
      reliableWritten += counters.reliable?.written || 0;
      snapshotWritten += counters.snapshot?.written || 0;
      snapshotDropped += counters.snapshot?.dropped || 0;
      delayed += (counters.reliable?.delay.count || 0) + (counters.snapshot?.delay.count || 0);
      requireDelay(counters.reliable?.delay); requireDelay(counters.snapshot?.delay);
    }
  } else {
    const profile = result.impairment?.profile, counters = result.impairment?.counters;
    if (!profile || profile.delay_ms !== options.delayMs || profile.loss_percent !== options.lossPercent || profile.seed !== options.seed) throw new Error('Actual service impairment profile differs from requested profile');
    if (counters.queue_overflow_closes) throw new Error('Relay impairment queue overflowed');
    reliableWritten = counters.relay_reliable_written;
    snapshotWritten = counters.relay_snapshot_written;
    snapshotDropped = counters.relay_snapshot_dropped;
    delayed = counters.relay_frames_delayed;
    requireDelay(counters.observed_write_delay_ms?.reliable); requireDelay(counters.observed_write_delay_ms?.snapshot);
  }
  if (!reliableWritten || (options.lossPercent < 100 && !snapshotWritten)) throw new Error('No observed impaired reliable/snapshot traffic');
  if (options.lossPercent && !snapshotDropped) throw new Error('Requested snapshot loss produced no observed drop');
  if (options.delayMs && (!delayed || !observedDelays.length || observedDelays.some(value => !Number.isFinite(value.min) || value.min < options.delayMs - 1))) throw new Error('Requested delay lacks matching observed write delays');
  return true;
}
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
async function within(promise, milliseconds, description) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${description} exceeded ${milliseconds}ms`)), milliseconds); })]); }
  finally { clearTimeout(timer); }
}
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function stopProcess(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const closed = new Promise(resolve => child.once('close', resolve));
  child.kill();
  try { await within(closed, 3000, 'child shutdown'); }
  catch {
    if (process.platform === 'win32') execFileSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, timeout: 5000, stdio: 'ignore' });
    else child.kill('SIGKILL');
    await within(closed, 3000, 'forced child shutdown');
  }
}

// Observation only: every constructor and channel remains the browser's own.
// Store public message types, command outcomes and RTC counts, never tokens.
export function observeBrowser({ render, directProfile }, impairChannel) {
  if (render) Object.defineProperty(navigator, 'webdriver', { get: () => false });
  window.addEventListener('PhoenixReady', () => { window.__matrixPhoenixReady = true; });
  const evidence = window.__matrixEvidence = { counts: {}, outcomes: [], relayReady: 0, relayFrames: 0, rtcCreated: 0, rtcOffers: 0, signalOffersSent: 0, directImpairment: {} };
  const peers = [];
  function message(raw) {
    try {
      const value = JSON.parse(raw);
      if (value.type === 'relay-ready') evidence.relayReady++;
      if (value.type === 'relay' && typeof value.payload === 'string') { evidence.relayFrames++; return message(value.payload); }
      if (typeof value.type === 'string') evidence.counts[value.type] = (evidence.counts[value.type] || 0) + 1;
      if (['ActionFeedback'].includes(value.type)) {
        evidence.outcomes.push(value.data); evidence.outcomes = evidence.outcomes.slice(-64);
      }
    } catch { /* Binary/non-game traffic is not interpreted as JSON. */ }
  }
  const NativeSocket = window.WebSocket;
  window.WebSocket = new Proxy(NativeSocket, { construct(target, args) {
    const socket = new target(...args);
    socket.addEventListener('message', event => message(event.data));
    const send = socket.send.bind(socket);
    socket.send = raw => {
      try { const frame = JSON.parse(raw); if (frame.type === 'signal' && frame.payload?.sdp?.type === 'offer') evidence.signalOffersSent++; } catch { /* not a signalling frame */ }
      return send(raw);
    };
    return socket;
  } });
  const NativePeer = window.RTCPeerConnection;
  window.RTCPeerConnection = new Proxy(NativePeer, { construct(target, args) {
    const peer = new target(...args); peers.push(peer); evidence.rtcCreated++;
    const offer = peer.createOffer.bind(peer);
    peer.createOffer = (...values) => { evidence.rtcOffers++; return offer(...values); };
    const listen = channel => {
      channel.addEventListener('message', event => message(event.data));
      if (directProfile) impairChannel(channel, directProfile, evidence.directImpairment);
    };
    peer.addEventListener('datachannel', event => listen(event.channel));
    const create = peer.createDataChannel.bind(peer);
    peer.createDataChannel = (...values) => { const channel = create(...values); listen(channel); return channel; };
    return peer;
  } });
  window.__matrixRead = async () => {
    const rtc = [];
    for (const peer of peers) {
      const selected = [];
      const stats = await peer.getStats();
      for (const record of stats.values()) if (record.type === 'transport' && record.selectedCandidatePairId) {
        const pair = stats.get(record.selectedCandidatePairId);
        selected.push({ state: pair?.state, bytesSent: pair?.bytesSent, bytesReceived: pair?.bytesReceived,
          localType: stats.get(pair?.localCandidateId)?.candidateType,
          remoteType: stats.get(pair?.remoteCandidateId)?.candidateType });
      }
      rtc.push({ connectionState: peer.connectionState, selected });
    }
    return { ...evidence, rtc, mesh: window.__hostMeshStatus?.(), fleet: window.__hostFleetState?.(),
      gm: window.__hostGmStartState?.(), gmSession: window.__hostGmSessionState?.(), phase: window.__saveSlotsPhase || window.lobbyState?.phase,
      station: window.lobbyState?.players?.filter(player => player.connected).map(player => ({ name: player.name, station: player.station, ready: player.ready })),
      alert: window.simState?.redAlert };
  };
}

export async function bundleHashes(directory) {
  const hashes = {};
  async function walk(relative, ancestors = new Set()) {
    const canonical = await realpath(path.join(directory, relative));
    if (ancestors.has(canonical)) throw new Error('Cyclic bundle directory link');
    const visited = new Set(ancestors).add(canonical);
    for (const entry of await readdir(path.join(directory, relative), { withFileTypes: true })) {
      const name = path.join(relative, entry.name);
      if (entry.isDirectory() || (entry.isSymbolicLink() && (await stat(path.join(directory, name))).isDirectory())) await walk(name, visited);
      else hashes[name.replaceAll('\\', '/')] = sha(await readFile(path.join(directory, name)));
    }
  }
  await walk(''); return hashes;
}
async function serve(directory, port) {
  const mime = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json', '.css': 'text/css', '.toml': 'text/plain' };
  const server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
      let file = path.resolve(directory, `.${pathname}`);
      if (!file.startsWith(directory + path.sep) && file !== directory) { response.writeHead(403).end(); return; }
      if ((await stat(file)).isDirectory()) file = path.join(file, 'index.html');
      response.setHeader('Content-Type', mime[path.extname(file)] || 'application/octet-stream');
      response.end(await readFile(file));
    } catch { response.writeHead(404).end(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  return server;
}
async function waitService(base, child) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (child.exitCode !== null) throw new Error(`Rendezvous exited ${child.exitCode}`);
    try { if ((await fetch(base + '/v1/health', { signal: AbortSignal.timeout(2000) })).ok) return; } catch { /* startup */ }
    await sleep(100);
  }
  throw new Error('Rendezvous did not become ready');
}
async function runCase(browser, options, route) {
  const directory = path.join(options.out, route); await mkdir(directory);
  const result = { route, profile: { delayMs: options.delayMs, lossPercent: options.lossPercent, seed: options.seed, boundary: route === 'direct' ? 'actual RTCDataChannel.send' : 'loopback rendezvous outgoing relay frames', unit: 'application frames; not IP packets' }, startedUtc: new Date().toISOString(), status: 'running', peers: [], clients: [], steps: [] };
  const contexts = [], pages = [];
  const base = `http://127.0.0.1:${options.port}`;
  const rendezvous = `http://127.0.0.1:${options.rendezvousPort}`;
  const args = [options['service-script'] || path.join(root, 'scripts/rendezvous-dev-server.mjs'), '--port', String(options.rendezvousPort)];
  if (route === 'automatic-fallback') args.push('--block-rtc-offers');
  if (route !== 'direct' && (options.delayMs || options.lossPercent)) args.push('--delay-ms', String(options.delayMs), '--loss-percent', String(options.lossPercent), '--seed', String(options.seed));
  result.serviceArgs = args;
  result.serviceSha256 = sha(await readFile(args[0]));
  const service = spawn(process.execPath, args, { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let serviceLog = ''; const appendLog = bytes => { serviceLog = (serviceLog + bytes).slice(-100000); };
  service.stdout.on('data', appendLog); service.stderr.on('data', appendLog);
  service.on('error', error => { serviceLog += String(error); });
  const step = name => { result.steps.push({ name, utc: new Date().toISOString() }); process.stderr.write(`${route}: ${name}\n`); };
  const query = new URLSearchParams({ rendezvous, ...(route === 'automatic-fallback' ? {} : { transport: route }) });
  async function newPage(label) {
    const context = await browser.newContext(); contexts.push(context);
    const instrumentation = { render: options.render, directProfile: route === 'direct' && (options.delayMs || options.lossPercent) ? { delayMs: options.delayMs, lossPercent: options.lossPercent, seed: options.seed } : null };
    await context.addInitScript({ content: `(${observeBrowser.toString()})(${JSON.stringify(instrumentation)}, ${impairDataChannel.toString()});` });
    const page = await context.newPage(); page.setDefaultTimeout(options.timeout * 1000);
    const evaluate = page.evaluate.bind(page);
    page.evaluate = (...args) => within(evaluate(...args), options.timeout * 1000, label + ' evaluation');
    const row = { label, page, errors: [], logs: [] }; pages.push(row);
    page.on('pageerror', error => { row.errors.push(String(error).slice(0, 2000)); if (row.errors.length > 100) row.errors.shift(); });
    page.on('console', message => {
      if (['warning', 'error'].includes(message.type())) row.logs.push(message.text().slice(0, 2000));
      if (row.logs.length > 100) row.logs.shift();
    });
    return page;
  }
  try {
    await waitService(rendezvous, service); step('service ready');
    if (route === 'automatic-fallback' || (route !== 'direct' && (options.delayMs || options.lossPercent))) {
      const response = await fetch(`${rendezvous}/__impairment`, { signal: AbortSignal.timeout(5000) });
      if (!response.ok) throw new Error('Rendezvous lacks impairment support');
      result.requestedProfile = await response.json();
    }
    const ships = [], gms = [], clients = [];
    let code;
    async function bootShip(index) {
      const page = await newPage(`ship-${index + 1}`); ships[index] = page;
      await page.goto(`${base}/?${query}&scenario=assets/worlds/probe_fleet_six_peer.toml&ship=assets/entities/alliance_cruiser.toml`);
      await page.waitForFunction(() => window.__matrixPhoenixReady === true && window.__matrixEvidence.counts.hosted > 0);
      if (!index) {
        await page.evaluate(() => window.__hostFleetOpen());
        await page.waitForFunction(() => window.__hostFleetState?.().suffix);
        code = await page.evaluate(() => window.__hostFleetState().suffix);
      } else {
        await page.evaluate(value => window.__hostFleetJoin(value), code);
        await page.waitForFunction(() => window.__hostFleetState?.().open && document.querySelectorAll('#fleet-slots li').length > 1);
      }
      step(`ship ${index + 1} joined`);
    }
    await bootShip(0);
    await Promise.all([1, 2, 3].map(bootShip));
    await Promise.all([0, 1].map(async index => {
      const page = await newPage(`gm-${index + 1}`); gms[index] = page;
      await page.goto(`${base}/?${query}&gm=1&scenario=assets/worlds/probe_fleet_six_peer.toml`);
      await page.waitForFunction(() => window.__matrixPhoenixReady === true && window.__matrixEvidence.counts.hosted > 0);
      await page.evaluate(value => window.__hostFleetJoin(value), code);
      await page.waitForFunction(() => window.__hostGmStartState?.().admitted && window.__hostGmStartState?.().localValidation);
      step(`GM ${index + 1} admitted`);
    }));
    await Promise.all(ships.map(async (ship, index) => {
      const crewCode = await ship.locator('#join-code').textContent();
      await Promise.all(['captain', 'helm', 'engineering'].map(async station => {
        const page = await newPage(`ship-${index + 1}-${station}`);
        await page.goto(`${base}/client/?${query}#${crewCode.trim()}`);
        await page.waitForFunction(() => window.phoenixLink?.connected && window.lobbyState?.players?.length > 0);
        await page.evaluate(station => window.phoenixLink.send('SelectStation', { station }, 'reliable'), station);
        await page.waitForFunction(station => window.lobbyState.players.some(player => player.station === station && player.connected), station);
        clients.push({ page, station, ship: index + 1 });
      }));
      step(`ship ${index + 1} has three client documents`);
    }));
    for (const { page } of clients) await page.evaluate(() => window.phoenixLink.send('SetReady', { ready: true }, 'reliable'));
    for (const page of gms) await page.evaluate(() => document.getElementById('gm-ready-btn').click());
    await Promise.all([...ships, ...gms].map(page => page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress' && window.__hostMeshStatus?.().in_fleet)));
    step('automatic six-peer launch');
    const initial = await Promise.all([...ships, ...gms].map(page => page.evaluate(() => window.__hostMeshStatus())));
    result.initial = initial;
    const deadline = Date.now() + options.seconds * 1000;
    let wave = 0;
    while (Date.now() < deadline) {
      for (const { page, station, ship } of clients) {
        await page.evaluate(({ station, ship, wave }) => {
          const correlation = `matrix-${ship}-${station}-${wave}`;
          const action = station === 'captain' ? { action: 'set_red_alert', active: wave % 2 === 0, correlation }
            : station === 'helm' ? { action: 'set_boost', active: wave % 2 === 0, correlation }
              : { action: 'set_power', target: 'weapons', level: wave % 2 ? 2 : 3, correlation };
          const send = (type, data) => window.phoenixLink.send(type, data, 'reliable');
          window.dispatchConsoleAction(action, send);
          if (station === 'helm') window.dispatchConsoleAction({ action: 'set_helm_thrust', value: .2 + (wave % 2) * .1 }, send);
        }, { station, ship, wave });
      }
      wave++; await sleep(500);
    }
    result.commandWaves = wave;
    // Two ordinary GM session controls, with their authoritative readback.
    await gms[0].evaluate(() => document.getElementById('gm-session-pause').click());
    await Promise.all(gms.map(page => page.waitForFunction(() => window.__hostGmSessionState?.().paused)));
    await gms[1].evaluate(() => document.getElementById('gm-session-resume').click());
    await Promise.all(gms.map(page => page.waitForFunction(() => !window.__hostGmSessionState?.().paused)));
    result.gmActions = ['gm-1 pause observed by both GMs', 'gm-2 resume observed by both GMs'];
    await Promise.all([...ships, ...gms].map((page, index) => page.waitForFunction(tick => window.__hostMeshStatus().tick > tick + 30 && window.__hostMeshStatus().samples > 0 && window.__hostMeshStatus().peers_heard.length === 5, initial[index].tick)));
    step('active command interval and GM actions completed');
    result.status = 'passed';
  } catch (error) { result.status = 'failed'; result.error = String(error.stack || error); }
  finally {
    for (const row of pages) {
      const snapshot = { label: row.label, errors: [...row.errors], logs: [...row.logs] };
      try { snapshot.state = await row.page.evaluate(() => window.__matrixRead()); } catch (error) { snapshot.readError = String(error); }
      if (row.label.match(/^ship-\d$|^gm-/)) result.peers.push(snapshot); else result.clients.push(snapshot);
      if (result.status !== 'passed') {
        try { await row.page.screenshot({ path: path.join(directory, `${row.label}.png`), timeout: 5000 }); } catch { /* crashed page */ }
      }
    }
    try { result.impairment = await (await fetch(`${rendezvous}/__impairment`, { signal: AbortSignal.timeout(5000) })).json(); } catch { result.impairment = null; }
    if (result.status === 'passed') {
      try { verifyEvidence(result); verifyImpairment(result, options); }
      catch (error) {
        result.status = 'failed'; result.error = String(error);
        try { await pages[0]?.page.screenshot({ path: path.join(directory, 'evidence-gate-failure.png'), timeout: 5000 }); } catch { /* page closed */ }
      }
    }
    await Promise.allSettled(contexts.map(context => within(context.close(), 5000, 'context shutdown')));
    try { await stopProcess(service); } catch (error) { result.status = 'failed'; result.cleanupError = String(error); }
    await writeFile(path.join(directory, 'service.log'), serviceLog);
    result.finishedUtc = new Date().toISOString();
    await writeFile(path.join(directory, 'result.json'), JSON.stringify(result, null, 2));
  }
  return result;
}

export async function main(args = process.argv.slice(2)) {
  const options = optionsFrom(args);
  await mkdir(path.dirname(options.out), { recursive: true }); await mkdir(options.out);
  await stat(path.join(options.dist, 'client/index.html')).catch(() => { throw new Error('Missing built client/index.html; run node scripts/build-client.mjs after Trunk'); });
  const require = createRequire(path.join(options.dependencies, 'package.json'));
  const { chromium } = require('@playwright/test');
  const git = args => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
  const manifest = { status: 'running', kind: 'phoenix-real-browser-matrix-v1', options, revision: git(['rev-parse', 'HEAD']),
    sourcePatch: git(['diff', 'HEAD']), bundleHashes: await bundleHashes(options.dist),
    runnerSha256: sha(await readFile(fileURLToPath(import.meta.url))), channelImpairmentSha256: sha(await readFile(path.join(root, 'scripts/fleet-channel-impairment.mjs'))), node: process.version, os: { platform: platform(), release: release(), cpus: cpus()[0]?.model, ramBytes: totalmem() },
    composition: { shipSimulations: 4, gmSimulations: 2, stationDocuments: 12 },
    limitations: ['Single-machine loopback; no mobile/internet evidence', 'No native peers',
      options.render ? 'Software-rendered browser viewscreens' : 'Real WASM simulation; webdriver disables Bevy rendering',
      'Short bounded check, not endurance or performance acceptance'], results: [] };
  if (options['wasm-build-receipt']) manifest.wasmBuildReceipt = await readVerifiedWasmReceipt(options['wasm-build-receipt'], root, manifest.bundleHashes);
  const save = () => writeFile(path.join(options.out, 'manifest.json'), JSON.stringify(manifest, null, 2));
  await save();
  let server, browser, browserServer;
  try {
    server = await serve(options.dist, options.port);
    browserServer = await chromium.launchServer({ headless: true, args: ['--autoplay-policy=no-user-gesture-required', '--disable-background-timer-throttling', '--disable-renderer-backgrounding',
      ...(options.render ? ['--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'] : [])] });
    browser = await chromium.connect(browserServer.wsEndpoint(), { timeout: 30000 });
    manifest.browser = browser.version(); await save();
    for (const route of options.routes) { const result = await runCase(browser, options, route); manifest.results.push({ route, status: result.status, error: result.error }); await save(); }
    manifest.status = manifest.results.every(result => result.status === 'passed') ? 'passed' : 'failed';
    if (manifest.status === 'failed') process.exitCode = 1;
  } catch (error) { manifest.status = 'failed'; manifest.error = String(error); throw error; }
  finally {
    const failures = [];
    for (const close of [() => browser?.close(), () => browserServer?.close()]) {
      try { await within(Promise.resolve(close()), 5000, 'browser shutdown'); } catch (error) { failures.push(String(error)); }
    }
    try { await stopProcess(browserServer?.process()); } catch (error) { failures.push(String(error)); }
    if (server) {
      server.closeAllConnections();
      try { await within(new Promise(resolve => server.close(resolve)), 3000, 'HTTP shutdown'); } catch (error) { failures.push(String(error)); }
    }
    if (failures.length) { manifest.status = 'failed'; manifest.cleanupErrors = failures; process.exitCode = 1; }
    await save();
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) main().then(() => process.exit(process.exitCode || 0), error => { console.error(error); process.exit(1); });
