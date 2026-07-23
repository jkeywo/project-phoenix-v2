/**
 * gui/helm-dispatch.js — the Helm console's outbound command surface.
 *
 * This module is the client-side dispatch seam for the PASM entity
 * `helm-console-interface`. It owns the translation from what the Helm player
 * did (push the throttle, turn the stick, charge impulse, toggle boost, dodge
 * laterally) into the per-axis wire payloads the host admission layer expects,
 * and it hands each one to the explicit client command gateway below.
 *
 * The `import` on the next line is the whole point of this file: the Helm
 * console reaches the network *through* `client-network-interface`, and that is
 * now an observable code edge (issue #745) rather than a prose claim — the same
 * seam `gui/repair-dispatch.js` established for the Repair console.
 *
 * Deliberately DOM-free and side-effect-free at import time so it is unit
 * testable in Node: the transport is resolved by `command-gateway.js`, either
 * from the explicit `send` the action-map threads in or the live
 * `ConnectionManager` singleton.
 *
 * NOTE: this module carries no gameplay values. Every axis value is passed
 * straight through from the caller; the wire target strings are structural
 * addressing (they mirror the fine helm system ids the host declares), not
 * tunable gameplay data.
 */
import { sendControlSystem } from './command-gateway.js';

/**
 * Fine helm system ids the host routes each per-axis command to. Structural
 * addressing, not gameplay values — they mirror the `HELM_*_SYSTEM_ID`
 * constants in `src/ship/system_registry.rs` (issue #801 split the combined
 * helm wire per axis).
 */
export const HELM_THRUST_SYSTEM_ID = 'helm-thrust';
export const HELM_STEERING_SYSTEM_ID = 'helm-steering';
export const HELM_IMPULSE_SYSTEM_ID = 'helm-impulse';
export const HELM_BOOST_SYSTEM_ID = 'helm-boost';
export const LATERAL_THRUST_SYSTEM_ID = 'helm-lateral-thrust';

/**
 * Send a per-axis thrust request. Targets the `helm-thrust` fine system so
 * admission gates the throttle axis on its own control source (issue #801).
 *
 * @param {number} value normalized throttle `[-1, 1]`
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function sendThrust(value, send) {
  return sendControlSystem(
    HELM_THRUST_SYSTEM_ID,
    { type: 'SetThrust', data: { value: value || 0 } },
    send,
  );
}

/**
 * Send a per-axis steering request. Targets the `helm-steering` fine system.
 *
 * @param {number} value normalized yaw `[-1, 1]`
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function sendSteering(value, send) {
  return sendControlSystem(
    HELM_STEERING_SYSTEM_ID,
    { type: 'SetSteering', data: { value: value || 0 } },
    send,
  );
}

/**
 * Fan one joystick action out to the two per-axis wire messages (issue #801):
 * `SetThrust` -> `helm-thrust` and `SetSteering` -> `helm-steering`. Admission
 * gates each axis independently, so a ship with an AI-held throttle and a
 * human-held stick admits exactly the axis the human owns.
 *
 * @param {number} thrust normalized throttle `[-1, 1]`
 * @param {number} steering normalized yaw `[-1, 1]`
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 */
export function sendHelmInput(thrust, steering, send) {
  sendThrust(thrust, send);
  sendSteering(steering, send);
}

/**
 * Send a lateral (sideways) thrust request. Targets the `helm-lateral-thrust`
 * fine system.
 *
 * @param {number} lateral normalized lateral `[-1, 1]`
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function sendLateralThrust(lateral, send) {
  return sendControlSystem(
    LATERAL_THRUST_SYSTEM_ID,
    { type: 'LateralThrustInput', data: { lateral: lateral || 0 } },
    send,
  );
}

/**
 * Begin charging the impulse drive. Targets `helm-impulse` (issue #801).
 *
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function startImpulseCharge(send) {
  return sendControlSystem(HELM_IMPULSE_SYSTEM_ID, { type: 'StartImpulseCharge' }, send);
}

/**
 * Cancel a charging or active impulse drive. Targets `helm-impulse`.
 *
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function cancelImpulse(send) {
  return sendControlSystem(HELM_IMPULSE_SYSTEM_ID, { type: 'CancelImpulse' }, send);
}

/**
 * Toggle the boost drive on/off. Targets `helm-boost` (issue #801).
 *
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function toggleBoost(send) {
  return sendControlSystem(HELM_BOOST_SYSTEM_ID, { type: 'ToggleBoost' }, send);
}

/**
 * Explicitly set boost on or off (hold-to-boost). Targets `helm-boost`.
 *
 * @param {boolean} active
 * @param {((type: string, data?: object) => void)} [send] explicit transport
 * @returns {object|null} the envelope that was sent, or null when offline.
 */
export function setBoost(active, send) {
  return sendControlSystem(
    HELM_BOOST_SYSTEM_ID,
    { type: 'SetBoost', data: { active: !!active } },
    send,
  );
}
