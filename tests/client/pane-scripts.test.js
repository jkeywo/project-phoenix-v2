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
 * what the page's own `handleMessage` is handed. `pane_link.js` reimplements a
 * contract `gui/connection-manager.js` has a Vitest suite for; this is that
 * suite's counterpart.
 *
 * Both files are read off disk and evaluated here rather than imported, because
 * that is how they run in production: the boot script is a classic `<script>`
 * at the top of `<head>` (it has to see the fragment before the page's own
 * inline script wants the token), and the link is a module injected before
 * `</body>`.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const PANES = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../src/native_host/panes',
);
const BOOT = readFileSync(path.join(PANES, 'pane_boot.js'), 'utf8');
const LINK = readFileSync(path.join(PANES, 'pane_link.js'), 'utf8');

/** The import specifier the link uses for the string table. */
const STRINGS_SPECIFIER = "'./gui/strings.js'";

/**
 * Evaluate the boot script the way the document does: a classic script, in the
 * page's global scope, with `location.hash` already set.
 */
function runBoot(fragment) {
  window.location.hash = fragment;
  // eslint-disable-next-line no-new-func
  new Function(BOOT)();
}

/**
 * Import the link module with its one dependency stubbed.
 *
 * `localiseTree` is replaced by a marker so the assertion below is about
 * *whether the link applies it*, not about what the real string table does with
 * a payload. The specifier it replaces is asserted separately — the relative
 * `./gui/strings.js` only resolves because the pane document is published at
 * the client directory's own depth, which is the whole reason for that path.
 */
