// tests/smoke/rendezvous-shim.js — the ONE transport stand-in for the browser
// smoke suite (issues #1111, #1112).
//
// CI has no real WebRTC and no deployed rendezvous worker, so the suite needs a
// fake for both. It used to need two — this and a whole `window.Peer`
// replacement (`peerjs-shim.js`) — because two transports were live at once.
// #1112 retired PeerJS, and this file went with it from "the #1111 tracer's
// fake" to "the transport every smoke spec runs on".
//
// Two fakes, installed as window.PhoenixTransportFactories, which
// gui/rendezvous-transport.js takes its WebSocket and RTCPeerConnection from:
//
//   socket(url)  a WebSocket onto a rendezvous registry running IN THE HOST
//                PAGE. Not a re-implementation: it dynamically imports the
//                real worker-rendezvous/src/registry.js (served to the page by
//                a Playwright route), so the code minting, the typed lookup and
//                the signalling relay under test here are the shipped ones —
//                only the socket between them is fake.
//   peer(config) an RTCPeerConnection that pairs two pages' DataChannels over a
//                BroadcastChannel once an offer/answer has crossed the (real)
//                relay. Honours the `createDataChannel` init bag, so the lossy
//                'snapshot' channel is distinguishable from the reliable one.
//
// The first page to open a `/v1/host` socket owns the registry; every other
// page relays its frames to that page over a BroadcastChannel. The product is
// one host and N crew clients, so one owner is enough — a multi-host smoke case
// (#1114) would need an election here.
//
// It also carries the two things the whole suite is wired to, inherited from
// the shim it replaced:
//
//   window.__wasmReady / the 'wasm-ready' event — set once the host page has
//       BOTH opened its rendezvous socket AND dispatched PhoenixReady. Every
//       spec gates on it through fixtures.js's waitForWasmReady.
//   window.__transportShim.sever()/revive() — a page-local kill switch for the
//       "phone's radio slept" failure: channels close on both ends without
//       either side calling close(), and every retry keeps failing until the
//       test explicitly revives, so auto-reconnect cannot race the assertion.

