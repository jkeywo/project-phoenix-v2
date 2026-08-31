/** Parent-realm catalogue of every semantic action available to Settings. */

import { createSemanticActionRegistry } from './semantic-action-registry.js';
import {
  CAPTAIN_ACTIONS,
} from './stations/captain-actions.js';
import { HELM_STEERING_ACTION } from './stations/helm-actions.js';

export function createClientSemanticActionRegistry() {
  const registry = createSemanticActionRegistry();
  for (const action of CAPTAIN_ACTIONS) registry.register(action);
  registry.register(HELM_STEERING_ACTION);
  return registry;
}

if (typeof window !== 'undefined') {
  window.createClientSemanticActionRegistry = createClientSemanticActionRegistry;
}
