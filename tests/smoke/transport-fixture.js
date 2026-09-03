// tests/smoke/transport-fixture.js — the seam where every transport-specific
// assumption in the smoke suite is concentrated (issue #1112).
//
// `fixtures.js` exposes a stable, transport-agnostic surface to specs: a
// neutral "read the host's join target off the page" helper and a `TestClient`
// façade (`send`/`waitForMessage`/`lastMessage`/`close`). This module is the
// one place that knows *how* that surface is actually implemented — that the
// join target is a structured Phoenix join code living in the QR link's URL
// fragment, that connecting means a rendezvous socket, an SDP exchange, a pair
// of DataChannels and a compatibility handshake, and that the fake transport
// injected for CI is `rendezvous-shim.js`.
//
// It used to know a different set of facts: `new window.Peer()`,
// `peer.connect(peerId)` and a `peerjs-shim.js` riding a BroadcastChannel. That
// swap is the whole of #1112 as far as this suite is concerned, and it is why
// this file exists at all — `fixtures.js` and the ~40 spec files that import
// `readHostPeerId` / `createTestClient` from it did not change with it.

import fs from 'fs';
import path from 'path';

/** The fake transport injected into every smoke page — see its own header
 *  comment for how it fakes the rendezvous socket and RTCPeerConnection over
 *  BroadcastChannel, including the `window.__wasmReady` latch and the
 *  `window.__transportShim` sever/revive test-only control. */
const SHIM = fs.readFileSync(path.join(__dirname, 'rendezvous-shim.js'), 'utf-8');

/**
 * The delivery stamp the built client bundle declares
 * (`<meta name="phoenix-client-stamp">`, written by scripts/build-client.mjs).
 *
 * A test client is a blank page, not the client bundle, so it has no meta tag
 * of its own — but since #1112 the host REFUSES a joiner that presents no
 * stamp, so it cannot simply omit one. Reading the real bundle's field keeps
 * these clients honest: they present exactly what a phone presents, and a
 * protocol or content bump that stopped reaching the client page would fail
 * here too rather than being quietly waved through.
 */
export const CLIENT_STAMP = (() => {
  try {
    const html = fs.readFileSync(
      path.join(__dirname, '..', '..', 'dist', 'client', 'index.html'),
      'utf-8',
    );
    const m = /<meta\s+name="phoenix-client-stamp"\s+content="([^"]*)"/.exec(html);
    return m ? m[1] : '';
  } catch {
    return '';
  }
})();

/**
 * Install the fake transport into a fresh BrowserContext: inject the shim as
 * an init script so it runs before any page script and
 * `gui/rendezvous-transport.js` finds `window.PhoenixTransportFactories`
 * already published. Called once from the `context` fixture in `fixtures.js` —
 * every page created in that context inherits it.
 *
 * There is no CDN script to stub any more: #1112 deleted the PeerJS tag from
 * both pages, and the transport is a module island served from the same origin
 * as everything else.
 */
export async function installTransportFixture(ctx) {
  await ctx.addInitScript({ content: SHIM });
}

/**
 * Read the host's join target off a live server page.
 *
 * That target is the structured join code the rendezvous service issued,
 * embedded in the QR link's URL fragment (`client/index.html#<full code>`) —
 * the same string a phone's camera opens and a guest can paste. Treat the
 * return value as an opaque join target, not as "the peer id": that assumption
 * is exactly what this helper exists to hide, and it stopped being true in
 * #1112.
 */
