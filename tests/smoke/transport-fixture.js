// tests/smoke/transport-fixture.js — the seam where every PeerJS-specific
// assumption in the smoke suite is concentrated (prep for issue #1112, the
// PeerJS→Phoenix transport replacement).
//
// `fixtures.js` exposes a stable, transport-agnostic surface to specs: a
// neutral "read the host's join target off the page" helper and a
// `TestClient` façade (`send`/`waitForMessage`/`lastMessage`/`close`). This
// module is the one place that knows *how* that surface is actually
// implemented today — that the join target is a PeerJS peer id living in the
// QR link's URL hash, that connecting means `new window.Peer()` +
// `peer.connect(...)`, and that the fake transport injected for CI is
// `peerjs-shim.js` riding a `BroadcastChannel`.
//
// When #1112 swaps the real transport, the intent is that only this file and
// its fake counterpart (`peerjs-shim.js`, or whatever replaces it) need to
// change. `fixtures.js` and the ~40 spec files that import `readHostPeerId` /
// `createTestClient` from it should not.

import fs from 'fs';
import path from 'path';

/** The fake transport injected into every smoke page — see its own header
 *  comment for how it fakes `window.Peer` over BroadcastChannel, including
 *  the `window.__wasmReady` latch and the `window.__peerjsShim` sever/revive
 *  test-only control. Exported so `shim.spec.js` can unit-test it directly
 *  without going through a live page. */
export const SHIM = fs.readFileSync(path.join(__dirname, 'peerjs-shim.js'), 'utf-8');

// Stub the real transport CDN script so it can't clobber the shim's
// window.Peer, and so a slow/blocked CDN in CI can't stall page load behind
// a synchronous <script src="...">.
export const STUB_TRANSPORT_SCRIPT = `'use strict';
// No-op — window.Peer is already provided by the transport-fixture shim's
// addInitScript.
if (typeof window.Peer === 'undefined') { window.Peer = function Peer() {}; };
`;

// The URL glob the real transport library ships from today, intercepted by
// installTransportFixture() below so the stub above always wins.
const TRANSPORT_CDN_GLOB = '**/peerjs*.js';

/**
 * Install the fake transport into a fresh BrowserContext: inject the CDN
 * stub and the BroadcastChannel shim as init scripts (so they run before any
 * page script), and intercept the CDN script itself so a real network
 * fetch can't race the stub. Called once from the `context` fixture in
 * `fixtures.js` — every page created in that context inherits it.
 */
export async function installTransportFixture(ctx) {
  await ctx.addInitScript({ content: STUB_TRANSPORT_SCRIPT });
  await ctx.addInitScript({ content: SHIM });
  await ctx.route(TRANSPORT_CDN_GLOB, (route) =>
    route.fulfill({ contentType: 'application/javascript', body: STUB_TRANSPORT_SCRIPT }),
  );
}

/**
 * Read the host's join target off a live server page.
 *
 * Today that target is a PeerJS peer id embedded in the QR link's URL hash
 * (`client/index.html#<peerId>`); the #1111 brief has the host keep
 * `#qr-link.href` populated with whatever a rendezvous join code looks like,
 * so this scrape is expected to survive that change with only its docstring
 * needing an update. Treat the return value as an opaque join target, not as
 * "the peer id" — that's exactly the assumption this helper exists to hide.
 */
export async function readHostJoinTarget(serverPage) {
  await serverPage.waitForFunction(
    () => {
      const el = document.getElementById('qr-link');
      return el?.href?.includes('#');
    },
    { timeout: 20_000 },
  );
  return serverPage.evaluate(() => {
    const href = document.getElementById('qr-link').href;
    return href.split('#')[1];
  });
}

/**
 * Connect `page` (already navigated to a same-origin blank page) to the host
 * at `joinTarget` as a test client, and return the message-interaction
 * surface `createTestClient` (in `fixtures.js`) folds into the stable
 * `TestClient` façade alongside `page`/`token`/`close`.
 *
 * This is the single PeerJS-shaped chunk of the client-connect path: `new
 * window.Peer()`, `peer.connect(joinTarget)`, the reliable
 * `DataConnection`'s `on('open'|'data')`, and sending the initial `Identify`.
 * It also defines the page-side contract — `window.__messages` (the inbound
 * message log) and `window.__conn` (the reliable connection) — that the
 * three PeerJS-API-shaped specs (`shim.spec.js`, `snapshot-channel.spec.js`,
 * `reconnect-midgame-sever.spec.js`) still reach into directly from their own
 * `page.evaluate` calls; rewriting those three as behaviour tests against
 * the replacement transport is #1112's job, not this module's.
 *
 * @param {import('@playwright/test').Page} page
 * @param {string} joinTarget
 * @param {{ token: string, name: string, waitFor: string }} opts
 * @returns {Promise<{
 *   send: (type: string, data?: object) => Promise<void>,
 *   waitForMessage: (type: string, timeout?: number) => Promise<object>,
 *   lastMessage: (type: string) => Promise<object|null>,
 * }>}
 */
export async function connectTestClient(page, joinTarget, { token, name, waitFor }) {
  await page.evaluate(
    ({ joinTarget, token, name, waitFor }) =>
      new Promise((resolve, reject) => {
        window.__messages = [];
        const peer = new window.Peer();
        peer.on('open', () => {
          const conn = peer.connect(joinTarget);
          window.__conn = conn;
          conn.on('open', () => {
            conn.send(JSON.stringify({ type: 'Identify', data: { token, name } }));
          });
          conn.on('data', (raw) => {
            try { window.__messages.push(JSON.parse(raw)); } catch { /* ignore */ }
          });
        });
        const t = setInterval(() => {
          if (window.__messages?.some((m) => m.type === waitFor)) {
            clearInterval(t);
            resolve();
          }
        }, 50);
        setTimeout(() => { clearInterval(t); reject(new Error(`${waitFor} timeout (token=${token})`)); }, 15_000);
      }),
    { joinTarget, token, name, waitFor },
  );

  return {
    async send(type, data) {
      await page.evaluate(
        ({ type, data }) => {
          const msg = data !== undefined ? { type, data } : { type };
          window.__conn.send(JSON.stringify(msg));
        },
        { type, data },
      );
    },

    async waitForMessage(type, timeout = 15_000) {
      await page.waitForFunction(
        (t) => window.__messages?.some((m) => m.type === t),
        type,
        { timeout },
      );
      return page.evaluate(
        (t) => window.__messages.find((m) => m.type === t),
        type,
      );
    },

    async lastMessage(type) {
      return page.evaluate(
        (t) => {
          const msgs = window.__messages || [];
          return msgs.filter((m) => m.type === t).pop() ?? null;
        },
        type,
      );
    },
  };
}
