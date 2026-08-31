/**
 * Context-scoped Tactical semantic actions.
 *
 * These adapters deliberately choose only from the live Tactical projection.
 * They never alter a target, readiness, ammunition, cooldown or Combat Lock
 * locally: the existing action map and admitted command consumers remain the
 * sole authority for those facts.
 */

import { familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const TACTICAL_ACTION_CONTEXT = 'tactical';
export const TACTICAL_TARGET_ACTION_ID = 'tactical.target-selection';
export const TACTICAL_PHASER_MODE_ACTION_ID = 'tactical.phaser-mode';
export const TACTICAL_PHASER_FIRE_ACTION_ID = 'tactical.phaser-fire';
export const TACTICAL_BLASTER_CHARGE_ACTION_ID = 'tactical.blaster-charge';
export const TACTICAL_BLASTER_FIRE_ACTION_ID = 'tactical.blaster-fire';
export const TACTICAL_BLASTER_CANCEL_ACTION_ID = 'tactical.blaster-cancel';
export const TACTICAL_TORPEDO_VOLLEY_DOWN_ACTION_ID = 'tactical.torpedo-volley-down';
export const TACTICAL_TORPEDO_VOLLEY_UP_ACTION_ID = 'tactical.torpedo-volley-up';
export const TACTICAL_TORPEDO_FIRE_ACTION_ID = 'tactical.torpedo-fire';

function action(id, label, accessibility, keyboard, gamepad) {
  return Object.freeze({
    id,
    contexts: Object.freeze([TACTICAL_ACTION_CONTEXT]),
    labelId: `semantic_action.tactical.${label}.label`,
    accessibilityLabelId: `semantic_action.tactical.${accessibility}.accessibility`,
    authoritativeFeedback: true,
    bindings: Object.freeze([keyboard, gamepad]),
  });
}

const key = (code) => Object.freeze({
  type: 'keyboard', code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false,
});
const button = (control) => Object.freeze({ type: 'gamepad', input: 'button', control });
const dpad = (control) => Object.freeze({ type: 'gamepad', input: 'dpad', control });
const axis = (control, direction) => Object.freeze({
  type: 'gamepad', input: 'axis', control, direction, threshold: 0.5,
});

export const TACTICAL_ACTIONS = Object.freeze([
  action(TACTICAL_TARGET_ACTION_ID, 'target', 'target', key('KeyT'), dpad('dpad-right')),
  action(TACTICAL_PHASER_MODE_ACTION_ID, 'phaser_mode', 'phaser_mode', key('KeyM'), dpad('dpad-up')),
  action(TACTICAL_PHASER_FIRE_ACTION_ID, 'phaser_fire', 'phaser_fire', key('KeyF'), button('face-bottom')),
  action(TACTICAL_BLASTER_CHARGE_ACTION_ID, 'blaster_charge', 'blaster_charge', key('KeyC'), dpad('dpad-left')),
  action(TACTICAL_BLASTER_FIRE_ACTION_ID, 'blaster_fire', 'blaster_fire', key('KeyB'), dpad('dpad-down')),
  action(TACTICAL_BLASTER_CANCEL_ACTION_ID, 'blaster_cancel', 'blaster_cancel', key('KeyX'), axis('left-stick-x', 'negative')),
  action(TACTICAL_TORPEDO_VOLLEY_DOWN_ACTION_ID, 'torpedo_volley_down', 'torpedo_volley_down', key('KeyQ'), axis('left-stick-x', 'positive')),
  action(TACTICAL_TORPEDO_VOLLEY_UP_ACTION_ID, 'torpedo_volley_up', 'torpedo_volley_up', key('KeyE'), axis('left-stick-y', 'negative')),
  action(TACTICAL_TORPEDO_FIRE_ACTION_ID, 'torpedo_fire', 'torpedo_fire', key('KeyG'), axis('left-stick-y', 'positive')),
]);

export function tacticalActionView(state) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, TACTICAL_ACTION_CONTEXT);
  return Object.keys(projected).length > 0 ? projected : state;
}

function correlatedPayload(actionId, correlation, inputMs, payload) {
  if (typeof correlation !== 'string' || !correlation) return null;
  return { ...payload, correlation, semantic_action: actionId, __input_ms: inputMs };
}

function entryWithId(values, id) {
  return (Array.isArray(values) ? values : [])
    .find((entry) => entry && entry.id === id) || null;
}

// Explicit visible controls retain their exact bank/tube identity; these
// predicates are for an unqualified keyboard/gamepad shortcut only. They mirror
// the panels' enablement so a shortcut cannot pick a cooling/offline first item
// when a later visible item is actually operable.
function selectedOrEligible(values, selected, eligible) {
  if (selected != null) return entryWithId(values, selected);
  return (Array.isArray(values) ? values : []).find((entry) => (
    entry && typeof entry.id === 'string' && entry.id && eligible(entry)
  )) || null;
}

function readinessReady(entry) {
  const readiness = entry && entry.readiness;
  return readiness && typeof readiness.blocking_reason === 'string'
    ? !!readiness.ready && readiness.blocking_reason === 'Ready'
    : null;
}

function readinessOffline(entry) {
  return entry && entry.readiness && entry.readiness.blocking_reason === 'Offline';
}

