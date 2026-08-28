// Pane link — the LAST script in a pane document (issue #1122).
//
// Native-side glue, not client code. It replaces the page's PeerJS connection
// manager with the in-process pane transport, and it is a module injected just
// before </body> for two reasons: it has to run AFTER gui/connection-manager.js
// has published `window.connectionManager` (module scripts evaluate in document
// order) so that replacing it wins, and it has to import `localiseTree` from
// gui/strings.js, which only a module can do.
//
// The client page needs no change for any of this. Its link contract is four
// members — `connected`, `send(type, data, deliveryClass)`, `connect(hostId,
// options)` and `retryNow()` — and `currentLink()` reads whichever of
// `activeLink` / `window.connectionManager` is present. What arrives on
// `options.onData` is what the page's own `handleMessage` folds into
// `window.simState`, exactly as it does for a phone.
import { localiseTree } from './gui/strings.js';

const pane = window.__phoenixPane;
let options = null;

const link = {
  // Always true: there is no socket to be up or down. A pane that has lost its
  // host has lost its process.
  get connected() {
    return true;
  },

  connect(_hostId, opts) {
    options = opts || {};
    if (typeof options.onStatus === 'function') options.onStatus('ready');

    // Identify with the token and name the HOST minted, not with whatever the
    // page computed. The host refuses any other token anyway — a pane may only
    // identify as itself — so presenting the page's would simply never join.
    link.send('Identify', { token: pane.token, name: pane.name });

    pane.deliver = (json) => {
      if (typeof options.onData !== 'function') return;
      try {
        // The same ingress boundary a phone crosses: server-sent string ids
        // become display text once, here, so no console has to know which of
        // its fields are localisable.
        options.onData(localiseTree(JSON.parse(json)));
      } catch (e) {
        console.warn('[pane] undecodable message from the host', e);
      }
    };
    // Anything that arrived while the modules were still evaluating.
    pane.drainInbox();
  },

  send(type, data) {
    // The wire shape gui/connection-manager.js sends, unchanged: the host
    // decodes it with the same core::codec function it uses for a phone.
    window.phoenixPaneOut.send(JSON.stringify(data !== undefined ? { type, data } : { type }));
  },

  // Nothing to retry and nothing to tear down; both exist because the page
  // calls them.
  retryNow() {},
  disconnect() {},
};

window.connectionManager = link;

// The page fetches ICE servers before connecting and probes a TURN relay for
// its diagnostics readout. A pane has no WebRTC connection to configure, and
// both would otherwise reach for the network — on a bridge machine that may
// have none — and stall the boot behind a timeout.
window.fetchIceServers = async () => ({ servers: [], relayAvailable: false, relaySource: null });
window.probeTurnRelay = async () => 'unreachable';
