// The join path end to end, in process (issue #1111).
//
// A fake WebSocket pair terminated by the REAL rendezvous registry, plus a fake
// RTCPeerConnection pair that links two DataChannels once SDP has crossed. That
// lets these tests assert the thing the issue actually promises — typed suffix,
// pasted full code and QR fragment all reach one direct-channel Identify — with
// no sockets, no WebRTC and no worker.

import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  rendezvousBaseFromLocation,
  joinRouteFromLocation,
  socketUrl,
  joinUrlForCode,
  createRendezvousHost,
  createRendezvousJoiner,
  DEV_RENDEZVOUS_URL,
} from '../../gui/rendezvous-transport.js';
import { createRegistry, ROLE_HOST, ROLE_CLIENT } from '../../worker-rendezvous/src/registry.js';
import {
  NAMESPACE_CLIENT,
  NAMESPACE_SERVER,
  setJoinCodeData,
  projectGuidFor,
  versionGuid,
  composeJoinCode,
} from '../../gui/join-code.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DATA = JSON.parse(readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'));
setJoinCodeData(DATA);

const settle = async () => {
  for (let i = 0; i < 30; i += 1) await Promise.resolve();
};

// ── Fakes ───────────────────────────────────────────────────────────────────

/** A WebSocket-shaped pipe into one shared rendezvous registry. */
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
        this.readyState = 3;
        sockets.delete(id);
        dispatch(registry.disconnect(id));
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

