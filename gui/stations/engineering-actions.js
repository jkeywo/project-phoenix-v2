/**
 * Shared semantic actions for Power, internal Repair, and Engineering-only
 * auxiliary controls.
 *
 * The adapters below read only the latest authoritative family projections.
 * They retain the exact authored PowerGroupId/SystemId/team slot selected by a
 * visible control and never alter a blackboard locally.  A parameter-free
 * keyboard/gamepad activation chooses the first currently operable item in
 * authoritative display order, which gives every remappable action an honest
 * non-pointer path without inventing selection state.
 */

import { familySystemId, familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const POWER_ACTION_CONTEXT = 'power';
export const REPAIR_ACTION_CONTEXT = 'repair';
export const ENGINEERING_ACTION_CONTEXT = 'engineering';
export const CAPTAIN_ENGINEERING_ACTION_CONTEXT = 'captain';

export const POWER_DECREASE_ACTION_ID = 'power.decrease-allocation';
export const POWER_INCREASE_ACTION_ID = 'power.increase-allocation';
export const REPAIR_DISPATCH_ACTION_ID = 'repair.dispatch-team';
export const REPAIR_PRIORITY_ACTION_ID = 'repair.prioritise-system';
export const TRACTOR_TOGGLE_ACTION_ID = 'engineering.tractor';
export const UMBILICAL_TOGGLE_ACTION_ID = 'engineering.umbilical';
export const EXTERNAL_REPAIR_TOGGLE_ACTION_ID = 'repair.external-dispatch';

const POWER_CONTEXTS = Object.freeze([
  POWER_ACTION_CONTEXT,
  ENGINEERING_ACTION_CONTEXT,
  CAPTAIN_ENGINEERING_ACTION_CONTEXT,
]);
const REPAIR_CONTEXTS = Object.freeze([
  REPAIR_ACTION_CONTEXT,
  ENGINEERING_ACTION_CONTEXT,
  CAPTAIN_ENGINEERING_ACTION_CONTEXT,
]);
const ENGINEERING_CONTEXTS = Object.freeze([ENGINEERING_ACTION_CONTEXT]);
const EXTERNAL_REPAIR_CONTEXTS = Object.freeze([
  REPAIR_ACTION_CONTEXT,
  ENGINEERING_ACTION_CONTEXT,
]);

// Contexts that own an Engineering-family surface. `captain` is deliberately
// absent: it is an activation context for Courier's compact composite, not a
// reason for every dedicated Captain console to install these adapters.
export const ENGINEERING_ACTION_REGISTRATION_CONTEXTS = Object.freeze([
  POWER_ACTION_CONTEXT,
  REPAIR_ACTION_CONTEXT,
  ENGINEERING_ACTION_CONTEXT,
]);

const keyboard = (code, modifiers = {}) => Object.freeze({
  type: 'keyboard', code,
  ctrlKey: !!modifiers.ctrlKey,
  shiftKey: !!modifiers.shiftKey,
  altKey: !!modifiers.altKey,
  metaKey: !!modifiers.metaKey,
});
const button = (control) => Object.freeze({ type: 'gamepad', input: 'button', control });
const dpad = (control) => Object.freeze({ type: 'gamepad', input: 'dpad', control });
const axis = (control, direction) => Object.freeze({
  type: 'gamepad', input: 'axis', control, direction, threshold: 0.5,
});

function action(id, contexts, family, label, keyboardBinding, gamepadBinding) {
  return Object.freeze({
    id,
    contexts,
    labelId: `semantic_action.${family}.${label}.label`,
    accessibilityLabelId: `semantic_action.${family}.${label}.accessibility`,
    authoritativeFeedback: true,
    bindings: Object.freeze([keyboardBinding, gamepadBinding]),
  });
}

export const POWER_DECREASE_ACTION = action(
  POWER_DECREASE_ACTION_ID,
  POWER_CONTEXTS,
  'power',
  'decrease_allocation',
  keyboard('KeyQ'),
  axis('right-stick-x', 'negative'),
);
export const POWER_INCREASE_ACTION = action(
  POWER_INCREASE_ACTION_ID,
  POWER_CONTEXTS,
  'power',
  'increase_allocation',
  keyboard('KeyE'),
  axis('right-stick-x', 'positive'),
);
export const REPAIR_DISPATCH_ACTION = action(
  REPAIR_DISPATCH_ACTION_ID,
  REPAIR_CONTEXTS,
  'repair',
  'dispatch_team',
  keyboard('KeyD', { shiftKey: true }),
  dpad('dpad-up'),
);
export const REPAIR_PRIORITY_ACTION = action(
  REPAIR_PRIORITY_ACTION_ID,
  REPAIR_CONTEXTS,
  'repair',
  'prioritise_system',
  keyboard('KeyP', { shiftKey: true }),
  dpad('dpad-down'),
);
export const TRACTOR_TOGGLE_ACTION = action(
  TRACTOR_TOGGLE_ACTION_ID,
  ENGINEERING_CONTEXTS,
  'engineering',
  'tractor',
  keyboard('KeyT'),
  button('face-bottom'),
);
export const UMBILICAL_TOGGLE_ACTION = action(
  UMBILICAL_TOGGLE_ACTION_ID,
  ENGINEERING_CONTEXTS,
  'engineering',
  'umbilical',
  keyboard('KeyU'),
  dpad('dpad-left'),
);
export const EXTERNAL_REPAIR_TOGGLE_ACTION = action(
  EXTERNAL_REPAIR_TOGGLE_ACTION_ID,
  EXTERNAL_REPAIR_CONTEXTS,
  'repair',
  'external_dispatch',
  keyboard('KeyX'),
  dpad('dpad-right'),
);

export const ENGINEERING_ACTIONS = Object.freeze([
  POWER_DECREASE_ACTION,
  POWER_INCREASE_ACTION,
  REPAIR_DISPATCH_ACTION,
  REPAIR_PRIORITY_ACTION,
  TRACTOR_TOGGLE_ACTION,
  UMBILICAL_TOGGLE_ACTION,
  EXTERNAL_REPAIR_TOGGLE_ACTION,
]);

function projectedOrFlat(state, family, recognisesFlat) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, family);
  if (Object.keys(projected).length > 0) return projected;
  return recognisesFlat(state) ? state : null;
}

