/**
 * gui/stations/captain-actions.js — real Captain semantic action adapters.
 *
 * The registry context below is presentation/input scope only. Command
 * authority remains at the existing action-map → ControlSystem → admission
 * path; this module emits the same legacy console action as the visible Red
 * Alert control always did.
 */

import { defineStationAction, createCorrelatedActionSender, keyboard, button } from './action-support.js';

import { familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const CAPTAIN_ACTION_CONTEXT = 'captain';
export const CAPTAIN_RED_ALERT_ACTION_ID = 'captain.red-alert';
export const CAPTAIN_VIEW_ACTION_ID = 'captain.view';
export const CAPTAIN_OBJECTIVE_PRIORITY_ACTION_ID = 'captain.objective-priority';

/** Stable metadata plus the two-slot default binding contract. */
export const CAPTAIN_RED_ALERT_ACTION = defineStationAction({
  id: CAPTAIN_RED_ALERT_ACTION_ID, contexts: [CAPTAIN_ACTION_CONTEXT], labelKey: 'captain.red_alert',
  bindings: [keyboard('KeyR'), button('face-bottom')],
});

export const CAPTAIN_VIEW_ACTION = defineStationAction({
  id: CAPTAIN_VIEW_ACTION_ID, contexts: [CAPTAIN_ACTION_CONTEXT], labelKey: 'captain.view',
  bindings: [keyboard('KeyV'), null],
});

export const CAPTAIN_OBJECTIVE_PRIORITY_ACTION = defineStationAction({
  id: CAPTAIN_OBJECTIVE_PRIORITY_ACTION_ID, contexts: [CAPTAIN_ACTION_CONTEXT], labelKey: 'captain.objective_priority',
  bindings: [keyboard('KeyO'), null],
});

// Red Alert is the Captain's only firing-posture lever since issue #1398. The
// Weapons Hold action that sat beside it from #1041 (id `captain.weapons-hold`,
// bound to KeyH) is retired along with the `set_weapons_hold` command: restraint
// is a POWER order now, made from Engineering.
export const CAPTAIN_ACTIONS = Object.freeze([
  CAPTAIN_RED_ALERT_ACTION,
  CAPTAIN_VIEW_ACTION,
  CAPTAIN_OBJECTIVE_PRIORITY_ACTION,
]);

/** Resolve the Captain family view without guessing a System id. */
export function captainActionView(state) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, CAPTAIN_ACTION_CONTEXT);
  return Object.keys(projected).length > 0 ? projected : state;
}

function selectedOrNext(values, current, selected) {
  const choices = Array.isArray(values)
    ? values.filter((value) => typeof value === 'string' && value)
    : [];
  if (selected != null) {
    return typeof selected === 'string' && choices.includes(selected) ? selected : null;
  }
  if (choices.length === 0) return null;
  const index = choices.indexOf(current);
  return choices[(index + 1 + choices.length) % choices.length];
}

/** Register the shipped Captain command family on an isolated registry. */
export function registerCaptainActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('captain action registration requires a registry');
  }
  const getState = typeof options.getState === 'function' ? options.getState : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;
  const send = createCorrelatedActionSender(sendAction);
  const getAvailableCameraViews = typeof options.getAvailableCameraViews === 'function'
    ? options.getAvailableCameraViews
    : null;

  registry.register(CAPTAIN_RED_ALERT_ACTION, ({ actionId, correlation, inputMs } = {}) => {
    const view = captainActionView(getState());
    // `red_alert_auto` is presentation of authoritative Control Source, not a
    // new authority decision. The host remains responsible for admission.
    if (!view || view.red_alert_auto || !sendAction) return false;
    return send(actionId, correlation, inputMs, 'set_red_alert', { active: !Boolean(view.red_alert) });
  });
  registry.register(CAPTAIN_VIEW_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = captainActionView(getState());
    if (!view || view.viewscreen_auto || !sendAction) return false;
    const direction = selectedOrNext(
      getAvailableCameraViews ? getAvailableCameraViews() : view.camera_views,
      view.view_direction,
      detail && detail.direction,
    );
    if (!direction) return false;
    return send(actionId, correlation, inputMs, 'set_view', { direction });
  });
  registry.register(CAPTAIN_OBJECTIVE_PRIORITY_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = captainActionView(getState());
    if (!view || !sendAction) return false;
    const objectiveIds = (Array.isArray(view.objectives) ? view.objectives : [])
      .map((objective) => objective && !objective.unassigned && objective.id)
      .filter((id) => typeof id === 'string' && id);
    const id = selectedOrNext(
      objectiveIds,
      view.boosted_objective_id,
      detail && detail.id,
    );
    if (!id) return false;
    return send(actionId, correlation, inputMs, 'set_objective_priority', { id });
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
