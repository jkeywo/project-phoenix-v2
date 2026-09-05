import { describe, expect, it, vi } from 'vitest';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import { CAPTAIN_ACTIONS } from '../../gui/stations/captain-actions.js';
import {
  CAPTAIN_ENGINEERING_ACTION_CONTEXT,
  createEngineeringActionRegistry,
  ENGINEERING_ACTIONS,
  ENGINEERING_ACTION_CONTEXT,
  EXTERNAL_REPAIR_TOGGLE_ACTION_ID,
  POWER_ACTION_CONTEXT,
  POWER_DECREASE_ACTION_ID,
  POWER_INCREASE_ACTION_ID,
  REPAIR_ACTION_CONTEXT,
  REPAIR_DISPATCH_ACTION_ID,
  REPAIR_PRIORITY_ACTION_ID,
  TRACTOR_TOGGLE_ACTION_ID,
  UMBILICAL_TOGGLE_ACTION_ID,
} from '../../gui/stations/engineering-actions.js';

let sequence = 0;
function registry(options) {
  return createEngineeringActionRegistry({
    ...options,
    actionFeedback: new ActionFeedbackLifecycle({
      now: () => 1287,
      correlation: () => `engineering-${++sequence}`,
      onTransition: options.onTransition || (() => {}),
    }),
  });
}

const keyed = (systems, families) => ({
  systems,
  system_ids: Object.keys(systems),
  system_families: families,
});

const power = () => ({
  system_id: 'power-reactor',
  power_auto: false,
  locked: false,
  consoles: [
    { id: 'helm', level: 1, commanded_level: 3, min_level: 1, max_level: 4 },
    { id: 'weapons', level: 2, commanded_level: 2, min_level: 1, max_level: 3 },
  ],
});

const repair = () => ({
  system_id: 'repair',
  repair_auto: false,
  teams: [{ id: 0, status: 'idle' }, { id: 1, status: 'repairing' }],
  dispatch_targets: [{ id: 'core' }, { id: 'helm' }],
  damaged_systems: [
    { system_id: 'reactor-core', in_progress: true, prioritisable: false },
    { system_id: 'power-reactor', in_progress: false, prioritisable: true },
  ],
});