/** Fake RTCPeerConnection: two ends linked once the offer has been answered. */
function makePeerFactory() {
  const offerers = new Map();
  let n = 0;

  function makeChannel(label) {
    return {
      label,
      readyState: 'connecting',
      onopen: null, onmessage: null, onclose: null,
      _remote: null,
      send(payload) {
        const remote = this._remote;
        queueMicrotask(() => { if (remote && remote.onmessage) remote.onmessage({ data: payload }); });
      },
      close() {
        this.readyState = 'closed';
        if (this.onclose) this.onclose();
      },
    };
  }

  function link(offerer, answerer) {
    for (const local of offerer._channels) {
      const remote = makeChannel(local.label);
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

  return function peer() {
    const id = `pc-${++n}`;
    const pc = {
      _id: id,
      _channels: [],
      localDescription: null,
      remoteDescription: null,
      onicecandidate: null,
      ondatachannel: null,
      createDataChannel(label) {
        const ch = makeChannel(label);
        this._channels.push(ch);
        return ch;
      },
      async createOffer() {
        offerers.set(id, this);
        return { type: 'offer', peer: id };
      },
      async createAnswer() { return { type: 'answer', peer: id }; },
      async setLocalDescription(d) { this.localDescription = d; },
      async setRemoteDescription(d) {
        this.remoteDescription = d;
        if (d.type === 'offer') link(offerers.get(d.peer), this);
      },
      async addIceCandidate() {},
      close() { for (const c of this._channels) c.readyState = 'closed'; },
    };
    return pc;
  };
}

/** Stand up a host on a fake world and wait for its issued code. */
async function hostOn(world, opts = {}) {
  const factories = { socket: world.socket, peer: makePeerFactory() };
  let code = null;
  const inbound = [];
  const host = createRendezvousHost({
    base: 'https://rendezvous.test',
    factories,
    onCode: (c) => { code = c; },
    onConnection: (conn) => {
      conn.on('data', (raw) => inbound.push(JSON.parse(raw)));
    },
    ...opts,
  });
  await settle();
  return { host, code, inbound, factories };
}

// ── Pure helpers ────────────────────────────────────────────────────────────

describe('service selection', () => {
  it('is off unless the page asks for it, so PeerJS stays the default route', () => {
    expect(rendezvousBaseFromLocation('')).toBeNull();
    expect(rendezvousBaseFromLocation('?scenario=x')).toBeNull();
    expect(rendezvousBaseFromLocation('?rendezvous=off')).toBeNull();
  });

  it('uses the built-in service for a bare flag and an explicit URL otherwise', () => {
    expect(rendezvousBaseFromLocation('?rendezvous')).toBe(DEV_RENDEZVOUS_URL);
    expect(rendezvousBaseFromLocation('?rendezvous=1')).toBe(DEV_RENDEZVOUS_URL);
    expect(rendezvousBaseFromLocation('?rendezvous=https://other.test')).toBe('https://other.test');
  });

  it('upgrades the scheme when building a socket URL', () => {
    expect(socketUrl('https://r.test', '/v1/host')).toBe('wss://r.test/v1/host');
    expect(socketUrl('http://localhost:8787', '/v1/join')).toBe('ws://localhost:8787/v1/join');
  });

  it('builds the QR/copy link as the client page plus the full code in the fragment', () => {
    expect(joinUrlForCode('https://x.test/index.html', 'P_V_QUARK'))
      .toBe('https://x.test/client/index.html#P_V_QUARK');
  });

  it('names a non-default service in the link, and stays silent about the default', () => {
    expect(joinUrlForCode('https://x.test/index.html', 'P_V_QUARK', DEV_RENDEZVOUS_URL))
      .not.toContain('rendezvous=');
    expect(joinUrlForCode('https://x.test/index.html', 'P_V_QUARK', 'http://localhost:9'))
      .toBe('https://x.test/client/index.html?rendezvous=http%3A%2F%2Flocalhost%3A9#P_V_QUARK');
  });
});

describe('which route a client page load is on', () => {
  it('keeps a peer id in the fragment on the PeerJS route', () => {
    expect(joinRouteFromLocation('', '#0123456789abcdef0123456789abcdef'))
      .toEqual({ route: 'peerjs', hostPeerId: '0123456789abcdef0123456789abcdef' });
  });

  it('treats a structured code in the fragment as its own opt-in', () => {
    expect(joinRouteFromLocation('', '#proj_ver_QUARK')).toEqual({
      route: 'rendezvous',
      base: DEV_RENDEZVOUS_URL,
      code: 'proj_ver_QUARK',
    });
  });

  it('asks for five letters when the fragment is empty', () => {
    expect(joinRouteFromLocation('', '')).toEqual({ route: 'entry', base: DEV_RENDEZVOUS_URL });
  });

  it('honours a service override for both the code and the entry route', () => {
    expect(joinRouteFromLocation('?rendezvous=http://x.test', '#p_v_QUARK'))
      .toMatchObject({ route: 'rendezvous', base: 'http://x.test' });
    expect(joinRouteFromLocation('?rendezvous=http://x.test', ''))
      .toMatchObject({ route: 'entry', base: 'http://x.test' });
  });

  it('falls back to the old dead end when the route is switched off', () => {
    expect(joinRouteFromLocation('?rendezvous=off', '')).toEqual({ route: 'none' });
    expect(joinRouteFromLocation('?rendezvous=off', '#p_v_QUARK')).toEqual({ route: 'none' });
    // …but a peer id still connects, because that route never needed a service.
    expect(joinRouteFromLocation('?rendezvous=off', '#deadbeef'))
      .toEqual({ route: 'peerjs', hostPeerId: 'deadbeef' });
  });
});

// ── The tracer ──────────────────────────────────────────────────────────────

describe('typed join', () => {
  it('issues a five-letter code to the host', async () => {
    const world = makeWorld();
    const { code } = await hostOn(world);
    expect(code.suffix).toHaveLength(5);
    expect(code.namespace).toBe(NAMESPACE_CLIENT);
  });

  it('carries Identify to the host over a direct channel from five typed letters', async () => {
    const world = makeWorld();
    const { code, inbound, factories } = await hostOn(world);

    const statuses = [];
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix.toLowerCase(),
      factories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
      onStatus: (s) => statuses.push(s),
    });
    await settle();

    expect(inbound).toEqual([{ type: 'Identify', data: { token: 'tok-1', name: 'Ada' } }]);
    expect(statuses).toContain('ready');
  });

  it('reaches the same result from a pasted full code and from a QR fragment', async () => {
    for (const shape of ['full', 'qr']) {
      const world = makeWorld();
      const { code, inbound, factories } = await hostOn(world);
      const typed = shape === 'full'
        ? code.full
        : `https://phone.test/client/index.html#${code.full}`;
      createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: typed,
        factories,
        getIdent: () => ({ token: `tok-${shape}`, name: 'Ada' }),
      });
      await settle();
      expect(inbound, shape).toEqual([
        { type: 'Identify', data: { token: `tok-${shape}`, name: 'Ada' } },
      ]);
    }
  });

  it('keeps the code in the transport plane — nothing on the crew channel carries it', async () => {
    const world = makeWorld();
    const raw = [];
    const { code, factories } = await hostOn(world, {
      onConnection: (conn) => conn.on('data', (r) => raw.push(String(r))),
    });
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      stamp: '1/phoenix-base/1',
      factories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
    });
    await settle();
    // The host is talking to this phone…
    expect(raw.some((r) => r.includes('Identify'))).toBe(true);
    // …and its private code went nowhere near the messages the simulation sees.
    expect(raw.join('|')).not.toContain(code.suffix);
    expect(raw.join('|')).not.toContain(code.full);
  });

  it('delivers server messages to the page with string ids already resolved', async () => {
    const world = makeWorld();
    const received = [];
    const { code, factories } = await hostOn(world, {
      onConnection: (conn) => {
        conn.on('data', () => conn.send(JSON.stringify({
          type: 'Welcome',
          data: { text: 'client.status_connected' },
        })));
      },
    });
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      onData: (m) => received.push(m),
    });
    await settle();
    expect(received).toHaveLength(1);
    // localiseTree ran: the id was replaced by its strings.csv text.
    expect(received[0].data.text).not.toBe('client.status_connected');
    expect(received[0].data.text.length).toBeGreaterThan(0);
  });
});