export function powerActionView(state) {
  return projectedOrFlat(state, 'power', (value) => (
    Array.isArray(value.groups) || Array.isArray(value.consoles)
  ));
}

export function repairActionView(state) {
  return projectedOrFlat(state, 'repair', (value) => (
    Array.isArray(value.teams)
      || Array.isArray(value.dispatch_targets)
      || Array.isArray(value.damaged_systems)
      || value.external_dispatch != null
  ));
}

function correlatedPayload(actionId, correlation, inputMs, payload) {
  if (typeof correlation !== 'string' || !correlation) return null;
  return { ...payload, correlation, semantic_action: actionId, __input_ms: inputMs };
}

function ownerSystemId(state, view, family) {
  const candidates = [view && view.system_id, familySystemId(state, family)];
  return candidates.find((value) => typeof value === 'string' && value) || null;
}

function powerGroups(view) {
  return Array.isArray(view?.groups) ? view.groups
    : Array.isArray(view?.consoles) ? view.consoles : [];
}

function commandedLevel(group) {
  if (!group) return 0;
  return group.commanded_level != null && group.commanded_level > 0
    ? group.commanded_level : Number(group.level || 0);
}

function powerLimit(group, direction) {
  return direction < 0
    ? Number(group.min_level != null ? group.min_level : 1)
    : Number(group.max_level != null ? group.max_level : 4);
}

function choosePowerChange(view, direction, detail) {
  if (!view || view.power_auto || view.locked) return null;
  const groups = powerGroups(view);
  const explicitId = detail && typeof detail.target === 'string' ? detail.target : null;
  const candidates = explicitId
    ? groups.filter((group) => group && group.id === explicitId)
    : groups;
  for (const group of candidates) {
    if (!group || typeof group.id !== 'string' || !group.id) continue;
    const current = commandedLevel(group);
    const limit = powerLimit(group, direction);
    const requested = detail && Number.isInteger(detail.level)
      ? detail.level : current + direction;
    if ((direction < 0 && requested < current && requested >= limit)
        || (direction > 0 && requested > current && requested <= limit)) {
      return { target: group.id, level: requested };
    }
  }
  return null;
}

function chooseRepairDispatch(view, detail) {
  if (!view || view.repair_auto) return null;
  const teams = Array.isArray(view.teams) ? view.teams : [];
  const targets = Array.isArray(view.dispatch_targets) ? view.dispatch_targets : [];
  const requestedTeam = detail && Number.isInteger(detail.team_idx) ? detail.team_idx : null;
  const requestedTarget = detail && typeof detail.target === 'string' ? detail.target : null;
  // A live field-repair dispatch reserves one otherwise-Idle slot. The
  // authoritative RepairTeams pool keeps the lowest-numbered idle slots free
  // and takes commitments from the end of that list; mirror that ordering so
  // parameter-free input does not select a slot the owner must refuse.
  const idle = teams.filter((entry) => (
    entry && Number.isInteger(entry.id) && entry.status === 'idle'
  ));
  const externallyCommitted = view.external_dispatch?.target != null ? 1 : 0;
  const available = idle.slice(0, idle.length - Math.min(idle.length, externallyCommitted));
  const team = requestedTeam == null
    ? available[0]
    : available.find((entry) => entry.id === requestedTeam);
  const target = requestedTarget == null
    ? targets.find((entry) => entry && typeof entry.id === 'string' && entry.id)
    : targets.find((entry) => entry && entry.id === requestedTarget);
  return team && target ? { team_idx: team.id, target: target.id } : null;
}

