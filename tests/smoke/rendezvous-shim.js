// A local stand-in for the Phoenix rendezvous service and for WebRTC, so the
// browser smoke suite can drive the whole #1111 join tracer. CI has no real
// WebRTC and no deployed worker; this is the same trick tests/smoke/peerjs-shim.js
// plays for PeerJS, one layer lower.
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
//                relay.
//
// The first page to open a `/v1/host` socket owns the registry; every other
// page relays its frames to that page over a BroadcastChannel. #1111's tracer
// is one host and N crew clients, so one owner is enough — a multi-host smoke
// case (#1114) would need an election here.

(() => {
  const SIGNAL_BUS = 'phoenix-rendezvous-shim';
  const MEDIA_BUS = 'phoenix-rtc-shim';
  const REGISTRY_URL = '/__rendezvous-registry.js';
  const FORMAT_URL = '/assets/join/join-codes.json';

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
    if (role === 'host' && !registryReady) becomeOwner();

    const sock = {
      url,
      readyState: 0,
      onopen: null,
      onmessage: null,
      onerror: null,
      onclose: null,
      send(text) {
        callOwner({ op: 'receive', connId, frame: JSON.parse(text) });
      },
      close() {
        if (this.readyState === 3) return;
        this.readyState = 3;
        localSockets.delete(connId);
        callOwner({ op: 'disconnect', connId });
        if (this.onclose) this.onclose();
      },
    };
    localSockets.set(connId, sock);
    setTimeout(() => {
      sock.readyState = 1;
      if (sock.onopen) sock.onopen();
      callOwner({ op: 'connect', connId, role });
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
      channel.readyState = 'closed';
      if (channel.onclose) channel.onclose();
      return;
    }
    if (channel.onmessage) channel.onmessage({ data: msg.data });
  };

  function makeChannel(linkId, side, label) {
    return {
      label,
      readyState: 'connecting',
      onopen: null,
      onmessage: null,
      onclose: null,
      onerror: null,
      send(data) {
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
      channel.readyState = 'open';
      if (channel.onopen) channel.onopen();
    }
  }

  function makePeer() {
    let linkId = null;
    let side = null;
    let link = null;
    const localLabels = [];

    const pc = {
      localDescription: null,
      remoteDescription: null,
      onicecandidate: null,
      ondatachannel: null,
      iceConnectionState: 'connected',
      addEventListener() {},
      removeEventListener() {},
      getStats: () => Promise.resolve(new Map()),
      createDataChannel(label) {
        if (!linkId) {
          linkId = `link-${Math.random().toString(16).slice(2, 10)}`;
          side = 'offer';
          link = { side, channels: new Map() };
          links.set(linkId, link);
        }
        localLabels.push(label);
        const channel = makeChannel(linkId, side, label);
        link.channels.set(label, channel);
        return channel;
      },
      async createOffer() {
        return { type: 'offer', link: linkId, channels: [...localLabels] };
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
          for (const label of d.channels || []) {
            const channel = makeChannel(linkId, side, label);
            link.channels.set(label, channel);
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

  // A closed page must not leave a live record behind, or the next attempt
  // resolves a host that is not there.
  window.addEventListener('pagehide', () => {
    for (const sock of [...localSockets.values()]) sock.close();
  });
})();
