import { describe, it, expect, vi } from 'vitest';
import {
  createGamepadInputRuntime,
  enumerateGamepads,
  gamepadBindingsEqual,
  gamepadBindingPressed,
  normalizeContinuousAxis,
  normalizeGamepadBinding,
} from '../../gui/gamepad-input.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';

const button = (pressed = false) => ({ pressed, value: pressed ? 1 : 0 });

function pad(index, { mapping = 'standard', pressed = [], axes = [0, 0, 0, 0] } = {}) {
  const buttons = Array.from({ length: 17 }, () => button(false));
  for (const index of pressed) buttons[index] = button(true);
  return { index, mapping, buttons, axes, id: `hardware-name-${index}` };
}

function eventTarget(properties = {}) {
  const listeners = new Map();
  return Object.assign(properties, {
    addEventListener(type, listener) {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type).add(listener);
    },
    removeEventListener(type, listener) {
      listeners.get(type)?.delete(listener);
    },
    emit(type, event = {}) {
      for (const listener of [...(listeners.get(type) || [])]) listener(event);
    },
    listenerCount(type) {
      return listeners.get(type)?.size || 0;
    },
  });
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

const CONTINUOUS_ACTION = {
  id: 'helm.steering',
  contexts: ['helm'],
  labelId: 'semantic_action.helm.steering.label',
  accessibilityLabelId: 'semantic_action.helm.steering.accessibility',
  continuous: { min: -1, max: 1, neutral: 0, cadenceMs: 100 },
  tuning: { deadzone: 0.1, inverted: false },
  bindings: [
    { type: 'gamepad', input: 'axis', control: 'left-stick-x' },
    null,
  ],
};