describe('Engineering, Power, and Repair semantic actions', () => {
  it('publishes seven authoritative actions with exactly two slots and shipped contexts', () => {
    expect(ENGINEERING_ACTIONS).toHaveLength(7);
    for (const action of ENGINEERING_ACTIONS) {
      expect(action.authoritativeFeedback).toBe(true);
      expect(action.bindings).toHaveLength(2);
      expect(action.bindings.every((binding) => binding == null || binding.type)).toBe(true);
    }
    expect(ENGINEERING_ACTIONS.find((a) => a.id === POWER_INCREASE_ACTION_ID).contexts)
      .toEqual([POWER_ACTION_CONTEXT, ENGINEERING_ACTION_CONTEXT, CAPTAIN_ENGINEERING_ACTION_CONTEXT]);
    expect(ENGINEERING_ACTIONS.find((a) => a.id === REPAIR_DISPATCH_ACTION_ID).contexts)
      .toEqual([REPAIR_ACTION_CONTEXT, ENGINEERING_ACTION_CONTEXT, CAPTAIN_ENGINEERING_ACTION_CONTEXT]);
    expect(ENGINEERING_ACTIONS.find((a) => a.id === TRACTOR_TOGGLE_ACTION_ID).contexts)
      .toEqual([ENGINEERING_ACTION_CONTEXT]);
    expect(ENGINEERING_ACTIONS.find((a) => a.id === EXTERNAL_REPAIR_TOGGLE_ACTION_ID).contexts)
      .toEqual([REPAIR_ACTION_CONTEXT, ENGINEERING_ACTION_CONTEXT]);
  });

  it('keeps Courier Captain defaults conflict-free with Captain and Navigation defaults', () => {
    const catalogue = createSemanticActionRegistry();
    for (const action of CAPTAIN_ACTIONS) catalogue.register(action);
    for (const action of ENGINEERING_ACTIONS) catalogue.register(action);
    // #1286 owns this complete Navigation catalogue in the same compact
    // Captain context. Keep the cross-track sentinel complete: the map's four
    // directed axes are just as capable of colliding as its command buttons.
    const navigationDefaults = [
      ['navigation.chart', 'KeyJ', { input: 'button', control: 'left-trigger' }],
      ['navigation.waypoint-place', 'KeyP', { input: 'button', control: 'select' }],
      ['navigation.waypoint-anchor', 'KeyA', { input: 'button', control: 'start' }],
      ['navigation.waypoint-clear', 'KeyC', { input: 'button', control: 'left-stick-button' }],
      ['navigation.contact-selection', 'KeyN', { input: 'button', control: 'right-stick-button' }],
      ['navigation.civilian-order', 'KeyD', { input: 'button', control: 'right-trigger' }],
      ['navigation.map-pan-left', 'ArrowLeft', { input: 'axis', control: 'left-stick-x', direction: 'negative' }, { ctrlKey: true }],
      ['navigation.map-pan-right', 'ArrowRight', { input: 'axis', control: 'left-stick-x', direction: 'positive' }, { ctrlKey: true }],
      ['navigation.map-pan-up', 'ArrowUp', { input: 'axis', control: 'left-stick-y', direction: 'negative' }, { ctrlKey: true }],
      ['navigation.map-pan-down', 'ArrowDown', { input: 'axis', control: 'left-stick-y', direction: 'positive' }, { ctrlKey: true }],
      ['navigation.map-zoom-in', 'Equal', { input: 'axis', control: 'right-stick-y', direction: 'negative' }],
      ['navigation.map-zoom-out', 'Minus', { input: 'axis', control: 'right-stick-y', direction: 'positive' }],
    ];
    for (const [id, code, gamepad, modifiers = {}] of navigationDefaults) {
      catalogue.register({
        id,
        contexts: ['navigation', 'captain'],
        labelId: `semantic_action.${id}.label`,
        accessibilityLabelId: `semantic_action.${id}.accessibility`,
        bindings: [
          { type: 'keyboard', code, ...modifiers },
          { type: 'gamepad', ...gamepad },
        ],
      });
    }
    expect(catalogue.list().map((entry) => entry.id)).toContain(REPAIR_PRIORITY_ACTION_ID);
  });

  it('preserves the exact authored power group and steps from commanded state', () => {
    const sendAction = vi.fn();
    const state = keyed({ 'power-reactor': power() }, { 'power-reactor': 'power' });
    const actions = registry({ getState: () => state, sendAction });

    expect(actions.activate(POWER_DECREASE_ACTION_ID, {
      context: ENGINEERING_ACTION_CONTEXT,
      source: 'control',
      detail: { target: 'helm', level: 2 },
    })).toMatchObject({ claimed: true, handled: true });
    expect(sendAction).toHaveBeenCalledWith('set_power', expect.objectContaining({
      target: 'helm', level: 2, control_system_id: 'power-reactor',
      semantic_action: POWER_DECREASE_ACTION_ID,
    }));

    expect(actions.activate(POWER_INCREASE_ACTION_ID, {
      context: ENGINEERING_ACTION_CONTEXT,
      source: 'gamepad',
    })).toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenLastCalledWith('set_power', expect.objectContaining({
      target: 'helm', level: 4, control_system_id: 'power-reactor',
      semantic_action: POWER_INCREASE_ACTION_ID,
    }));
    expect(state.systems['power-reactor'].consoles[0].commanded_level).toBe(3);
  });

  it('refuses stale, out-of-bound, automatic, and locked power requests locally', () => {
    const sendAction = vi.fn();
    let view = power();
    const actions = registry({ getState: () => view, sendAction });
    for (const detail of [
      { target: 'missing', level: 3 },
      { target: 'helm', level: 4 },
      { target: 'helm', level: 0 },
    ]) {
      expect(actions.activate(POWER_DECREASE_ACTION_ID, {
        context: POWER_ACTION_CONTEXT, detail,
      })).toMatchObject({ claimed: true, handled: false });
    }
    view = { ...view, power_auto: true };
    expect(actions.activate(POWER_INCREASE_ACTION_ID, { context: POWER_ACTION_CONTEXT }))
      .toMatchObject({ handled: false });
    view = { ...power(), locked: true };
    expect(actions.activate(POWER_INCREASE_ACTION_ID, { context: POWER_ACTION_CONTEXT }))
      .toMatchObject({ handled: false });
    expect(sendAction).not.toHaveBeenCalled();
  });

  // Issue #1395: a group whose hull authored `min_level = 0` may be taken COLD.
  // The refusal case above proves helm at 0 is still refused, so this is a
  // per-group answer read off the wire rather than a floor lifted for everyone.
  it('lets a coldable group be commanded to level 0 and still refuses its neighbours', () => {
    const sendAction = vi.fn();
    const view = {
      ...power(),
      consoles: [
        { id: 'helm', level: 2, commanded_level: 2, min_level: 1, max_level: 4 },
        { id: 'weapons', level: 1, commanded_level: 1, min_level: 0, max_level: 4 },
      ],
    };
    const actions = registry({ getState: () => view, sendAction });

    expect(actions.activate(POWER_DECREASE_ACTION_ID, {
      context: POWER_ACTION_CONTEXT, detail: { target: 'weapons', level: 0 },
    })).toMatchObject({ claimed: true, handled: true });
    expect(sendAction).toHaveBeenCalledWith('set_power', expect.objectContaining({
      target: 'weapons', level: 0, control_system_id: 'power-reactor',
    }));

    sendAction.mockClear();
    expect(actions.activate(POWER_DECREASE_ACTION_ID, {
      context: POWER_ACTION_CONTEXT, detail: { target: 'helm', level: 0 },
    })).toMatchObject({ claimed: true, handled: false });
    expect(sendAction).not.toHaveBeenCalled();
  });

  // And the un-parameterised stepper, which is what a gamepad axis sends: one
  // press off a cold-capable group at 1 takes it to 0 rather than stopping.
  it('steps a coldable group down to 0 with no explicit level', () => {
    const sendAction = vi.fn();
    const view = {
      ...power(),
      consoles: [{ id: 'weapons', level: 1, commanded_level: 1, min_level: 0, max_level: 4 }],
    };
    const actions = registry({ getState: () => view, sendAction });

    expect(actions.activate(POWER_DECREASE_ACTION_ID, { context: POWER_ACTION_CONTEXT }))
      .toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenCalledWith('set_power', expect.objectContaining({
      target: 'weapons', level: 0,
    }));

    // …and a second press off 0 has nowhere to go.
    sendAction.mockClear();
    view.consoles[0] = { id: 'weapons', level: 0, commanded_level: 0, min_level: 0, max_level: 4 };
    expect(actions.activate(POWER_DECREASE_ACTION_ID, { context: POWER_ACTION_CONTEXT }))
      .toMatchObject({ handled: false });
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('routes exact repair slot, target, and named system without changing the projection', () => {
    const sendAction = vi.fn();
    const view = repair();
    const actions = registry({ getState: () => view, sendAction });
    expect(actions.activate(REPAIR_DISPATCH_ACTION_ID, {
      context: REPAIR_ACTION_CONTEXT,
      detail: { team_idx: 0, target: 'helm' },
    })).toMatchObject({ handled: true });
    expect(actions.activate(REPAIR_PRIORITY_ACTION_ID, {
      context: REPAIR_ACTION_CONTEXT,
      detail: { system_id: 'power-reactor' },
    })).toMatchObject({ handled: true });
    expect(sendAction.mock.calls).toEqual([
      ['dispatch_repair_team', expect.objectContaining({
        team_idx: 0, target: 'helm', control_system_id: 'repair',
      })],
      ['set_repair_target_priority', expect.objectContaining({
        system_id: 'power-reactor', control_system_id: 'repair',
      })],
    ]);
    expect(view.teams[0].status).toBe('idle');
    expect(view.damaged_systems[1]).not.toHaveProperty('prioritised');
  });

  it('gives Courier Captain the same parameter-free internal Repair path', () => {
    const sendAction = vi.fn();
    const state = keyed({ repair: repair() }, { repair: 'repair' });
    const actions = registry({ getState: () => state, sendAction });
    expect(actions.activate(REPAIR_DISPATCH_ACTION_ID, {
      context: CAPTAIN_ENGINEERING_ACTION_CONTEXT, source: 'keyboard',
    })).toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenCalledWith('dispatch_repair_team', expect.objectContaining({
      team_idx: 0, target: 'core', control_system_id: 'repair',
    }));
  });

  it('parameter-free priority skips worse visible damage outside the on-site sweep group', () => {
    const sendAction = vi.fn();
    const view = {
      ...repair(),
      damaged_systems: [
        {
          system_id: 'core', tier: 'Destroyed', damage_pct: 1,
          in_progress: false, prioritisable: false,
        },
        {
          system_id: 'power-battery', tier: 'Damaged', damage_pct: 0.2,
          in_progress: false, prioritisable: true,
        },
      ],
    };
    const actions = registry({ getState: () => view, sendAction });

    expect(actions.activate(REPAIR_PRIORITY_ACTION_ID, {
      context: REPAIR_ACTION_CONTEXT, source: 'gamepad',
    })).toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenCalledWith(
      'set_repair_target_priority',
      expect.objectContaining({ system_id: 'power-battery' }),
    );

    expect(actions.activate(REPAIR_PRIORITY_ACTION_ID, {
      context: REPAIR_ACTION_CONTEXT,
      detail: { system_id: 'core' },
    })).toMatchObject({ handled: false });
    expect(sendAction).toHaveBeenCalledTimes(1);
  });

  it('keeps the externally committed idle slot out of pointer and parameter-free dispatch', () => {
    const sendAction = vi.fn();
    const view = {
      ...repair(),
      teams: [
        { id: 0, status: 'repairing' },
        { id: 1, status: 'idle' },
        { id: 2, status: 'idle' },
      ],
      external_dispatch: { target: 'ally-1' },
    };
    const actions = registry({ getState: () => view, sendAction });

    expect(actions.activate(REPAIR_DISPATCH_ACTION_ID, {
      context: REPAIR_ACTION_CONTEXT, source: 'gamepad',
    })).toMatchObject({ handled: true });
    expect(sendAction).toHaveBeenLastCalledWith('dispatch_repair_team', expect.objectContaining({
      team_idx: 1, target: 'core',
    }));

    expect(actions.activate(REPAIR_DISPATCH_ACTION_ID, {
      context: REPAIR_ACTION_CONTEXT,
      detail: { team_idx: 2, target: 'core' },
    })).toMatchObject({ handled: false });
    expect(sendAction).toHaveBeenCalledTimes(1);
  });

  it('derives all three auxiliary toggles from authoritative family state', () => {
    const sendAction = vi.fn();
    let state = keyed({
      tractor: { engaged: false },
      umbilical: { running: true },
      repair: { ...repair(), external_dispatch: { target: 'ally-1' } },
    }, { tractor: 'tractor', umbilical: 'umbilical', repair: 'repair' });
    const actions = registry({ getState: () => state, sendAction });
    expect(actions.activate(TRACTOR_TOGGLE_ACTION_ID, { context: ENGINEERING_ACTION_CONTEXT })).toMatchObject({ handled: true });
    expect(actions.activate(UMBILICAL_TOGGLE_ACTION_ID, { context: ENGINEERING_ACTION_CONTEXT })).toMatchObject({ handled: true });
    expect(actions.activate(EXTERNAL_REPAIR_TOGGLE_ACTION_ID, { context: ENGINEERING_ACTION_CONTEXT })).toMatchObject({ handled: true });
    expect(sendAction.mock.calls.map(([name]) => name)).toEqual([
      'engage_tractor', 'stop_transfer', 'recall_external_repair',
    ]);
    expect(sendAction.mock.calls.map(([, payload]) => payload.control_system_id))
      .toEqual(['tractor', 'umbilical', 'repair']);

    state = keyed({
      tractor: { engaged: true },
      umbilical: { running: false },
      repair: { ...repair(), external_dispatch: { target: null } },
    }, { tractor: 'tractor', umbilical: 'umbilical', repair: 'repair' });
    actions.activate(TRACTOR_TOGGLE_ACTION_ID, { context: ENGINEERING_ACTION_CONTEXT });
    actions.activate(UMBILICAL_TOGGLE_ACTION_ID, { context: ENGINEERING_ACTION_CONTEXT });
    actions.activate(EXTERNAL_REPAIR_TOGGLE_ACTION_ID, { context: ENGINEERING_ACTION_CONTEXT });
    expect(sendAction.mock.calls.slice(3).map(([name]) => name)).toEqual([
      'release_tractor', 'start_transfer', 'dispatch_external_repair',
    ]);
    expect(sendAction.mock.calls.slice(3).map(([, payload]) => payload.control_system_id))
      .toEqual(['tractor', 'umbilical', 'repair']);
  });

  it('enters Pending only after a real authoritative request is emitted', () => {
    const transitions = [];
    const sendAction = vi.fn();
    const actions = registry({ getState: power, sendAction, onTransition: (value) => transitions.push(value) });
    const result = actions.activate(POWER_INCREASE_ACTION_ID, { context: POWER_ACTION_CONTEXT });
    expect(result).toMatchObject({ claimed: true, handled: true, correlation: expect.any(String) });
    expect(transitions.map((value) => value.state)).toEqual(['Pressed', 'Pending']);
    expect(sendAction).toHaveBeenCalledWith('set_power', expect.objectContaining({
      correlation: result.correlation,
    }));
  });
});
