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
  JOIN_ATTEMPTS_BEFORE_ENTRY,
} from '../../gui/rendezvous-transport.js';
import { createRegistry, ROLE_HOST, ROLE_CLIENT } from '../../worker-rendezvous/src/registry.js';
import { transportLeversFromLocation } from '../../gui/transport-levers.js';
import {
  NAMESPACE_CLIENT,
  NAMESPACE_SERVER,
  setJoinCodeData,
  projectGuidFor,
  versionGuid,
  composeJoinCode,
  reasonStringId,
} from '../../gui/join-code.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DATA = JSON.parse(readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'));
setJoinCodeData(DATA);

const settle = async () => {
  // Generous, because a queued world (see makeWorld) spends one microtask per
  // frame hop rather than running a whole round trip inside one dispatch.
  for (let i = 0; i < 200; i += 1) await Promise.resolve();
};

// ── Fakes ───────────────────────────────────────────────────────────────────

/**
 * A WebSocket-shaped pipe into one shared rendezvous registry.
 *
 * `queued: true` delivers each frame in its OWN task rather than inline, which
 * is what a real ordered WebSocket does and what the default synchronous
 * dispatch cannot reproduce. Inline delivery is re-entrant: handing the joiner
 * its `joined` frame runs the joiner's whole next round trip — `relay-open` in,
 * `relay-peer` out — before the outer loop has delivered the `peer-joined` that
 * was queued ahead of it. That inversion is invisible in a fixture and
 * impossible on a socket, so the relay specs that care about frame ORDER ask
 * for a queued world. Microtasks are FIFO, so order is preserved exactly.
 */
function makeWorld({ queued = false } = {}) {
  const registry = createRegistry({ data: DATA });
  const sockets = new Map();
  let n = 0;

  const deliver = ({ to, frame }) => {
    const ws = sockets.get(to);
    if (ws && ws.onmessage) ws.onmessage({ data: JSON.stringify(frame) });
  };
  const dispatch = (frames) => {
    for (const f of frames) {
      if (queued) queueMicrotask(() => deliver(f));
      else deliver(f);
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
        // Real sockets fire this, and tests/smoke/rendezvous-shim.js does too;
        // a fake that stays silent hides a whole class of failure.
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

/**
 * Fake RTCPeerConnection: two ends linked once the offer has been answered.
 *
 * Every channel it ever builds is recorded on the returned factory's
 * `.channels`, with the label, the `createDataChannel` init bag and the
 * payloads sent on it — that is how the tests below tell "this rode the lossy
 * channel" from "this fell back to the reliable one" without reaching inside
 * the module under test. `origin` is `'offer'` for the channel the joiner
 * created and `'answer'` for the host's mirror of it.
 *
 * Closing a channel propagates to its far end, which real WebRTC does and the
 * reconnect path depends on: a host dropping a connection has to be observable
 * on the phone as a closed channel, not merely as a local state change.
 */
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
    const pc = {
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
      close() { for (const c of this._channels) c.close(); },
    };
    return pc;
  };
  factory.channels = channels;
  return factory;
}

/** Channels of one label, newest last. */
const channelsNamed = (factories, label, origin) =>
  factories.peer.channels.filter((c) => c.label === label && (!origin || c.origin === origin));

/** Stand up a host on a fake world and wait for its issued code. */
async function hostOn(world, opts = {}) {
  const factories = { socket: world.socket, peer: makePeerFactory() };
  let code = null;
  const inbound = [];
  const announced = [];
  const host = createRendezvousHost({
    base: 'https://rendezvous.test',
    factories,
    onCode: (c) => { code = c; },
    onConnection: (conn) => {
      announced.push(conn);
      conn.on('data', (raw) => inbound.push(JSON.parse(raw)));
    },
    ...opts,
  });
  await settle();
  return { host, code, inbound, announced, factories };
}

/**
 * A joiner that is NOT gui/rendezvous-transport.js's — it speaks the same
 * rendezvous frames and opens the same reliable channel, but sends whatever the
 * test tells it to. That is the only way to ask what the host does about a peer
 * that does not play by the handshake: the shipped joiner always does.
 */
async function rogueJoin(factories, code, { onOpen, onMessage, lossy = false } = {}) {
  const socket = factories.socket('https://rendezvous.test/v1/join');
  let pc = null;
  let channel = null;
  let snapshot = null;
  socket.onmessage = async (e) => {
    const msg = JSON.parse(e.data);
    if (msg.type === 'ready') {
      socket.send(JSON.stringify({ v: 1, type: 'join', code }));
    } else if (msg.type === 'joined') {
      pc = factories.peer({ iceServers: [] });
      channel = pc.createDataChannel('reliable', { ordered: true });
      if (lossy) snapshot = pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 });
      channel.onopen = () => { if (onOpen) onOpen(channel); };
      channel.onmessage = (ev) => {
        if (onMessage) onMessage(JSON.parse(ev.data), channel);
      };
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      socket.send(JSON.stringify({ v: 1, type: 'signal', payload: { sdp: pc.localDescription } }));
    } else if (msg.type === 'signal' && msg.payload && msg.payload.sdp) {
      await pc.setRemoteDescription(msg.payload.sdp);
    }
  };
  await settle();
  return {
    get channel() { return channel; },
    get lossy() { return snapshot; },
  };
}

// ── Pure helpers ────────────────────────────────────────────────────────────

