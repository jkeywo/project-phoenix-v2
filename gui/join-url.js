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
