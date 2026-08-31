/**
 * gui/stations/helm-actions.js — real Helm continuous semantic adapter.
 *
 * Device sampling and local tuning live in the parent gamepad runtime. This
 * adapter receives one validated steering scalar and emits the existing narrow
 * console action; action-map.js remains the sole wire builder and Helm
 * admission remains authoritative.
 */

import { familyView } from '../console-payload.js';
import {
  createSemanticActionRegistry,
  SEMANTIC_HANDLED_WITHOUT_FEEDBACK,
} from '../semantic-action-registry.js';

export const HELM_ACTION_CONTEXT = 'helm';
export const HELM_THRUST_ACTION_ID = 'helm.thrust';
export const HELM_STEERING_ACTION_ID = 'helm.steering';
export const HELM_LATERAL_ACTION_ID = 'helm.lateral-thrust';
export const HELM_IMPULSE_ACTION_ID = 'helm.impulse';
export const HELM_BOOST_ACTION_ID = 'helm.boost';
export const HELM_VIEWSCREEN_ACTION_ID = 'helm.viewscreen';
export const HELM_DOCK_ACTION_ID = 'helm.dock';

const HELM_CONTEXTS = Object.freeze([HELM_ACTION_CONTEXT]);
const key = (code) => Object.freeze({
  type: 'keyboard', code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false,
});
const button = (control) => Object.freeze({ type: 'gamepad', input: 'button', control });

function axisAction(id, label, control) {
  return Object.freeze({
    id,
    contexts: HELM_CONTEXTS,
    labelId: `semantic_action.helm.${label}.label`,
    accessibilityLabelId: `semantic_action.helm.${label}.accessibility`,
    continuous: Object.freeze({ min: -1, max: 1, neutral: 0, cadenceMs: 100 }),
    tuning: Object.freeze({ deadzone: 0.1, inverted: false }),
    bindings: Object.freeze([
      Object.freeze({ type: 'gamepad', input: 'axis', control }),
      null,
    ]),
  });
}

function authoritativeAction(id, label, keyboard, gamepad, options = {}) {
  return Object.freeze({
    id,
    contexts: HELM_CONTEXTS,
    labelId: `semantic_action.helm.${label}.label`,
    accessibilityLabelId: `semantic_action.helm.${label}.accessibility`,
    authoritativeFeedback: true,
    hold: options.hold === true,
    bindings: Object.freeze([keyboard, gamepad]),
  });
}

export const HELM_THRUST_ACTION = axisAction(
  HELM_THRUST_ACTION_ID, 'thrust', 'left-stick-y',
);

export const HELM_STEERING_ACTION = axisAction(
  HELM_STEERING_ACTION_ID, 'steering', 'left-stick-x',
);

export const HELM_LATERAL_ACTION = axisAction(
  HELM_LATERAL_ACTION_ID, 'lateral_thrust', 'shoulder-pair',
);

export const HELM_IMPULSE_ACTION = authoritativeAction(
  HELM_IMPULSE_ACTION_ID, 'impulse', key('ControlLeft'), button('face-right'),
);

export const HELM_BOOST_ACTION = authoritativeAction(
  HELM_BOOST_ACTION_ID, 'boost', key('ShiftLeft'), button('face-bottom'), { hold: true },
);

export const HELM_VIEWSCREEN_ACTION = authoritativeAction(
  HELM_VIEWSCREEN_ACTION_ID, 'viewscreen', key('KeyR'), button('face-top'),
);

export const HELM_DOCK_ACTION = authoritativeAction(
  HELM_DOCK_ACTION_ID, 'dock', key('KeyK'), button('face-left'),
);

