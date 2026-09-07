/**
 * Context-scoped Sensors and Science semantic actions.
 *
 * The adapters choose only from the live authoritative Console Family
 * projections. They never invent a contact, scan result, impulse phase or
 * shield facing, and they never mutate client gameplay state optimistically.
 * The existing action map remains the sole wire builder and command admission
 * remains the sole authority gate.
 */

import { familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const SENSORS_TARGET_ACTION_ID = 'sensors.target-selection';
export const SENSORS_SCAN_ACTION_ID = 'sensors.scan';
export const SENSORS_VIEWSCREEN_ACTION_ID = 'sensors.viewscreen';
export const SENSORS_CANCEL_IMPULSE_ACTION_ID = 'sensors.cancel-impulse';
export const SCIENCE_SHIELD_FOCUS_ACTION_ID = 'science.shield-focus';

const SENSOR_BEARING_CONTEXTS = Object.freeze(['sensors', 'science', 'captain', 'tactical']);
const SHIELD_FOCUS_CONTEXTS = Object.freeze(['science', 'captain', 'engineering', 'shields']);

/** Every console context which mounts one of these shipped controls. */
export const SENSOR_SCIENCE_ACTION_CONTEXTS = Object.freeze([
  'sensors', 'science', 'captain', 'tactical', 'engineering', 'shields',
]);

const keyboard = (code, modifiers = {}) => Object.freeze({
  type: 'keyboard', code,
  ctrlKey: !!modifiers.ctrlKey,
  shiftKey: !!modifiers.shiftKey,
  altKey: !!modifiers.altKey,
  metaKey: !!modifiers.metaKey,
});

const gamepad = (input, control) => Object.freeze({ type: 'gamepad', input, control });

function action(
  id, contexts, label, keyboardCode, gamepadInput, gamepadControl, keyboardModifiers = {},
) {
  return Object.freeze({
    id,
    contexts,
    labelId: `semantic_action.${label}.label`,
    accessibilityLabelId: `semantic_action.${label}.accessibility`,
    authoritativeFeedback: true,
    bindings: Object.freeze([
      keyboard(keyboardCode, keyboardModifiers),
      gamepad(gamepadInput, gamepadControl),
    ]),
  });
}

export const SENSORS_TARGET_ACTION = action(
  SENSORS_TARGET_ACTION_ID,
  SENSOR_BEARING_CONTEXTS,
  'sensors.target_selection',
  'KeyS',
  'button',
  'face-left',
);

// The only shipped scan control is on the destroyer Captain surface. The
// action remains a Sensors identity because it commands the Sensors suite.
export const SENSORS_SCAN_ACTION = action(
  SENSORS_SCAN_ACTION_ID,
  Object.freeze(['captain']),
  'sensors.scan',
  'KeyN',
  'button',
  'face-top',
  { shiftKey: true },
);

export const SENSORS_VIEWSCREEN_ACTION = action(
  SENSORS_VIEWSCREEN_ACTION_ID,
  SENSOR_BEARING_CONTEXTS,
  'sensors.viewscreen',
  'KeyU',
  'button',
  'face-right',
);

export const SENSORS_CANCEL_IMPULSE_ACTION = action(
  SENSORS_CANCEL_IMPULSE_ACTION_ID,
  Object.freeze(['sensors']),
  'sensors.cancel_impulse',
  'KeyX',
  'button',
  'face-bottom',
);

// ph-shield-facings is shared by Science, Captain, Engineering and dedicated
// Shields variants. One identity keeps pointer, keyboard and gamepad behaviour
// the same wherever that shipped control is mounted.
export const SCIENCE_SHIELD_FOCUS_ACTION = action(
  SCIENCE_SHIELD_FOCUS_ACTION_ID,
  SHIELD_FOCUS_CONTEXTS,
  'science.shield_focus',
  'KeyF',
  'button',
  'left-shoulder',
);

export const SENSOR_SCIENCE_ACTIONS = Object.freeze([
  SENSORS_TARGET_ACTION,
  SENSORS_SCAN_ACTION,
  SENSORS_VIEWSCREEN_ACTION,
  SENSORS_CANCEL_IMPULSE_ACTION,
  SCIENCE_SHIELD_FOCUS_ACTION,
]);

/**
 * Resolve one authoritative family without falling through to another keyed
 * family's fields. Flat payloads remain supported for the dedicated legacy
 * Sensors console, but a keyed payload lacking the requested family is absent.
 */
function exactFamilyView(state, family) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, family);
  if (Object.keys(projected).length > 0) return projected;
  const keyed = state.systems || state.system_families || state.system_ids;
  return keyed ? null : state;
}

