/**
 * Context-scoped Sensors and Science semantic actions.
 *
 * The adapters choose only from the live authoritative Console Family
 * projections. They never invent a contact, scan result, impulse phase or
 * shield facing, and they never mutate client gameplay state optimistically.
 * The existing action map remains the sole wire builder and command admission
 * remains the sole authority gate.
 */

import { defineStationAction, createCorrelatedActionSender, keyboard, gamepad } from './action-support.js';

import { familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const SENSORS_TARGET_ACTION_ID = 'sensors.target-selection';
export const SENSORS_SCAN_ACTION_ID = 'sensors.scan';
export const SENSORS_CANCEL_SCAN_ACTION_ID = 'sensors.cancel-scan';
export const SENSORS_ATTACH_REFERENCE_ACTION_ID = 'sensors.attach-reference';
export const SENSORS_PASSIVE_ACTION_ID = 'sensors.passive-enabled';
export const SENSORS_VIEWSCREEN_ACTION_ID = 'sensors.viewscreen';
export const SENSORS_CANCEL_IMPULSE_ACTION_ID = 'sensors.cancel-impulse';
export const SCIENCE_SHIELD_FOCUS_ACTION_ID = 'science.shield-focus';
export const SCIENCE_SHIELD_RAISED_ACTION_ID = 'science.shield-raised';
export const SCIENCE_SHIELD_RECOVERY_ACTION_ID = 'science.shield-recovery';

const SENSOR_BEARING_CONTEXTS = Object.freeze(['sensors', 'science', 'captain', 'tactical']);
const SHIELD_FOCUS_CONTEXTS = Object.freeze(['science', 'captain', 'engineering', 'shields']);

/** Every console context which mounts one of these shipped controls. */
export const SENSOR_SCIENCE_ACTION_CONTEXTS = Object.freeze([
  'sensors', 'science', 'captain', 'tactical', 'engineering', 'shields',
]);

function action(id, contexts, label, keyboardCode, gamepadInput, gamepadControl, keyboardModifiers = {}) {
  return defineStationAction({ id, contexts, labelKey: label,
    bindings: [keyboard(keyboardCode, keyboardModifiers), gamepad(gamepadInput, gamepadControl)] });
}

export const SENSORS_TARGET_ACTION = action(
  SENSORS_TARGET_ACTION_ID,
  SENSOR_BEARING_CONTEXTS,
  'sensors.target_selection',
  'KeyS',
  'button',
  'face-left',
);

export const SENSORS_SCAN_ACTION = action(
  SENSORS_SCAN_ACTION_ID,
  Object.freeze(['captain', 'science', 'sensors']),
  'sensors.scan',
  'KeyN',
  'button',
  'face-top',
  { shiftKey: true },
);
export const SENSORS_CANCEL_SCAN_ACTION = defineStationAction({ id: SENSORS_CANCEL_SCAN_ACTION_ID,
  contexts: ['captain', 'science', 'sensors'], labelKey: 'sensors.cancel_scan', bindings: [null, null] });
export const SENSORS_PASSIVE_ACTION = defineStationAction({ id: SENSORS_PASSIVE_ACTION_ID,
  contexts: ['captain', 'science', 'sensors'], labelKey: 'sensors.passive_enabled', bindings: [null, null] });

export const SENSORS_ATTACH_REFERENCE_ACTION = defineStationAction({ id: SENSORS_ATTACH_REFERENCE_ACTION_ID,
  contexts: ['captain', 'science', 'sensors', 'tactical'], labelKey: 'sensing.reference.attach', bindings: [null, null] });

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

export const SCIENCE_SHIELD_RAISED_ACTION = defineStationAction({
  id: SCIENCE_SHIELD_RAISED_ACTION_ID, contexts: SHIELD_FOCUS_CONTEXTS,
  labelKey: 'science.shield_raised', bindings: [null, null],
});
export const SCIENCE_SHIELD_RECOVERY_ACTION = defineStationAction({
  id: SCIENCE_SHIELD_RECOVERY_ACTION_ID, contexts: SHIELD_FOCUS_CONTEXTS,
  labelKey: 'science.shield_recovery', bindings: [null, null],
});

export const SENSOR_SCIENCE_ACTIONS = Object.freeze([
  SENSORS_TARGET_ACTION,
  SENSORS_SCAN_ACTION,
  SENSORS_CANCEL_SCAN_ACTION,
  SENSORS_PASSIVE_ACTION,
  SENSORS_ATTACH_REFERENCE_ACTION,
  SENSORS_VIEWSCREEN_ACTION,
  SENSORS_CANCEL_IMPULSE_ACTION,
  SCIENCE_SHIELD_FOCUS_ACTION,
  SCIENCE_SHIELD_RAISED_ACTION,
  SCIENCE_SHIELD_RECOVERY_ACTION,
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

function visibleTarget(view, selected) {
  const candidates = (Array.isArray(view && view.blips) ? view.blips : [])
    .filter((blip) => blip && blip.selectable !== false && typeof blip.uuid === 'string' && blip.uuid)
    .map((blip) => blip.uuid);
  if (selected != null) return candidates.includes(selected) ? selected : null;
  if (candidates.length === 0) return null;
  const current = candidates.indexOf(view && view.target_uuid);
  return candidates[(current + 1 + candidates.length) % candidates.length];
}

function facingChoice(view, selected) {
  const facings = (Array.isArray(view && view.facings) ? view.facings : [])
    .filter((facing) => facing && facing.operation?.focus_available !== false && (
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
  const send = createCorrelatedActionSender(sendAction);

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
    const scans=view?.scans || [], scan=detail?.system_id ? scans.find(row=>row.system_id===detail.system_id) : scans.length===1 ? scans[0] : null;
    if (!view || view.target_track?.predicted || !scan?.capable || !scan.controllable || !scan.system_id || !uuid
        || (requested != null && requested !== uuid)) return false;
    const aim=detail?.aim ?? scan.default_aim ?? 'general', intrusive=detail?.intrusive ?? false;
    const supported=(scan.aims?.length ? scan.aims : [{aim:'general',intrusive:false}]).find(row=>row.aim===aim);
    if (!supported || typeof intrusive!=='boolean' || (intrusive && !supported.intrusive)) return false;
    return send(actionId, correlation, inputMs, 'scan_target', { uuid, control_system_id: scan.system_id, aim, intrusive });
  });
  registry.register(SENSORS_CANCEL_SCAN_ACTION, ({ actionId, correlation, inputMs, detail } = {}) => {
    const scans = sensorsActionView(getState())?.scans || [];
    const scan = detail?.system_id ? scans.find(row=>row.system_id===detail.system_id) : scans.length===1 ? scans[0] : null;
    if (!scan?.controllable || !scan.system_id || !scan.focused || (detail?.system_id && detail.system_id !== scan.system_id)) return false;
    return send(actionId, correlation, inputMs, 'cancel_focused_scan', { control_system_id: scan.system_id });
  });
  registry.register(SENSORS_PASSIVE_ACTION, ({ actionId, correlation, inputMs, detail } = {}) => {
    const row = sensorsActionView(getState())?.passive?.find(row => row.system_id === detail?.system_id);
    if (!row?.controllable || typeof detail?.enabled !== 'boolean') return false;
    return send(actionId, correlation, inputMs, 'set_passive_enabled', { control_system_id: row.system_id, enabled: detail.enabled });
  });

  registry.register(SENSORS_ATTACH_REFERENCE_ACTION, ({ actionId, correlation, inputMs, detail } = {}) => {
    const row = sensorsActionView(getState())?.reference_controls?.find(row => row.system_id === detail?.system_id);
    if (!row?.controllable || !row.contacts?.includes(detail?.contact) || !row.records?.some(record => record.reference?.id === detail?.reference)) return false;
    return send(actionId, correlation, inputMs, 'attach_sensor_reference', { control_system_id: row.system_id, contact: detail.contact, reference: detail.reference });
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
    if (!facing || (facing.operation && !facing.controllable)) return false;
    const isFocused = view.focused_facing === facing.id
      || view.focused_facing === facing.label;
    const focused = detail && typeof detail.focused === 'boolean'
      ? detail.focused : detail && detail.arc_id ? !isFocused : true;
    return send(actionId, correlation, inputMs, 'set_shield_focus', {
      arc_id: facing.id,
      ...(facing.system_id ? { control_system_id: facing.system_id } : {}),
      focused,
    });
  });

  for (const [definition, name] of [
    [SCIENCE_SHIELD_RAISED_ACTION, 'set_shield_raised'],
    [SCIENCE_SHIELD_RECOVERY_ACTION, 'set_shield_recovery'],
  ]) {
    registry.register(definition, ({ actionId, correlation, inputMs, detail } = {}) => {
      const view = shieldsActionView(getState());
      if (!view || view.shields_auto || view.auto) return false;
      const facing = view.facings?.find(f => f.system_id === detail?.system_id);
      if (!facing?.controllable || facing.delegated || !facing.operation) return false;
      const payload = { control_system_id: facing.system_id };
      if (name === 'set_shield_raised') {
        if (typeof detail?.raised !== 'boolean') return false;
        payload.raised = detail.raised;
      } else {
        if (detail?.target !== null && !facing.operation.recovery_targets?.includes(detail?.target)) return false;
        payload.target = detail.target;
      }
      return send(actionId, correlation, inputMs, name, payload);
    });
  }
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
