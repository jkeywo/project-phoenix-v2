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

/**
 * How big one relayed game payload is, in the units the authored
 * `max_relay_frame_bytes` bound is written in (issue #1113).
 *
 * It lives here, beside the revision, because BOTH ends measure against that
 * bound — the client refuses an oversized frame locally so it is not cut off at
 * the ceiling, and the service refuses one so it is not made to store it — and
 * two implementations of "how big is this" is exactly the drift that turns a
 * courtesy refusal into a dropped link.
 *
 * `String.length` counts UTF-16 code units, which under-counts every non-ASCII
 * character a display name or a comms line carries, so a bound written against
 * it would silently be a different, larger bound than the authored one.
 * `TextEncoder` is present in Workers, in browsers and in Node 20+; the
 * fallback is a defence for an exotic host rather than a path anything shipped
 * takes.
 */
export function relayPayloadBytes(payload) {
  if (typeof payload !== 'string') return Infinity;
  if (typeof TextEncoder !== 'undefined') return new TextEncoder().encode(payload).length;
  return payload.length;
}
