import { describe, expect, it, vi } from 'vitest';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import {
  createTacticalActionRegistry,
  TACTICAL_ACTIONS,
  TACTICAL_BLASTER_CANCEL_ACTION_ID,
  TACTICAL_BLASTER_CHARGE_ACTION_ID,
  TACTICAL_ACTION_CONTEXT,
  TACTICAL_PHASER_FIRE_ACTION_ID,
  TACTICAL_TARGET_ACTION_ID,
  TACTICAL_TORPEDO_FIRE_ACTION_ID,
  TACTICAL_TORPEDO_VOLLEY_UP_ACTION_ID,
} from '../../gui/stations/tactical-actions.js';

let sequence = 0;
function registry(options) {
  return createTacticalActionRegistry({
    ...options,
    actionFeedback: new ActionFeedbackLifecycle({
      now: () => 123,
      correlation: () => `tactical-${++sequence}`,
      onTransition: options.onTransition || (() => {}),
    }),
  });
}

const view = () => ({
  target_uuid: 'target-a',
  blips: [{ uuid: 'target-a' }, { uuid: 'target-b' }, { uuid: 'waypoint', kind: 'waypoint' }],
  phaser_mode: 'Auto',
  banks: [{ id: 'fore', fire_ready: true }],
  blasters: [{ id: 'port', fire_ready: true }],
  tubes: [{ id: 'fore_port', target_count: 1, volley_max: 3 }],
});

describe('Tactical semantic actions', () => {
  it('publishes every visible Tactical intent with a stable context and two device slots', () => {
    expect(TACTICAL_ACTIONS).toHaveLength(9);
    for (const action of TACTICAL_ACTIONS) {
      expect(action.contexts).toEqual([TACTICAL_ACTION_CONTEXT]);
      expect(action.bindings).toHaveLength(2);
      expect(action.authoritativeFeedback).toBe(true);
    }
  });

  it('uses the keyboard target binding to select from authoritative contacts without a local mutation path', () => {
    const sendAction = vi.fn();
    const actions = registry({ getState: view, sendAction });
    const result = actions.dispatchKeyboardEvent({
      type: 'keydown', code: 'KeyT', cancelable: true, preventDefault: vi.fn(),
    }, TACTICAL_ACTION_CONTEXT);
    expect(result).toMatchObject({ claimed: true, handled: true, actionId: TACTICAL_TARGET_ACTION_ID });
    expect(sendAction).toHaveBeenCalledWith('set_target', expect.objectContaining({
      uuid: 'target-b', semantic_action: TACTICAL_TARGET_ACTION_ID, correlation: expect.any(String),
    }));
  });

  it('uses a gamepad activation and exact visible bank through the same phaser adapter', () => {
    const sendAction = vi.fn();
    const actions = registry({ getState: view, sendAction });
    expect(actions.activate(TACTICAL_PHASER_FIRE_ACTION_ID, {
      context: TACTICAL_ACTION_CONTEXT, source: 'gamepad', detail: { bank: 'fore' },
    })).toMatchObject({ claimed: true, handled: true });
    expect(sendAction).toHaveBeenCalledWith('fire_phaser', expect.objectContaining({
      bank: 'fore', semantic_action: TACTICAL_PHASER_FIRE_ACTION_ID,
    }));
  });

  it('does not let an unqualified keyboard fire shortcut bypass phaser Auto mode', () => {
    const sendAction = vi.fn();
    const actions = registry({ getState: view, sendAction });
    expect(actions.dispatchKeyboardEvent({
      type: 'keydown', code: 'KeyF', cancelable: true, preventDefault: vi.fn(),
    }, TACTICAL_ACTION_CONTEXT)).toMatchObject({ claimed: true, handled: false });
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('chooses visible eligible mounts for unqualified gamepad actions', () => {
    const sendAction = vi.fn();
    const actions = registry({
      getState: () => ({
        target_uuid: 'target-a',
        phaser_mode: 'Manual',
        banks: [
          { id: 'fore', fire_ready: true, on_cooldown: true },
          { id: 'aft', fire_ready: true, on_cooldown: false },
        ],
        blasters: [
          { id: 'port', fire_ready: true, charge_progress: 0 },
          { id: 'starboard', fire_ready: false, charge_progress: 0.5 },
        ],
        tubes: [
          { id: 'fore_port', loaded: false, loaded_count: 0, target_count: 1, volley_max: 3 },
          { id: 'aft', loaded: true, loaded_count: 1, target_count: 1, volley_max: 3 },
        ],
      }),
      sendAction,
    });

    expect(actions.activate(TACTICAL_PHASER_FIRE_ACTION_ID, {
      context: TACTICAL_ACTION_CONTEXT, source: 'gamepad',
    })).toMatchObject({ handled: true });
    expect(actions.activate(TACTICAL_BLASTER_CHARGE_ACTION_ID, {
      context: TACTICAL_ACTION_CONTEXT, source: 'gamepad',
    })).toMatchObject({ handled: true });
    expect(actions.activate(TACTICAL_BLASTER_CANCEL_ACTION_ID, {
      context: TACTICAL_ACTION_CONTEXT, source: 'gamepad',
    })).toMatchObject({ handled: true });
    expect(actions.activate(TACTICAL_TORPEDO_FIRE_ACTION_ID, {
      context: TACTICAL_ACTION_CONTEXT, source: 'gamepad',
    })).toMatchObject({ handled: true });

    expect(sendAction.mock.calls.map(([name, payload]) => [name, payload.bank || payload.tube]))
      .toEqual([
        ['fire_phaser', 'aft'],
        ['charge_blaster_start', 'port'],
        ['charge_blaster_cancel', 'starboard'],
        ['fire_torpedo', 'aft'],
      ]);
  });

  it('keeps volley arithmetic in the adapter and returns authoritative accepted/refused feedback only by correlation', () => {
    const sendAction = vi.fn();
    const transitions = [];
    const actions = registry({ getState: view, sendAction, onTransition: (value) => transitions.push(value) });
    const result = actions.activate(TACTICAL_TORPEDO_VOLLEY_UP_ACTION_ID, {
      context: TACTICAL_ACTION_CONTEXT, source: 'control', detail: { tube: 'fore_port' },
    });
    expect(sendAction).toHaveBeenCalledWith('set_torpedo_volley_target', expect.objectContaining({
      tube: 'fore_port', count: 2, correlation: result.correlation,
    }));
    expect(transitions.map((entry) => entry.state)).toEqual(['Pressed', 'Pending']);
    expect(actions.activate(TACTICAL_TORPEDO_VOLLEY_UP_ACTION_ID, {
      context: TACTICAL_ACTION_CONTEXT, detail: { tube: 'missing' },
    })).toMatchObject({ claimed: true, handled: false });
    // The document feedback lifecycle refuses only when the authoritative
    // response reaches its exact correlation; a state render cannot settle it.
    expect(transitions.some((entry) => entry.correlation === result.correlation
      && entry.state === 'Refused')).toBe(false);
  });
});
