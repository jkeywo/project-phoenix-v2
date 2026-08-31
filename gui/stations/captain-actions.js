/**
 * gui/stations/captain-actions.js — real Captain semantic action adapters.
 *
 * The registry context below is presentation/input scope only. Command
 * authority remains at the existing action-map → ControlSystem → admission
 * path; this module emits the same legacy console action as the visible Red
 * Alert control always did.
 */

import { familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const CAPTAIN_ACTION_CONTEXT = 'captain';
export const CAPTAIN_RED_ALERT_ACTION_ID = 'captain.red-alert';
export const CAPTAIN_WEAPONS_HOLD_ACTION_ID = 'captain.weapons-hold';

/** Stable metadata plus the two-slot default binding contract. */
export const CAPTAIN_RED_ALERT_ACTION = Object.freeze({
  id: CAPTAIN_RED_ALERT_ACTION_ID,
  contexts: Object.freeze([CAPTAIN_ACTION_CONTEXT]),
  labelId: 'semantic_action.captain.red_alert.label',
  accessibilityLabelId: 'semantic_action.captain.red_alert.accessibility',
  authoritativeFeedback: true,
  bindings: Object.freeze([
    Object.freeze({
      type: 'keyboard',
      code: 'KeyR',
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    }),
    Object.freeze({
      type: 'gamepad',
      input: 'button',
      control: 'face-bottom',
    }),
  ]),
});

export const CAPTAIN_WEAPONS_HOLD_ACTION = Object.freeze({
  id: CAPTAIN_WEAPONS_HOLD_ACTION_ID,
  contexts: Object.freeze([CAPTAIN_ACTION_CONTEXT]),
  labelId: 'semantic_action.captain.weapons_hold.label',
  accessibilityLabelId: 'semantic_action.captain.weapons_hold.accessibility',
  bindings: Object.freeze([
    Object.freeze({
      type: 'keyboard',
      code: 'KeyH',
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    }),
    null,
  ]),
});

/** Resolve the Captain family view without guessing a System id. */
export function captainActionView(state) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, CAPTAIN_ACTION_CONTEXT);
  return Object.keys(projected).length > 0 ? projected : state;
}

/** Register the real Red Alert adapter on an isolated registry. */
export function registerCaptainActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('captain action registration requires a registry');
  }
  const getState = typeof options.getState === 'function' ? options.getState : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;

  registry.register(CAPTAIN_RED_ALERT_ACTION, ({ actionId, correlation, inputMs } = {}) => {
    const view = captainActionView(getState());
    // `red_alert_auto` is presentation of authoritative Control Source, not a
    // new authority decision. The host remains responsible for admission.
    if (!view || view.red_alert_auto || !sendAction) return false;
    const payload = { active: !Boolean(view.red_alert) };
    if (typeof correlation === 'string' && correlation) {
      payload.correlation = correlation;
      payload.semantic_action = actionId;
      payload.__input_ms = inputMs;
    }
    sendAction('set_red_alert', payload);
    return true;
  });
  registry.register(CAPTAIN_WEAPONS_HOLD_ACTION, () => {
    const view = captainActionView(getState());
    if (!view || view.red_alert_auto || !sendAction) return false;
    sendAction('set_weapons_hold', { held: !Boolean(view.weapons_hold) });
    return true;
  });
  return registry;
}

/** Convenience constructor used independently in parent and iframe realms. */
export function createCaptainActionRegistry(options = {}) {
  return registerCaptainActions(createSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
  }), options);
}

if (typeof window !== 'undefined') {
  window.createCaptainActionRegistry = createCaptainActionRegistry;
}
