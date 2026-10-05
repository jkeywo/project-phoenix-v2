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


/**
 * Optional frozen-fleet continuity vocabulary (#1534), additive to v1.
 *
 * Only a server-namespace record's current owner may register:
 *   fleet-successors {frozen:true, epoch:0, owner_slot, members:[{peer,slot}]}
 * The owner attests its already-admitted frozen roster. The service checks
 * joined connection ownership and distinct positive u32 slots, not game state.
 * At most max_peers_per_record members are retained, plus the owner. Successful
 * registration answers fleet-successors-set {epoch:0}; each authorized socket,
 * including the owner, privately receives fleet-capability {epoch,slot,capability}.
 * Repeating the same registration is idempotent; a changed roster is refused.
 *
 * Socket loss emits fleet-owner-lost {epoch} BEFORE relay-closed to delegates.
 * Their join sockets/presence survive, but relay mailboxes do not. A healthy
 * direct link need not migrate merely because its signalling socket dropped.
 * Simulation/link liveness remains the adapter's decision: no automatic takeover.
 *
 * A surviving delegate sends fleet-takeover {epoch,capability}. Only the lowest
 * currently connected authorized slot may reserve takeover while no host exists.
 * Other authorized candidates receive fleet-takeover-wait {epoch,slot}; the
 * winner privately receives fleet-takeover-grant {epoch:next,slot,suffix,capability}.
 * This NEW one-use grant reserves the original grace window and blocks reclaim.
 * It never contains or reuses the original owner's reclaim secret.
 *
 * The winner opens /v1/host and sends host-open {namespace:'server',version,
 * transports,takeover:{suffix,epoch,capability}}. Binding commits the next epoch,
 * preserves code/namespace/admission, rotates code.secret and responds hosted
 * with private fleet:{epoch,slot,capability} (the winner's original member proof).
 * Peers receive fleet-owner-changed {epoch,slot}, with no capabilities. A failed
 * takeover never silently mints another code. Explicit close, expiry or Durable
 * Object eviction still destroys the record and all proofs.
 *
 * Rejoining delegates use join {code,namespace:'server',continuation:
 * {epoch,slot,capability}}. A connected holder cannot be displaced; simultaneous
 * replacements therefore have one winner. Valid continuation passes closed
 * admission. joined, peer-joined and relay-peer carry only continuation:{epoch,
 * slot}; the new owner can bind that authenticated connection to its frozen
 * simulation identity without learning another member's private capability.
 *
 * A delegate that missed owner-changed can send fleet-state {code,namespace:
 * 'server',continuation:{slot,capability}}. Its private response is fleet-state
 * {epoch,owner_slot,available}. Stale continuation/takeover epochs receive this
 * state followed by error reason stale-fleet-epoch. Other stable refusals are
 * not-hosting-fleet, invalid-member, already-configured, forbidden-takeover,
 * host-present, takeover-pending, forbidden-resume, forbidden-continuation and
 * slot-connected, plus ordinary malformed/too-many-attempts. Proof failures
 * never echo submitted capabilities, nor expose them through diagnostics.
 *
 * Ordinary records without fleet-successors retain the existing reclaim and
 * relay-close behavior. The service does not select a simulation watermark,
 * merge frame histories, restore snapshots or guarantee partition reunification.
 */