export async function readHostJoinTarget(serverPage) {
  await serverPage.waitForFunction(
    () => {
      const el = document.getElementById('qr-link');
      return el?.href?.includes('#');
    },
    { timeout: 30_000 },
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
 * This is a joiner in its own right rather than a call into
 * `gui/rendezvous-transport.js`: it speaks the same rendezvous frames and the
 * same handshake, but it does NOT run `localiseTree` over inbound messages, so
 * `window.__messages` holds the raw wire shapes the specs assert on. Importing
 * the shipped module here would resolve every string id to display text and
 * silently change what ~29 spec files are asserting.
 *
 * It also defines the page-side contract specs reach into directly from their
 * own `page.evaluate` calls, BYPASSING the exported façade, and that blast
 * radius is large: `window.__messages` (the inbound message log) is
 * read/filtered/mutated directly by ~29 spec files, and `window.__conn` — the
 * reliable DataChannel — is used raw by `tactical-fire-flow.spec.js`
 * (`__conn.send`). Both names and shapes are preserved across the #1112 swap;
 * `window.__conn` is now the channel itself rather than a PeerJS
 * DataConnection, which is API-compatible for the one method that is used.
 * `window.__reliableMessages` / `window.__snapshotMessages` are new alongside
 * them: the same messages split by the channel they arrived on, which is how
 * snapshot-channel.spec.js asserts a delivery CLASS rather than merely that a
 * message turned up.
 *
 * @param {import('@playwright/test').Page} page
 * @param {string} joinTarget
 * @param {{ token: string, name: string, waitFor: string, snapshot?: boolean, stamp?: string }} opts
 *   `snapshot` (default true) negotiates the lossy unordered channel alongside
 *   the reliable one, exactly as the shipped client does. Pass `false` to model
 *   a client whose lossy channel never came up, which is what the host's
 *   per-token snapshot→reliable fallback exists for.
 * @returns {Promise<{
 *   send: (type: string, data?: object) => Promise<void>,
 *   waitForMessage: (type: string, timeout?: number) => Promise<object>,
 *   lastMessage: (type: string) => Promise<object|null>,
 * }>}
 */
export async function connectTestClient(
  page,
  joinTarget,
  { token, name, waitFor, snapshot = true, stamp = CLIENT_STAMP },
) {
  await page.evaluate(
    ({ joinTarget, token, name, waitFor, snapshot, stamp }) =>
      new Promise((resolve, reject) => {
        window.__messages = [];
        // The same inbound messages, split by the channel they arrived on, so a
        // spec can assert a delivery CLASS rather than only that a message
        // turned up. `__messages` stays the merged log every other spec reads.
        window.__reliableMessages = [];
        window.__snapshotMessages = [];
        const factories = window.PhoenixTransportFactories;
        const socket = factories.socket('https://rendezvous.test/v1/join');
        let pc = null;

        const record = (raw, log) => {
          try {
            const msg = JSON.parse(raw);
            window.__messages.push(msg);
            log.push(msg);
          } catch { /* ignore */ }
        };

        const offer = async () => {
          pc = factories.peer({ iceServers: [] });
          const conn = pc.createDataChannel('reliable', { ordered: true });
          window.__conn = conn;
          window.__snapshotConn = snapshot
            ? pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 })
            : null;
          if (window.__snapshotConn) {
            window.__snapshotConn.onmessage = (e) => record(e.data, window.__snapshotMessages);
          }
          conn.onopen = () => {
            // The host's compatibility handshake first; Identify only once it
            // has accepted this build.
            conn.send(JSON.stringify({ type: 'JoinHandshake', data: { stamp } }));
          };
          conn.onmessage = (e) => {
            let msg = null;
            try { msg = JSON.parse(e.data); } catch { return; }
            if (msg.type === 'JoinAccepted') {
              conn.send(JSON.stringify({ type: 'Identify', data: { token, name } }));
              return;
            }
            if (msg.type === 'JoinRefused') {
              reject(new Error(`host refused this client: ${msg.data?.code} ${msg.data?.detail || ''}`));
              return;
            }
            window.__messages.push(msg);
            window.__reliableMessages.push(msg);
          };
          const description = await pc.createOffer();
          await pc.setLocalDescription(description);
          socket.send(JSON.stringify({ v: 1, type: 'signal', payload: { sdp: pc.localDescription } }));
        };

        socket.onmessage = async (e) => {
          const msg = JSON.parse(e.data);
          if (msg.type === 'ready') {
            socket.send(JSON.stringify({ v: 1, type: 'join', code: joinTarget }));
          } else if (msg.type === 'joined') {
            await offer();
          } else if (msg.type === 'signal' && msg.payload?.sdp) {
            await pc.setRemoteDescription(msg.payload.sdp);
          } else if (msg.type === 'error' || msg.type === 'closed') {
            reject(new Error(`rendezvous refused the join: ${msg.reason}`));
          }
        };

        const t = setInterval(() => {
          if (window.__messages?.some((m) => m.type === waitFor)) {
            clearInterval(t);
            resolve();
          }
        }, 50);
        setTimeout(() => { clearInterval(t); reject(new Error(`${waitFor} timeout (token=${token})`)); }, 15_000);
      }),
    { joinTarget, token, name, waitFor, snapshot, stamp },
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
