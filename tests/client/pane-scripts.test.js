// @vitest-environment jsdom
/**
 * tests/client/pane-scripts.test.js — the two scripts a native Station pane
 * document injects (issue #1122).
 *
 * `src/native_host/panes/pane_boot.js` and `pane_link.js` are native-side glue
 * rather than `gui/` code — they are injected into a copy of the client page so
 * `gui/` and every console stay byte-for-byte what a phone loads — but they are
 * still JavaScript that runs in a browser engine, and between them they own two
 * things nothing else does: which participant a pane presents itself as, and
 * how the page reaches its host at all.
 *
 * **These tests drive the REAL seam.** The link installs
 * `window.PhoenixTransportFactories`, which is the override point
 * `gui/rendezvous-transport.js` documents for "a native in-process host", and
 * the page then runs its ordinary join through it. So the joiner here is the
 * repository's own `createRendezvousJoiner`, not a stand-in for it: what these
 * assert is that a pane joins by the same code path a phone does. A previous
 * version of this suite called methods on `window.connectionManager`, which
 * issue #1112 retired with PeerJS — every assertion passed and the page called
 * none of it.
 *
 * Both files are read off disk and evaluated here rather than imported, because
 * that is how they run in production: the boot script is a classic `<script>`
 * at the top of `<head>` (it has to see the fragment before the page's own
 * inline script wants the token), and the link is a module injected before
 * `</body>`.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  createRendezvousJoiner,
  joinRouteFromLocation,
  RELIABLE_CHANNEL,
} from '../../gui/rendezvous-transport.js';
import { NAMESPACE_CLIENT, parseJoinCode } from '../../gui/join-code.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const PANES = path.join(root, 'src/native_host/panes');
const BOOT = readFileSync(path.join(PANES, 'pane_boot.js'), 'utf8');
const LINK = readFileSync(path.join(PANES, 'pane_link.js'), 'utf8');
const DOCUMENT_RS = readFileSync(path.join(PANES, 'document.rs'), 'utf8');
const DATA = JSON.parse(readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'));

/** The import specifier the link uses for the transport's channel labels. */
const TRANSPORT_SPECIFIER = "'./gui/rendezvous-transport.js'";

/**
 * The join code Rust puts at the head of a pane's fragment.
 *
 * Read out of `document.rs` rather than duplicated, because a literal only Rust
 * declares and only JavaScript validates is a literal nothing checks — and the
 * failure it produces is silent from both sides: the page shows its join-entry
 * overlay over a console that is otherwise perfectly alive.
 */
const PANE_JOIN_CODE = (() => {
  const found = /pub const PANE_JOIN_CODE: &str = "([^"]+)";/.exec(DOCUMENT_RS);
  if (!found) throw new Error('document.rs no longer declares PANE_JOIN_CODE');
  return found[1];
})();

/** The fragment `pane_url` builds, for a token and a name. */
const fragment = (token, name) => `#${PANE_JOIN_CODE}&token=${token}&name=${name}`;

/**
 * Evaluate the boot script the way the document does: a classic script, in the
 * page's global scope, with `location.hash` already set.
 */
function runBoot(hash) {
  window.location.hash = hash;
  // eslint-disable-next-line no-new-func
  new Function(BOOT)();
}

/**
 * Import the link module with its one import pointed at the real transport.
 *
 * Not a stub: the channel labels it reads have to be the ones the joiner in
 * these tests actually opens, and a specifier rewritten to this file's own URL
 * resolves to the same module instance the static import above holds.
 */
async function importLink() {
  const real = pathToFileURL(path.join(root, 'gui/rendezvous-transport.js')).href;
  const source =
    LINK.replace(TRANSPORT_SPECIFIER, JSON.stringify(real)) +
    // ESM modules are cached by specifier; a fresh body per test keeps
    // `window.__phoenixPane` captures from leaking between them.
    `\n// ${Math.random()}\n`;
  return import(
    'data:text/javascript;base64,' + Buffer.from(source, 'utf8').toString('base64')
  );
}

/** A page→host queue the host would install with `queue_shim`. */
function installOutQueue() {
  const sent = [];
  window.phoenixPaneOut = { send: (record) => sent.push(record) };
  return sent;
}

