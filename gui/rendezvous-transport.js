/** Phoenix transport adapter: content namespaces, endpoint defaults and localisation. */
import './strings-boot.js';
import { localiseTree, rememberRawDeliveredMessage, setOverlayCatalogues } from './strings.js';
import { getJoinCodeData, reasonStringId } from './join-code.js';
import { DEV_RENDEZVOUS_URL, KNOWN_WEB_ORIGINS, joinUrlForCode, rendezvousBaseForOrigin } from './join-url.js';
import * as transport from '../packages/transport/src/rendezvous-transport.js';
export * from '../packages/transport/src/rendezvous-transport.js';
export { DEV_RENDEZVOUS_URL, KNOWN_WEB_ORIGINS, joinUrlForCode, rendezvousBaseForOrigin };
const { RENDEZVOUS_PROTOCOL, RELIABLE_CHANNEL, SNAPSHOT_CHANNEL, JOIN_ATTEMPTS_BEFORE_ENTRY, isRetryableReason, socketUrl } = transport;
/** Production ingress boundary: install Welcome catalogues before destructive localisation. */
export function localiseDeliveredMessage(msg) {
  if (msg?.type === 'Welcome') {
    setOverlayCatalogues(msg.data?.string_catalogues || []);
  }
  return rememberRawDeliveredMessage(localiseTree(msg), msg);
}

function pageRendezvousBase() {
  const origin = typeof location !== 'undefined' && location ? location.origin : '';
  return rendezvousBaseForOrigin(origin);
}
export function defaultFactories() {
  return { ...transport.defaultFactories(), ...((typeof window !== 'undefined' && window.PhoenixTransportFactories) || {}) };
}
export function rendezvousBaseFromLocation(search, defaultBase = pageRendezvousBase()) {
  return transport.rendezvousBaseFromLocation(search, defaultBase);
}
export function joinRouteFromLocation(search, hash, defaultBase = pageRendezvousBase()) {
  return transport.joinRouteFromLocation(search, hash, defaultBase);
}
export function createRendezvousHost(opts) {
  return transport.createRendezvousHost({ getJoinCodeData, ...opts, factories: opts.factories === undefined ? defaultFactories() : opts.factories });
}
export function createRendezvousJoiner(opts) {
  return transport.createRendezvousJoiner({ reasonStringId, ...opts,
    onAccepted: opts.onAccepted || (({ send }) => send(JSON.stringify({ type: 'Identify', data: (opts.getIdent || (() => ({})))() }))),
    onData: message => (opts.onData || (() => {}))(opts.localise === false ? message : localiseDeliveredMessage(message)),
    factories: opts.factories === undefined ? defaultFactories() : opts.factories });
}
if (typeof window !== 'undefined') {
  window.rendezvousTransport = {
    RENDEZVOUS_PROTOCOL,
    DEV_RENDEZVOUS_URL,
    RELIABLE_CHANNEL,
    SNAPSHOT_CHANNEL,
    JOIN_ATTEMPTS_BEFORE_ENTRY,
    isRetryableReason,
    rendezvousBaseFromLocation,
    rendezvousBaseForOrigin,
    joinRouteFromLocation,
    socketUrl,
    joinUrlForCode,
    defaultFactories,
    createRendezvousHost,
    createRendezvousJoiner,
  };
}