(() => {
  const SIGNAL_BUS = 'phoenix-rendezvous-shim';
  const MEDIA_BUS = 'phoenix-rtc-shim';
  const REGISTRY_URL = '/__rendezvous-registry.js';
  const FORMAT_URL = '/assets/join/join-codes.json';

  // Set by sever(); nothing on this page can reach the network while true.
  let offline = false;

  // ── The fake rendezvous socket ────────────────────────────────────────────

  const bus = new BroadcastChannel(SIGNAL_BUS);
  const localSockets = new Map(); // connId → fake socket on THIS page
  let registryReady = null; // Promise<registry> on the owner page only

  function becomeOwner() {
    registryReady = (async () => {
      const [{ createRegistry }, data] = await Promise.all([
        import(REGISTRY_URL),
        fetch(FORMAT_URL).then((r) => r.json()),
      ]);
      return createRegistry({ data });
    })();
  }

  function deliver(frames) {
    for (const { to, frame } of frames) {
      const local = localSockets.get(to);
      if (local) {
        if (local.onmessage) local.onmessage({ data: JSON.stringify(frame) });
      } else {
        bus.postMessage({ kind: 'out', to, frame });
      }
    }
  }

  // Non-owner pages cannot call the registry directly, so their calls travel
  // as descriptors rather than closures.
  function callOwner(call) {
    if (registryReady) {
      registryReady.then((registry) => {
        if (call.op === 'connect') deliver(registry.connect(call.connId, call.role));
        else if (call.op === 'receive') deliver(registry.receive(call.connId, call.frame));
        else if (call.op === 'disconnect') deliver(registry.disconnect(call.connId));
      });
    } else {
      bus.postMessage({ kind: 'in', call });
    }
  }

  bus.onmessage = (e) => {
    const msg = e.data;
    if (!msg) return;
    if (msg.kind === 'in' && registryReady) callOwner(msg.call);
    if (msg.kind === 'out') {
      const local = localSockets.get(msg.to);
      if (local && local.onmessage) local.onmessage({ data: JSON.stringify(msg.frame) });
    }
  };

  function makeSocket(url) {
    const connId = `shim-${Math.random().toString(16).slice(2, 10)}`;
    const role = String(url).endsWith('/v1/host') ? 'host' : 'client';

    const sock = {
      url,
      readyState: 0,
      onopen: null,
      onmessage: null,
      onerror: null,
      onclose: null,
      send(text) {
        if (this.readyState !== 1) return;
        callOwner({ op: 'receive', connId, frame: JSON.parse(text) });
      },
      close() {
        if (this.readyState === 3) return;
        const wasOpen = this.readyState === 1;
        this.readyState = 3;
        localSockets.delete(connId);
        if (wasOpen) callOwner({ op: 'disconnect', connId });
        if (this.onclose) this.onclose();
      },
    };

    // A severed page cannot reach the service at all. Fire the same
    // error-then-close a real socket does when the network is gone, so the
    // transport's own retry loop is what the test observes rather than a
    // fabricated one.
    if (offline) {
      setTimeout(() => {
        if (sock.readyState === 3) return;
        sock.readyState = 3;
        if (sock.onerror) sock.onerror();
        if (sock.onclose) sock.onclose();
      }, 0);
      return sock;
    }

    if (role === 'host' && !registryReady) becomeOwner();
    localSockets.set(connId, sock);
    setTimeout(() => {
      if (sock.readyState === 3) return;
      sock.readyState = 1;
      if (sock.onopen) sock.onopen();
      callOwner({ op: 'connect', connId, role });
      if (role === 'host') { hostSocketOpen = true; maybeDispatchReady(); }
    }, 0);
    return sock;
  }

  // ── The fake peer connection ──────────────────────────────────────────────

  const media = new BroadcastChannel(MEDIA_BUS);
  const links = new Map(); // linkId → { side, channels: Map<label, channel> }

  media.onmessage = (e) => {
    const msg = e.data;
    if (!msg) return;
    const link = links.get(msg.link);
    if (!link || msg.side === link.side) return;
    const channel = link.channels.get(msg.label);
    if (!channel) return;
    if (msg.close) {
      if (channel.readyState === 'closed') return;
      channel.readyState = 'closed';
      if (channel.onclose) channel.onclose();
      return;
    }
    if (offline) return;
    if (channel.onmessage) channel.onmessage({ data: msg.data });
  };

  function makeChannel(linkId, side, label, init) {
    const opts = init || {};
    return {
      label,
      readyState: 'connecting',
      // Recorded, and mirrored onto the answering end, so a spec can prove the
      // snapshot channel really negotiated unordered and non-retransmitting
      // rather than being the reliable channel under another name.
      ordered: opts.ordered !== false,
      maxRetransmits: opts.maxRetransmits ?? null,
      onopen: null,
      onmessage: null,
      onclose: null,
      onerror: null,
      send(data) {
        if (offline || this.readyState !== 'open') return;
        media.postMessage({ link: linkId, side, label, data });
      },
      close() {
        if (this.readyState === 'closed') return;
        this.readyState = 'closed';
        media.postMessage({ link: linkId, side, label, close: true });
        if (this.onclose) this.onclose();
      },
    };
  }

  function openChannels(link) {
    for (const channel of link.channels.values()) {
      if (channel.readyState !== 'connecting') continue;
      channel.readyState = 'open';
      if (channel.onopen) channel.onopen();
    }
  }

  function makePeer() {
    let linkId = null;
    let side = null;
    let link = null;
    const localChannels = [];

    const pc = {
      localDescription: null,
      remoteDescription: null,
      onicecandidate: null,
      oniceconnectionstatechange: null,
      ondatachannel: null,
      iceConnectionState: 'connected',
      addEventListener() {},
      removeEventListener() {},
      getStats: () => Promise.resolve(new Map()),
      createDataChannel(label, init) {
        if (!linkId) {
          linkId = `link-${Math.random().toString(16).slice(2, 10)}`;
          side = 'offer';
          link = { side, channels: new Map() };
          links.set(linkId, link);
        }
        localChannels.push({ label, init: init || {} });
        const channel = makeChannel(linkId, side, label, init);
        link.channels.set(label, channel);
        return channel;
      },
      async createOffer() {
        return { type: 'offer', link: linkId, channels: localChannels.map((c) => ({ ...c })) };
      },
      async createAnswer() {
        return { type: 'answer', link: linkId };
      },
      async setLocalDescription(d) {
        this.localDescription = d;
      },
      async setRemoteDescription(d) {
        this.remoteDescription = d;
        if (d.type === 'offer') {
          linkId = d.link;
          side = 'answer';
          link = { side, channels: new Map() };
          links.set(linkId, link);
          for (const spec of d.channels || []) {
            const channel = makeChannel(linkId, side, spec.label, spec.init);
            link.channels.set(spec.label, channel);
            if (this.ondatachannel) this.ondatachannel({ channel });
          }
          // Both ends are wired the moment the answerer has seen the offer;
          // the answer still travels, exactly as it would over a real relay.
          setTimeout(() => openChannels(link), 0);
        } else if (d.type === 'answer' && link) {
          setTimeout(() => openChannels(link), 0);
        }
      },
      async addIceCandidate() {},
      close() {
        if (!link) return;
        for (const channel of link.channels.values()) channel.close();
        links.delete(linkId);
      },
    };
    return pc;
  }

  window.PhoenixTransportFactories = { socket: makeSocket, peer: makePeer };

  // ── wasm-ready signalling ─────────────────────────────────────────────────
  //
  // __wasmReady is set when BOTH:
  //   1. the host page has opened its rendezvous socket (so the join panel is
  //      about to carry a code readHostJoinTarget can scrape), AND
  //   2. PhoenixReady has fired — dispatched by server.html's finishInit()
  //      after the async config preload completes and wasm_init() has been
  //      called.
  //
  // Both halves matter: TrunkApplicationStarted fires before the async
  // map/entity config fetch sequence completes, which used to cause Welcome
  // timeouts.

  let hostSocketOpen = false;
  let phoenixReady = false;
  let readyFired = false;

  function maybeDispatchReady() {
    if (!hostSocketOpen || !phoenixReady || readyFired) return;
    readyFired = true;
    window.__wasmReady = true;
    window.dispatchEvent(new CustomEvent('wasm-ready'));
  }

  window.addEventListener('PhoenixReady', () => {
    phoenixReady = true;
    maybeDispatchReady();
  });

  // ── Test-only kill/revive support (issue #614, re-homed in #1112) ─────────
  //
  // Simulates a silently-dropped link — a phone's radio sleeping mid-game —
  // without either side calling close() on its own connection. Real WebRTC
  // failures like this fire the DataChannel's close on BOTH ends, which is
  // exactly what the transport's reconnect-with-backoff loop needs to observe.
  //
  // Page-scoped rather than addressed by a pair of ids, because that is what a
  // dropped radio actually is: everything THIS page has goes, and everything it
  // tries next fails, until the test brings it back. The severed page also
  // cannot reach the rendezvous service, so its automatic retries keep failing
  // and cannot race ahead of the assertion that the retry control appeared.
  window.__transportShim = {
    /** Kill every link this page holds and keep it off the network. */
    sever() {
      offline = true;
      // Channels first: the far end learns the link is gone from the media
      // frames, and the send() guard above cannot swallow them because they
      // are posted by close() before `offline` blocks anything else.
      for (const [id, link] of links) {
        for (const channel of link.channels.values()) {
          if (channel.readyState === 'closed') continue;
          channel.readyState = 'closed';
          media.postMessage({ link: id, label: channel.label, side: link.side, close: true });
          if (channel.onclose) channel.onclose();
        }
      }
      for (const sock of [...localSockets.values()]) sock.close();
    },

    /**
     * Put this page back on the network. Existing links stay dead — the
     * transport must retry to build fresh ones, which is the real user-visible
     * "retry now" path.
     */
    revive() {
      offline = false;
    },

    /**
     * Every DataChannel this page holds, for inspection:
     * `{ "<label>@<link>": { readyState, ordered, maxRetransmits, side } }`.
     */
    dataChannels() {
      const out = {};
      for (const [id, link] of links) {
        for (const [label, channel] of link.channels) {
          out[`${label}@${id}`] = {
            readyState: channel.readyState,
            ordered: channel.ordered,
            maxRetransmits: channel.maxRetransmits,
            side: link.side,
          };
        }
      }
      return out;
    },
  };

  // A closed page must not leave a live record behind, or the next attempt
  // resolves a host that is not there — and the host must run its
  // wasm_player_disconnected lifecycle rather than keeping a console
  // "occupied" by a phone that has gone.
  window.addEventListener('pagehide', () => {
    for (const link of links.values()) {
      for (const channel of link.channels.values()) {
        try { channel.close(); } catch { /* already gone */ }
      }
    }
    for (const sock of [...localSockets.values()]) sock.close();
  });
})();
