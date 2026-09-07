/** Explicit, attributed GM session pause/resume semantic actions (issue #1292). */

import { createSemanticActionRegistry } from './semantic-action-registry.js';

export const GM_ACTION_CONTEXT = 'gm';
export const GM_PAUSE_ACTION_ID = 'gm.session.pause';
export const GM_RESUME_ACTION_ID = 'gm.session.resume';
export const GM_SESSION_CONFIRMATION_CATEGORY = 'session.pause';

const key = (code) => Object.freeze({
  type: 'keyboard',
  code,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  metaKey: false,
});

const definition = (id, verb, code) => Object.freeze({
  id,
  contexts: Object.freeze([GM_ACTION_CONTEXT]),
  labelId: `semantic_action.gm.session_${verb}.label`,
  accessibilityLabelId: `semantic_action.gm.session_${verb}.accessibility`,
  authoritativeFeedback: true,
  confirmationCategory: GM_SESSION_CONFIRMATION_CATEGORY,
  confirmationDefault: 'immediate',
  bindings: Object.freeze([key(code), null]),
});

export const GM_SESSION_ACTION_DEFINITIONS = Object.freeze([
  definition(GM_PAUSE_ACTION_ID, 'pause', 'KeyP'),
  definition(GM_RESUME_ACTION_ID, 'resume', 'KeyR'),
]);

/**
 * Register the two explicit state-setting adapters.
 *
 * `submitSessionPaused` receives an absolute boolean and the opaque correlation
 * minted by the shared feedback lifecycle. It is never a toggle callback.
 */
export function registerGmSessionActions(registry, { submitSessionPaused } = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('GM session actions require a semantic action registry');
  }
  const register = (definitionValue, active) => {
    registry.register(definitionValue, ({ correlation } = {}) => {
      if (typeof submitSessionPaused !== 'function'
          || typeof correlation !== 'string' || correlation.length === 0) return false;
      return submitSessionPaused(active, correlation) !== false;
    });
  };
  register(GM_SESSION_ACTION_DEFINITIONS[0], true);
  register(GM_SESSION_ACTION_DEFINITIONS[1], false);
  return registry;
}

/** Convenience factory for the GM page and pure tests. */
export function createGmSessionActionRegistry({ actionFeedback, submitSessionPaused } = {}) {
  return registerGmSessionActions(
    createSemanticActionRegistry({ actionFeedback }),
    { submitSessionPaused },
  );
}
