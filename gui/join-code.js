import { JOIN_CODE_FORMAT_VERSION, NAMESPACE_CLIENT, NAMESPACE_SERVER, checkJoinCodeFormat, setJoinCodeData, getJoinCodeData, loadJoinCodeData as loadTransportJoinCodeData, canonicaliseSuffix, deniedSuffixes, isDenied, validateSuffix, projectGuidFor, namespaceOf, versionGuid, composeJoinCode, joinCodeForSuffix, parseJoinCode, mintSuffix } from '../packages/transport/src/join-code.js';
export * from '../packages/transport/src/join-code.js';
export function loadJoinCodeData(url) {
  return loadTransportJoinCodeData(url || new URL('../assets/join/join-codes.json', import.meta.url));
}


/**
 * The single map from a machine reason to a `strings.csv` id, shared by the
 * phone entry field and the host page so one failure never gets two wordings.
 *
 * Three sources feed it, and all three must be covered or a real refusal
 * renders as the misleading `unknown` fallback — "No ship is using that code",
 * which sends a guest back to re-type a code that was already right:
 *
 *   1. this module — `validateSuffix`, `parseJoinCode`;
 *   2. the rendezvous service's `error` frames (worker-rendezvous/src/registry.js).
 *      tests/client/join-code.test.js reads the reasons that module actually
 *      emits out of its source and demands a row for each, rather than
 *      iterating this map's own keys: a coverage test driven by the map can
 *      only ever agree with itself, which is how every relay refusal #1113
 *      added — `relay-full`, `relay-too-large`, `not-relaying`, `no-peer` —
 *      shipped unmapped underneath a green test;
 *   3. the HOST's authoritative compatibility verdict, whose codes are Rust's
 *      `StampMismatch::code()` (src/delivery/stamp.rs) plus the native host's
 *      token refusal codes (src/native_host/relay_transport.rs), relayed
 *      verbatim through `JoinRefused`. tests/client/join-code.test.js pins that
 *      list too, so adding a variant there without a row here fails the
 *      editor-test job.
 */
const REASON_STRING_IDS = {
  empty: 'client.join.error_empty',
  length: 'client.join.error_length',
  charset: 'client.join.error_charset',
  denied: 'client.join.error_denied',
  malformed: 'client.join.error_malformed',
  // A GUID belonging to no Phoenix namespace at all is NOT "the other typed
  // namespace" — saying "that is a fleet code" about it would be a false
  // statement about somebody else's identifier.
  'unknown-project': 'client.join.error_not_phoenix',
  'unknown-namespace': 'client.join.error_not_phoenix',
  unknown: 'client.join.error_unknown',
  'wrong-type': 'client.join.error_wrong_type',
  'version-mismatch': 'client.join.error_version',
  'not-joinable': 'client.join.error_wrong_type',
  'admission-closed': 'client.join.error_closed',
  'host-gone': 'client.join.error_host_gone',
  // ── The fleet owner's own answers (gui/host-mesh.js, issue #1114) ─────────
  // Host-to-host refusals, never seen by a phone — but they are machine
  // reasons on the same wire vocabulary and the host page renders them through
  // this same map, so a fleet refusal cannot be the one failure in the project
  // that arrives as raw English.
  'fleet-full': 'server.fleet.error_full',
  'recovery-only': 'server.fleet.error_frozen',
  exhausted: 'client.join.error_exhausted',
  unreachable: 'client.join.error_unreachable',
  'too-many-attempts': 'client.join.error_too_many',
  'unsupported-protocol': 'client.join.error_version',
  // ── The WebSocket game relay's refusals (issue #1113) ─────────────────────
  // A relay that is full is the ONE degraded state a guest actually meets: a
  // correct code, a live host, and a network so restrictive that the service is
  // already carrying its authored maximum of phones for that ship. The remedy
  // is somebody else disconnecting or a different network, and neither is
  // discoverable from "check the viewscreen and try again".
  'relay-full': 'client.join.error_relay_full',
  // The rest are link-level: the service would not carry one frame, or has
  // stopped carrying this connection. The joiner retries them, so the sentence
  // says the service rather than the code.
  'relay-too-large': 'client.join.error_relay_lost',
  'not-relaying': 'client.join.error_relay_lost',
  'already-relaying': 'client.join.error_relay_lost',
  'no-peer': 'client.join.error_relay_lost',
  'host-closed': 'client.join.error_relay_lost',
  'relay-overflow': 'client.join.error_relay_lost',
  // The service and this page disagree about what state this connection is in
  // — it was swept, or a frame arrived out of order. Nothing to do but reload.
  'not-joined': 'client.join.error_unreachable',
  'not-connected': 'client.join.error_unreachable',
  'forbidden-role': 'client.join.error_unreachable',
  // HOST-side refusals. A phone cannot produce either — they answer a `host-open`
  // — but they are mapped rather than left to the fallback so that the coverage
  // test's demand is met by a DECISION about each reason rather than by silence.
  'already-hosting': 'client.join.error_unknown',
  'not-hosting': 'client.join.error_unknown',
  // Frozen-fleet continuity can refuse a correct code for reasons unrelated to
  // code lookup. The host-only reasons point straight to host copy; the shared
  // reasons below have host overrides so each surface tells the truth.
  'host-present': 'client.join.error_fleet_host_present',
  'takeover-pending': 'server.fleet.error_takeover_pending',
  'forbidden-resume': 'server.fleet.error_forbidden_resume',
  'not-hosting-fleet': 'server.fleet.error_not_hosting',
  'invalid-member': 'server.fleet.error_invalid_member',
  'already-configured': 'server.fleet.error_already_configured',
  'forbidden-continuation': 'client.join.error_forbidden_continuation',
  'forbidden-takeover': 'client.join.error_forbidden_takeover',
  'stale-fleet-epoch': 'client.join.error_stale_fleet_epoch',
  'slot-connected': 'client.join.error_slot_connected',
  // ── The host's own verdict (StampMismatch::code()) ────────────────────────
  // A protocol difference and a content difference have the same fix for the
  // player — reload this page against the ship they are joining — but they are
  // two different sentences, because "different version" is a lie when the
  // build is right and the mission content is not.
  'protocol-mismatch': 'client.join.error_version',
  'content-id-mismatch': 'client.join.error_content',
  'content-epoch-mismatch': 'client.join.error_content',
  'bundle-content-missing': 'client.join.error_content',
  'client-stamp-missing': 'client.join.error_stamp_missing',
  // Not a StampMismatch: the native host refuses a peer that claims a token
  // only the host runtime may use (`__local_console__`, the `ai:` prefix).
  'reserved-token': 'client.join.error_reserved_token',
  'invalid-token': 'client.join.error_invalid_token',
};

