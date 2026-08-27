/**
 * gui/rendezvous-protocol.js — the one declaration of the rendezvous frame
 * vocabulary's revision (issue #1111).
 *
 * Both ends of the join path hard-refuse a frame whose `v` is not this number
 * (worker-rendezvous/src/registry.js `receive`), so two copies of the literal
 * held together by a comment would make a total join outage one careless edit
 * away. #1114 (server-code joining) and #1115 (code rotation) both extend this
 * vocabulary, which is exactly when that edit happens.
 *
 * This module is deliberately dependency-free — no DOM, no strings, no join
 * table — because the Cloudflare Worker bundles it through
 * worker-rendezvous/src/registry.js and must not drag the browser client's
 * module graph in with it.
 */

/** Frame-vocabulary revision. Bump only for an incompatible change. */
export const RENDEZVOUS_PROTOCOL = 1;
