/**
 * gui/transport-levers.js — pin the crew transport to one path (issue #1113).
 *
 * Phoenix has three ways onto the wire, and in the field they are tried in this
 * order, each a fallback for the last:
 *
 *   direct    WebRTC over host/srflx candidates — LAN, or anything with a
 *             workable NAT. No relay of any kind in the path.
 *   turn      WebRTC over a TURN relay candidate. The CGNAT/hotspot case, and
 *             what the credential worker exists for.
 *   ws-relay  the game's own frames over the rendezvous WebSocket, when neither
 *             of the above can be built at all — see gui/rendezvous-relay.js.
 *
 * The trouble with a fallback chain is that the interesting paths are the ones
 * you cannot get onto deliberately: on a healthy network every join takes the
 * first rung and the other two are never exercised. So this module turns the
 * chain into something a test — and a human on a real network, working through
 * docs/acceptance/1113-networks.md — can pin.
 *
 * It is a URL lever rather than a build flag on purpose. The acceptance kit
 * needs to force a path against the DEPLOYED service from an ordinary phone,
 * with no build and no console, and `?forceRelay=1` on the end of a join link
 * is the whole of what that takes.
 *
 * ## Why forcing is safe to ship
 *
 * Every value here can only make the transport try LESS than it otherwise
 * would: pinning TURN-only refuses host candidates, pinning ws-relay skips the
 * WebRTC ladder, pinning direct withholds the relay servers AND refuses the
 * ws-relay fallback. None of them reaches a host, a peer or a payload a default
 * load could not, so the worst a hostile link can do with one is give its
 * recipient a worse connection than they would have had — visibly, because a
 * pinned transport is named in the diagnostics readout on both pages.
 *
 * Pure and dependency-free, so both pages, the transport, the smoke suite and
 * the vitest suites read one implementation of what a lever means.
 */

/** The default: try every rung, in order, falling back as each fails. */
export const TRANSPORT_AUTO = 'auto';
/**
 * Direct WebRTC only — no TURN candidates, no WebSocket fallback.
 *
 * Made real rather than merely named: `iceTransportPolicy` has no 'no-relay'
 * value, so the only way to keep ICE off a TURN allocation is to hand the peer
 * connection NO ice servers at all, which is what `useIceServers: false`
 * below is for. Before that this mode gave `iceTransportPolicy: 'all'` —
 * identical to auto — so ICE was free to pick a relayed pair, and the one
 * lever whose whole job is proving a direct link proved nothing.
 */
export const TRANSPORT_DIRECT = 'direct';
/** WebRTC, but only over a TURN relay candidate. */
export const TRANSPORT_TURN = 'turn';
/** The WebSocket game relay, straight away, with no WebRTC attempt at all. */
export const TRANSPORT_WS_RELAY = 'ws-relay';

const MODES = new Set([TRANSPORT_AUTO, TRANSPORT_DIRECT, TRANSPORT_TURN, TRANSPORT_WS_RELAY]);

/** `?flag`, `?flag=1`, `?flag=true`, `?flag=on`, `?flag=yes` — all true. */
function flagIsOn(value) {
  if (value === null || value === undefined) return false;
  const v = String(value).trim().toLowerCase();
  return v === '' || v === '1' || v === 'true' || v === 'on' || v === 'yes';
}

/**
 * Which transport path this page load is pinned to, if any.
 *
 * `?transport=` is the full lever and takes the four mode names above.
 * `?forceRelay` is the shorthand the acceptance kit and the smoke suite both
 * use for the TURN-only case, because that is the one a human needs on a phone
 * and "forceRelay" is what the issue named it. An unrecognised value is ignored
 * rather than guessed at — an old link must open the game, not pin it to a mode
 * that does not exist.
 *
 * @param {string} search `location.search`
 * @returns {{
 *   mode: string,
 *   iceTransportPolicy: 'all'|'relay',
 *   useIceServers: boolean,
 *   wsRelay: 'auto'|'only'|'off',
 *   pinned: boolean,
 * }}
 */
export function transportLeversFromLocation(search) {
  const params = new URLSearchParams(search || '');
  let mode = TRANSPORT_AUTO;

  const asked = (params.get('transport') || '').trim().toLowerCase();
  if (MODES.has(asked)) mode = asked;
  // The shorthand wins over `?transport=auto` (an explicit auto is the default
  // anyway) but never over an explicit non-auto mode, so a link carrying both
  // has one obvious reading rather than an ordering puzzle.
  if (mode === TRANSPORT_AUTO && flagIsOn(params.get('forceRelay'))) mode = TRANSPORT_TURN;

  return {
    mode,
    // The RTCPeerConnection config field. 'relay' makes the browser discard
    // host and server-reflexive candidates, so any pair that forms is over a
    // TURN allocation — which is what "prove the relay path works" means.
    iceTransportPolicy: mode === TRANSPORT_TURN ? 'relay' : 'all',
    // Whether the STUN/TURN server list reaches the peer connection at all.
    // False ONLY for the direct pin, and it is what makes that pin real: with
    // no TURN server configured there is no allocation to make and no relay
    // candidate to gather, so any pair ICE forms is host or server-reflexive.
    // (STUN goes with it. A pin whose point is "no relay in the path" is a LAN
    // check, and the honest counterpart to forceRelay's TURN-only.)
    useIceServers: mode !== TRANSPORT_DIRECT,
    // Whether the WebSocket game relay may be used, and whether it is the only
    // thing to try. 'off' for the two WebRTC modes, because a fallback that
    // fires would hide the very failure the pin exists to expose.
    wsRelay:
      mode === TRANSPORT_WS_RELAY ? 'only'
        : mode === TRANSPORT_AUTO ? 'auto'
          : 'off',
    pinned: mode !== TRANSPORT_AUTO,
  };
}

/** The lever's own defaults, for a caller with no URL to read. */
export function defaultTransportLevers() {
  return transportLeversFromLocation('');
}

// Published for the classic-script halves of both pages, which cannot import.
if (typeof window !== 'undefined') {
  window.transportLevers = {
    TRANSPORT_AUTO,
    TRANSPORT_DIRECT,
    TRANSPORT_TURN,
    TRANSPORT_WS_RELAY,
    transportLeversFromLocation,
    defaultTransportLevers,
  };
}