/** The two surfaces a refusal can be read on. */
export const SURFACE_CLIENT = 'client';
export const SURFACE_SERVER = 'server';

/**
 * The rows a HOST operator gets instead, when the phone's sentence would be
 * wrong rather than merely terse (issue #1114).
 *
 * The map above is worded for the one surface that had refusals until #1114: a
 * phone, joining a ship's crew. Read out on a viewscreen through
 * `server.fleet.error_joining`, several of those sentences are actively
 * misleading — a ship host that types the lead's CREW code into the fleet field
 * is told "That is a fleet code", the exact inverse of what happened, and a
 * closed fleet is described as a closed crew list nobody was joining.
 *
 * Only the reasons whose wording DEPENDS on the surface are listed. Everything
 * else ("Enter a code first.", "A join code is five letters.") says the same
 * true thing on either screen and falls through to the one map, so there is no
 * second copy of it to drift.
 */
const SERVER_STRING_IDS = {
  'wrong-type': 'server.fleet.error_wrong_type',
  'not-joinable': 'server.fleet.error_wrong_type',
  'admission-closed': 'server.fleet.error_closed',
  unknown: 'server.fleet.error_unknown',
  'host-gone': 'server.fleet.error_host_gone',
  'version-mismatch': 'server.fleet.error_version',
  'protocol-mismatch': 'server.fleet.error_version',
  'content-id-mismatch': 'server.fleet.error_content',
  'content-epoch-mismatch': 'server.fleet.error_content',
  'bundle-content-missing': 'server.fleet.error_content',
  'client-stamp-missing': 'server.fleet.error_stamp_missing',
  'host-present': 'server.fleet.error_host_present',
  'forbidden-takeover': 'server.fleet.error_forbidden_takeover',
  'stale-fleet-epoch': 'server.fleet.error_stale_fleet_epoch',
};

/**
 * String id for a machine reason; a reason with no row falls back to unknown.
 *
 * @param {string} reason machine reason
 * @param {string} [surface] {@link SURFACE_CLIENT} (the default, a phone's join
 *   screen) or {@link SURFACE_SERVER} (a host page's fleet panel).
 */
export function reasonStringId(reason, surface = SURFACE_CLIENT) {
  const key = String(reason == null ? '' : reason);
  if (surface === SURFACE_SERVER && SERVER_STRING_IDS[key]) return SERVER_STRING_IDS[key];
  return REASON_STRING_IDS[key] || REASON_STRING_IDS.unknown;
}

/** Every reason this build can put on screen. Read by the coverage tests. */
export function knownReasons() {
  return Object.keys(REASON_STRING_IDS);
}

/** Every reason the HOST surface words differently. Read by the coverage tests. */
export function serverSurfaceReasons() {
  return Object.keys(SERVER_STRING_IDS);
}

// Expose for classic-script consumers (client.html / server.html are not modules).
if (typeof window !== 'undefined') {
  window.joinCode = {
    JOIN_CODE_FORMAT_VERSION,
    NAMESPACE_CLIENT,
    NAMESPACE_SERVER,
    SURFACE_CLIENT,
    SURFACE_SERVER,
    checkJoinCodeFormat,
    setJoinCodeData,
    getJoinCodeData,
    loadJoinCodeData,
    canonicaliseSuffix,
    deniedSuffixes,
    isDenied,
    validateSuffix,
    projectGuidFor,
    namespaceOf,
    versionGuid,
    composeJoinCode,
    joinCodeForSuffix,
    parseJoinCode,
    mintSuffix,
    reasonStringId,
    knownReasons,
    serverSurfaceReasons,
  };
}
