#!/usr/bin/env node
// Real loopback WebSockets through the shipped registry and fleet protocol.
// No simulation, client documents, renderer or acceptance performance claim.
import { spawn, execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { createFleetOwner, createFleetMember } from '../gui/fleet-session.js';
import { setJoinCodeData } from '../gui/join-code.js';
import { hostFrame, encodeHostFrame } from '../gui/host-mesh.js';
import { transportLeversFromLocation } from '../gui/transport-levers.js';

const port = Number(process.env.PHOENIX_RELAY_PROBE_PORT || 18788);
const base = `http://127.0.0.1:${port}`;
const rounds = Number(process.env.PHOENIX_RELAY_PROBE_ROUNDS || 200);
const warmupRounds = Number(process.env.PHOENIX_RELAY_PROBE_WARMUP_ROUNDS || 20);
if (!Number.isSafeInteger(rounds) || rounds < 1 || rounds > 10_000
    || !Number.isSafeInteger(warmupRounds) || warmupRounds < 0 || warmupRounds > 10_000) {
  throw new Error('Probe round counts must be bounded nonnegative integers (measured rounds > 0)');
}
const data = JSON.parse(readFileSync('assets/join/join-codes.json', 'utf8'));
setJoinCodeData(data);
const service = spawn(process.execPath, ['scripts/rendezvous-dev-server.mjs', '--port', String(port)],
  { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
const members = [];
const pending = new Map();
const samples = [];
const routes = [];
let owner;
const wait = async predicate => {
  const until = performance.now() + 10_000;
  while (!predicate()) {
    if (service.exitCode !== null) throw new Error('Probe service exited before readiness');
    if (performance.now() > until) throw new Error('Probe deadline expired');
    await new Promise(resolve => setTimeout(resolve, 10));
  }
};
try {
  let listening = false;
  service.stdout.on('data', chunk => { if (String(chunk).includes('listening')) listening = true; });
  service.stderr.on('data', chunk => process.stderr.write(chunk));
  await wait(() => listening);
  const factories = { socket: url => new WebSocket(url),
    peer: () => { throw new Error('This probe requires relay'); } };
  const levers = transportLeversFromLocation('?transport=ws-relay');
  owner = createFleetOwner({ base, factories, levers,
    transports: ['ws-relay'], maxSlots: data.limits.max_fleet_hosts,
    ship: { template_path: 'probe-only-no-world.toml' }, checkStamp: () => ({ ok: true }),
    onSimulationFrame: raw => {
      const frame = JSON.parse(raw);
      owner.broadcast(encodeHostFrame(hostFrame('tick', { from: 1, probe_id: frame.d.probe_id })));
    },
  });
  await wait(() => owner.code);
  for (let slot = 2; slot <= 6; slot++) {
    const member = createFleetMember({ base, factories, levers, data,
      code: owner.code.suffix, stamp: 'loopback-protocol-probe',
      role: slot <= 4 ? 'ship' : 'gm',
      ship: slot <= 4 ? { template_path: 'probe-only-no-world.toml' } : null,
      onDiag: event => { if (event.event === 'transport') routes.push({ slot, transport: event.transport }); },
      onSimulationFrame: raw => {
        const id = JSON.parse(raw).d.probe_id;
        const sample = pending.get(id);
        if (sample?.slot !== slot) return;
        samples.push({ slot, round_trip_ms: performance.now() - sample.start });
        pending.delete(id);
      },
    });
    members.push(member);
    await wait(() => member.slot);
  }
  owner.freeze();
  await wait(() => members.every(member => member.roster()?.frozen));
  let measuredAt;
  for (let round = 0; round < warmupRounds + rounds; round++) {
    if (round === warmupRounds) { samples.length = 0; measuredAt = performance.now(); }
    for (let index = 0; index < members.length; index++) {
      const slot = index + 2;
      const id = `${slot}-${round}`;
      pending.set(id, { slot, start: performance.now() });
      members[index].broadcast(encodeHostFrame(hostFrame('tick', { from: slot, probe_id: id })));
      await wait(() => !pending.has(id));
    }
  }
  const sorted = samples.map(sample => sample.round_trip_ms).sort((a, b) => a - b);
  const measuredDurationMs = performance.now() - measuredAt;
  const percentile = p => sorted[Math.ceil(p * sorted.length) - 1];
  const sourcePaths = ['gui/fleet-session.js', 'gui/rendezvous-transport.js',
    'worker-rendezvous/src/registry.js', 'assets/join/join-codes.json',
    'scripts/rendezvous-dev-server.mjs', 'scripts/fleet-relay-probe.mjs'];
  const hashes = Object.fromEntries(sourcePaths.map(path =>
    [path, createHash('sha256').update(readFileSync(path)).digest('hex')]));
  process.stdout.write(JSON.stringify({
    kind: 'loopback-protocol-only', timestamp: new Date().toISOString(), runtime: process.version,
    revision: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
    dirty: execFileSync('git', ['status', '--porcelain'], { encoding: 'utf8' }).trim() !== '',
    tracked_source_patch: execFileSync('git', ['diff', '--', ...sourcePaths], { encoding: 'utf8' }),
    hashes, composition: { ship_protocol_peers: 4, gm_protocol_peers: 2,
      simulations: 0, station_client_documents: 0 },
    profile: { address: 'loopback', added_delay_ms: 0, added_loss_percent: 0 },
    routes, warmup_rounds: warmupRounds, measured_rounds: rounds, measured_duration_ms: measuredDurationMs,
    count: samples.length, p50_ms: percentile(.5), p95_ms: percentile(.95),
    p99_ms: percentile(.99), max_ms: sorted.at(-1), samples,
    limitations: 'Synthetic frames; no authoritative command application, stalls, recovery, rendering or real-network acceptance.',
  }, null, 2) + '\n');
} finally {
  for (const member of members) member.close();
  owner?.close();
  if (service.exitCode === null) {
    const stopped = once(service, 'exit');
    service.kill();
    await stopped.catch(() => {});
  }
}
