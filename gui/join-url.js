/**
 * gui/join-url.js — where a join code sends a phone (issues #1111, #1112,
 * #1329).
 *
 * Two values, lifted out of `gui/rendezvous-transport.js` so that a document
 * which needs to know where a code points does not have to load the whole crew
 * transport to find out. That document is the native host's lobby surface: it
 * shows the join QR (issue #1329) and speaks no WebRTC at all — its host does
 * the transport in Rust — so importing the transport module there would pull in
 * the socket, the peer factory, the levers and the string table for one pure
 * string join.
 *
 * `gui/rendezvous-transport.js` re-exports both, so every existing importer and
 * `window.rendezvousTransport.joinUrlForCode` keep working. This is the
 * definition; that is the doorway.
 */

/**
 * The dev rendezvous service. One hardcoded literal, in the same shape as the
 * TURN worker's at gui/connection-manager.js — keep it a whole string rather
 * than building it from parts, because a deploy-time URL sweep can only find a
 * literal.
 *
 * NOTE, and this is the honest state of it: no such sweep exists for THIS
 * literal yet, and worker-rendezvous/ has never been deployed.
 * .github/workflows/deploy-demo.yml sweeps `DEV_TURN_URL` only. Both are open
 * items in docs/delivery-checklist.md §3a, and since #1112 they are BLOCKING
 * rather than cosmetic: PeerJS is gone, so a build pointed at a service that is
 * not there has no join path at all.
 */
export const DEV_RENDEZVOUS_URL = 'https://phoenix-rendezvous.project-phoenix.workers.dev';

/**
 * The origins the BROWSER game is published at.
 *
 * The twin of `ALLOWED_ORIGIN` in `worker-rendezvous/wrangler.toml`, and it has
 * to stay the twin: that variable is the list of origins the cloud rendezvous
 * will accept a socket from, so a page on any other origin could not use that
 * service even if it tried. `tests/client/join-url.test.js` reads the wrangler
 * file and fails if the two lists drift.
 *
 * pp-dev is the canonical custom domain, the github.io origin covers direct
 * Pages access, and the three localhost ports are the dev servers that serve
 * THIS bundle: 3000 is the smoke suite's `serve dist`, 3911 the local
 * dist-preview server and 8080 `trunk serve`.
 *
 * One caveat worth knowing, because 8080 is also `phoenix-host`'s own default
 * delivery port: a browser that opens a native host at `http://localhost:8080`
 * is on a known web origin and will dial the cloud service, not the host in
 * front of it. Every phone gets the LAN address from the QR instead, which is
 * not on this list; an operator who wants the same on their own machine can use
 * the LAN address too.
 */
export const KNOWN_WEB_ORIGINS = Object.freeze([
  'https://pp-dev.kiwigamedesign.co.uk',
  'https://jkeywo.github.io',
  'http://localhost:3000',
  'http://localhost:3911',
  'http://localhost:8080',
]);

/**
 * Which rendezvous service a page on `pageOrigin` should dial (issue #1353).
 *
 * **The service that served you the page is the service you dial.** A native
 * host now accepts join sockets on its own delivery port
 * (`src/native_host/direct_join.rs`), so a phone that loaded the bundle from
 * `http://192.168.1.5:8080` opens `ws://192.168.1.5:8080/v1/join` and the LAN
 * game needs no external service at all. A page served from one of the
 * {@link KNOWN_WEB_ORIGINS} was served by a static host that cannot accept a
 * socket, so it keeps the built-in cloud service.
 *
 * This deliberately SUPERSEDES the `?rendezvous=` posture question (issue
 * #1336) for the served-by-a-host case: there is no parameter, so there is no
 * link somebody can be handed that points their join somewhere else. The
 * parameter survives as what it always was, a loopback-only development lever
 * (`rendezvousBaseFromLocation` in `gui/rendezvous-transport.js`), and it still
 * wins where it is honoured.
 *
 * A page with no usable origin — `file://`, a sandboxed iframe's `null` — gets
 * the built-in service: it was not served by anything dialable.
 */
export function rendezvousBaseForOrigin(pageOrigin, defaultBase = DEV_RENDEZVOUS_URL) {
  const origin = String(pageOrigin || '').trim().replace(/\/+$/, '');
  if (!/^https?:\/\/[^/]+$/i.test(origin)) return defaultBase;
  return KNOWN_WEB_ORIGINS.includes(origin.toLowerCase()) ? defaultBase : origin;
}

/**
 * The link a QR encodes and a guest reads aloud: the client page with the full
 * structured code in the fragment. There is no QR *scanner* in the product —
 * the phone's own camera opens this URL — so "QR entry" and "pasted full code"
 * are the same string arriving by two routes.
 *
 * `pageHref` is the page the client bundle sits BESIDE, and it is what makes
 * this one function serve both hosts. A browser host passes its own
 * `location.href`, so a phone is sent back to the origin the operator is
 * already on. A native host has no such page in front of it, so its lobby
 * surface is handed the address its OWN delivery server is reachable at
 * (`native_host::host_lobby::join`) — never the loopback address the embedded
 * view itself loaded from, which is the one URL in the building a phone cannot
 * open.
 */
export function joinUrlForCode(pageHref, fullCode, base = DEV_RENDEZVOUS_URL) {
  const dir = String(pageHref).replace(/[?#].*$/, '').replace(/[^/]*$/, '');
  // Only a non-default service needs saying: the built-in one is what a bare
  // structured code already implies, and a shorter URL is a shorter QR.
  const search = base && base !== DEV_RENDEZVOUS_URL
    ? `?rendezvous=${encodeURIComponent(base)}`
    : '';
  return `${dir}client/index.html${search}#${fullCode}`;
}