describe('service selection', () => {
  it('uses the built-in service on an ordinary page load — there is no off', () => {
    // #1112 retired PeerJS and with it the ?rendezvous opt-in. A page with no
    // parameter is not "off", it is on the only route there is; an old
    // ?rendezvous=off bookmark selects the default rather than a dead end,
    // because there is no longer anything for it to fall back to.
    expect(rendezvousBaseFromLocation('')).toBe(DEV_RENDEZVOUS_URL);
    expect(rendezvousBaseFromLocation('?scenario=x')).toBe(DEV_RENDEZVOUS_URL);
  });

  it('ignores the retired opt-in spellings instead of dialling a host called "on"', () => {
    // A bookmark from the #1111 era must still open the game. Honouring these
    // as service URLs would throw building the socket URL, which is a worse
    // answer than the default they were always pointing at.
    for (const legacy of ['?rendezvous', '?rendezvous=1', '?rendezvous=on', '?rendezvous=off']) {
      expect(rendezvousBaseFromLocation(legacy), legacy).toBe(DEV_RENDEZVOUS_URL);
    }
  });

  it('honours the override for a loopback service — the dev lever that survives', () => {
    expect(rendezvousBaseFromLocation('?rendezvous=http://localhost:8787')).toBe('http://localhost:8787');
    expect(rendezvousBaseFromLocation('?rendezvous=http://127.0.0.1:8787')).toBe('http://127.0.0.1:8787');
    expect(rendezvousBaseFromLocation('?rendezvous=http://[::1]:8787')).toBe('http://[::1]:8787');
    expect(rendezvousBaseFromLocation('?rendezvous=https://phoenix.localhost')).toBe('https://phoenix.localhost');
  });

  it('refuses to send a guest\'s signalling to an off-machine service named in a link', () => {
    // Under #1111 this parameter was also the route opt-in, so an arbitrary
    // origin only mattered to somebody deliberately turning the route on.
    // Since #1112 it applies on every ordinary client load, and
    // `client/index.html?rendezvous=https://attacker.example#<code>` handed to
    // a guest would put their SDP, their ICE candidates and — through a
    // hostile relay standing in as the host — their session token and display
    // name in front of a third party, with nothing on screen saying so.
    for (const hostile of [
      '?rendezvous=https://attacker.example',
      '?rendezvous=http://192.168.0.9:8787',
      '?rendezvous=https://staging.kiwigamedesign.co.uk',
      '?rendezvous=https://localhost.attacker.example',
    ]) {
      expect(rendezvousBaseFromLocation(hostile), hostile).toBe(DEV_RENDEZVOUS_URL);
    }
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
  it('joins straight away with a structured code in the fragment', () => {
    expect(joinRouteFromLocation('', '#proj_ver_QUARK')).toEqual({
      route: 'rendezvous',
      base: DEV_RENDEZVOUS_URL,
      code: 'proj_ver_QUARK',
    });
  });

  it('asks for five letters on a bare page load', () => {
    // No opt-in, no dead end: the rendezvous route is the only route since
    // #1112, so a client page with nothing in its fragment offers the field
    // rather than telling the guest there is no host id in the URL.
    expect(joinRouteFromLocation('', '')).toEqual({
      route: 'entry',
      base: DEV_RENDEZVOUS_URL,
    });
    expect(joinRouteFromLocation('?scenario=x', '')).toMatchObject({ route: 'entry' });
  });

  it('sends a stale PeerJS-era peer id through the same code parse', () => {
    // A bookmarked `#<32 hex>` from before the cutover is not a third route.
    // It is a string that is not a code, and gui/join-code.js is the one place
    // that gets to say so — with a reason the guest can act on, in front of the
    // entry field, rather than a hang on a status line.
    expect(joinRouteFromLocation('', '#0123456789abcdef0123456789abcdef')).toEqual({
      route: 'rendezvous',
      base: DEV_RENDEZVOUS_URL,
      code: '0123456789abcdef0123456789abcdef',
    });
  });

  it('honours a loopback service override for both the code and the entry route', () => {
    expect(joinRouteFromLocation('?rendezvous=http://localhost:8787', '#p_v_QUARK'))
      .toMatchObject({ route: 'rendezvous', base: 'http://localhost:8787' });
    expect(joinRouteFromLocation('?rendezvous=http://localhost:8787', ''))
      .toMatchObject({ route: 'entry', base: 'http://localhost:8787' });
    // …and an off-machine one on neither route.
    expect(joinRouteFromLocation('?rendezvous=http://x.test', '#p_v_QUARK'))
      .toMatchObject({ route: 'rendezvous', base: DEV_RENDEZVOUS_URL });
  });
});

// ── The tracer ──────────────────────────────────────────────────────────────

describe('typed join', () => {
  it('issues a code of the authored length to the host', async () => {
    const world = makeWorld();
    const { code } = await hostOn(world);
    expect(code.suffix).toHaveLength(DATA.suffix.length);
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
    expect(await errorsFor('ZZZZZZZZ', world, factories)).toContain('unknown');
  });

  it('says wrong-type for a code minted in the server namespace', async () => {
    const world = makeWorld();
    // The code comes from the `hosted` frame that issued it, not from the
    // registry's diagnostics snapshot — that view deliberately carries no
    // suffix, because the suffix is the private client code.
    const { code, factories } = await hostOn(world, { namespace: NAMESPACE_SERVER });
    expect(code.namespace).toBe(NAMESPACE_SERVER);
    expect(await errorsFor(code.suffix, world, factories)).toContain('wrong-type');
  });

  it('says version-mismatch for the right namespace under another release', async () => {
    const world = makeWorld();
    const { code, factories } = await hostOn(world, { namespace: NAMESPACE_CLIENT });
    // Re-address the same suffix to a release the registry does not hold.
    const other = composeJoinCode({
      project: projectGuidFor(NAMESPACE_CLIENT, DATA),
      version: '11112222-3333-4444-5555-666677778888',
      suffix: code.suffix,
    });
    expect(await errorsFor(other, world, factories)).toContain('version-mismatch');
  });

  it('refuses a denied word before it ever reaches the service', async () => {
    const world = makeWorld();
    const { factories } = await hostOn(world);
    const sent = vi.fn();
    expect(await errorsFor('ADMINXYZ', world, factories)).toContain('denied');
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
    // Not off the `closed` frame, which arrives while the direct channel is
    // still perfectly healthy — that frame is a statement about the rendezvous
    // RECORD. The channel then closes with its host, the joiner re-resolves
    // the same code on its backoff, and the service gives the honest answer:
    // nothing holds those five letters any more.
    vi.useFakeTimers();
    try {
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
      expect(errors).toEqual([]);

      await vi.advanceTimersByTimeAsync(1_000);
      expect(errors).toContain('unknown');
    } finally {
      vi.useRealTimers();
    }
  });

  it('never hands the page a peer that skipped the handshake', async () => {
    // An open DataChannel is not admission. A peer whose first frame is
    // Identify has never been checked, so it must not reach onConnection —
    // which in server.html IS attachHostConn, the Identify gate and
    // dispatchToWasm behind it.
    const world = makeWorld();
    const { code, inbound, announced, factories } = await hostOn(world, {
      checkStamp: () => ({ ok: true }),
    });
    const rogue = await rogueJoin(factories, code.suffix, {
      onOpen: (channel) => channel.send(JSON.stringify({
        type: 'Identify',
        data: { token: 'sneaky', name: 'Mallory' },
      })),
    });
    await settle();
    expect(announced).toHaveLength(0);
    expect(inbound).toHaveLength(0);

    // …and the channel really was open and delivering all along: the same peer
    // doing the handshake IS admitted, and its next frame arrives. Without
    // this, the assertions above would pass just as well on a broken fixture.
    rogue.channel.send(JSON.stringify({ type: 'JoinHandshake', data: { stamp: '1/x/1' } }));
    await settle();
    rogue.channel.send(JSON.stringify({ type: 'Identify', data: { token: 'proper' } }));
    await settle();
    expect(announced).toHaveLength(1);
    expect(inbound).toEqual([{ type: 'Identify', data: { token: 'proper' } }]);
  });

  it('severs BOTH channels when the host evicts a connection', async () => {
    // server.html has exactly one eviction mechanism — conn.close() — and it
    // uses it in two security-relevant places: the reserved-token gate ("refuse
    // the connection outright rather than dispatch a single message under it")
    // and the duplicate-token dance ("its WebRTC link is severed"). Closing
    // only the reliable channel left the lossy one open with its own inbound
    // handler still wired to deliver(), so an evicted device could keep
    // dispatching into the simulation down the other pipe.
    const world = makeWorld();
    const { code, inbound, announced, factories } = await hostOn(world, {
      checkStamp: () => ({ ok: true }),
    });
    const rogue = await rogueJoin(factories, code.suffix, {
      lossy: true,
      onOpen: (channel) => channel.send(JSON.stringify({
        type: 'JoinHandshake',
        data: { stamp: '1/x/1' },
      })),
    });
    await settle();
    rogue.channel.send(JSON.stringify({ type: 'Identify', data: { token: 'proper' } }));
    await settle();
    expect(announced).toHaveLength(1);
    expect(inbound).toHaveLength(1);

    announced[0].close();
    await settle();

    rogue.lossy.send(JSON.stringify({ type: 'SetThrust', data: { value: 1 } }));
    rogue.channel.send(JSON.stringify({ type: 'SetThrust', data: { value: 2 } }));
    await settle();

    // Nothing arrived on EITHER channel after the eviction.
    expect(inbound).toHaveLength(1);
    expect(rogue.channel.readyState).toBe('closed');
    expect(rogue.lossy.readyState).toBe('closed');
  });

  it('drops everything a refused peer sends after the refusal', async () => {
    // The refusal is followed by a 250 ms drain so the client learns WHY it
    // was dropped. That window is a send-flush, not an admission window.
    const world = makeWorld();
    const seen = [];
    const { code, inbound, announced, factories } = await hostOn(world, {
      checkStamp: () => ({ ok: false, code: 'protocol-mismatch', detail: 'host 1, client 2' }),
    });
    await rogueJoin(factories, code.suffix, {
      onOpen: (channel) => channel.send(JSON.stringify({
        type: 'JoinHandshake',
        data: { stamp: '2/phoenix-base/1' },
      })),
      onMessage: (msg, channel) => {
        seen.push(msg.type);
        if (msg.type === 'JoinRefused') {
          channel.send(JSON.stringify({ type: 'Identify', data: { token: 'after-refusal' } }));
        }
      },
    });
    await settle();
    expect(seen).toEqual(['JoinRefused']);
    expect(announced).toHaveLength(0);
    expect(inbound).toHaveLength(0);
  });

  it('renders every host refusal code as its own sentence, never as unknown', async () => {
    // Through to the string id client.html actually renders — an onError
    // assertion alone would have passed while the phone said "no ship is using
    // that code" for a build the host had authoritatively refused.
    const unknownId = reasonStringId('unknown');
    for (const refusal of [
      'protocol-mismatch',
      'content-id-mismatch',
      'content-epoch-mismatch',
      'bundle-content-missing',
      'client-stamp-missing',
    ]) {
      const world = makeWorld();
      const { code, factories } = await hostOn(world, {
        checkStamp: () => ({ ok: false, code: refusal, detail: 'because' }),
      });
      const rendered = [];
      createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: code.suffix,
        stamp: '9/other-content/3',
        factories,
        onError: (reason) => rendered.push(reasonStringId(reason)),
      });
      await settle();
      expect(rendered, refusal).not.toContain(unknownId);
      expect(rendered, refusal).toContain(reasonStringId(refusal));
    }
  });
});

