/**
 * gui/command-gateway.js — the explicit client command gateway.
 *
 * Every in-game command a console raises leaves the phone through this module.
 * It is the one place that knows the shape of a `ClientMessage::ControlSystem`
 * envelope (`{ target, payload }`) and the one place that knows how to reach
 * the PeerJS transport owned by `gui/connection-manager.js`.
 *
 * This module is the client-side half of the PASM entity
 * `client-network-interface`. Console modules (e.g. `gui/repair-dispatch.js`)
 * import it rather than hand-rolling envelopes, so "this console talks to the
 * network" is an observable code edge instead of a convention.
 *
 * Deliberately DOM-free and side-effect-free at import time so it is unit
 * testable in Node: the transport is resolved lazily, either from an explicit
 * `send` argument (how `gui/action-map.js` calls in) or from the live
 * `window.connectionManager` singleton.
 *
 * NOTE: this module carries no gameplay values. `target` and `payload` are
 * passed straight through from the caller; anything tunable lives in TOML on
 * the host and reaches the client over the wire.
 */

/** The `ClientMessage` variant every in-game console command travels in. */
export const CONTROL_SYSTEM = 'ControlSystem';

/**
 * Build a `ControlSystem` envelope. Throws on a malformed call so a broken
 * console fails loudly in tests rather than silently sending junk the host
 * admission layer would have to reject.
 *
 * @param {string} target  SystemId string, e.g. `'repair'`
 * @param {{type: string, data?: object}} payload  SystemControlPayload
 * @returns {{type: string, data: {target: string, payload: object}}}
 */
export function controlSystemEnvelope(target, payload) {
  if (typeof target !== 'string' || target.length === 0) {
    throw new TypeError('command-gateway: ControlSystem target must be a non-empty string');
  }
  if (!payload || typeof payload.type !== 'string' || payload.type.length === 0) {
    throw new TypeError('command-gateway: ControlSystem payload must have a `type`');
  }
  return { type: CONTROL_SYSTEM, data: { target, payload } };
}

/**
 * Resolve the function used to put a message on the wire.
 *
 * Prefers an explicit `send(type, data)` (the action-map / test path), then
 * falls back to the live `ConnectionManager` singleton, whose `send` has the
 * identical `(type, data, deliveryClass)` signature.
 *
 * @param {((type: string, data?: object) => void)} [send]
 * @returns {((type: string, data?: object) => void) | null} null when offline
 */
export function resolveTransport(send) {
  if (typeof send === 'function') return send;
  const win = (typeof window !== 'undefined') ? window : null;
  const cm = win && win.connectionManager;
  if (cm && typeof cm.send === 'function') {
    return (type, data) => cm.send(type, data);
  }
  return null;
}

/**
 * Send a `ControlSystem` command through the gateway.
 *
 * @param {string} target
 * @param {{type: string, data?: object}} payload
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 * @returns {object|null} the envelope that was sent, or null when there was no
 *   transport available (client not connected yet).
 */
export function sendControlSystem(target, payload, send) {
  const envelope = controlSystemEnvelope(target, payload);
  const transport = resolveTransport(send);
  if (!transport) return null;
  transport(envelope.type, envelope.data);
  return envelope;
}
