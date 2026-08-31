/**
 * gui/stations/helm-actions.js — real Helm continuous semantic adapter.
 *
 * Device sampling and local tuning live in the parent gamepad runtime. This
 * adapter receives one validated steering scalar and emits the existing narrow
 * console action; action-map.js remains the sole wire builder and Helm
 * admission remains authoritative.
 */

import { familyView } from '../console-payload.js';

export const HELM_ACTION_CONTEXT = 'helm';
export const HELM_STEERING_ACTION_ID = 'helm.steering';

export const HELM_STEERING_ACTION = Object.freeze({
  id: HELM_STEERING_ACTION_ID,
  contexts: Object.freeze([HELM_ACTION_CONTEXT]),
  labelId: 'semantic_action.helm.steering.label',
  accessibilityLabelId: 'semantic_action.helm.steering.accessibility',
  continuous: Object.freeze({
    min: -1,
    max: 1,
    neutral: 0,
    cadenceMs: 100,
  }),
  tuning: Object.freeze({ deadzone: 0.1, inverted: false }),
  bindings: Object.freeze([
    Object.freeze({ type: 'gamepad', input: 'axis', control: 'left-stick-x' }),
    null,
  ]),
});

export function helmActionView(state) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, HELM_ACTION_CONTEXT);
  return Object.keys(projected).length > 0 ? projected : state;
}

export function registerHelmActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('helm action registration requires a registry');
  }
  const getState = typeof options.getState === 'function' ? options.getState : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;

  registry.register(HELM_STEERING_ACTION, ({ value } = {}) => {
    const view = helmActionView(getState());
    if (!view || view.helm_auto || !sendAction || !Number.isFinite(value)) return false;
    sendAction('set_helm_steering', { value });
    return true;
  });
  return registry;
}

if (typeof window !== 'undefined') {
  window.registerHelmActions = registerHelmActions;
}
