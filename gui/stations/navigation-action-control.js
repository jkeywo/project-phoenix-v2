/** Visible Navigation controls share the document semantic dispatcher. */
import { NAVIGATION_ACTION_CONTEXT } from './navigation-actions.js';

export function activateNavigationAction(owner, actionId, detail, fallback) {
  const activate = typeof window !== 'undefined' && window.activateSemanticAction;
  if (typeof activate === 'function') {
    return activate(actionId, {
      context: NAVIGATION_ACTION_CONTEXT,
      source: 'control',
      detail,
      surface: owner,
    });
  }
  if (typeof fallback === 'function') fallback();
  return false;
}
