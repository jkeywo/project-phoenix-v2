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
import { EXTERNAL_REPAIR_TARGET } from '../repair-dispatch.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const POWER_ACTION_CONTEXT = 'power';
export const REPAIR_ACTION_CONTEXT = 'repair';
export const ENGINEERING_ACTION_CONTEXT = 'engineering';
export const CAPTAIN_ENGINEERING_ACTION_CONTEXT = 'captain';

export const POWER_DECREASE_ACTION_ID = 'power.decrease-allocation';
export const POWER_INCREASE_ACTION_ID = 'power.increase-allocation';
export const REPAIR_DISPATCH_ACTION_ID = 'repair.dispatch-team';
export const REPAIR_RECALL_ACTION_ID = 'repair.recall-team';
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
// Internal recall (issue #1385). Its own identity rather than a mode of the
// dispatch above: dispatch chooses a destination for a team that is standing
// still, recall chooses nothing at all for a team that is already moving, and
// folding them into one toggle would make the parameter-free activation depend
// on which team the pointer last happened to open.
export const REPAIR_RECALL_ACTION = action(
  REPAIR_RECALL_ACTION_ID,
  REPAIR_CONTEXTS,
  'repair',
  'recall_team',
  keyboard('KeyR', { shiftKey: true }),
  button('right-shoulder'),
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
  REPAIR_RECALL_ACTION,
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

// How far the stepper may go in `direction`, off the group's OWN authored
// bounds. The `!= null` tests are load-bearing rather than defensive style: a
// group whose hull authored `min_level = 0` may be taken COLD (issue #1395),
// and a truthiness check would read that 0 as "absent" and quietly floor the
// group at 1 — refusing the one order the feature exists for.
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

// Which slot the ship's one field-repair claim holds, or `null` (issue #1386).
// Authoritative: the host names it on the repair blackboard. Before #1386 this
// was reconstructed here (and in three other places) by truncating the idle
// list, which was only ever right because nobody could choose which team went.
function abroadTeamId(view) {
  const idx = view && view.external_dispatch ? view.external_dispatch.team_idx : null;
  return Number.isInteger(idx) ? idx : null;
}

function chooseRepairDispatch(view, detail) {
  if (!view || view.repair_auto) return null;
  const teams = Array.isArray(view.teams) ? view.teams : [];
  const targets = Array.isArray(view.dispatch_targets) ? view.dispatch_targets : [];
  const requestedTeam = detail && Number.isInteger(detail.team_idx) ? detail.team_idx : null;
  const requestedTarget = detail && typeof detail.target === 'string' ? detail.target : null;
  // The team abroad is Idle on the wire but is not available: it is already
  // somewhere. Excluded BY NAME off the claim, so parameter-free input does not
  // select the one slot the owner must refuse.
  const abroad = abroadTeamId(view);
  const available = teams.filter((entry) => (
    entry && Number.isInteger(entry.id) && entry.status === 'idle' && entry.id !== abroad
  ));
  const team = requestedTeam == null
    ? available[0]
    : available.find((entry) => entry.id === requestedTeam);
  if (!team) return null;
  // The field target is not in `dispatch_targets` and never will be — the host
  // builds that list out of the hull's own stations, and this destination is off
  // the ship (issue #1386). It is reachable only by naming it, so a
  // parameter-free activation still picks a station: "send somebody somewhere"
  // should not quietly mean "send somebody to whatever Tactical is looking at".
  if (requestedTarget === EXTERNAL_REPAIR_TARGET) {
    // Offered only on a hull that authored the capability, and only while the
    // one claim is free — the two stale-UI cases the owner refuses.
    const canReachTheField = view.external_dispatch != null && abroad == null;
    return canReachTheField
      ? { team_idx: team.id, target: EXTERNAL_REPAIR_TARGET }
      : null;
  }
  const target = requestedTarget == null
    ? targets.find((entry) => entry && typeof entry.id === 'string' && entry.id)
    : targets.find((entry) => entry && entry.id === requestedTarget);
  return target ? { team_idx: team.id, target: target.id } : null;
}

// Which teams a recall may name (issues #1385, #1386). A team is recallable
// exactly while it is OUT — travelling to a station, working on site, or abroad
// on the field target. An idle team has nothing to recall and a returning one is
// already on its way, so both are refused by the owner; mirroring that reading
// here keeps a parameter-free activation from selecting a slot the host must
// refuse. It is a reading of the AUTHORITATIVE status and the AUTHORITATIVE
// claim, not a second copy of the host's rule: the owner still decides, and
// still answers on the feedback seam.
//
// The abroad team has to be named separately because its wire status is `Idle`:
// it never walked anywhere on this hull, so the slot machinery has nothing to
// say about it and the claim is the only thing that does.
function isRecallableTeam(entry, abroad) {
  return !!entry
    && Number.isInteger(entry.id)
    && (entry.status === 'travelling'
      || entry.status === 'repairing'
      || (entry.status === 'idle' && entry.id === abroad));
}

function chooseRepairRecall(view, detail) {
  if (!view || view.repair_auto) return null;
  const teams = Array.isArray(view.teams) ? view.teams : [];
  const abroad = abroadTeamId(view);
  const requested = detail && Number.isInteger(detail.team_idx) ? detail.team_idx : null;
  const candidates = requested == null
    ? teams
    : teams.filter((entry) => entry && entry.id === requested);
  const team = candidates.find((entry) => isRecallableTeam(entry, abroad));
  return team ? { team_idx: team.id } : null;
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

  registry.register(REPAIR_RECALL_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const state = getState();
    const view = repairActionView(state);
    const selected = chooseRepairRecall(view, detail);
    const controlSystemId = ownerSystemId(state, view, 'repair');
    return selected && controlSystemId
      ? send(actionId, correlation, inputMs, 'recall_repair_team', {
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
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const state = getState();
    const view = repairActionView(state);
    const dispatch = view && view.external_dispatch;
    const systemId = ownerSystemId(state, view, 'repair');
    if (!dispatch || !systemId) return false;
    // This explicitly requests the field destination, unlike the generic
    // parameter-free dispatch action. Preserve a requested team, otherwise
    // use the same first-available convention as internal dispatch. Every human
    // input names its team on the wire; the fieldless verbs remain for AI.
    const recalling = dispatch.target != null;
    if (recalling && !Number.isInteger(dispatch.team_idx)) return false;
    if (!recalling && (!dispatch.candidate_name || dispatch.candidate_refusal)) return false;
    const selected = recalling
      ? chooseRepairRecall(view, { team_idx: dispatch.team_idx })
      : chooseRepairDispatch(view, { ...detail, target: EXTERNAL_REPAIR_TARGET });
    return selected
      ? send(actionId, correlation, inputMs,
          recalling ? 'recall_repair_team' : 'dispatch_repair_team',
          { ...selected, control_system_id: systemId })
      : false;
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
