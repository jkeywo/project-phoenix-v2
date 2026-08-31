/** Parent-realm catalogue of every semantic action retained by the private
 * operator profile. Individual surfaces filter which entries they display. */

import { createSemanticActionRegistry } from './semantic-action-registry.js';
import {
  CAPTAIN_ACTIONS,
} from './stations/captain-actions.js';
import { HELM_STEERING_ACTION } from './stations/helm-actions.js';
import { MOD_ACTIONS } from './editor-mod-actions.js';

export function createClientSemanticActionRegistry({ adapters = {}, actionFeedback } = {}) {
  const registry = createSemanticActionRegistry({ actionFeedback });
  for (const action of [...CAPTAIN_ACTIONS, HELM_STEERING_ACTION, ...MOD_ACTIONS]) {
    registry.register(action, adapters[action.id]);
  }
  return registry;
}

/** Settings on a play surface omit editor-only controls while retaining their
 * bindings in the shared private profile. */
export function clientSettingsSemanticActions(registry) {
  if (!registry || typeof registry.list !== 'function') return [];
  return registry.list().filter((entry) => (
    entry.contexts.some((context) => !context.startsWith('editor.'))
  ));
}

if (typeof window !== 'undefined') {
  window.createClientSemanticActionRegistry = createClientSemanticActionRegistry;
  window.clientSettingsSemanticActions = clientSettingsSemanticActions;
}
