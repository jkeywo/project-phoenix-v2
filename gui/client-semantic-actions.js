/** Parent-realm catalogue of every semantic action available to Settings. */

import { createSemanticActionRegistry } from './semantic-action-registry.js';
import {
  CAPTAIN_RED_ALERT_ACTION,
  CAPTAIN_WEAPONS_HOLD_ACTION,
} from './stations/captain-actions.js';
import { HELM_STEERING_ACTION } from './stations/helm-actions.js';

export function createClientSemanticActionRegistry() {
  const registry = createSemanticActionRegistry();
  registry.register(CAPTAIN_RED_ALERT_ACTION);
  registry.register(CAPTAIN_WEAPONS_HOLD_ACTION);
  registry.register(HELM_STEERING_ACTION);
  return registry;
}

if (typeof window !== 'undefined') {
  window.createClientSemanticActionRegistry = createClientSemanticActionRegistry;
}
