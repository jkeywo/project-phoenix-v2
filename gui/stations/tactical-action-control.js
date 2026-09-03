/** Visible Tactical controls share the document semantic dispatcher.
 *
 * The fallback keeps standalone component embeddings working; a real console
 * always supplies `activateSemanticAction`, so normal operator input cannot
 * bypass the action identity or correlated feedback lifecycle.
 */
import { TACTICAL_ACTION_CONTEXT } from './tactical-actions.js';

export function activateTacticalAction(owner, actionId, detail, legacyAction) {
  const activate = typeof window !== 'undefined' && window.activateSemanticAction;
  if (typeof activate === 'function') {
    return activate(actionId, {
      context: TACTICAL_ACTION_CONTEXT,
      source: 'control',
      detail,
    });
  }
  if (owner && typeof owner.sendAction === 'function') owner.sendAction(legacyAction, detail);
  return false;
}