/** Let the pane's stand-ins work through their turns. */
async function settle(turns = 24) {
  for (let i = 0; i < turns; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

/**
 * Run the page's ordinary join over whatever `window.PhoenixTransportFactories`
 * currently is, and hand back what the page saw.
 *
 * This is `client.html`'s `startPhoenixJoin` reduced to the call it makes: the
 * same `createRendezvousJoiner`, the same code the fragment carries, the same
 * four callbacks. Nothing here knows it is talking to a pane.
 */
async function joinAsThePageWould(ident = { token: 'page-computed', name: 'Page' }) {
  const received = [];
  const statuses = [];
  const errors = [];
  const joiner = createRendezvousJoiner({
    base: 'https://rendezvous.invalid',
    data: DATA,
    code: PANE_JOIN_CODE,
    stamp: 'a-client-stamp',
    getIdent: () => ident,
    onData: (msg) => received.push(msg),
    onStatus: (state) => statuses.push(state),
    onError: (reason, detail) => errors.push([reason, detail]),
  });
  await settle();
  return { joiner, received, statuses, errors };
}

/** The boot script replaces both of these; every test gets them back. */
let nativeRaf;
let nativeCaf;

beforeEach(() => {
  nativeRaf = window.requestAnimationFrame;
  nativeCaf = window.cancelAnimationFrame;
  delete window.__phoenixPane;
  delete window.__phoenixPaneApply;
  delete window.phoenixPaneOut;
  delete window.PhoenixTransportFactories;
  delete window.PhoenixOperatorCapabilities;
  delete window.connectionManager;
  window.sessionStorage.clear();
  window.history.replaceState(null, '', '#');
  window.location.hash = '';
});

afterEach(() => {
  window.requestAnimationFrame = nativeRaf;
  window.cancelAnimationFrame = nativeCaf;
});

describe('pane_boot.js — the identity comes out of the fragment', () => {
  it('declares native capability gaps before the shared client profile loads', () => {
    // #1124 remains the only pane input route: no browser-shaped Gamepad API
    // stub is allowed to create a second path. #1127 remains the Accessibility
    // path; the declaration only filters active capabilities and retains data.
    runBoot(fragment('abcd', 'Ada'));
    expect(window.PhoenixOperatorCapabilities).toEqual({
      surface: 'native-pane',
      keyboard: true,
      gamepad: false,
      vibration: false,
      semanticCues: true,
      accessibility: true,
    });
    expect(BOOT.indexOf('window.PhoenixOperatorCapabilities ='))
      .toBeLessThan(BOOT.indexOf('window.__phoenixPane'));
  });

  it('reads the token and name the host put in location.hash', () => {
    // The point of the fragment: a browser never transmits it, so a live
    // participant's session token is in no byte the host serves. The boot
    // script is where it re-enters the page.
    runBoot(fragment('3f1a6c2e-0a11-4b3c-9d55-000000000001', 'Ada'));
    expect(window.__phoenixPane.token).toBe('3f1a6c2e-0a11-4b3c-9d55-000000000001');
    expect(window.__phoenixPane.name).toBe('Ada');
  });

  it('takes an underscore in a name at face value, because it is no longer routing', () => {
    // `fragment_encode` used to escape `_` as %5F to keep a participant called
    // `ada_lovelace` off the rendezvous route. Since #1112 EVERY non-empty
    // fragment is that route, so the escape bought nothing and the pane takes
    // the route deliberately instead — see document.rs.
    runBoot(fragment('tok_en', 'ada_lovelace'));
    expect(window.__phoenixPane.token).toBe('tok_en');
    expect(window.__phoenixPane.name).toBe('ada_lovelace');
  });

  it('decodes an apostrophe, a space and non-ASCII without losing the other field', () => {
    runBoot(fragment('a%20b', "O%27Neil%C3%A9"));
    expect(window.__phoenixPane.token).toBe('a b');
    expect(window.__phoenixPane.name).toBe("O'Neilé");
  });

  it('keeps one malformed escape from costing the other field', () => {
    // `decodeURIComponent` throws on a lone `%`, and losing the token because
    // the name was mistyped would be a pane that silently never joins.
    runBoot(fragment('abcd', '%E0%A4%A'));
    expect(window.__phoenixPane.token).toBe('abcd');
    expect(window.__phoenixPane.name).toBe('');
  });

  it('ignores anything in the fragment it does not own', () => {
    runBoot(`${fragment('abcd', 'Ada')}&rendezvous=nope`);
    expect(window.__phoenixPane.token).toBe('abcd');
    expect(Object.keys(window.__phoenixPane)).toContain('inbox');
    expect(window.__phoenixPane.rendezvous).toBeUndefined();
  });

  it('seeds the session-token key gui/session-token.js reads', () => {
    // This is how the page comes to compute the right token: `decideToken`
    // reuses whatever sessionStorage already holds for this tab.
    runBoot(fragment('abcd', 'Ada'));
    expect(window.sessionStorage.getItem('session-token')).toBe('abcd');
    expect(window.sessionStorage.getItem('player-name')).toBe('Ada');
  });
});

describe('pane_boot.js — the fragment is normalised into a join route', () => {
  it('leaves the page a fragment its one route can actually resolve', () => {
    // The contract that matters, and the one that broke: `client.html` reads
    // `location.hash` through `joinRouteFromLocation`, and ANY non-empty
    // fragment is the rendezvous route. `token=…&name=…` reaches
    // `parseJoinCode`, is refused, and the join-entry overlay covers the
    // console — a pane that never joins, in front of a field nobody will type
    // into. The boot script consumes the identity and leaves the code.
    runBoot(fragment('3f1a6c2e-0a11', 'Ada'));
    expect(window.location.hash).toBe(`#${PANE_JOIN_CODE}`);

    const route = joinRouteFromLocation(window.location.search, window.location.hash);
    expect(route.route).toBe('rendezvous');
    expect(parseJoinCode(route.code, NAMESPACE_CLIENT, DATA).ok).toBe(true);
  });

  it('takes the identity out of the URL while it is there', () => {
    // Not the defence — the fragment was never transmitted — but a real
    // consequence: by the time any page code, console iframe or `location`
    // reader runs, the pane's session token is no longer in its own URL.
    runBoot(fragment('3f1a6c2e-0a11', 'Ada'));
    expect(window.location.hash).not.toContain('token');
    expect(window.location.href).not.toContain('3f1a6c2e-0a11');
  });

  it('uses a code the authored join-code table accepts', () => {
    // The cross-language pin. `document.rs` declares the literal; this is the
    // only place it is checked against `assets/join/join-codes.toml`'s alphabet,
    // length, confusable map and deny list — through the very function
    // `client.html` runs on it.
    const parsed = parseJoinCode(PANE_JOIN_CODE, NAMESPACE_CLIENT, DATA);
    expect(parsed.ok).toBe(true);
    expect(parsed.suffix).toBe(PANE_JOIN_CODE);
    expect(parsed.namespace).toBe(NAMESPACE_CLIENT);
  });
});

describe('pane_boot.js — the render loop runs on a timer, not on the engine', () => {
  it('drives requestAnimationFrame off a timer, and cancels one', async () => {
    // The page's whole render loop is one outstanding rAF:
    //
    //     if (_renderFrame !== null) return;
    //     _renderFrame = requestAnimationFrame(() => { _renderFrame = null; render(…); });
    //
    // An offscreen Ultralight view services rAF only inside a rendering update
    // and only runs one when the page is dirty, so a pane whose only pending
    // work IS that render deadlocks: the callback never fires, `_renderFrame`
    // stays non-null, and every later `scheduleRender()` returns at line one.
    // Observed as roughly one console in four coming up frozen behind the
    // lobby, holding a Station it knew perfectly well it held. Timers are not
    // starved that way.
    runBoot(fragment('abcd', 'Ada'));

    const stamps = [];
    const id = window.requestAnimationFrame((t) => stamps.push(t));
    expect(typeof id).not.toBe('undefined');
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(stamps).toHaveLength(1);
    // rAF hands its callback a monotonic timestamp; consoles integrate against
    // it, so a shim that passed nothing would make every animation jump.
    expect(typeof stamps[0]).toBe('number');

    const cancelled = [];
    window.cancelAnimationFrame(window.requestAnimationFrame(() => cancelled.push(1)));
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(cancelled).toHaveLength(0);
  });

  it('installs it before the module that captures it', () => {
    // gui/bg-raf-keepalive.js takes one reference to `window.requestAnimationFrame`
    // at evaluation time and delegates to it whenever the document is visible —
    // which a pane always is. Replacing rAF after that module has run would
    // leave the captured engine reference in charge and change nothing.
    expect(BOOT.indexOf('window.requestAnimationFrame =')).toBeGreaterThan(-1);
    expect(BOOT.indexOf('window.requestAnimationFrame ='))
      .toBeLessThan(BOOT.indexOf('window.__phoenixPane'));
  });
});

describe('pane_boot.js — the inbox is bounded', () => {
  it('holds what arrives before the page has joined', () => {
    runBoot(fragment('abcd', 'Ada'));
    window.__phoenixPaneApply('{"type":"Welcome"}');
    window.__phoenixPaneApply('{"type":"GameStarted"}');
    expect(window.__phoenixPane.inbox).toHaveLength(2);

    const seen = [];
    window.__phoenixPane.deliver = (json) => seen.push(json);
    window.__phoenixPane.drainInbox();
    expect(seen).toEqual(['{"type":"Welcome"}', '{"type":"GameStarted"}']);
    expect(window.__phoenixPane.inbox).toHaveLength(0);
  });

  it('throws once the page has stopped draining, rather than growing forever', () => {
    // This is what makes the host's reliable-overflow close reachable at all.
    // The host's own cap only sees what it has NOT handed over, so a page that
    // accepted every push and did nothing with it would never trip it — the
    // backlog would sit here instead. The throw becomes a
    // `PaneSurfaceError::Script`, which `pump_pane` requeues.
    runBoot(fragment('abcd', 'Ada'));
    let accepted = 0;
    expect(() => {
      for (let i = 0; i < 5000; i += 1) {
        window.__phoenixPaneApply(`{"n":${i}}`);
        accepted += 1;
      }
    }).toThrow(/not draining/);
    expect(accepted).toBeGreaterThan(0);
    expect(accepted).toBeLessThan(5000);
    expect(window.__phoenixPane.inbox.length).toBe(accepted);
  });

  it('never fills up while the page is draining', () => {
    // The cap is a wedged-page bound, not a throughput one.
    runBoot(fragment('abcd', 'Ada'));
    let delivered = 0;
    window.__phoenixPane.deliver = () => {
      delivered += 1;
    };
    for (let i = 0; i < 5000; i += 1) window.__phoenixPaneApply(`{"n":${i}}`);
    expect(delivered).toBe(5000);
    expect(window.__phoenixPane.inbox).toHaveLength(0);
  });
});

describe('pane_link.js — it attaches through the seam the page actually uses', () => {
  it('installs the transport factories and publishes no link of its own', async () => {
    runBoot(fragment('abcd', 'Ada'));
    installOutQueue();
    await importLink();

    expect(typeof window.PhoenixTransportFactories.socket).toBe('function');
    expect(typeof window.PhoenixTransportFactories.peer).toBe('function');
    // `currentLink()` in client.html reads a closure variable only
    // `startPhoenixJoin` assigns, so a link object published on `window` is
    // unreachable — which is exactly what the retired `connectionManager`
    // arrangement was.
    expect(window.connectionManager).toBeUndefined();
    expect(window.phoenixLink).toBeUndefined();
  });

  it('imports the channel labels rather than assuming them', () => {
    // A pane that spelled 'reliable' itself would keep working right up until
    // the transport renamed it, and then fail with a page that looks fine.
    expect(LINK).toContain(TRANSPORT_SPECIFIER);
    expect(LINK).toContain('RELIABLE_CHANNEL');
  });

  it('opens the two channels the joiner asks for, and reports ready', async () => {
    runBoot(fragment('abcd', 'Ada'));
    installOutQueue();
    await importLink();

    const { joiner, statuses } = await joinAsThePageWould();
    expect(statuses).toContain('ready');
    expect(joiner.connected).toBe(true);
    expect(RELIABLE_CHANNEL).toBe('reliable');
  });
});

describe('pane_link.js — the page joins as the pane the host minted', () => {
  it('puts the page’s own Identify on the host queue, in the wire shape', async () => {
    runBoot(fragment('3f1a6c2e-0a11-4b3c-9d55-000000000001', 'Ada'));
    const sent = installOutQueue();
    await importLink();

    // The identity the PAGE would compute, which is the one pane_boot seeded.
    await joinAsThePageWould({
      token: '3f1a6c2e-0a11-4b3c-9d55-000000000001',
      name: 'Ada',
    });

    expect(sent).toHaveLength(1);
    expect(JSON.parse(sent[0])).toEqual({
      type: 'Identify',
      data: { token: '3f1a6c2e-0a11-4b3c-9d55-000000000001', name: 'Ada' },
    });
  });

  it('never lets the compatibility handshake reach the host', async () => {
    // `JoinHandshake` is transport-plane and is not a `ClientMessage`; the host
    // decodes what arrives on this queue with `core::codec`. A pane and its host
    // are one process running one bundle, so the pane's own transport answers
    // it — there is no version to disagree about.
    runBoot(fragment('abcd', 'Ada'));
    const sent = installOutQueue();
    await importLink();
    await joinAsThePageWould({ token: 'abcd', name: 'Ada' });

    expect(sent.map((json) => JSON.parse(json).type)).toEqual(['Identify']);
  });

  it('identifies as itself even when the page computed something else', async () => {
    // Ordinarily the page computes the seeded token and this changes nothing.
    // It is here for when it cannot: storage can be refused, and a pane
    // presenting any other token is refused at `PaneBus::submit` and simply
    // never joins, with a clean log on both sides.
    runBoot(fragment('abcd', 'Ada'));
    const sent = installOutQueue();
    await importLink();

    await joinAsThePageWould({ token: 'a-token-of-its-own', name: 'Somebody Else' });
    expect(JSON.parse(sent[0]).data).toEqual({ token: 'abcd', name: 'Ada' });
  });

  it('sends other messages in the same envelope, and omits absent data', async () => {
    runBoot(fragment('abcd', 'Ada'));
    const sent = installOutQueue();
    await importLink();
    const { joiner } = await joinAsThePageWould({ token: 'abcd', name: 'Ada' });

    joiner.send('SetReady', { ready: true }, 'reliable');
    joiner.send('ReleaseStation', undefined, 'reliable');
    expect(JSON.parse(sent[1])).toEqual({ type: 'SetReady', data: { ready: true } });
    expect(JSON.parse(sent[2])).toEqual({ type: 'ReleaseStation' });
  });

  it('carries a snapshot-class send too, rather than dropping it', async () => {
    // `send(type, payload, 'snapshot')` prefers the lossy channel. Both of a
    // pane's channels are the same in-process pipe, so nothing is lost — but a
    // channel that was never opened would have swallowed it silently.
    runBoot(fragment('abcd', 'Ada'));
    const sent = installOutQueue();
    await importLink();
    const { joiner } = await joinAsThePageWould({ token: 'abcd', name: 'Ada' });

    joiner.send('SetThrust', { value: 1 }, 'snapshot');
    expect(JSON.parse(sent[1])).toEqual({ type: 'SetThrust', data: { value: 1 } });
  });
});

describe('pane_link.js — inbound crosses the same ingress boundary a phone does', () => {
  it('hands onData a localised message, through the joiner’s own deliver', async () => {
    // `localiseTree` is applied by `createRendezvousJoiner`, not by anything
    // pane-specific: server-sent string ids become display text once, at the
    // boundary, so no console has to know which of its fields are localisable.
    // That is the point of joining through the page's front door.
    runBoot(fragment('abcd', 'Ada'));
    installOutQueue();
    await importLink();
    const { received } = await joinAsThePageWould({ token: 'abcd', name: 'Ada' });

    window.__phoenixPaneApply('{"type":"Welcome","data":{"name":"Ada"}}');
    expect(received).toEqual([{ type: 'Welcome', data: { name: 'Ada' } }]);
  });

  it('delivers the backlog that arrived before the page had joined', async () => {
    // `load_url` returns before the document's own scripts are guaranteed to
    // have run, and the host starts pushing as soon as the document reports it
    // has loaded. The message most likely to be in that first batch is Welcome.
    runBoot(fragment('abcd', 'Ada'));
    installOutQueue();
    await importLink();
    window.__phoenixPaneApply('{"type":"Welcome"}');
    window.__phoenixPaneApply('{"type":"GameStarted"}');
    expect(window.__phoenixPane.inbox).toHaveLength(2);

    const { received } = await joinAsThePageWould({ token: 'abcd', name: 'Ada' });
    expect(received.map((m) => m.type)).toEqual(['Welcome', 'GameStarted']);
  });

  it('survives an undecodable message rather than wedging the link', async () => {
    runBoot(fragment('abcd', 'Ada'));
    installOutQueue();
    await importLink();
    const { received, joiner } = await joinAsThePageWould({ token: 'abcd', name: 'Ada' });

    expect(() => window.__phoenixPaneApply('not json at all')).not.toThrow();
    window.__phoenixPaneApply('{"type":"GameStarted"}');
    expect(received.map((m) => m.type)).toEqual(['GameStarted']);
    expect(joiner.connected).toBe(true);
  });

  it('stubs out the two network probes the page would otherwise wait on', async () => {
    // A pane has no WebRTC connection to configure, and a bridge machine may
    // have no network at all — both would stall the boot behind a timeout.
    runBoot(fragment('abcd', 'Ada'));
    installOutQueue();
    await importLink();

    await expect(window.fetchIceServers()).resolves.toEqual({
      servers: [],
      relayAvailable: false,
      relaySource: null,
    });
    await expect(window.probeTurnRelay()).resolves.toBe('unreachable');
  });
});