function chooseRepairPriority(view, detail) {
  if (!view || view.repair_auto) return null;
  const rows = Array.isArray(view.damaged_systems) ? view.damaged_systems : [];
  const requested = detail && typeof detail.system_id === 'string' ? detail.system_id : null;
  const candidates = requested
    ? rows.filter((row) => row && row.system_id === requested)
    : rows;
  const row = candidates.find((entry) => (
    entry && typeof entry.system_id === 'string' && entry.system_id
      && entry.prioritisable === true
  ));
  return row ? { system_id: row.system_id } : null;
}

/** Register every shipped Engineering/Power/Repair adapter on one registry. */
export function registerEngineeringActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('engineering action registration requires a registry');
  }
  const getState = typeof options.getState === 'function' ? options.getState : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;
  const send = (actionId, correlation, inputMs, name, payload = {}) => {
    const correlated = correlatedPayload(actionId, correlation, inputMs, payload);
    if (!correlated || !sendAction) return false;
    sendAction(name, correlated);
    return true;
  };

  const power = (direction) => ({ actionId, correlation, inputMs, detail } = {}) => {
    const state = getState();
    const view = powerActionView(state);
    const selected = choosePowerChange(view, direction, detail);
    const controlSystemId = ownerSystemId(state, view, 'power');
    return selected && controlSystemId
      ? send(actionId, correlation, inputMs, 'set_power', {
          ...selected, control_system_id: controlSystemId,
        })
      : false;
  };
  registry.register(POWER_DECREASE_ACTION, power(-1));
  registry.register(POWER_INCREASE_ACTION, power(1));

  registry.register(REPAIR_DISPATCH_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const state = getState();
    const view = repairActionView(state);
    const selected = chooseRepairDispatch(view, detail);
    const controlSystemId = ownerSystemId(state, view, 'repair');
    return selected && controlSystemId
      ? send(actionId, correlation, inputMs, 'dispatch_repair_team', {
          ...selected, control_system_id: controlSystemId,
        })
      : false;
  });

  registry.register(REPAIR_PRIORITY_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const state = getState();
    const view = repairActionView(state);
    const selected = chooseRepairPriority(view, detail);
    const controlSystemId = ownerSystemId(state, view, 'repair');
    return selected && controlSystemId
      ? send(actionId, correlation, inputMs, 'set_repair_target_priority', {
          ...selected, control_system_id: controlSystemId,
        })
      : false;
  });

  registry.register(TRACTOR_TOGGLE_ACTION, ({ actionId, correlation, inputMs } = {}) => {
    const state = getState();
    const view = familyView(state, 'tractor');
    const systemId = ownerSystemId(state, view, 'tractor');
    if (!systemId) return false;
    return send(
      actionId,
      correlation,
      inputMs,
      view.engaged ? 'release_tractor' : 'engage_tractor',
      { control_system_id: systemId },
    );
  });

  registry.register(UMBILICAL_TOGGLE_ACTION, ({ actionId, correlation, inputMs } = {}) => {
    const state = getState();
    const view = familyView(state, 'umbilical');
    const systemId = ownerSystemId(state, view, 'umbilical');
    if (!systemId) return false;
    return send(
      actionId,
      correlation,
      inputMs,
      view.running ? 'stop_transfer' : 'start_transfer',
      { control_system_id: systemId },
    );
  });

  registry.register(EXTERNAL_REPAIR_TOGGLE_ACTION, ({
    actionId, correlation, inputMs,
  } = {}) => {
    const state = getState();
    const view = repairActionView(state);
    const dispatch = view && view.external_dispatch;
    const systemId = ownerSystemId(state, view, 'repair');
    if (!dispatch || !systemId) return false;
    return send(
      actionId,
      correlation,
      inputMs,
      dispatch.target != null ? 'recall_external_repair' : 'dispatch_external_repair',
      { control_system_id: systemId },
    );
  });
  return registry;
}

export function createEngineeringActionRegistry(options = {}) {
  return registerEngineeringActions(createSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
  }), options);
}

if (typeof window !== 'undefined') {
  window.registerEngineeringActions = registerEngineeringActions;
}
