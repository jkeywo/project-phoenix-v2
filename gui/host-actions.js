/** Host-chrome semantic action definitions and adapters (issue #1281). */

import { ACTION_FEEDBACK_STATE } from './action-feedback.js';
import { createSemanticActionRegistry } from './semantic-action-registry.js';

export const HOST_ACTION_CONTEXT = 'host';
export const HOST_GM_ACTION_CONTEXT = 'gm';
export const HOST_QR_CODE_ACTION_ID = 'host.qr-code';

export const HOST_ACTION_DEFINITIONS = Object.freeze([
  Object.freeze({
    id: HOST_QR_CODE_ACTION_ID,
    // QR belongs to both host roles. The overlap with `gm` also makes the one
    // shared registry detect/remap conflicts against privileged GM actions.
    contexts: Object.freeze([HOST_ACTION_CONTEXT, HOST_GM_ACTION_CONTEXT]),
    labelId: 'semantic_action.host.qr_code.label',
    accessibilityLabelId: 'semantic_action.host.qr_code.accessibility',
    feedback: 'local',
    bindings: Object.freeze([
      Object.freeze({
        type: 'keyboard',
        code: 'KeyQ',
        ctrlKey: false,
        shiftKey: false,
        altKey: false,
        metaKey: false,
      }),
      null,
    ]),
  }),
]);

/** Register the real host QR adapter over the existing host-page toggle seam. */
export function registerHostActions(registry, { toggleQrCode } = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('host actions require a semantic action registry');
  }
  registry.register(HOST_ACTION_DEFINITIONS[0], ({ settleFeedback }) => {
    if (typeof toggleQrCode !== 'function' || typeof settleFeedback !== 'function') return false;
    if (toggleQrCode() === false) return false;
    settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
    return true;
  });
  return registry;
}

/** Convenience factory for pure tests and host-page mounting. */
export function createHostActionRegistry({ actionFeedback, toggleQrCode } = {}) {
  return registerHostActions(
    createSemanticActionRegistry({ actionFeedback }),
    { toggleQrCode },
  );
}
