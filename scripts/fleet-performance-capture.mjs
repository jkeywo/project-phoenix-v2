#!/usr/bin/env node
// Bounded six-peer browser workload capture. Uses the real #1530 matrix and
// #1534 fault gate; no synthetic protocol echo or timing threshold is passed.
import { readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { main as browserMatrix } from './fleet-browser-matrix.mjs';
import { failureHook } from './fleet-browser-recovery.mjs';
import { installBrowserPerformanceObserver, installHostPerformanceObserver } from './fleet-performance-observer.mjs';
import { FORMAT, summarize } from './fleet-performance-evidence.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
export function captureOptions(args) {
  const matrix = [], own = { measureSeconds: 10, faultSeconds: 60 };
  for (let i = 0; i < args.length; i++) {
    const key = args[i];
    if (key === '--measure-seconds' || key === '--fault-seconds') {
      const value = Number(args[++i]);
      const max = key === '--measure-seconds' ? 120 : 180;
      if (!Number.isInteger(value) || value < 1 || value > max) throw new Error(`Invalid ${key}`);
      own[key === '--measure-seconds' ? 'measureSeconds' : 'faultSeconds'] = value;
    } else matrix.push(key);
  }
  if (!matrix.includes('--wasm-build-receipt')) {
    throw new Error('A verified --wasm-build-receipt is required for source-matched capture');
  }
  return { matrix, ...own };
}
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export function verifyObservedRecoveryRows(peer, rows, faultId) {
  for (const kind of ['fault', 'loss_detected', 'progress_resumed', 'digest_verified']) {
    if (rows.filter(row => row.kind === kind && row.correlation === faultId).length !== 1) {
      throw new Error(`${peer}: missing or repeated observed ${kind}`);
    }
  }
  if (rows.filter(row => row.kind === 'tick').length < 2) {
    throw new Error(`${peer}: no measured host tick interval`);
  }
  return true;
}
export async function captureHook({ result, ships, gms, clients, step, options }, measureSeconds, faultSeconds) {
  const route = result.route;
  const peers = [...ships.map((page, index) => ({ page, label: `ship-${index + 1}` })),
    ...gms.map((page, index) => ({ page, label: `gm-${index + 1}` }))];
  const runId = `${route}-${Date.now()}`;
  for (const peer of peers) await peer.page.evaluate(installHostPerformanceObserver,
    { peer: peer.label, clock: `${runId}/${peer.label}` });
  for (const client of clients) {
    client.label = `ship-${client.ship}/${client.station}`;
    await client.page.evaluate(installBrowserPerformanceObserver,
      { peer: client.label, clock: `${runId}/${client.label}` });
  }
  const started = performance.now();
  let wave = 0;
  while (performance.now() - started < measureSeconds * 1000) {
    for (const client of clients) await client.page.evaluate(({ route, ship, station, wave }) => {
      const correlation = `performance-${route}-${ship}-${station}-${wave}`;
      const action = station === 'captain' ? { action: 'set_red_alert', active: wave % 2 === 0, correlation }
        : station === 'helm' ? { action: 'set_boost', active: wave % 2 === 0, correlation }
          : { action: 'set_power', target: 'weapons', level: wave % 2 ? 2 : 3, correlation };
      window.__fleetPerformance.input(correlation);
      window.dispatchConsoleAction(action, (type, data) => window.phoenixLink.send(type, data, 'reliable'));
    }, { route, ship: client.ship, station: client.station, wave });
    wave++;
    await sleep(500);
  }
  if (!wave) throw new Error('No measured command wave');
  for (const client of clients) await client.page.waitForFunction(expected =>
    window.__fleetPerformance.read().filter(row => row.kind === 'applied_receipt'
      && row.correlation.startsWith('performance-')).length >= expected, wave,
  { timeout: 15000 });
  step(`${wave} measured command waves received on twelve real clients`);

  // One real ship departure under the existing recovery gate. Mark the fault
  // on each survivor clock before closing the victim; this is a conservative
  // observation boundary that includes Playwright close latency.
  const victim = 'ship-2';
  const survivors = peers.filter(peer => peer.label !== victim);
  const victimSlot = await ships[1].evaluate(() => window.__hostMeshStatus().slot);
  const afterTick = Math.max(...await Promise.all(survivors.map(peer =>
    peer.page.evaluate(() => window.__hostMeshStatus().tick))));
  const faultId = `performance-${route}-ship-loss`;
  for (const peer of survivors) await peer.page.evaluate(({ id, slot, tick }) =>
    window.__fleetHostPerformance.fault(id, slot, tick),
  { id: faultId, slot: victimSlot, tick: afterTick });
  await failureHook('ship', { faultSeconds })({ result, ships, gms, step });
  for (const peer of survivors) await peer.page.evaluate(id =>
    window.__fleetHostPerformance.verified(id), faultId);
  const events = [];
  for (const client of clients) events.push(...await client.page.evaluate(() => window.__fleetPerformance.read()));
  for (const peer of survivors) {
    const rows = await peer.page.evaluate(() => {
      window.__fleetHostPerformance.stop();
      return window.__fleetHostPerformance.read();
    });
    verifyObservedRecoveryRows(peer.label, rows, faultId);
    events.push(...rows);
  }
  const measured = { format: FORMAT, expectedTickMs: 1000 / 60, events,
    workload: { shipSimulations: 4, gmSimulations: 2, stationDocuments: 12,
      gmConsoles: 2, commandWaves: wave, fault: victim,
      browserDocuments: 18, measuredCommandSeconds: measureSeconds },
    recoveryGate: result.recovery?.outcome,
    limitations: ['Browser-only six-peer loopback; native and mixed cells remain unmeasured',
      'Applied receipt includes return delivery; authoritative application clock is unavailable',
      'Fault start is marked before confirmed close; recovery intervals are conservative',
      'One bounded ship departure, not one-hour or mobile-network evidence'] };
  result.performance = { eventCount: events.length, commandWaves: wave, fault: victim };
  return measured;
}

export async function main(args = process.argv.slice(2), runMatrix = browserMatrix) {
  const { matrix, measureSeconds, faultSeconds } = captureOptions(args);
  if (!matrix.includes('--seconds')) matrix.push('--seconds', '1');
  if (!matrix.includes('--routes')) matrix.push('--routes', 'direct');
  const outIndex = matrix.indexOf('--out');
  if (outIndex < 0 || !matrix[outIndex + 1]) throw new Error('Choose --out <fresh directory>');
  const output = path.resolve(matrix[outIndex + 1]);
  if (output === root || output.startsWith(root + path.sep)) {
    throw new Error('Choose an output outside the source checkout so the WASM receipt stays valid');
  }
  const captures = new Map();
  await runMatrix(matrix, {
    kind: 'phoenix-t5-browser-performance-capture-v1',
    provenance: { measureSeconds, faultSeconds },
    afterHealthy: async context => {
      const captured = await captureHook(context, measureSeconds, faultSeconds);
      captures.set(context.result.route, captured);
    },
  });
  // The shared matrix writes its manifest as the evidence artifact and does
  // not return it. Read the final record so a failed matrix remains the
  // reported failure instead of becoming an unrelated wrapper TypeError.
  const manifest = JSON.parse(await readFile(path.join(output, 'manifest.json'), 'utf8'));
  const runnerHash = sha(await readFile(fileURLToPath(import.meta.url)));
  const observerHash = sha(await readFile(path.join(root, 'scripts/fleet-performance-observer.mjs')));
  const reducerHash = sha(await readFile(path.join(root, 'scripts/fleet-performance-evidence.mjs')));
  const worldHash = sha(await readFile(path.join(root, 'assets/worlds/probe_fleet_six_peer.toml')));
  for (const [route, captured] of captures) {
    const result = JSON.parse(await readFile(path.join(output, route, 'result.json'), 'utf8'));
    const trace = { ...captured, provenance: {
      revision: manifest.revision, trackedPatch: manifest.sourcePatch,
      content: 'phoenix-base@1:probe_fleet_six_peer',
      runtime: { node: manifest.node, browser: manifest.browser, os: manifest.os },
      artifactHashes: { runner: runnerHash, observer: observerHash, reducer: reducerHash,
        matrix: manifest.runnerSha256, world: worldHash,
        browserBundle: sha(JSON.stringify(manifest.bundleHashes)) },
      profile: { route, boundary: result.profile?.boundary, delayMs: result.profile?.delayMs,
        lossPercent: result.profile?.lossPercent, seed: result.profile?.seed,
        impairment: result.impairment, host: manifest.os,
        shaping: 'matrix application-frame impairment, not IP packet loss' },
      wasmBuildReceipt: manifest.wasmBuildReceipt,
    } };
    const directory = path.join(output, route);
    await writeFile(path.join(directory, 'performance-trace.json'), JSON.stringify(trace, null, 2));
    try {
      await writeFile(path.join(directory, 'performance-summary.json'),
        JSON.stringify(summarize(trace), null, 2));
    } catch (error) {
      await writeFile(path.join(directory, 'performance-reduction-error.txt'), String(error));
      throw error;
    }
  }
  return manifest;
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().then(result => { if (result.status !== 'passed') process.exitCode = 1; }, error => {
    console.error(error); process.exitCode = 1;
  });
}