describe('a joiner that fails cleans up after itself', () => {
  it('reports a signalling socket that drops mid-join instead of hanging', async () => {
    const world = makeWorld();
    const { code } = await hostOn(world);
    const joinSockets = [];
    const factories = {
      socket: (url) => {
        const s = world.socket(url);
        if (String(url).endsWith('/v1/join')) joinSockets.push(s);
        return s;
      },
      peer: makePeerFactory(),
    };

    vi.useFakeTimers();
    try {
      const errors = [];
      const statuses = [];
      createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: code.suffix,
        factories,
        // Pinned to direct WebRTC. Since #1113 an unpinned joiner that runs out
        // of direct attempts escalates to the WebSocket relay instead of giving
        // up, and the relay in this fixture works — so without the pin this
        // test would prove the fallback rather than the claim it is making,
        // which is that a spent ladder ends in front of the guest.
        levers: transportLeversFromLocation('?transport=direct'),
        onError: (reason) => errors.push(reason),
        onStatus: (s) => statuses.push(s),
      });
      // Far enough for the join to be sent, nowhere near a direct channel.
      await Promise.resolve();
      await Promise.resolve();
      joinSockets[0].close();
      await settle();

      // A server-initiated close fires `close` with no preceding `error`, so
      // without an onclose handler the guest sat on "connecting…" forever. It
      // is ACTED on immediately — a second attempt, on the backoff — rather
      // than reported immediately, which is what makes the escalating connect
      // timeout reachable on a first join.
      await vi.advanceTimersByTimeAsync(1_000);
      expect(joinSockets.length).toBeGreaterThan(1);

      // …and once the bounded pre-acceptance attempts are spent the guest is
      // told, rather than left watching a silent backoff forever.
      await vi.advanceTimersByTimeAsync(200_000);
      expect(errors).toContain('unreachable');
      expect(statuses).toContain('error');
    } finally {
      vi.useRealTimers();
    }
  });

  it('closes itself on a terminal frame, so a dead joiner cannot fire later', async () => {
    const world = makeWorld();
    const { factories } = await hostOn(world);
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: 'ZZZZZZZZ',
      factories,
    });
    await settle();
    // The service answered `unknown`. Retrying is the ordinary path through
    // this screen, and each abandoned attempt used to leave a live socket, a
    // registry connection and a peer connection still wired to the page.
    expect(joiner.connected).toBe(false);
    expect(world.registry.snapshot()[0].peers).toBe(0);
  });

  it('ignores a socket error on the signalling socket once the direct channel is open', async () => {
    // An abnormal WS termination fires `error` BEFORE `close` (MDN), so the
    // `linked()` guard `onclose` already had was not enough — a stray error
    // event on the now-expendable signalling socket used to pop the join
    // overlay back over a session that had already connected.
    const world = makeWorld();
    const { code, factories: base } = await hostOn(world);
    const joinSockets = [];
    const factories = {
      socket: (url) => {
        const s = base.socket(url);
        if (String(url).endsWith('/v1/join')) joinSockets.push(s);
        return s;
      },
      peer: base.peer,
    };

    const errors = [];
    const statuses = [];
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
      onError: (reason) => errors.push(reason),
      onStatus: (s) => statuses.push(s),
    });
    await settle();
    expect(joiner.connected).toBe(true);
    expect(statuses).toContain('ready');

    joinSockets[0].onerror();
    await settle();

    expect(errors).toHaveLength(0);
    expect(statuses).not.toContain('error');
  });
});

// ── #1112: the transport replaces PeerJS ────────────────────────────────────

describe('one host, several crew clients', () => {
  it('admits three phones through one code and keeps their identities apart', async () => {
    const world = makeWorld();
    const { code, inbound, announced, factories } = await hostOn(world);

    for (const who of ['ada', 'bo', 'cy']) {
      createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: code.suffix,
        factories,
        getIdent: () => ({ token: `tok-${who}`, name: who }),
      });
      await settle();
    }

    expect(announced).toHaveLength(3);
    // Three distinct rendezvous peers, so the host's peer→token map cannot
    // collapse two phones onto one seat.
    expect(new Set(announced.map((c) => c.peer)).size).toBe(3);
    expect(inbound.map((m) => m.data.token))
      .toEqual(['tok-ada', 'tok-bo', 'tok-cy']);
  });
});

