import { describe, it, expect, vi } from 'vitest';
import {
  createGamepadInputRuntime,
  enumerateGamepads,
  gamepadBindingPressed,
  normalizeGamepadBinding,
} from '../../gui/gamepad-input.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';

const button = (pressed = false) => ({ pressed, value: pressed ? 1 : 0 });

function pad(index, { mapping = 'standard', pressed = [], axes = [0, 0, 0, 0] } = {}) {
  const buttons = Array.from({ length: 17 }, () => button(false));
  for (const index of pressed) buttons[index] = button(true);
  return { index, mapping, buttons, axes, id: `hardware-name-${index}` };
}

const GAMEPAD_ACTION = {
  id: 'captain.red-alert',
  contexts: ['captain'],
  labelId: 'semantic_action.captain.red_alert.label',
  accessibilityLabelId: 'semantic_action.captain.red_alert.accessibility',
  bindings: [
    { type: 'keyboard', code: 'KeyR' },
    { type: 'gamepad', input: 'button', control: 'face-bottom' },
  ],
};

describe('standard gamepad bindings', () => {
  it('normalizes portable button, D-pad and axis-direction identities', () => {
    expect(normalizeGamepadBinding({ type: 'gamepad', control: 'face-bottom' })).toEqual({
      type: 'gamepad', input: 'button', control: 'face-bottom',
    });
    expect(normalizeGamepadBinding({ type: 'gamepad', control: 'dpad-up' })).toEqual({
      type: 'gamepad', input: 'dpad', control: 'dpad-up',
    });
    expect(normalizeGamepadBinding({
      type: 'gamepad', control: 'left-stick-x', direction: 'positive', threshold: 0.7,
    })).toEqual({
      type: 'gamepad', input: 'axis', control: 'left-stick-x',
      direction: 'positive', threshold: 0.7,
    });
    expect(() => normalizeGamepadBinding({
      type: 'gamepad', control: 'left-stick-x', direction: 'positive', threshold: 1.1,
    })).toThrow(/threshold/i);
  });

  it('samples only standard-mapped snapshots at their standard indices', () => {
    const dpad = { type: 'gamepad', input: 'dpad', control: 'dpad-left' };
    const axis = {
      type: 'gamepad', input: 'axis', control: 'left-stick-y',
      direction: 'negative', threshold: 0.6,
    };
    expect(gamepadBindingPressed(dpad, pad(0, { pressed: [14] }))).toBe(true);
    expect(gamepadBindingPressed(axis, pad(0, { axes: [0, -0.7, 0, 0] }))).toBe(true);
    expect(gamepadBindingPressed(dpad, pad(0, { mapping: '', pressed: [14] }))).toBe(false);
    expect(enumerateGamepads([
      pad(0), pad(1, { mapping: '' }), null,
    ])).toEqual([
      { index: 0, supported: true }, { index: 1, supported: false },
    ]);
  });
});