async function importLink() {
  const stub =
    'data:text/javascript,' +
    encodeURIComponent(
      'export const localiseTree = (value) => ({ localised: true, value });',
    );
  const source =
    LINK.replace(STRINGS_SPECIFIER, JSON.stringify(stub)) +
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

beforeEach(() => {
  delete window.__phoenixPane;
  delete window.__phoenixPaneApply;
  delete window.phoenixPaneOut;
  delete window.connectionManager;
  window.sessionStorage.clear();
  window.location.hash = '';
});

describe('pane_boot.js — the identity comes out of the fragment', () => {
  it('reads the token and name the host put in location.hash', () => {
    // The point of the fragment: a browser never transmits it, so a live
    // participant's session token is in no byte the host serves. The boot
    // script is where it re-enters the page.
    runBoot('#native&token=3f1a6c2e-0a11-4b3c-9d55-000000000001&name=Ada');
    expect(window.__phoenixPane.token).toBe('3f1a6c2e-0a11-4b3c-9d55-000000000001');
    expect(window.__phoenixPane.name).toBe('Ada');
  });

  it('decodes the escaped underscore the host had to introduce', () => {
    // `joinRouteFromLocation` reads an underscore in the fragment as a
    // rendezvous join code, so `fragment_encode` escapes it as %5F. A
    // participant called `ada_lovelace` must still join as one.
    runBoot('#native&token=tok%5Fen&name=ada%5Flovelace');
    expect(window.__phoenixPane.token).toBe('tok_en');
    expect(window.__phoenixPane.name).toBe('ada_lovelace');
  });

  it('decodes an apostrophe, a space and non-ASCII without losing the other field', () => {
    runBoot("#native&token=a%20b&name=O%27Neil%C3%A9");
    expect(window.__phoenixPane.token).toBe('a b');
    expect(window.__phoenixPane.name).toBe("O'Neilé");
  });

  it('keeps one malformed escape from costing the other field', () => {
    // `decodeURIComponent` throws on a lone `%`, and losing the token because
    // the name was mistyped would be a pane that silently never joins.
    runBoot('#native&token=abcd&name=%E0%A4%A');
    expect(window.__phoenixPane.token).toBe('abcd');
    expect(window.__phoenixPane.name).toBe('');
  });

  it('ignores the join-route prefix and anything it does not own', () => {
    runBoot('#native&token=abcd&name=Ada&rendezvous=nope');
    expect(window.__phoenixPane.token).toBe('abcd');
    expect(Object.keys(window.__phoenixPane)).toContain('inbox');
    expect(window.__phoenixPane.rendezvous).toBeUndefined();
  });

  it('seeds the session-token key gui/session-token.js reads', () => {
    // Belt to the link's braces: the link pins both values directly when it
    // sends Identify, but the page computes a token of its own at parse time
    // if this key is empty.
    runBoot('#native&token=abcd&name=Ada');
    expect(window.sessionStorage.getItem('session-token')).toBe('abcd');
    expect(window.sessionStorage.getItem('player-name')).toBe('Ada');
  });
});

describe('pane_boot.js — the inbox is bounded', () => {
  it('holds what arrives before the link installs its delivery function', () => {
    runBoot('#native&token=abcd&name=Ada');
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
    runBoot('#native&token=abcd&name=Ada');
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
    runBoot('#native&token=abcd&name=Ada');
    let delivered = 0;
    window.__phoenixPane.deliver = () => {
      delivered += 1;
    };
    for (let i = 0; i < 5000; i += 1) window.__phoenixPaneApply(`{"n":${i}}`);
    expect(delivered).toBe(5000);
    expect(window.__phoenixPane.inbox).toHaveLength(0);
  });
});

describe('pane_link.js — the page joins as the pane the host minted', () => {
  it('imports localiseTree by a path that only resolves at the client depth', () => {
    // The pane document is published at `/client/pane-<n>-<nonce>.html` so that
    // every relative URL in the page resolves exactly as it does for a phone.
    // This import is one of them.
    expect(LINK).toContain(STRINGS_SPECIFIER);
  });

  it('sends exactly the Identify wire shape, on the host-minted values', async () => {
    runBoot('#native&token=3f1a6c2e-0a11-4b3c-9d55-000000000001&name=Ada');
    const sent = installOutQueue();
    await importLink();

    window.connectionManager.connect('ignored-host-id', {});
    expect(sent).toHaveLength(1);
    expect(JSON.parse(sent[0])).toEqual({
      type: 'Identify',
      data: { token: '3f1a6c2e-0a11-4b3c-9d55-000000000001', name: 'Ada' },
    });
  });

  it('identifies as itself even when the page computed something else', async () => {
    // The host refuses any other token at the bus, so presenting the page's own
    // would simply never join — this is why the link pins both values rather
    // than reading them back out of sessionStorage.
    runBoot('#native&token=abcd&name=Ada');
    window.sessionStorage.setItem('session-token', 'a-token-of-its-own');
    const sent = installOutQueue();
    await importLink();

    window.connectionManager.connect('ignored', {});
    expect(JSON.parse(sent[0]).data.token).toBe('abcd');
  });

  it('sends other messages in the same envelope, and omits absent data', async () => {
    runBoot('#native&token=abcd&name=Ada');
    const sent = installOutQueue();
    await importLink();
    window.connectionManager.connect('ignored', {});

    window.connectionManager.send('SetReady', { ready: true });
    window.connectionManager.send('ReleaseStation');
    expect(JSON.parse(sent[1])).toEqual({ type: 'SetReady', data: { ready: true } });
    expect(JSON.parse(sent[2])).toEqual({ type: 'ReleaseStation' });
  });

  it('reports ready on connect and stays connected, because there is no socket', async () => {
    // A pane that has lost its host has lost its process. `retryNow` and
    // `disconnect` exist because the page calls them, and must not throw.
    runBoot('#native&token=abcd&name=Ada');
    installOutQueue();
    await importLink();

    const statuses = [];
    expect(window.connectionManager.connected).toBe(true);
    window.connectionManager.connect('ignored', { onStatus: (s) => statuses.push(s) });
    expect(statuses).toEqual(['ready']);
    expect(() => window.connectionManager.retryNow()).not.toThrow();
    expect(() => window.connectionManager.disconnect()).not.toThrow();
    expect(window.connectionManager.connected).toBe(true);
  });
});

describe('pane_link.js — inbound crosses the same ingress boundary a phone does', () => {
  it('hands onData localiseTree(JSON.parse(json))', async () => {
    // The transform `gui/connection-manager.js` owns for a phone: server-sent
    // string ids become display text once, at the boundary, so no console has
    // to know which of its fields are localisable.
    runBoot('#native&token=abcd&name=Ada');
    installOutQueue();
    await importLink();

    const received = [];
    window.connectionManager.connect('ignored', { onData: (m) => received.push(m) });
    window.__phoenixPaneApply('{"type":"Welcome","data":{"name":"Ada"}}');

    expect(received).toEqual([
      { localised: true, value: { type: 'Welcome', data: { name: 'Ada' } } },
    ]);
  });

  it('delivers the backlog that arrived before the modules had run', async () => {
    // `load_url` returns before the document's own scripts are guaranteed to
    // have run, and the host starts pushing as soon as the document reports it
    // has loaded. The message most likely to be in that first batch is Welcome.
    runBoot('#native&token=abcd&name=Ada');
    installOutQueue();
    window.__phoenixPaneApply('{"type":"Welcome"}');
    window.__phoenixPaneApply('{"type":"GameStarted"}');
    await importLink();

    const received = [];
    window.connectionManager.connect('ignored', { onData: (m) => received.push(m) });
    expect(received.map((m) => m.value.type)).toEqual(['Welcome', 'GameStarted']);
  });

  it('survives an undecodable message rather than wedging the link', async () => {
    runBoot('#native&token=abcd&name=Ada');
    installOutQueue();
    await importLink();

    const received = [];
    window.connectionManager.connect('ignored', { onData: (m) => received.push(m) });
    expect(() => window.__phoenixPaneApply('not json at all')).not.toThrow();
    window.__phoenixPaneApply('{"type":"GameStarted"}');
    expect(received.map((m) => m.value.type)).toEqual(['GameStarted']);
  });

  it('stubs out the two network probes the page would otherwise wait on', async () => {
    // A pane has no WebRTC connection to configure, and a bridge machine may
    // have no network at all — both would stall the boot behind a timeout.
    runBoot('#native&token=abcd&name=Ada');
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