function phaserReady(view, bank) {
  if (!view || view.phaser_mode === 'Auto') return false;
  const readiness = readinessReady(bank);
  return readiness == null
    ? view.target_valid !== false && !!bank.fire_ready && !bank.on_cooldown
    : readiness;
}

function blasterCharging(bank) {
  return !bank.on_cooldown && Number(bank.charge_progress || 0) > 0;
}

function blasterStartReady(bank) {
  const readiness = readinessReady(bank);
  return readiness == null
    ? bank.fire_ready !== false && !bank.on_cooldown && !blasterCharging(bank)
    : readiness;
}

function tubeCanFire(tube) {
  const loaded = typeof tube.loaded_count === 'number' ? tube.loaded_count > 0 : !!tube.loaded;
  return loaded && !readinessOffline(tube);
}

function nextTarget(view, selected) {
  const candidates = (Array.isArray(view.blips) ? view.blips : [])
    .filter((blip) => blip && typeof blip.uuid === 'string' && blip.uuid
      && blip.kind !== 'waypoint' && blip.selectable !== false)
    .map((blip) => blip.uuid);
  if (selected != null) return candidates.includes(selected) ? selected : null;
  if (candidates.length === 0) return null;
  const current = candidates.indexOf(view.target_uuid);
  return candidates[(current + 1 + candidates.length) % candidates.length];
}

function tube(view, selected, eligible) {
  const tubes = Array.isArray(view.tubes) ? view.tubes : [];
  return selectedOrEligible(tubes, selected, eligible || (() => true));
}

/** Register Tactical adapters against this console document's existing transport. */
export function registerTacticalActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('tactical action registration requires a registry');
  }
  const getState = typeof options.getState === 'function' ? options.getState : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;
  const send = (actionId, correlation, inputMs, name, payload) => {
    const correlated = correlatedPayload(actionId, correlation, inputMs, payload);
    if (!correlated || !sendAction) return false;
    sendAction(name, correlated);
    return true;
  };

  registry.register(TACTICAL_ACTIONS[0], ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = tacticalActionView(getState());
    const uuid = view && nextTarget(view, detail && detail.uuid);
    return uuid ? send(actionId, correlation, inputMs, 'set_target', { uuid }) : false;
  });
  registry.register(TACTICAL_ACTIONS[1], ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = tacticalActionView(getState());
    if (!view || view.tactical_auto) return false;
    const mode = detail && detail.mode;
    const next = mode === 'Auto' || mode === 'Manual'
      ? mode : view.phaser_mode === 'Auto' ? 'Manual' : 'Auto';
    return send(actionId, correlation, inputMs, 'set_phaser_mode', { mode: next });
  });
  registry.register(TACTICAL_ACTIONS[2], ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = tacticalActionView(getState());
    const bank = view && selectedOrEligible(view.banks, detail && detail.bank,
      (entry) => phaserReady(view, entry));
    return bank ? send(actionId, correlation, inputMs, 'fire_phaser', { bank: bank.id }) : false;
  });
  registry.register(TACTICAL_ACTIONS[3], ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = tacticalActionView(getState());
    const bank = view && selectedOrEligible(view.blasters, detail && detail.bank, blasterStartReady);
    return bank ? send(actionId, correlation, inputMs, 'charge_blaster_start', { bank: bank.id }) : false;
  });
  registry.register(TACTICAL_ACTIONS[4], ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = tacticalActionView(getState());
    const bank = view && selectedOrEligible(view.blasters, detail && detail.bank, blasterStartReady);
    return bank ? send(actionId, correlation, inputMs, 'fire_blaster', { bank: bank.id }) : false;
  });
  registry.register(TACTICAL_ACTIONS[5], ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = tacticalActionView(getState());
    const bank = view && selectedOrEligible(view.blasters, detail && detail.bank, blasterCharging);
    return bank ? send(actionId, correlation, inputMs, 'charge_blaster_cancel', { bank: bank.id }) : false;
  });
  const volley = (delta) => ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = tacticalActionView(getState());
    const selected = tube(view || {}, detail && detail.tube, (entry) => {
      const count = entry.target_count;
      const max = typeof entry.volley_max === 'number' ? entry.volley_max : 1;
      return typeof count === 'number' && count + delta >= 0 && count + delta <= max;
    });
    if (!selected || typeof selected.target_count !== 'number') return false;
    const max = typeof selected.volley_max === 'number' ? selected.volley_max : 1;
    const count = selected.target_count + delta;
    if (count < 0 || count > max) return false;
    return send(actionId, correlation, inputMs, 'set_torpedo_volley_target', {
      tube: selected.id, count,
    });
  };
  registry.register(TACTICAL_ACTIONS[6], volley(-1));
  registry.register(TACTICAL_ACTIONS[7], volley(1));
  registry.register(TACTICAL_ACTIONS[8], ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = tacticalActionView(getState());
    const selected = tube(view || {}, detail && detail.tube, tubeCanFire);
    return selected ? send(actionId, correlation, inputMs, 'fire_torpedo', {
      tube: selected.id, target_uuid: view.target_uuid || null,
    }) : false;
  });
  return registry;
}

export function createTacticalActionRegistry(options = {}) {
  return registerTacticalActions(createSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
  }), options);
}

if (typeof window !== 'undefined') window.registerTacticalActions = registerTacticalActions;
