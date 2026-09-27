// Two ship hosts assembling a fleet, in process (issue #1114).
//
// The same shape as tests/client/rendezvous-transport.test.js's world: a fake
// WebSocket pair terminated by the REAL rendezvous registry, plus a fake
// RTCPeerConnection pair that links two DataChannels once SDP has crossed. So
// what is under test here is the whole path a second host actually takes —
// typed fleet code, service lookup, offer, compatibility handshake, host-mesh
// hello — with no sockets, no WebRTC and no worker.


import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { createFleetOwner, createFleetMember } from '../../gui/fleet-session.js';
import { createRegistry, ROLE_HOST, ROLE_CLIENT } from '../../worker-rendezvous/src/registry.js';
import {
  ADMISSION_CLOSED,
  ADMISSION_OPEN,
  HOST_FRAME_HOST_LOSS,
  HOST_FRAME_TICK,
  HOST_ROLE_GM,
  HOST_ROLE_SHIP,
  HOST_ROLE_SHIP_GM,
  asHostFrame,
  encodeHostFrame,
  helloFrame,
  hostFrame,
  simulationFrame,
  startForceFrame,
} from '../../gui/host-mesh.js';
import {
  NAMESPACE_CLIENT,
  NAMESPACE_SERVER,
  setJoinCodeData,
  projectGuidFor,
  versionGuid,
  composeJoinCode,
} from '../../gui/join-code.js';
import {
  RELIABLE_CHANNEL,
  createRendezvousHost,
  createRendezvousJoiner,
} from '../../gui/rendezvous-transport.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DATA = JSON.parse(readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'));
setJoinCodeData(DATA);

const MAX_SLOTS = DATA.limits.max_fleet_hosts;
const STAMP = '1/phoenix-base/1';
const settle = async () => {
  for (let i = 0; i < 60; i += 1) await Promise.resolve();
};

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((accepted, refused) => {
    resolve = accepted;
    reject = refused;
  });
  return { promise, resolve, reject };
}

// ── Fakes (same construction as the transport suite's) ──────────────────────

function makeWorld() {
  const registry = createRegistry({ data: DATA });
  const sockets = new Map();
  let n = 0;
  const dispatch = (frames) => {
    for (const { to, frame } of frames) {
      const ws = sockets.get(to);
      if (ws && ws.onmessage) ws.onmessage({ data: JSON.stringify(frame) });
    }
  };
  function socket(url) {
    const id = `conn-${++n}`;
    const role = url.endsWith('/v1/host') ? ROLE_HOST : ROLE_CLIENT;
    const ws = {
      readyState: 1,
      onopen: null, onmessage: null, onerror: null, onclose: null,
      send(text) { dispatch(registry.receive(id, JSON.parse(text))); },
      close() {
        if (this.readyState === 3) return;
        this.readyState = 3;
        sockets.delete(id);
        dispatch(registry.disconnect(id));
        if (this.onclose) this.onclose();
      },
    };
    sockets.set(id, ws);
    queueMicrotask(() => {
      if (ws.onopen) ws.onopen();
      dispatch(registry.connect(id, role));
    });
    return ws;
  }
  return { registry, socket };
}

function makePeerFactory() {
  const offerers = new Map();
  const channels = [];
  let n = 0;
  function makeChannel(label, init, origin) {
    const ch = {
      label,
      init: init || {},
      origin,
      sent: [],
      readyState: 'connecting',
      onopen: null, onmessage: null, onclose: null, onerror: null,
      _remote: null,
      send(payload) {
        this.sent.push(payload);
        const remote = this._remote;
        queueMicrotask(() => { if (remote && remote.onmessage) remote.onmessage({ data: payload }); });
      },
      close() {
        if (this.readyState === 'closed') return;
        this.readyState = 'closed';
        if (this.onclose) this.onclose();
        const remote = this._remote;
        if (remote && remote.readyState !== 'closed') queueMicrotask(() => remote.close());
      },
    };
    channels.push(ch);
    return ch;
  }
  function link(offerer, answerer) {
    for (const local of offerer._channels) {
      const remote = makeChannel(local.label, local.init, 'answer');
      local._remote = remote;
      remote._remote = local;
      answerer._channels.push(remote);
      queueMicrotask(() => {
        local.readyState = 'open';
        remote.readyState = 'open';
        if (answerer.ondatachannel) answerer.ondatachannel({ channel: remote });
        if (remote.onopen) remote.onopen();
        if (local.onopen) local.onopen();
      });
    }
  }
  const factory = function peer() {
    const id = `pc-${++n}`;
    return {
      _id: id,
      _channels: [],
      localDescription: null,
      remoteDescription: null,
      onicecandidate: null,
      oniceconnectionstatechange: null,
      iceConnectionState: 'checking',
      ondatachannel: null,
      createDataChannel(label, init) {
        const ch = makeChannel(label, init, 'offer');
        this._channels.push(ch);
        return ch;
      },
      async createOffer() { offerers.set(id, this); return { type: 'offer', peer: id }; },
      async createAnswer() { return { type: 'answer', peer: id }; },
      async setLocalDescription(d) { this.localDescription = d; },
      async setRemoteDescription(d) {
        this.remoteDescription = d;
        if (d.type === 'offer') link(offerers.get(d.peer), this);
      },
      async addIceCandidate() {},
      close() { for (const c of this._channels) c.close(); },
    };
  };
  factory.channels = channels;
  return factory;
}