describe('the lossy snapshot channel', () => {
  const joinWith = async (factories, code, opts = {}) => {
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code,
      factories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
      ...opts,
    });
    await settle();
    return joiner;
  };

  it('negotiates an unordered, non-retransmitting channel alongside the reliable one', async () => {
    const world = makeWorld();
    const { code, factories } = await hostOn(world);
    await joinWith(factories, code.suffix);

    const [lossy] = channelsNamed(factories, 'snapshot', 'offer');
    expect(lossy).toBeTruthy();
    // A "snapshot" channel that quietly negotiated as ordered/retransmitting
    // would be the reliable channel under another name, and every head-of-line
    // stall this split exists to avoid would still be there.
    expect(lossy.init).toEqual({ ordered: false, maxRetransmits: 0 });
    expect(channelsNamed(factories, 'reliable', 'offer')[0].init).toEqual({ ordered: true });
  });

  it('hands the host the lossy channel for the token that opened it', async () => {
    const world = makeWorld();
    const { code, announced, factories } = await hostOn(world);
    await joinWith(factories, code.suffix);

    // server.html puts exactly this object into tokenSnapshotConns, which is
    // what gui/host-peer-routing.js routes snapshot deliveries down.
    const conn = announced[0];
    expect(conn.snapshotChannel).toBeTruthy();
    expect(conn.snapshotChannel.label).toBe('snapshot');
    expect(conn.snapshotChannel.readyState).toBe('open');
  });

  it('delivers snapshot-class traffic to the page exactly like reliable traffic', async () => {
    const world = makeWorld();
    const received = [];
    const { code, announced, factories } = await hostOn(world);
    await joinWith(factories, code.suffix, { onData: (m) => received.push(m) });

    announced[0].snapshotChannel.send(JSON.stringify({ type: 'SimState', data: { tick: 7 } }));
    announced[0].send(JSON.stringify({ type: 'Welcome', data: {} }));
    await settle();

    expect(received.map((m) => m.type).sort()).toEqual(['SimState', 'Welcome']);
  });

  it('prefers the lossy channel for a snapshot send and falls back when it is gone', async () => {
    const world = makeWorld();
    const { code, factories } = await hostOn(world);
    const joiner = await joinWith(factories, code.suffix);
    const lossy = channelsNamed(factories, 'snapshot', 'offer')[0];
    const reliable = channelsNamed(factories, 'reliable', 'offer')[0];
    const reliableBefore = reliable.sent.length;

    joiner.send('Ping', { n: 1 }, 'snapshot');
    expect(lossy.sent).toHaveLength(1);
    expect(reliable.sent).toHaveLength(reliableBefore);

    // The lossy channel is allowed to go without taking the session with it.
    lossy.onclose = null;
    lossy.readyState = 'closed';
    joiner.send('Ping', { n: 2 }, 'snapshot');
    expect(lossy.sent).toHaveLength(1);
    expect(reliable.sent).toHaveLength(reliableBefore + 1);
    expect(JSON.parse(reliable.sent.at(-1))).toEqual({ type: 'Ping', data: { n: 2 } });
  });

  it('sends commands on the reliable channel even with the lossy one up', async () => {
    const world = makeWorld();
    const { code, factories } = await hostOn(world);
    const joiner = await joinWith(factories, code.suffix);
    const lossy = channelsNamed(factories, 'snapshot', 'offer')[0];
    const reliable = channelsNamed(factories, 'reliable', 'offer')[0];

    joiner.send('SelectStation', { station: 'Helm' }, 'reliable');
    expect(lossy.sent).toHaveLength(0);
    expect(JSON.parse(reliable.sent.at(-1)))
      .toEqual({ type: 'SelectStation', data: { station: 'Helm' } });
  });
});

describe('automatic reconnect', () => {
  it('re-resolves the same code and re-sends Identify with the same token', async () => {
    const world = makeWorld();
    const { code, inbound, announced, factories } = await hostOn(world);
    const statuses = [];
    const errors = [];
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
      onStatus: (s) => statuses.push(s),
      onError: (r) => errors.push(r),
    });
    await settle();
    expect(inbound).toEqual([{ type: 'Identify', data: { token: 'tok-1', name: 'Ada' } }]);

    // The phone's radio slept: the channel goes on both ends and nobody said
    // goodbye. This is the failure gui/connection-manager.js's backoff loop
    // used to own, and #1112 moved here with it.
    announced[0].close();
    await settle();
    expect(joiner.connected).toBe(false);
    // "reconnecting", NOT the join screen: the code was already accepted, so
    // nothing goes back in front of the guest.
    expect(statuses.at(-1)).toBe('disconnected');
    expect(errors).toHaveLength(0);

    // The page's "retry now" control, short-circuiting the backoff wait.
    joiner.retryNow();
    await settle();

    expect(joiner.connected).toBe(true);
    expect(announced).toHaveLength(2);
    // Same token, no second code typed: this is what makes the host restore
    // the held station and push the current projection.
    expect(inbound).toEqual([
      { type: 'Identify', data: { token: 'tok-1', name: 'Ada' } },
      { type: 'Identify', data: { token: 'tok-1', name: 'Ada' } },
    ]);
    expect(joiner.full).toBe(code.full);
  });

  it('rebuilds the lossy channel with the reconnected link', async () => {
    const world = makeWorld();
    const { code, announced, factories } = await hostOn(world);
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
    });
    await settle();

    announced[0].close();
    await settle();
    joiner.retryNow();
    await settle();

    // Not the first link's channel wearing the second link's name.
    const lossy = channelsNamed(factories, 'snapshot', 'offer');
    expect(lossy).toHaveLength(2);
    expect(announced[1].snapshotChannel).toBe(lossy[1]._remote);
    expect(announced[1].snapshotChannel.readyState).toBe('open');
  });

  it('gives up on an answer a retry cannot change, and says which one', async () => {
    // The host's compatibility verdict and a dead record are answers about the
    // BUILD and the CODE. Retrying either only gets the same sentence more
    // slowly, so both end the loop and go back to the entry field.
    //
    // The dead-record case reaches that answer through ONE retry rather than
    // straight off the `closed` frame. While the direct channel is up, that
    // frame is signalling-plane news about a record the session no longer
    // needs; the channel closing is what ends the session, and the re-resolve
    // that follows is where the service says the code names nothing.
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const { host, code, factories } = await hostOn(world);
      const errors = [];
      const statuses = [];
      const joiner = createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: code.suffix,
        factories,
        getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
        onError: (r) => errors.push(r),
        onStatus: (s) => statuses.push(s),
      });
      await settle();
      expect(joiner.connected).toBe(true);

      host.close();
      await settle();
      expect(joiner.connected).toBe(false);
      // Still in the loop, so still "reconnecting" — nothing in front of the
      // guest yet.
      expect(errors).toEqual([]);
      expect(statuses.at(-1)).toBe('disconnected');

      await vi.advanceTimersByTimeAsync(1_000);
      expect(errors).toContain('unknown');
      // A stopped loop must not read as "reconnecting…" with a retry control.
      expect(statuses.at(-1)).toBe('error');
      expect(joiner.connected).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('retries a link failure before acceptance too, then hands the page the reason', async () => {
    // Retrying only once ESTABLISHED gave a first join exactly one 8s attempt,
    // which made connectTimeoutMs's 8/16/30s ladder unreachable by the guest
    // it was added for — TURN-over-TCP allocation on a cellular network is a
    // first-join problem. The loop is bounded before acceptance, though: a
    // guest who has never got in may be reading the wrong five letters, and a
    // silent backoff would never say so.
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const { code } = await hostOn(world);
      const joinSockets = [];
      const factories = {
        socket: (url) => {
          const s = world.socket(url);
          if (String(url).endsWith('/v1/join')) joinSockets.push(s);
          return s;
        },
        peer: makePeerFactory(),
      };
      const errors = [];
      const statuses = [];
      const attempts = [];
      createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: code.suffix,
        factories,
        // Direct only: this test is about the SHAPE of the direct ladder — four
        // attempts, escalating timeouts, then the entry field. #1113's relay
        // fallback is a fifth thing that happens after all of that, and it has
        // its own cases below.
        levers: transportLeversFromLocation('?transport=direct'),
        onError: (r) => errors.push(r),
        onStatus: (s) => statuses.push(s),
        onDiag: (e) => { if (e.event === 'attempt') attempts.push(e.attempt); },
      });

      await settle();
      joinSockets[0].close();
      await settle();
      // Nothing in front of the guest, and no "Disconnected — reconnecting…"
      // for a connection they never had.
      expect(errors).toEqual([]);
      expect(statuses).not.toContain('disconnected');

      // The second attempt gets the SECOND rung of the ladder, 16s — which is
      // the whole point: it was unreachable while the loop needed acceptance.
      await vi.advanceTimersByTimeAsync(8_200);
      expect(attempts).toEqual([1, 2]);
      expect(errors).toEqual([]);

      await vi.advanceTimersByTimeAsync(200_000);
      expect(attempts).toEqual([1, 2, 3, 4]);
      expect(attempts).toHaveLength(JOIN_ATTEMPTS_BEFORE_ENTRY);
      expect(errors).toEqual(['unreachable']);
      expect(statuses.at(-1)).toBe('error');
    } finally {
      vi.useRealTimers();
    }
  });

  it('reports the connection diagnostics the page renders under the link', async () => {
    const world = makeWorld();
    const { code, factories } = await hostOn(world);
    const events = [];
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
      onDiag: (e) => events.push(e),
    });
    await settle();

    // client.html's #conn-diag readout is written against exactly these event
    // names; a transport that stopped emitting them would blank the one
    // on-screen explanation a stuck phone has.
    expect(events).toContainEqual({ event: 'attempt', attempt: 1 });
    expect(events).toContainEqual({ event: 'signaling', state: 'connecting' });
    expect(events).toContainEqual({ event: 'signaling', state: 'open' });
    expect(events).toContainEqual({ event: 'open' });
  });
});