describe('explicit connection ownership and discrete edges', () => {
  it('does nothing before selection, samples only the selected pad and fires rising edges', () => {
    let snapshot = [pad(0), pad(1)];
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot,
      getContext: () => 'captain',
      getActions: () => [GAMEPAD_ACTION],
      activate,
    });
    snapshot = [pad(0, { pressed: [0] }), pad(1)];
    runtime.poll(snapshot);
    expect(activate).not.toHaveBeenCalled();

    snapshot = [pad(0), pad(1)];
    runtime.select(0);
    runtime.poll(snapshot); // neutral gate opens
    snapshot = [pad(0), pad(1, { pressed: [0] })];
    runtime.poll(snapshot);
    expect(activate).not.toHaveBeenCalled();
    snapshot = [pad(0, { pressed: [0] }), pad(1)];
    runtime.poll(snapshot);
    runtime.poll(snapshot);
    expect(activate).toHaveBeenCalledOnce();
    expect(activate).toHaveBeenCalledWith('captain.red-alert', expect.objectContaining({
      context: 'captain', source: 'gamepad',
    }));
  });

  it('supports D-pad and configured axis edges', () => {
    let snapshot = [pad(0)];
    const action = { ...GAMEPAD_ACTION, bindings: [
      { type: 'gamepad', input: 'dpad', control: 'dpad-up' },
      {
        type: 'gamepad', input: 'axis', control: 'left-stick-x',
        direction: 'positive', threshold: 0.75,
      },
    ] };
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => 'captain',
      getActions: () => [action], activate,
    });
    runtime.select(0);
    runtime.poll(snapshot);
    snapshot = [pad(0, { pressed: [12] })];
    runtime.poll(snapshot);
    snapshot = [pad(0)];
    runtime.poll(snapshot);
    snapshot = [pad(0, { axes: [0.8, 0, 0, 0] })];
    runtime.poll(snapshot);
    expect(activate).toHaveBeenCalledTimes(2);
  });

  it('disconnects without transfer and index reuse requires explicit neutral reselection', () => {
    let snapshot = [pad(0), pad(1)];
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => 'captain',
      getActions: () => [GAMEPAD_ACTION], activate,
    });
    runtime.select(0);
    runtime.poll(snapshot);
    snapshot = [null, pad(1, { pressed: [0] })];
    expect(runtime.poll(snapshot).status).toBe('disconnected');
    expect(activate).not.toHaveBeenCalled();

    // A new connection reuses index zero. The old generation owns nothing.
    snapshot = [pad(0, { pressed: [0] }), pad(1, { pressed: [0] })];
    expect(runtime.poll(snapshot).status).toBe('disconnected');
    runtime.select(0);
    runtime.poll(snapshot);
    expect(runtime.state().status).toBe('neutral');
    expect(activate).not.toHaveBeenCalled();
    snapshot = [pad(0), pad(1, { pressed: [0] })];
    runtime.poll(snapshot);
    snapshot = [pad(0, { pressed: [0] }), pad(1, { pressed: [0] })];
    runtime.poll(snapshot);
    expect(activate).toHaveBeenCalledOnce();
  });

  it('neutral-gates context changes and captures without gameplay dispatch', () => {
    let context = 'captain';
    let snapshot = [pad(0)];
    const activate = vi.fn();
    const captures = [];
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => context,
      getActions: () => [GAMEPAD_ACTION], activate,
      onCapture: (target, binding) => captures.push({ target, binding }),
    });
    runtime.select(0);
    runtime.poll(snapshot);
    runtime.beginCapture('captain.red-alert', 1);
    runtime.poll(snapshot);
    snapshot = [pad(0, { pressed: [13] })];
    runtime.poll(snapshot);
    expect(captures).toEqual([expect.objectContaining({
      target: { actionId: 'captain.red-alert', slot: 1 },
      binding: { type: 'gamepad', input: 'dpad', control: 'dpad-down' },
    })]);
    expect(activate).not.toHaveBeenCalled();

    // Capture completion is a new neutral gate; holding the same control is inert.
    runtime.poll(snapshot);
    context = 'helm';
    runtime.poll(snapshot);
    context = 'captain';
    runtime.poll(snapshot);
    expect(activate).not.toHaveBeenCalled();
  });

  it('leaves the independent keyboard matcher active while the pad is disconnected', () => {
    let snapshot = [pad(0)];
    const keyboardAdapter = vi.fn(() => true);
    const registry = createSemanticActionRegistry();
    registry.register(GAMEPAD_ACTION, keyboardAdapter);
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => 'captain',
      getActions: () => registry.list('captain'),
    });
    runtime.select(0);
    snapshot = [];
    expect(runtime.poll(snapshot).status).toBe('disconnected');
    expect(registry.dispatchKeyboardEvent({
      type: 'keydown', code: 'KeyR', preventDefault() {},
    }, 'captain')).toMatchObject({ claimed: true, handled: true });
    expect(keyboardAdapter).toHaveBeenCalledOnce();
  });
});