export const HELM_ACTIONS = Object.freeze([
  HELM_THRUST_ACTION,
  HELM_STEERING_ACTION,
  HELM_LATERAL_ACTION,
  HELM_IMPULSE_ACTION,
  HELM_BOOST_ACTION,
  HELM_VIEWSCREEN_ACTION,
  HELM_DOCK_ACTION,
]);

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
  const hasThrustControl = typeof options.hasThrustControl === 'function'
    ? options.hasThrustControl : () => true;
  const hasSteeringControl = typeof options.hasSteeringControl === 'function'
    ? options.hasSteeringControl : () => true;
  const hasLateralControl = typeof options.hasLateralControl === 'function'
    ? options.hasLateralControl : () => true;
  const hasImpulseControl = typeof options.hasImpulseControl === 'function'
    ? options.hasImpulseControl : () => true;
  const hasBoostControl = typeof options.hasBoostControl === 'function'
    ? options.hasBoostControl : () => true;
  const hasViewscreenControl = typeof options.hasViewscreenControl === 'function'
    ? options.hasViewscreenControl : () => true;
  const hasDockControl = typeof options.hasDockControl === 'function'
    ? options.hasDockControl : () => true;
  const send = (actionId, correlation, inputMs, name, payload) => {
    if (!sendAction || typeof correlation !== 'string' || !correlation) return false;
    sendAction(name, {
      ...(payload || {}), correlation, semantic_action: actionId, __input_ms: inputMs,
    });
    return true;
  };

  registry.register(HELM_THRUST_ACTION, ({ value } = {}) => {
    const view = helmActionView(getState());
    if (!view || view.helm_auto || !hasThrustControl() || !sendAction
        || typeof view.thrust_system_id !== 'string' || !view.thrust_system_id
        || !Number.isFinite(value)) return false;
    sendAction('set_helm_thrust', { value, control_system_id: view.thrust_system_id });
    return true;
  });

  registry.register(HELM_STEERING_ACTION, ({ value } = {}) => {
    const view = helmActionView(getState());
    if (!view || view.helm_auto || !hasSteeringControl() || !sendAction
        || typeof view.steering_system_id !== 'string' || !view.steering_system_id
        || !Number.isFinite(value)) return false;
    sendAction('set_helm_steering', { value, control_system_id: view.steering_system_id });
    return true;
  });

  registry.register(HELM_LATERAL_ACTION, ({ value } = {}) => {
    const view = helmActionView(getState());
    if (!view || view.lateral_auto || !hasLateralControl()
        || typeof view.lateral_system_id !== 'string' || !view.lateral_system_id
        || !sendAction || !Number.isFinite(value)) return false;
    sendAction('set_helm_lateral', { value, control_system_id: view.lateral_system_id });
    return true;
  });

  registry.register(HELM_IMPULSE_ACTION, ({
    actionId, correlation, inputMs,
  } = {}) => {
    const view = helmActionView(getState());
    if (!view || view.helm_auto || !hasImpulseControl()
        || typeof view.impulse_system_id !== 'string' || !view.impulse_system_id) return false;
    return Number(view.impulse_charge_progress) > 0
      ? send(actionId, correlation, inputMs, 'cancel_impulse', {
        control_system_id: view.impulse_system_id,
      })
      : send(actionId, correlation, inputMs, 'start_impulse_charge', {
        control_system_id: view.impulse_system_id,
      });
  });

  const boostHolds = new Set();
  let boostOwnerId = null;
  registry.register(HELM_BOOST_ACTION, ({
    actionId, correlation, inputMs, source, event, binding, detail, pressed,
  } = {}) => {
    const sourceId = detail && typeof detail.holdSource === 'string' && detail.holdSource
      ? `control:${detail.holdSource}`
      : source === 'keyboard'
        ? `keyboard:${event?.code || binding?.code || 'key'}`
        : source === 'gamepad'
          ? `gamepad:${binding?.control || 'button'}`
          : `${source || 'control'}:default`;
    if (pressed !== false) {
      const view = helmActionView(getState());
      if (!view || view.helm_auto || !hasBoostControl() || !view.boost_enabled
          || typeof view.boost_system_id !== 'string' || !view.boost_system_id
          || (!view.boost_active && Number(view.boost_battery) < 1)) return false;
      if (boostHolds.has(sourceId)) return false;
      const alreadyHeld = boostHolds.size > 0;
      if (alreadyHeld) {
        boostHolds.add(sourceId);
        return SEMANTIC_HANDLED_WITHOUT_FEEDBACK;
      }
      const handled = send(actionId, correlation, inputMs, 'set_boost', {
        active: true, control_system_id: view.boost_system_id,
      });
      if (!handled) return false;
      boostOwnerId = view.boost_system_id;
      boostHolds.add(sourceId);
      return true;
    }
    if (!boostHolds.delete(sourceId)) return false;
    if (boostHolds.size > 0) return SEMANTIC_HANDLED_WITHOUT_FEEDBACK;
    const ownerId = boostOwnerId;
    boostOwnerId = null;
    if (typeof ownerId !== 'string' || !ownerId) return false;
    return send(actionId, correlation, inputMs, 'set_boost', {
      active: false, control_system_id: ownerId,
    });
  });

  registry.register(HELM_VIEWSCREEN_ACTION, ({
    actionId, correlation, inputMs,
  } = {}) => {
    const view = helmActionView(getState());
    return view && !view.viewscreen_auto && hasViewscreenControl()
      && typeof view.viewscreen_system_id === 'string' && view.viewscreen_system_id
      ? send(actionId, correlation, inputMs, 'set_radar_view', {
        control_system_id: view.viewscreen_system_id,
      })
      : false;
  });

  registry.register(HELM_DOCK_ACTION, ({
    actionId, correlation, inputMs,
  } = {}) => {
    const view = helmActionView(getState());
    const dock = view && view.dock;
    if (!dock || !hasDockControl() || typeof dock.system_id !== 'string' || !dock.system_id
        || (!dock.available && !dock.engaged && !dock.docked)) return false;
    return send(
      actionId,
      correlation,
      inputMs,
      dock.docked ? 'undock' : 'dock',
      { target: dock.system_id, control_system_id: dock.system_id },
    );
  });
  return registry;
}

export function createHelmActionRegistry(options = {}) {
  return registerHelmActions(createSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
  }), options);
}

if (typeof window !== 'undefined') {
  window.registerHelmActions = registerHelmActions;
}