describe('signalling loss never reaches an established link', () => {
  it('keeps every admitted crew connection when the host loses its record', async () => {
    // The signalling socket and the DataChannels are independent planes. A
    // Durable Object eviction, a worker redeploy or the registry's lazy TTL
    // sweep all reach the host as a dead socket or an `unreachable` frame, and
    // none of them is observable to a phone whose direct link is carrying the
    // game. Tearing those down here emitted `close` on every admitted adapter,
    // which in server.html is wasm_player_disconnected(token) — a whole crew
    // dropped mid-mission, behind a code nobody could read yet.
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const hostSockets = [];
      const factories = {
        socket: (url) => {
          const s = world.socket(url);
          if (String(url).endsWith('/v1/host')) hostSockets.push(s);
          return s;
        },
        peer: makePeerFactory(),
      };
      const codes = [];
      const inbound = [];
      const announced = [];
      const severed = [];
      createRendezvousHost({
        base: 'https://rendezvous.test',
        factories,
        onCode: (c) => codes.push(c),
        onConnection: (conn) => {
          announced.push(conn);
          conn.on('data', (raw) => inbound.push(JSON.parse(raw)));
          conn.on('close', () => severed.push(conn.peer));
        },
      });
      await settle();

      const received = [];
      const joinerStatuses = [];
      const joinerErrors = [];
      const joiner = createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: codes[0].suffix,
        factories,
        getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
        onData: (m) => received.push(m),
        onStatus: (s) => joinerStatuses.push(s),
        onError: (r) => joinerErrors.push(r),
      });
      await settle();
      expect(announced).toHaveLength(1);
      expect(inbound).toEqual([{ type: 'Identify', data: { token: 'tok-1', name: 'Ada' } }]);

      // The service goes.
      hostSockets[0].onerror();
      await vi.advanceTimersByTimeAsync(2_000);

      // The SAME code comes back — reclaimed with the secret this host's own
      // earlier `hosted` frame carried (issue #1115), well within the
      // authored grace window, so a guest who has not got in yet can still
      // read the letters off the viewscreen and have them work…
      expect(codes.length).toBeGreaterThan(1);
      expect(codes[1].suffix).toBe(codes[0].suffix);
      // …and nobody aboard was disconnected.
      expect(severed).toEqual([]);
      expect(announced[0].open).toBe(true);
      expect(joiner.connected).toBe(true);
      expect(joinerErrors).toEqual([]);
      expect(joinerStatuses).not.toContain('error');

      // Still a two-way link, not merely an object that has not been nulled.
      joiner.send('SetThrust', { value: 1 }, 'reliable');
      announced[0].send(JSON.stringify({ type: 'Welcome', data: {} }));
      await settle();
      expect(inbound.at(-1)).toEqual({ type: 'SetThrust', data: { value: 1 } });
      expect(received.map((m) => m.type)).toContain('Welcome');
    } finally {
      vi.useRealTimers();
    }
  });

  it('plays on when the service tells a linked joiner the record has closed', async () => {
    // `closed` is a statement about the rendezvous RECORD — registry.js's
    // dropRecord sends it on the host socket closing AND on the TTL sweep —
    // not about a host. Acting on it tore down a healthy RTCPeerConnection,
    // and because 'host-gone' is terminal the joiner then stopped for good
    // with the direct link still up.
    const world = makeWorld();
    const { code, inbound, factories: base } = await hostOn(world);
    const joinSockets = [];
    const factories = {
      socket: (url) => {
        const s = base.socket(url);
        if (String(url).endsWith('/v1/join')) joinSockets.push(s);
        return s;
      },
      peer: base.peer,
    };
    const errors = [];
    const statuses = [];
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
      onError: (r) => errors.push(r),
      onStatus: (s) => statuses.push(s),
    });
    await settle();
    expect(joiner.connected).toBe(true);

    for (const frame of [{ type: 'closed', reason: 'host-gone' }, { type: 'error', reason: 'not-joined' }]) {
      joinSockets[0].onmessage({ data: JSON.stringify({ v: 1, ...frame }) });
      await settle();
    }

    expect(joiner.connected).toBe(true);
    expect(errors).toEqual([]);
    expect(statuses).not.toContain('error');
    expect(statuses).not.toContain('disconnected');

    // And the link is still carrying commands, not merely reporting open.
    joiner.send('SetThrust', { value: 3 }, 'reliable');
    await settle();
    expect(inbound.at(-1)).toEqual({ type: 'SetThrust', data: { value: 3 } });
  });
});