describe('standard gamepad bindings', () => {
  it('normalizes portable button, D-pad and axis-direction identities', () => {
    for (const control of [
      'face-bottom', 'face-right', 'face-left', 'face-top',
      'left-shoulder', 'right-shoulder', 'left-trigger', 'right-trigger',
      'select', 'start', 'left-stick-button', 'right-stick-button',
    ]) {
      expect(normalizeGamepadBinding({ type: 'gamepad', control })).toEqual({
        type: 'gamepad', input: 'button', control,
      });
    }
    expect(normalizeGamepadBinding({ type: 'gamepad', control: 'dpad-up' })).toEqual({
      type: 'gamepad', input: 'dpad', control: 'dpad-up',
    });
    expect(normalizeGamepadBinding({
      type: 'gamepad', control: 'left-stick-x', direction: 'positive', threshold: 0.7,
    })).toEqual({
      type: 'gamepad', input: 'axis', control: 'left-stick-x',
      direction: 'positive', threshold: 0.7,
    });
    expect(normalizeGamepadBinding({
      type: 'gamepad', control: 'right-stick-x', direction: 'negative', threshold: 0.65,
    })).toEqual({
      type: 'gamepad', input: 'axis', control: 'right-stick-x',
      direction: 'negative', threshold: 0.65,
    });
    expect(() => normalizeGamepadBinding({
      type: 'gamepad', control: 'left-stick-x', direction: 'positive', threshold: 1.1,
    })).toThrow(/threshold/i);
  });

  it('treats threshold variants on the same directed axis as one input', () => {
    const lowerThreshold = {
      type: 'gamepad', input: 'axis', control: 'left-stick-x',
      direction: 'positive', threshold: 0.5,
    };
    expect(gamepadBindingsEqual(lowerThreshold, {
      ...lowerThreshold, threshold: 0.9,
    })).toBe(true);
    expect(gamepadBindingsEqual(lowerThreshold, {
      ...lowerThreshold, direction: 'negative', threshold: 0.5,
    })).toBe(false);
  });

  it('samples only standard-mapped snapshots at their standard indices', () => {
    const dpad = { type: 'gamepad', input: 'dpad', control: 'dpad-left' };
    const axis = {
      type: 'gamepad', input: 'axis', control: 'left-stick-y',
      direction: 'negative', threshold: 0.6,
    };
    expect(gamepadBindingPressed(dpad, pad(0, { pressed: [14] }))).toBe(true);
    expect(gamepadBindingPressed(
      { type: 'gamepad', input: 'button', control: 'select' },
      pad(0, { pressed: [8] }),
    )).toBe(true);
    expect(gamepadBindingPressed(
      { type: 'gamepad', input: 'button', control: 'right-stick-button' },
      pad(0, { pressed: [11] }),
    )).toBe(true);
    expect(gamepadBindingPressed(axis, pad(0, { axes: [0, -0.7, 0, 0] }))).toBe(true);
    expect(gamepadBindingPressed({
      type: 'gamepad', input: 'axis', control: 'right-stick-x',
      direction: 'positive', threshold: 0.6,
    }, pad(0, { axes: [0, 0, 0.7, 0] }))).toBe(true);
    expect(gamepadBindingPressed(dpad, pad(0, { mapping: '', pressed: [14] }))).toBe(false);
    expect(enumerateGamepads([
      pad(0), pad(1, { mapping: '' }), null,
    ])).toEqual([
      { index: 0, supported: true }, { index: 1, supported: false },
    ]);
  });

  it('maps every portable standard button through index 11', () => {
    for (const [control, index] of [
      ['face-bottom', 0], ['face-right', 1], ['face-left', 2], ['face-top', 3],
      ['left-shoulder', 4], ['right-shoulder', 5],
      ['left-trigger', 6], ['right-trigger', 7],
      ['select', 8], ['start', 9],
      ['left-stick-button', 10], ['right-stick-button', 11],
    ]) {
      const binding = { type: 'gamepad', input: 'button', control };
      expect(gamepadBindingPressed(binding, pad(0, { pressed: [index] }))).toBe(true);
      expect(gamepadBindingPressed(binding, pad(0, {
        pressed: [(index + 1) % 12],
      }))).toBe(false);
    }
  });

  it('maps both portable right-stick axes at their standard indices', () => {
    expect(gamepadBindingPressed({
      type: 'gamepad', input: 'axis', control: 'right-stick-x',
      direction: 'positive', threshold: 0.5,
    }, pad(0, { axes: [0, 0, 0.75, 0] }))).toBe(true);
    expect(gamepadBindingPressed({
      type: 'gamepad', input: 'axis', control: 'right-stick-y',
      direction: 'negative', threshold: 0.5,
    }, pad(0, { axes: [0, 0, 0, -0.75] }))).toBe(true);
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

  it('restores a preferred logical slot without granting a replacement connection control', () => {
    let snapshot = [];
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => 'captain',
      getActions: () => [GAMEPAD_ACTION], activate,
    });
    expect(runtime.restorePreferred(2)).toEqual({ status: 'disconnected', index: 2 });
    expect(runtime.state()).toMatchObject({ selectedIndex: 2, status: 'disconnected' });

    // A later device in that browser slot is not silently granted ownership.
    snapshot = [null, null, pad(2, { pressed: [0] })];
    runtime.poll(snapshot);
    expect(runtime.state().status).toBe('disconnected');
    expect(activate).not.toHaveBeenCalled();
    runtime.select(2);
    runtime.poll(snapshot);
    expect(runtime.state().status).toBe('neutral');
    snapshot = [null, null, pad(2)];
    runtime.poll(snapshot);
    snapshot = [null, null, pad(2, { pressed: [0] })];
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

  it('keeps a held input gated while an iframe catalogue finishes loading', () => {
    let snapshot = [pad(0)];
    let actions = null;
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot,
      getContext: () => 'captain',
      getActions: () => actions,
      activate,
    });
    runtime.select(0);

    // The seam is absent while the new iframe loads. An empty-looking result
    // must not satisfy the neutral gate while its future binding is held.
    snapshot = [pad(0, { pressed: [0] })];
    runtime.poll(snapshot);
    expect(runtime.state().status).toBe('neutral');
    actions = [GAMEPAD_ACTION];
    runtime.poll(snapshot);
    expect(runtime.state().status).toBe('neutral');
    expect(activate).not.toHaveBeenCalled();

    snapshot = [pad(0)];
    runtime.poll(snapshot);
    expect(runtime.state().status).toBe('ready');
    snapshot = [pad(0, { pressed: [0] })];
    runtime.poll(snapshot);
    expect(activate).toHaveBeenCalledOnce();
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

describe('continuous standard-gamepad axes', () => {
  it('swallows drift, smoothly rescales the live band, clamps, and inverts', () => {
    const spec = CONTINUOUS_ACTION.continuous;
    expect(normalizeContinuousAxis(0.1, { deadzone: 0.1 }, spec)).toBe(0);
    expect(normalizeContinuousAxis(-0.09, { deadzone: 0.1 }, spec)).toBe(0);
    expect(normalizeContinuousAxis(0.55, { deadzone: 0.1 }, spec)).toBeCloseTo(0.5, 10);
    expect(normalizeContinuousAxis(-0.55, {
      deadzone: 0.1, inverted: true,
    }, spec)).toBeCloseTo(0.5, 10);
    expect(normalizeContinuousAxis(2, { deadzone: 0.1 }, spec)).toBe(1);
  });

  it('sends the selected axis immediately, then at authored cadence, with one release neutral', () => {
    let snapshot = [pad(0), pad(1, { axes: [1, 0, 0, 0] })];
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot,
      getContext: () => 'helm',
      getActions: (context) => context ? [CONTINUOUS_ACTION] : [CONTINUOUS_ACTION],
      activate,
    });
    runtime.select(0);
    runtime.poll(snapshot, 0);

    snapshot = [pad(0, { axes: [0.55, 0, 0, 0] }), pad(1, { axes: [1, 0, 0, 0] })];
    runtime.poll(snapshot, 1);
    expect(activate).toHaveBeenCalledTimes(1);
    expect(activate.mock.calls[0][0]).toBe('helm.steering');
    expect(activate.mock.calls[0][1]).toMatchObject({ context: 'helm' });
    expect(activate.mock.calls[0][1].value).toBeCloseTo(0.5, 10);

    snapshot = [pad(0, { axes: [0.82, 0, 0, 0] }), pad(1, { axes: [-1, 0, 0, 0] })];
    runtime.poll(snapshot, 50);
    expect(activate).toHaveBeenCalledTimes(1);
    runtime.poll(snapshot, 101);
    expect(activate).toHaveBeenCalledTimes(2);
    expect(activate.mock.calls[1][1].value).toBeCloseTo(0.8, 10);

    snapshot = [pad(0), pad(1, { axes: [-1, 0, 0, 0] })];
    runtime.poll(snapshot, 102);
    runtime.poll(snapshot, 202);
    expect(activate).toHaveBeenCalledTimes(3);
    expect(activate).toHaveBeenLastCalledWith('helm.steering', expect.objectContaining({
      context: 'helm', value: 0, neutral: true,
    }));
  });

  it('uses greatest deflection with deterministic slot order on a tie', () => {
    const action = {
      ...CONTINUOUS_ACTION,
      tuning: { deadzone: 0, inverted: false },
      bindings: [
        { type: 'gamepad', input: 'axis', control: 'left-stick-x' },
        { type: 'gamepad', input: 'axis', control: 'left-stick-y' },
      ],
    };
    let snapshot = [pad(0)];
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => 'helm',
      getActions: () => [action], activate,
    });
    runtime.select(0);
    runtime.poll(snapshot, 0);
    snapshot = [pad(0, { axes: [0.4, -0.8, 0, 0] })];
    runtime.poll(snapshot, 1);
    expect(activate.mock.calls[0][1].value).toBe(-0.8);

    runtime.neutralize(2);
    snapshot = [pad(0)];
    runtime.poll(snapshot, 3);
    snapshot = [pad(0, { axes: [0.6, -0.6, 0, 0] })];
    runtime.poll(snapshot, 4);
    expect(activate).toHaveBeenLastCalledWith('helm.steering', expect.objectContaining({
      value: 0.6,
    }));
  });

  it('neutralizes once on disconnect, never transfers, and gates every bound axis after reselection', () => {
    const action = {
      ...CONTINUOUS_ACTION,
      bindings: [
        { type: 'gamepad', input: 'axis', control: 'left-stick-x' },
        { type: 'gamepad', input: 'axis', control: 'left-stick-y' },
      ],
    };
    let snapshot = [pad(0), pad(1)];
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => 'helm',
      getActions: () => [action], activate,
    });
    runtime.select(0);
    runtime.poll(snapshot, 0);
    snapshot = [pad(0, { axes: [0.6, 0, 0, 0] }), pad(1, { axes: [-1, 0, 0, 0] })];
    runtime.poll(snapshot, 1);

    snapshot = [null, pad(1, { axes: [-1, 0, 0, 0] })];
    expect(runtime.poll(snapshot, 2).status).toBe('disconnected');
    runtime.poll(snapshot, 3);
    expect(activate).toHaveBeenCalledTimes(2);
    expect(activate).toHaveBeenLastCalledWith('helm.steering', expect.objectContaining({ value: 0 }));

    // A replacement at the same browser index cannot inherit ownership, and
    // explicit reselection remains gated while either bound axis is displaced.
    snapshot = [pad(0, { axes: [0.6, 0.7, 0, 0] }), pad(1, { axes: [-1, 0, 0, 0] })];
    runtime.poll(snapshot, 4);
    runtime.select(0);
    runtime.poll(snapshot, 5);
    snapshot = [pad(0, { axes: [0, 0.7, 0, 0] }), pad(1, { axes: [-1, 0, 0, 0] })];
    runtime.poll(snapshot, 6);
    expect(runtime.state().status).toBe('neutral');
    expect(activate).toHaveBeenCalledTimes(2);
    snapshot = [pad(0), pad(1, { axes: [-1, 0, 0, 0] })];
    runtime.poll(snapshot, 7);
    expect(runtime.state().status).toBe('ready');
    snapshot = [pad(0, { axes: [-0.55, 0, 0, 0] }), pad(1, { axes: [-1, 0, 0, 0] })];
    runtime.poll(snapshot, 8);
    expect(activate).toHaveBeenCalledTimes(3);
    expect(activate.mock.calls[2][1].value).toBeCloseTo(-0.5, 10);
  });

  it('neutral-gates tuning, context changes, and continuous axis capture', () => {
    let context = 'helm';
    let action = { ...CONTINUOUS_ACTION, tuning: { deadzone: 0.1, inverted: false } };
    let snapshot = [pad(0)];
    const activate = vi.fn();
    const captures = [];
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => context,
      getActions: () => [action], activate,
      onCapture: (target, binding) => captures.push({ target, binding }),
    });
    runtime.select(0);
    runtime.poll(snapshot, 0);
    snapshot = [pad(0, { axes: [0.5, 0, 0, 0] })];
    runtime.poll(snapshot, 1);

    action = { ...action, tuning: { deadzone: 0.2, inverted: true } };
    runtime.neutralize(2);
    runtime.poll(snapshot, 3);
    expect(runtime.state().status).toBe('neutral');
    snapshot = [pad(0)];
    runtime.poll(snapshot, 4);
    snapshot = [pad(0, { axes: [0.6, 0, 0, 0] })];
    runtime.poll(snapshot, 5);
    expect(activate).toHaveBeenLastCalledWith('helm.steering', expect.objectContaining({
      context: 'helm',
    }));
    expect(activate.mock.calls.at(-1)[1].value).toBeCloseTo(-0.5, 10);

    context = 'captain';
    runtime.poll(snapshot, 6);
    expect(activate).toHaveBeenLastCalledWith('helm.steering', expect.objectContaining({
      context: 'helm', value: 0, neutral: true,
    }));
    context = 'helm';
    snapshot = [pad(0)];
    runtime.poll(snapshot, 7);
    runtime.beginCapture('helm.steering', 1);
    runtime.poll(snapshot, 8);
    snapshot = [pad(0, { axes: [0, -0.8, 0, 0] })];
    runtime.poll(snapshot, 9);
    expect(captures).toEqual([{
      target: { actionId: 'helm.steering', slot: 1 },
      binding: { type: 'gamepad', input: 'axis', control: 'left-stick-y' },
    }]);
  });

  it('emits one immediate neutral when capture, remap/tuning, or deselection interrupts output', () => {
    let snapshot = [pad(0)];
    const activate = vi.fn();
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot, getContext: () => 'helm',
      getActions: () => [CONTINUOUS_ACTION], activate,
    });
    runtime.select(0);
    runtime.poll(snapshot, 0);
    snapshot = [pad(0, { axes: [0.6, 0, 0, 0] })];
    runtime.poll(snapshot, 1);
    runtime.beginCapture('helm.steering', 0);
    runtime.endCapture('helm.steering', 0);
    expect(activate).toHaveBeenCalledTimes(2);
    expect(activate.mock.calls[0][1].value).toBeCloseTo(5 / 9, 10);
    expect(activate.mock.calls[1][1].value).toBe(0);

    snapshot = [pad(0)];
    runtime.poll(snapshot, 2);
    snapshot = [pad(0, { axes: [-0.55, 0, 0, 0] })];
    runtime.poll(snapshot, 3);
    runtime.neutralize(4); // parent remap/tuning hook
    runtime.neutralize(5);
    expect(activate.mock.calls.filter((call) => call[1].value === 0)).toHaveLength(2);

    snapshot = [pad(0)];
    runtime.poll(snapshot, 6);
    snapshot = [pad(0, { axes: [0.55, 0, 0, 0] })];
    runtime.poll(snapshot, 7);
    runtime.select(null);
    runtime.select(null);
    expect(activate.mock.calls.filter((call) => call[1].value === 0)).toHaveLength(3);
  });

  it('neutralizes once when hidden and requires foreground neutral before fresh output', () => {
    let snapshot = [pad(0)];
    const activate = vi.fn();
    const action = {
      ...CONTINUOUS_ACTION,
      bindings: [
        { type: 'gamepad', input: 'axis', control: 'left-stick-x' },
        { type: 'gamepad', input: 'axis', control: 'left-stick-y' },
      ],
    };
    const doc = eventTarget({ hidden: false, visibilityState: 'visible' });
    const win = eventTarget({ document: doc });
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot,
      getContext: () => 'helm',
      getActions: () => [action],
      activate,
      requestAnimationFrame: vi.fn(() => 7),
      cancelAnimationFrame: vi.fn(),
    });
    runtime.select(0);
    const dispose = runtime.start(win);
    snapshot = [pad(0, { axes: [0.6, 0, 0, 0] })];
    runtime.poll(snapshot, 1);

    doc.hidden = true;
    doc.visibilityState = 'hidden';
    doc.emit('visibilitychange');
    doc.emit('visibilitychange');
    expect(activate).toHaveBeenCalledTimes(2);
    expect(activate).toHaveBeenLastCalledWith('helm.steering', expect.objectContaining({
      value: 0, neutral: true,
    }));

    // Sampling in the background cannot satisfy the gate. Becoming visible
    // re-arms it, so the still-displaced stick remains inert.
    snapshot = [pad(0)];
    runtime.poll(snapshot, 2);
    snapshot = [pad(0, { axes: [0.6, 0, 0, 0] })];
    runtime.poll(snapshot, 3);
    doc.hidden = false;
    doc.visibilityState = 'visible';
    doc.emit('visibilitychange');
    runtime.poll(snapshot, 4);
    expect(runtime.state().status).toBe('neutral');
    expect(activate).toHaveBeenCalledTimes(2);

    snapshot = [pad(0, { axes: [0, 0.6, 0, 0] })];
    runtime.poll(snapshot, 5);
    expect(runtime.state().status).toBe('neutral');
    snapshot = [pad(0)];
    runtime.poll(snapshot, 6);
    expect(runtime.state().status).toBe('ready');
    snapshot = [pad(0, { axes: [-0.55, 0, 0, 0] })];
    runtime.poll(snapshot, 7);
    expect(activate).toHaveBeenCalledTimes(3);
    expect(activate.mock.calls.at(-1)[1].value).toBeCloseTo(-0.5, 10);
    dispose();
  });

  it('pagehide plus repeated disposer teardown emits one neutral and removes every listener', () => {
    let snapshot = [pad(0)];
    const activate = vi.fn();
    const cancelAnimationFrame = vi.fn();
    const doc = eventTarget({ hidden: false, visibilityState: 'visible' });
    const win = eventTarget({ document: doc });
    const runtime = createGamepadInputRuntime({
      getGamepads: () => snapshot,
      getContext: () => 'helm',
      getActions: () => [CONTINUOUS_ACTION],
      activate,
      requestAnimationFrame: vi.fn(() => 11),
      cancelAnimationFrame,
    });
    runtime.select(0);
    const dispose = runtime.start(win);
    expect(runtime.start(win)).toBe(dispose);
    expect(win.listenerCount('gamepadconnected')).toBe(1);
    expect(win.listenerCount('gamepaddisconnected')).toBe(1);
    expect(win.listenerCount('pagehide')).toBe(1);
    expect(doc.listenerCount('visibilitychange')).toBe(1);

    snapshot = [pad(0, { axes: [0.6, 0, 0, 0] })];
    runtime.poll(snapshot, 1);
    win.emit('pagehide');
    dispose();
    dispose();
    win.emit('pagehide');
    expect(activate).toHaveBeenCalledTimes(2);
    expect(activate.mock.calls.at(-1)[1]).toMatchObject({ value: 0, neutral: true });
    expect(cancelAnimationFrame).toHaveBeenCalledTimes(1);
    expect(win.listenerCount('gamepadconnected')).toBe(0);
    expect(win.listenerCount('gamepaddisconnected')).toBe(0);
    expect(win.listenerCount('pagehide')).toBe(0);
    expect(doc.listenerCount('visibilitychange')).toBe(0);
  });

  it('stop emits one live neutral but no neutral after transport is unavailable', () => {
    for (const transportLiveAtStop of [true, false]) {
      let snapshot = [pad(0)];
      let transportLive = true;
      const activate = vi.fn();
      const doc = eventTarget({ hidden: false, visibilityState: 'visible' });
      const win = eventTarget({ document: doc });
      const runtime = createGamepadInputRuntime({
        getGamepads: () => snapshot,
        getContext: () => 'helm',
        getActions: () => [CONTINUOUS_ACTION],
        activate,
        isTransportLive: () => transportLive,
        requestAnimationFrame: vi.fn(() => 13),
        cancelAnimationFrame: vi.fn(),
      });
      runtime.select(0);
      const dispose = runtime.start(win);
      snapshot = [pad(0, { axes: [0.6, 0, 0, 0] })];
      runtime.poll(snapshot, 1);
      transportLive = transportLiveAtStop;
      dispose();
      dispose();
      expect(activate).toHaveBeenCalledTimes(transportLiveAtStop ? 2 : 1);
      if (transportLiveAtStop) {
        expect(activate.mock.calls.at(-1)[1]).toMatchObject({ value: 0, neutral: true });
      }
    }
  });
});