// ── Harness ─────────────────────────────────────────────────────────────────

/** A fleet lead on a fresh world, plus everything a test wants to look at. */
async function leadOn(world, factories, opts = {}) {
  const rosters = [];
  const policies = [];
  const grants = [];
  const forceResults = [];
  const simulationRosters = [];
  const simulationFrames = [];
  const events = [];
  let code = null;
  const fleet = createFleetOwner({
    base: 'https://rendezvous.test',
    factories,
    maxSlots: MAX_SLOTS,
    checkStamp: () => ({ ok: true }),
    name: 'Lead',
    onCode: (c) => { code = c; },
    onRoster: (r) => rosters.push(r),
    onSimulationRoster: (roster) => {
      simulationRosters.push(roster);
      events.push({ type: 'simulation-roster', roster });
    },
    onStartPolicy: (policy) => policies.push(policy),
    onStartGrant: (grant) => {
      grants.push(grant);
      events.push({ type: 'start-grant', grant });
    },
    onForceResult: (result) => forceResults.push(result),
    onSimulationFrame: (raw, authSlot) => {
      simulationFrames.push({ raw, authSlot });
      events.push({ type: 'simulation-frame', raw, authSlot });
    },
    ...opts,
  });
  await settle();
  return {
    fleet,
    rosters,
    policies,
    grants,
    forceResults,
    simulationRosters,
    simulationFrames,
    events,
    get code() { return code; },
  };
}

/** A second ship host typing a fleet code. */
async function memberOn(world, factories, code, opts = {}) {
  const rosters = [];
  const refusals = [];
  const notices = [];
  const statuses = [];
  const policies = [];
  const grants = [];
  const forceResults = [];
  const simulationRosters = [];
  const gmBootstraps = [];
  const simulationFrames = [];
  const events = [];
  const member = createFleetMember({
    base: 'https://rendezvous.test',
    data: DATA,
    code,
    stamp: STAMP,
    factories,
    name: 'Two',
    onRoster: (r) => rosters.push(r),
    onSimulationRoster: (roster) => {
      simulationRosters.push(roster);
      events.push({ type: 'simulation-roster', roster });
    },
    onGmJoinBootstrap: (id, roster) => {
      gmBootstraps.push({ id, roster });
      events.push({ type: 'gm-join-bootstrap', id, roster });
      return true;
    },
    onError: (reason, detail) => refusals.push({ reason, detail }),
    onRefusedSlot: (reason, detail) => notices.push({ reason, detail }),
    onStatus: (s) => statuses.push(s),
    onStartPolicy: (policy) => policies.push(policy),
    // Deliberately still supplied as a tripwire: member handles no longer
    // consume asynchronous JS start grants, so this array must stay empty.
    onStartGrant: (grant) => grants.push(grant),
    onForceResult: (result) => forceResults.push(result),
    onSimulationFrame: (raw, authSlot) => {
      simulationFrames.push({ raw, authSlot });
      events.push({ type: 'simulation-frame', raw, authSlot });
    },
    ...opts,
  });
  await settle();
  return {
    member,
    rosters,
    refusals,
    notices,
    statuses,
    policies,
    grants,
    forceResults,
    simulationRosters,
    gmBootstraps,
    simulationFrames,
    events,
  };
}

/** One world, one shared peer factory — both ends must see the same links. */
async function fleetOf(opts = {}) {
  const world = makeWorld();
  const factories = { socket: world.socket, peer: makePeerFactory() };
  const lead = await leadOn(world, factories, opts);
  return { world, factories, lead };
}

const lastRoster = (side) => side.rosters[side.rosters.length - 1];


export { DATA, MAX_SLOTS, STAMP, settle, deferred, makeWorld, makePeerFactory, leadOn, memberOn, fleetOf, lastRoster };