describe('peer-left is a per-peer signalling relay, not an eviction', () => {
  // registry.js's leave() sends `peer-left` to the HOST when THAT peer's own
  // rendezvous WebSocket dies — a DO eviction, a worker redeploy, a phone
  // radio dropping the WS on a lock screen. Since the joiner-side H2 fix (see
  // the describe block above) a linked joiner deliberately ignores its own
  // signalling socket dying, so before this fix the host would unilaterally
  // evict (wasm_player_disconnected + pc.close) a mid-mission player whose
  // DataChannel was perfectly healthy, while that player's own page still
  // believed it was connected.

  it('does not evict an admitted, still-open joiner whose own signalling socket died', async () => {
    const world = makeWorld();
    const hostFactories = { socket: world.socket, peer: makePeerFactory() };
    let code = null;
    const announced = [];
    const inbound = [];
    const severed = [];
    createRendezvousHost({
      base: 'https://rendezvous.test',
      factories: hostFactories,
      onCode: (c) => { code = c; },
      onConnection: (conn) => {
        announced.push(conn);
        conn.on('data', (raw) => inbound.push(JSON.parse(raw)));
        conn.on('close', () => severed.push(conn.peer));
      },
    });
    await settle();

    const joinSockets = [];
    const joinFactories = {
      socket: (url) => {
        const s = world.socket(url);
        if (String(url).endsWith('/v1/join')) joinSockets.push(s);
        return s;
      },
      peer: hostFactories.peer,
    };
    const received = [];
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories: joinFactories,
      getIdent: () => ({ token: 'tok-1', name: 'Ada' }),
      onData: (m) => received.push(m),
    });
    await settle();
    expect(joiner.connected).toBe(true);
    expect(announced).toHaveLength(1);

    // The joiner's OWN rendezvous WebSocket dies. Its DataChannel is
    // untouched — closing this fake socket goes through the REAL registry's
    // disconnect() → leave() and delivers a genuine `peer-left` to the host
    // for exactly this peer id, with no `closed`/`error` frame and no
    // interaction with the RTCPeerConnection at all.
    joinSockets[0].close();
    await settle();

    // The admitted link must be untouched: no close emitted, still open.
    expect(severed).toEqual([]);
    expect(announced[0].open).toBe(true);
    expect(joiner.connected).toBe(true);

    // Not merely un-severed — still a live two-way link.
    joiner.send('SetThrust', { value: 1 }, 'reliable');
    announced[0].send(JSON.stringify({ type: 'Welcome', data: {} }));
    await settle();
    expect(inbound.at(-1)).toEqual({ type: 'SetThrust', data: { value: 1 } });
    expect(received.map((m) => m.type)).toContain('Welcome');
  });

  it('control: still tears down a peer that is mid-signalling (never admitted)', async () => {
    // The same frame for a peer that never got as far as the compatibility
    // handshake must still be torn down — dropping this teardown entirely
    // would leak an RTCPeerConnection per abandoned join attempt forever.
    const world = makeWorld();
    const basePeer = makePeerFactory();
    const closedPcs = new Set();
    const peerFactory = (iceOpts) => {
      const pc = basePeer(iceOpts);
      const origClose = pc.close.bind(pc);
      pc.close = () => { closedPcs.add(pc); origClose(); };
      return pc;
    };
    peerFactory.channels = basePeer.channels;

    let code = null;
    const iceEvents = [];
    createRendezvousHost({
      base: 'https://rendezvous.test',
      factories: { socket: world.socket, peer: peerFactory },
      onCode: (c) => { code = c; },
      onPeerIce: (peer, state) => iceEvents.push({ peer, state }),
    });
    await settle();

    // A joiner that only ever sends `join` — never creates an
    // RTCPeerConnection or exchanges SDP — is exactly "mid-signalling": the
    // host has a `peerState()` entry (built on `peer-joined`) with a `pc` but
    // no adapter and `admitted: false`.
    const joinSocket = world.socket('https://rendezvous.test/v1/join');
    let joined = false;
    joinSocket.onmessage = (e) => {
      const msg = JSON.parse(e.data);
      if (msg.type === 'ready') joinSocket.send(JSON.stringify({ v: 1, type: 'join', code: code.suffix }));
      else if (msg.type === 'joined') joined = true;
    };
    await settle();
    expect(joined).toBe(true);
    expect(closedPcs.size).toBe(0);

    // That joiner's own signalling socket now dies too, still mid-signalling.
    joinSocket.close();
    await settle();

    // Unlike the admitted case above, today's full teardown still applies.
    expect(closedPcs.size).toBe(1);
    expect(iceEvents).toContainEqual({ peer: expect.any(String), state: 'closed' });
  });
});