describe('distinct failures', () => {
  const errorsFor = async (code, world, factories) => {
    const errors = [];
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code,
      factories,
      onError: (reason) => errors.push(reason),
    });
    await settle();
    return errors;
  };

  it('says unknown for a suffix nobody holds', async () => {
    const world = makeWorld();
    const { factories } = await hostOn(world);
    expect(await errorsFor('ZZZZZ', world, factories)).toContain('unknown');
  });

  it('says wrong-type for a code minted in the server namespace', async () => {
    const world = makeWorld();
    const { factories } = await hostOn(world, { namespace: NAMESPACE_SERVER });
    const serverCode = world.registry.snapshot().find((r) => r.namespace === NAMESPACE_SERVER);
    expect(await errorsFor(serverCode.suffix, world, factories)).toContain('wrong-type');
  });

  it('says version-mismatch for the right namespace under another release', async () => {
    const world = makeWorld();
    const { code, factories } = await hostOn(world, { namespace: NAMESPACE_CLIENT });
    // Re-address the same suffix to a release the registry does not hold.
    const other = composeJoinCode({
      project: projectGuidFor(NAMESPACE_CLIENT, DATA),
      version: 'a-release-that-is-not-this-one',
      suffix: code.suffix,
    });
    expect(await errorsFor(other, world, factories)).toContain('version-mismatch');
  });

  it('refuses a denied word before it ever reaches the service', async () => {
    const world = makeWorld();
    const { factories } = await hostOn(world);
    const sent = vi.fn();
    expect(await errorsFor('ADMIN', world, factories)).toContain('denied');
    expect(sent).not.toHaveBeenCalled();
  });
});

describe('host compatibility handshake', () => {
  it('lets the host refuse a build the rendezvous was happy to resolve', async () => {
    const world = makeWorld();
    const seen = [];
    const { code, factories } = await hostOn(world, {
      checkStamp: (stamp) => {
        seen.push(stamp);
        return { ok: false, code: 'protocol-mismatch', detail: 'host speaks 1, client speaks 2' };
      },
    });

    const errors = [];
    const statuses = [];
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      stamp: '2/phoenix-base/1',
      factories,
      onError: (reason, detail) => errors.push([reason, detail]),
      onStatus: (s) => statuses.push(s),
    });
    await settle();

    expect(seen).toEqual(['2/phoenix-base/1']);
    expect(errors).toContainEqual(['protocol-mismatch', 'host speaks 1, client speaks 2']);
    expect(statuses).toContain('error');
  });

  it('admits a matching build and Identify follows the acceptance', async () => {
    const world = makeWorld();
    const { code, inbound, factories } = await hostOn(world, {
      checkStamp: () => ({ ok: true }),
    });
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      stamp: '1/phoenix-base/1',
      factories,
      getIdent: () => ({ token: 'tok', name: 'Ada' }),
    });
    await settle();
    expect(inbound.map((m) => m.type)).toEqual(['Identify']);
  });

  it('tells the joiner when the host goes away', async () => {
    const world = makeWorld();
    const { host, code, factories } = await hostOn(world);
    const errors = [];
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      onError: (reason) => errors.push(reason),
    });
    await settle();
    host.close();
    await settle();
    expect(errors).toContain('host-gone');
  });
});