export function sensorsActionView(state) {
  return exactFamilyView(state, 'sensors');
}

export function shieldsActionView(state) {
  return exactFamilyView(state, 'shields');
}

function correlatedPayload(actionId, correlation, inputMs, payload) {
  if (typeof correlation !== 'string' || !correlation) return null;
  return { ...payload, correlation, semantic_action: actionId, __input_ms: inputMs };
}

function visibleTarget(view, selected) {
  const candidates = (Array.isArray(view && view.blips) ? view.blips : [])
    .filter((blip) => blip && typeof blip.uuid === 'string' && blip.uuid)
    .map((blip) => blip.uuid);
  if (selected != null) return candidates.includes(selected) ? selected : null;
  if (candidates.length === 0) return null;
  const current = candidates.indexOf(view && view.target_uuid);
  return candidates[(current + 1 + candidates.length) % candidates.length];
}

function facingChoice(view, selected) {
  const facings = (Array.isArray(view && view.facings) ? view.facings : [])
    .filter((facing) => facing && (
      (typeof facing.arc_id === 'string' && facing.arc_id)
      || (typeof facing.id === 'string' && facing.id)
    ))
    .map((facing) => ({
      ...facing,
      id: typeof facing.arc_id === 'string' && facing.arc_id ? facing.arc_id : facing.id,
    }));
  if (selected != null) return facings.find((facing) => facing.id === selected) || null;
  if (facings.length === 0) return null;
  const current = facings.findIndex((facing) => (
    facing.id === view.focused_facing || facing.label === view.focused_facing
  ));
  return facings[(current + 1 + facings.length) % facings.length];
}

/** Register every shipped Sensors/Science control on one console registry. */
export function registerSensorScienceActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('Sensors action registration requires a registry');
  }
  const getState = typeof options.getState === 'function' ? options.getState : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;
  const send = (actionId, correlation, inputMs, name, payload) => {
    const correlated = correlatedPayload(actionId, correlation, inputMs, payload);
    if (!correlated || !sendAction) return false;
    sendAction(name, correlated);
    return true;
  };

  registry.register(SENSORS_TARGET_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = sensorsActionView(getState());
    const uuid = view && visibleTarget(view, detail && detail.uuid);
    return uuid
      ? send(actionId, correlation, inputMs, 'set_sensors_target', { uuid })
      : false;
  });

  registry.register(SENSORS_SCAN_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = sensorsActionView(getState());
    const uuid = view && typeof view.target_uuid === 'string' ? view.target_uuid : '';
    const requested = detail && detail.uuid;
    if (!view || !view.scan || !view.scan.capable || !uuid
        || (requested != null && requested !== uuid)) return false;
    return send(actionId, correlation, inputMs, 'scan_target', { uuid });
  });

  registry.register(SENSORS_VIEWSCREEN_ACTION, ({ actionId, correlation, inputMs } = {}) => {
    const view = sensorsActionView(getState());
    return view
      ? send(actionId, correlation, inputMs, 'set_view', { direction: 'SensorsRadar' })
      : false;
  });

  registry.register(SENSORS_CANCEL_IMPULSE_ACTION, ({
    actionId, correlation, inputMs,
  } = {}) => {
    const view = sensorsActionView(getState());
    if (!view || view.sensors_auto || !(Number(view.impulse_charge_progress) > 0)) return false;
    return send(actionId, correlation, inputMs, 'cancel_impulse', {});
  });

  registry.register(SCIENCE_SHIELD_FOCUS_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = shieldsActionView(getState());
    if (!view || view.shields_auto || view.auto) return false;
    const facing = facingChoice(view, detail && detail.arc_id);
    if (!facing) return false;
    const isFocused = view.focused_facing === facing.id
      || view.focused_facing === facing.label;
    const focused = detail && typeof detail.focused === 'boolean'
      ? detail.focused : detail && detail.arc_id ? !isFocused : true;
    return send(actionId, correlation, inputMs, 'set_shield_focus', {
      arc_id: facing.id,
      focused,
    });
  });

  return registry;
}

export function createSensorScienceActionRegistry(options = {}) {
  return registerSensorScienceActions(createSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
  }), options);
}

if (typeof window !== 'undefined') {
  window.registerSensorScienceActions = registerSensorScienceActions;
}