describe('a host that loses its record (issue #1115)', () => {
  it('re-registers and reclaims the SAME code rather than sitting unjoinable', async () => {
    // There is no PeerJS underneath any more: a host whose socket blipped and
    // simply reported it would be unreachable until someone reloaded the
    // viewscreen. The replacement registration presents the resume secret
    // this host's own earlier `hosted` frame carried, well within the
    // authored grace window — so it is issued the letters already on screen
    // and QR'd across the room, not a fresh set nobody in the room has read.
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const hostSockets = [];
      const factories = {
        socket: (url) => {
          const s = world.socket(url);
          if (String(url).endsWith('/v1/host')) hostSockets.push(s);
          return s;
        },
        peer: makePeerFactory(),
      };
      const codes = [];
      const errors = [];
      const host = createRendezvousHost({
        base: 'https://rendezvous.test',
        factories,
        onCode: (c) => codes.push(c.suffix),
        onError: (r) => errors.push(r),
      });
      await vi.advanceTimersByTimeAsync(0);
      expect(codes).toHaveLength(1);
      // A resume token is held from the very first successful registration
      // onward — there is now something worth reclaiming if the socket dies.
      expect(host.resuming).toBe(true);

      hostSockets[0].onerror();
      await vi.advanceTimersByTimeAsync(0);
      expect(errors).toEqual(['unreachable']);
      // Still held across the loss, so the diagnostics line server.html
      // renders can honestly say "reconnecting your code" rather than "a new
      // code is coming" while this reconnect is in flight.
      expect(host.resuming).toBe(true);

      await vi.advanceTimersByTimeAsync(2_000);
      expect(codes).toHaveLength(2);
      expect(codes[1]).toBe(codes[0]);
      expect(hostSockets).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it('stops claiming a reclaim once the loss outlasts the grace window (issue #1115)', async () => {
    // `resuming` words server.html's diagnostics line: "reconnecting your code"
    // vs "a new code is coming". Once a loss streak passes the authored
    // reclaim_grace_seconds the registry has dropped the held record, so the
    // pending reconnect will be issued a FRESH suffix — the line must stop
    // promising the old one back. `reregister: false` holds the loss open so
    // the clock can cross the window without a reconnect racing the assertion.
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const hostSockets = [];
      const factories = {
        socket: (url) => {
          const s = world.socket(url);
          if (String(url).endsWith('/v1/host')) hostSockets.push(s);
          return s;
        },
        peer: makePeerFactory(),
      };
      const host = createRendezvousHost({
        base: 'https://rendezvous.test',
        factories,
        reregister: false,
        onCode: () => {},
        onError: () => {},
      });
      await vi.advanceTimersByTimeAsync(0);
      // A token is held from the first registration — reclaimable, so far.
      expect(host.resuming).toBe(true);

      hostSockets[0].onerror();
      await vi.advanceTimersByTimeAsync(0);
      // Still within the window: the same code can genuinely come back.
      expect(host.resuming).toBe(true);

      // Past it: the held record is gone, a fresh suffix is what's coming.
      await vi.advanceTimersByTimeAsync(DATA.limits.reclaim_grace_seconds * 1000 + 1);
      expect(host.resuming).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('stays put when asked not to re-register', async () => {
    const world = makeWorld();
    const hostSockets = [];
    const factories = {
      socket: (url) => {
        const s = world.socket(url);
        if (String(url).endsWith('/v1/host')) hostSockets.push(s);
        return s;
      },
      peer: makePeerFactory(),
    };
    const errors = [];
    const host = createRendezvousHost({
      base: 'https://rendezvous.test',
      factories,
      reregister: false,
      onError: (r) => errors.push(r),
    });
    await settle();
    hostSockets[0].onerror();
    await settle();
    expect(errors).toEqual(['unreachable']);
    expect(host.code).toBeNull();
    host.close();
  });
});

// ── The WebSocket game relay (issue #1113) ──────────────────────────────────
//
// The third rung of the transport ladder: when no direct WebRTC link can be
// built, the rendezvous service carries the game's own frames. These cases run
// the whole path in process — a real registry, a real relay hub, the shipped
// host and joiner — so what they prove is that the FALLBACK reaches the same
// admission gate and the same delivery classes the direct path does. What they
// cannot prove is that a network which blocks WebRTC lets a `wss:` socket
// through; that is the acceptance kit's (docs/acceptance/1113-networks.md).

/**
 * A joiner whose WebRTC never links, because its peer factory is its own and so
 * has nothing to link to. That is exactly the shape of the real failure the
 * relay exists for: signalling works, the media path does not.
 */
function unlinkableFactories(world, sink = []) {
  return {
    socket: (url) => {
      const s = world.socket(url);
      if (String(url).endsWith('/v1/join')) sink.push(s);
      return s;
    },
    peer: makePeerFactory(),
  };
}

/**
 * The same world, with every frame the HOST socket receives recorded by type.
 *
 * The relay's whole failure mode was an ordering one, so a spec that claims to
 * exercise the production order has to be able to prove it did rather than
 * assert it in a comment.
 */
function watchHostFrames(world) {
  const types = [];
  return {
    types,
    world: {
      registry: world.registry,
      socket: (url) => {
        const s = world.socket(url);
        if (!String(url).endsWith('/v1/host')) return s;
        let sink = null;
        Object.defineProperty(s, 'onmessage', {
          configurable: true,
          get: () => (sink
            ? (e) => { types.push(JSON.parse(e.data).type); sink(e); }
            : null),
          set: (fn) => { sink = fn; },
        });
        return s;
      },
    },
  };
}

describe('the WebSocket game relay', () => {
  it('gets a joiner all the way in when no direct link is possible', async () => {
    const world = makeWorld();
    const { code, inbound, announced } = await hostOn(world);
    const statuses = [];
    const diag = [];
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories: unlinkableFactories(world),
      levers: transportLeversFromLocation('?transport=ws-relay'),
      getIdent: () => ({ token: 'tok-relay', name: 'Ada' }),
      onStatus: (s) => statuses.push(s),
      onDiag: (e) => diag.push(e),
    });
    await settle();

    // The host was handed an ORDINARY connection: same adapter, same
    // compatibility handshake, same Identify. Nothing downstream of the
    // transport can tell which path this crew member came in on.
    expect(announced).toHaveLength(1);
    expect(inbound).toContainEqual({
      type: 'Identify',
      data: { token: 'tok-relay', name: 'Ada' },
    });
    expect(statuses).toContain('ready');
    expect(joiner.connected).toBe(true);
    expect(diag).toContainEqual(
      expect.objectContaining({ event: 'transport', transport: 'ws-relay' }),
    );
    joiner.close();
  });

  it('builds the relay pair when peer-joined lands FIRST, as a real socket delivers it', async () => {
    // The order the deployed service produces, and the one the in-process
    // fixture cannot: `clientJoin` emits `peer-joined` to the host one hop
    // after `join`, while `relay-peer` needs a further round trip (the joiner
    // has to receive `joined`, send `relay-open`, and be answered). So the host
    // always holds a placeholder WebRTC entry for this peer BEFORE it is asked
    // to carry it, and a `relay-peer` handler that returned that entry built no
    // channel pair, ran no admission gate, and dropped every `relay` frame
    // after it — the fallback rung dead in the field and green in vitest.
    const watched = watchHostFrames(makeWorld({ queued: true }));
    const { code, inbound, announced } = await hostOn(watched.world);
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories: unlinkableFactories(watched.world),
      levers: transportLeversFromLocation('?transport=ws-relay'),
      getIdent: () => ({ token: 'tok-order', name: 'Ada' }),
    });
    await settle();

    // The spec's own premise, asserted rather than assumed.
    expect(watched.types.indexOf('peer-joined')).toBeGreaterThanOrEqual(0);
    expect(watched.types.indexOf('relay-peer'))
      .toBeGreaterThan(watched.types.indexOf('peer-joined'));

    // And the join completes anyway: the placeholder was upgraded, so the
    // compatibility handshake ran and the page was handed a connection.
    expect(announced).toHaveLength(1);
    expect(inbound).toContainEqual({
      type: 'Identify',
      data: { token: 'tok-order', name: 'Ada' },
    });
    expect(joiner.connected).toBe(true);
    joiner.close();
  });

  it('keeps the snapshot class distinct from the reliable one over the relay', async () => {
    // The relay's whole risk: a WebSocket is reliable and ordered, so a naive
    // implementation silently upgrades the lossy class. The two channels stay
    // separate objects with separate classes on the wire, which is what the
    // host's per-token snapshot routing needs to keep working.
    const world = makeWorld();
    const { code, announced } = await hostOn(world);
    const relayFrames = [];
    const factories = unlinkableFactories(world);
    const openSocket = factories.socket;
    factories.socket = (url) => {
      const s = openSocket(url);
      const send = s.send.bind(s);
      s.send = (text) => {
        const f = JSON.parse(text);
        if (f.type === 'relay') relayFrames.push(f);
        send(text);
      };
      return s;
    };

    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      levers: transportLeversFromLocation('?transport=ws-relay'),
      getIdent: () => ({ token: 'tok-relay', name: 'Ada' }),
    });
    await settle();
    joiner.send('ControlSystem', { target: 'Helm' }, 'snapshot');
    joiner.send('SetReady', { ready: true });
    await settle();

    const classOf = (type) =>
      relayFrames.find((f) => JSON.parse(f.payload).type === type).class;
    expect(classOf('ControlSystem')).toBe('snapshot');
    expect(classOf('SetReady')).toBe('reliable');
    // And the host's side of the pair is a lossy channel the outbound router
    // can prefer per token, exactly as a negotiated DataChannel would be.
    expect(announced[0].snapshotChannel).toBeTruthy();
    expect(announced[0].snapshotChannel.label).toBe('snapshot');
    joiner.close();
  });

  it('falls back on its own once the direct ladder is spent, and says so', async () => {
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const { code, announced } = await hostOn(world);
      const diag = [];
      const joiner = createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: code.suffix,
        factories: unlinkableFactories(world),
        getIdent: () => ({ token: 'tok-fallback', name: 'Ada' }),
        onDiag: (e) => diag.push(e),
      });

      // The four direct attempts, each timing out on its own rung of the
      // connect-timeout ladder, and nothing relayed yet.
      await vi.advanceTimersByTimeAsync(200_000);
      const transports = diag.filter((e) => e.event === 'transport');
      expect(transports.slice(0, JOIN_ATTEMPTS_BEFORE_ENTRY).map((e) => e.transport))
        .toEqual(Array(JOIN_ATTEMPTS_BEFORE_ENTRY).fill('direct'));
      // …then the escalation, named with the reason a readout can render.
      expect(diag).toContainEqual(
        expect.objectContaining({ transport: 'ws-relay', reason: 'direct-exhausted' }),
      );
      // …and the guest is IN, rather than in front of the entry field.
      expect(announced).toHaveLength(1);
      expect(joiner.connected).toBe(true);
      joiner.close();
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not fall back when a lever pinned the transport to WebRTC', async () => {
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const { code, announced } = await hostOn(world);
      const errors = [];
      createRendezvousJoiner({
        base: 'https://rendezvous.test',
        data: DATA,
        code: code.suffix,
        factories: unlinkableFactories(world),
        // The point of a pin: a fallback that fired would hide the very
        // failure the acceptance kit is trying to observe.
        levers: transportLeversFromLocation('?forceRelay=1'),
        onError: (r) => errors.push(r),
      });
      await vi.advanceTimersByTimeAsync(200_000);
      expect(announced).toHaveLength(0);
      expect(errors).toEqual(['unreachable']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('offers only relay candidates when ?forceRelay pins the ICE policy', async () => {
    const world = makeWorld();
    const { code } = await hostOn(world);
    const configs = [];
    const inner = makePeerFactory();
    const factories = {
      socket: world.socket,
      peer: (config) => { configs.push(config); return inner(config); },
    };
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories,
      levers: transportLeversFromLocation('?forceRelay=1'),
    });
    await settle();
    expect(configs).not.toHaveLength(0);
    expect(configs.every((c) => c.iceTransportPolicy === 'relay')).toBe(true);
  });

  it('ends a relayed session when the record behind it dies', async () => {
    // A DataChannel outlives the record that introduced it; a relayed link
    // cannot, because the record IS the link. The joiner must learn that
    // rather than sitting on a socket nothing will arrive on.
    const world = makeWorld();
    const { code, host } = await hostOn(world);
    const statuses = [];
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories: unlinkableFactories(world),
      levers: transportLeversFromLocation('?transport=ws-relay'),
      onStatus: (s) => statuses.push(s),
    });
    await settle();
    expect(joiner.connected).toBe(true);

    host.close();
    await settle();
    expect(joiner.connected).toBe(false);
    joiner.close();
  });

  it('tells a relayed phone it has been evicted instead of dropping it silently', async () => {
    // `connectionAdapter.close()` is the host's ONLY eviction mechanism — the
    // reserved-token refusal and the duplicate-token dance in server.html. For
    // a WebRTC peer it severs real DataChannels the phone observes as a close;
    // for a relayed peer it closed local JavaScript objects and sent nothing,
    // so the evicted device kept a status line reading connected and kept
    // sending commands the host dropped on the floor.
    const world = makeWorld({ queued: true });
    const { code, announced } = await hostOn(world);
    const statuses = [];
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories: unlinkableFactories(world),
      levers: transportLeversFromLocation('?transport=ws-relay'),
      getIdent: () => ({ token: 'tok-evicted', name: 'Ada' }),
      onStatus: (s) => statuses.push(s),
    });
    await settle();
    expect(joiner.connected).toBe(true);
    expect(announced).toHaveLength(1);

    announced[0].close();
    await settle();

    // The phone learns. Without the service being asked to detach it, its own
    // channels stayed 'open' locally and nothing ever told it otherwise.
    expect(joiner.connected).toBe(false);
    // …and the service has stopped carrying it, so its relay slot is free.
    expect(world.registry.snapshot()[0].relayPeers).toBe(0);
    joiner.close();
  });

  it('tells a relayed phone it has been detached when the host cannot fit a reliable frame', async () => {
    // The one frame that will never fit (gui/rendezvous-relay.js's header)
    // fails the LINK rather than silently dropping a command. That failure
    // is local to this host's half of the pair — without asking the service
    // to detach the peer too, the phone would keep sitting on a status line
    // reading "connected" and the service would keep the mailbox open, the
    // exact one-sided eviction the explicit-close case above already covers.
    const world = makeWorld({ queued: true });
    const { code, announced } = await hostOn(world);
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories: unlinkableFactories(world),
      levers: transportLeversFromLocation('?transport=ws-relay'),
      getIdent: () => ({ token: 'tok-oversized', name: 'Ada' }),
    });
    await settle();
    expect(joiner.connected).toBe(true);
    expect(announced).toHaveLength(1);

    const over = JSON.stringify({
      type: 'Welcome',
      data: { pad: 'x'.repeat(DATA.limits.max_relay_frame_bytes + 1) },
    });
    announced[0].send(over);
    await settle();

    // The phone learns, exactly as the explicit host-close case does.
    expect(joiner.connected).toBe(false);
    // …and the service has stopped carrying it, so its relay slot — the
    // phone's mailbox — is freed rather than leaked.
    expect(world.registry.snapshot()[0].relayPeers).toBe(0);
    joiner.close();
  });

  it('tells the host a relayed crew member is being carried by the service', async () => {
    const world = makeWorld();
    const iceStates = [];
    const { code } = await hostOn(world, {
      onPeerIce: (peer, state) => iceStates.push(state),
    });
    const joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: code.suffix,
      factories: unlinkableFactories(world),
      levers: transportLeversFromLocation('?transport=ws-relay'),
    });
    await settle();
    // Not one of WebRTC's five ICE states: no ICE was negotiated, and a readout
    // that printed "connected" here would be claiming a result nothing produced.
    expect(iceStates).toContain('ws-relay');
    joiner.close();
  });
});
