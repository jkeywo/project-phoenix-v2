import { describe, it, expect, vi } from 'vitest';
import {
  CAPTAIN_ACTION_CONTEXT,
  CAPTAIN_RED_ALERT_ACTION,
  CAPTAIN_RED_ALERT_ACTION_ID,
  createCaptainActionRegistry,
} from '../../gui/stations/captain-actions.js';

function key(code, overrides = {}) {
  return {
    type: 'keydown', code, cancelable: true, preventDefault: vi.fn(), ...overrides,
  };
}

describe('real Captain Red Alert semantic adapter', () => {
  it('declares the stable identity, Captain context and two slots', () => {
    expect(CAPTAIN_RED_ALERT_ACTION).toMatchObject({
      id: 'captain.red-alert',
      contexts: ['captain'],
      labelId: expect.any(String),
      accessibilityLabelId: expect.any(String),
    });
    expect(CAPTAIN_RED_ALERT_ACTION.bindings).toHaveLength(2);
    expect(CAPTAIN_RED_ALERT_ACTION.bindings[0]).toMatchObject({ code: 'KeyR' });
    expect(CAPTAIN_RED_ALERT_ACTION.bindings[1]).toBeNull();
  });

  it('emits the existing explicit set_red_alert envelope from the default binding', () => {
    const sendAction = vi.fn();
    const registry = createCaptainActionRegistry({
      getState: () => ({ red_alert: false, red_alert_auto: false }),
      sendAction,
    });
    const result = registry.dispatchKeyboardEvent(key('KeyR'), CAPTAIN_ACTION_CONTEXT);
    expect(result).toEqual({
      claimed: true, actionId: CAPTAIN_RED_ALERT_ACTION_ID, handled: true,
    });
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', { active: true });
  });

  it('derives the opposite explicit state from a keyed Captain family view', () => {
    const sendAction = vi.fn();
    const registry = createCaptainActionRegistry({
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
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', { active: false });
  });

  it('dispatches a remapped binding through the same adapter and identity', () => {
    const sendAction = vi.fn();
    const registry = createCaptainActionRegistry({
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
    expect(sendAction).toHaveBeenCalledWith('set_red_alert', { active: true });
  });

  it('does not emit while authoritative state says the system is AI-run', () => {
    const sendAction = vi.fn();
    const registry = createCaptainActionRegistry({
      getState: () => ({ red_alert: false, red_alert_auto: true }),
      sendAction,
    });
    expect(registry.activate(CAPTAIN_RED_ALERT_ACTION_ID, {
      context: CAPTAIN_ACTION_CONTEXT,
    }).handled).toBe(false);
    expect(sendAction).not.toHaveBeenCalled();
  });
});
