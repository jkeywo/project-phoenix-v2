import { describe, it, expect, vi } from 'vitest';
import {
  CAPTAIN_ACTIONS,
  CAPTAIN_ACTION_CONTEXT,
  CAPTAIN_OBJECTIVE_PRIORITY_ACTION,
  CAPTAIN_OBJECTIVE_PRIORITY_ACTION_ID,
  CAPTAIN_RED_ALERT_ACTION,
  CAPTAIN_RED_ALERT_ACTION_ID,
  CAPTAIN_VIEW_ACTION,
  CAPTAIN_VIEW_ACTION_ID,
  CAPTAIN_WEAPONS_HOLD_ACTION,
  CAPTAIN_WEAPONS_HOLD_ACTION_ID,
  createCaptainActionRegistry,
} from '../../gui/stations/captain-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';

let correlationSequence = 0;

function captainRegistry(options) {
  return createCaptainActionRegistry({
    ...options,
    actionFeedback: new ActionFeedbackLifecycle({
      correlation: () => `captain-test-${++correlationSequence}`,
      now: () => 123,
    }),
  });
}

function key(code, overrides = {}) {
  return {
    type: 'keydown', code, cancelable: true, preventDefault: vi.fn(), ...overrides,
  };
}

describe('real Captain Red Alert semantic adapter', () => {
  it('registers every shipped Captain command family with stable two-slot metadata', () => {
    expect(CAPTAIN_ACTIONS).toEqual([
      CAPTAIN_RED_ALERT_ACTION,
      CAPTAIN_WEAPONS_HOLD_ACTION,
      CAPTAIN_VIEW_ACTION,
      CAPTAIN_OBJECTIVE_PRIORITY_ACTION,
    ]);
    for (const action of CAPTAIN_ACTIONS) {
      expect(action.contexts).toEqual([CAPTAIN_ACTION_CONTEXT]);
      expect(action.bindings).toHaveLength(2);
      expect(action.authoritativeFeedback).toBe(true);
    }
  });

  it('declares the stable identity, Captain context and two slots', () => {
    expect(CAPTAIN_RED_ALERT_ACTION).toMatchObject({
      id: 'captain.red-alert',
      contexts: ['captain'],
      labelId: expect.any(String),
      accessibilityLabelId: expect.any(String),
    });
    expect(CAPTAIN_RED_ALERT_ACTION.bindings).toHaveLength(2);
    expect(CAPTAIN_RED_ALERT_ACTION.bindings[0]).toMatchObject({ code: 'KeyR' });
    expect(CAPTAIN_RED_ALERT_ACTION.bindings[1]).toEqual({
      type: 'gamepad', input: 'button', control: 'face-bottom',
    });
  });

  it('declares Weapons Hold as a second real Captain action with two slots', () => {
    expect(CAPTAIN_WEAPONS_HOLD_ACTION).toMatchObject({
      id: 'captain.weapons-hold',
      contexts: ['captain'],
      labelId: expect.any(String),
      accessibilityLabelId: expect.any(String),
    });
    expect(CAPTAIN_WEAPONS_HOLD_ACTION.bindings).toHaveLength(2);
    expect(CAPTAIN_WEAPONS_HOLD_ACTION.bindings[0]).toMatchObject({ code: 'KeyH' });
    expect(CAPTAIN_WEAPONS_HOLD_ACTION.bindings[1]).toBeNull();
  });

  it('emits the existing explicit set_red_alert envelope from the default binding', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({ red_alert: false, red_alert_auto: false }),
      sendAction,
    });
    const result = registry.dispatchKeyboardEvent(key('KeyR'), CAPTAIN_ACTION_CONTEXT);
    expect(result).toMatchObject({
      claimed: true, actionId: CAPTAIN_RED_ALERT_ACTION_ID, handled: true,
    });
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', expect.objectContaining({
      active: true,
      correlation: expect.any(String),
      semantic_action: CAPTAIN_RED_ALERT_ACTION_ID,
      __input_ms: 123,
    }));
  });

  it('derives the opposite explicit state from a keyed Captain family view', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({
        systems: { command: { red_alert: true, red_alert_auto: false } },
        system_ids: ['command'],
        system_families: { command: 'captain' },
      }),
      sendAction,
    });
    registry.activate(CAPTAIN_RED_ALERT_ACTION_ID, {
      context: CAPTAIN_ACTION_CONTEXT,
      source: 'control',
    });
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', expect.objectContaining({
      active: false, correlation: expect.any(String),
    }));
  });

  it('routes a gamepad activation through the same Red Alert adapter', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({ red_alert: false, red_alert_auto: false }),
      sendAction,
    });
    expect(registry.activate(CAPTAIN_RED_ALERT_ACTION_ID, {
      context: CAPTAIN_ACTION_CONTEXT,
      source: 'gamepad',
    })).toMatchObject({
      claimed: true, actionId: CAPTAIN_RED_ALERT_ACTION_ID, handled: true,
    });
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', expect.objectContaining({
      active: true, correlation: expect.any(String),
    }));
  });

  it('dispatches a remapped binding through the same adapter and identity', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({ red_alert: false, red_alert_auto: false }),
      sendAction,
    });
    registry.setBinding(CAPTAIN_RED_ALERT_ACTION_ID, 0, {
      code: 'KeyA', shiftKey: true,
    });
    const result = registry.dispatchKeyboardEvent(
      key('KeyA', { shiftKey: true }), CAPTAIN_ACTION_CONTEXT,
    );
    expect(result.actionId).toBe(CAPTAIN_RED_ALERT_ACTION_ID);
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', expect.objectContaining({
      active: true, correlation: expect.any(String),
    }));
  });

  it('routes default and remapped Weapons Hold through the existing explicit envelope', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({ weapons_hold: false, red_alert_auto: false }),
      sendAction,
    });
    expect(registry.dispatchKeyboardEvent(key('KeyH'), CAPTAIN_ACTION_CONTEXT))
      .toMatchObject({ claimed: true, actionId: CAPTAIN_WEAPONS_HOLD_ACTION_ID, handled: true });
    expect(sendAction).toHaveBeenLastCalledWith('set_weapons_hold', expect.objectContaining({
      held: true,
      correlation: expect.any(String),
      semantic_action: CAPTAIN_WEAPONS_HOLD_ACTION_ID,
    }));

    registry.setBinding(CAPTAIN_WEAPONS_HOLD_ACTION_ID, 0, { code: 'KeyJ' });
    registry.dispatchKeyboardEvent(key('KeyJ'), CAPTAIN_ACTION_CONTEXT);
    expect(sendAction).toHaveBeenCalledTimes(2);
    expect(sendAction).toHaveBeenLastCalledWith('set_weapons_hold', expect.objectContaining({
      held: true, correlation: expect.any(String),
    }));
  });

  it('does not emit while authoritative state says the system is AI-run', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({ red_alert: false, red_alert_auto: true }),
      sendAction,
    });
    expect(registry.activate(CAPTAIN_RED_ALERT_ACTION_ID, {
      context: CAPTAIN_ACTION_CONTEXT,
    }).handled).toBe(false);
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('uses one parameterised Viewscreen identity for visible choices and the remapped cycle', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({
        camera_views: ['camera_fore', 'camera_aft', 'cinematic'],
        view_direction: 'camera_fore',
        viewscreen_auto: false,
      }),
      sendAction,
    });

    registry.activate(CAPTAIN_VIEW_ACTION_ID, {
      context: CAPTAIN_ACTION_CONTEXT,
      detail: { direction: 'cinematic' },
    });
    expect(sendAction).toHaveBeenLastCalledWith('set_view', expect.objectContaining({
      direction: 'cinematic',
      correlation: expect.any(String),
      semantic_action: CAPTAIN_VIEW_ACTION_ID,
    }));

    registry.dispatchKeyboardEvent(key('KeyV'), CAPTAIN_ACTION_CONTEXT);
    expect(sendAction).toHaveBeenLastCalledWith('set_view', expect.objectContaining({
      direction: 'camera_aft',
    }));
  });

  it('refuses unavailable Viewscreen detail instead of inventing a ship variant', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({
        camera_views: ['camera_fore', 'cinematic'],
        view_direction: 'camera_fore',
        viewscreen_auto: false,
      }),
      sendAction,
    });
    expect(registry.activate(CAPTAIN_VIEW_ACTION_ID, {
      context: CAPTAIN_ACTION_CONTEXT,
      detail: { direction: 'camera_aft' },
    }).handled).toBe(false);
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('uses objective detail from a visible row and cycles from authoritative priority', () => {
    const sendAction = vi.fn();
    const registry = captainRegistry({
      getState: () => ({
        objectives: [{ id: 'one' }, { id: 'two' }],
        boosted_objective_id: 'one',
      }),
      sendAction,
    });
    registry.activate(CAPTAIN_OBJECTIVE_PRIORITY_ACTION_ID, {
      context: CAPTAIN_ACTION_CONTEXT,
      detail: { id: 'one' },
    });
    expect(sendAction).toHaveBeenLastCalledWith(
      'set_objective_priority',
      expect.objectContaining({
        id: 'one',
        correlation: expect.any(String),
        semantic_action: CAPTAIN_OBJECTIVE_PRIORITY_ACTION_ID,
      }),
    );
    registry.dispatchKeyboardEvent(key('KeyO'), CAPTAIN_ACTION_CONTEXT);
    expect(sendAction).toHaveBeenLastCalledWith(
      'set_objective_priority',
      expect.objectContaining({ id: 'two' }),
    );
  });
});
