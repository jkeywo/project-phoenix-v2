import { describe, it, expect, vi } from 'vitest';
import {
  createSemanticActionRegistry,
  isSemanticInputTarget,
  isReservedKeyboardBinding,
  keyboardBindingsEqual,
  keyboardBindingDisplay,
  keyboardBindingFromEvent,
  normalizeBindingSlots,
  normalizeKeyboardBinding,
  formatSemanticBinding,
} from '../../gui/semantic-action-registry.js';
import { t } from '../../gui/strings.js';

const ACTION = {
  id: 'captain.red-alert',
  contexts: ['captain'],
  labelId: 'semantic_action.captain.red_alert.label',
  accessibilityLabelId: 'semantic_action.captain.red_alert.accessibility',
  bindings: [{ code: 'KeyR' }, null],
};

function key(code, overrides = {}) {
  return {
    type: 'keydown',
    code,
    cancelable: true,
    preventDefault: vi.fn(),
    ...overrides,
  };
}

describe('semantic action registration and normalization', () => {
  it('normalizes keyboard modifiers and pads to exactly two slots', () => {
    expect(normalizeBindingSlots([{ code: 'KeyR', shiftKey: true }])).toEqual([
      {
        type: 'keyboard', code: 'KeyR', ctrlKey: false, shiftKey: true,
        altKey: false, metaKey: false,
      },
      null,
    ]);
    expect(normalizeKeyboardBinding(null)).toBeNull();
    expect(() => normalizeBindingSlots([{ code: 'KeyR' }, null, null])).toThrow(/two/i);
  });

  it('requires stable identity, context, display and accessibility metadata', () => {
    const registry = createSemanticActionRegistry();
    expect(() => registry.register({ ...ACTION, id: '' })).toThrow(/id/i);
    expect(() => registry.register({ ...ACTION, contexts: [] })).toThrow(/context/i);
    expect(() => registry.register({ ...ACTION, labelId: '' })).toThrow(/metadata/i);
    expect(() => registry.register({ ...ACTION, accessibilityLabelId: '' })).toThrow(/metadata/i);
  });

  it('rejects duplicate semantic identities', () => {
    const registry = createSemanticActionRegistry();
    registry.register(ACTION);
    expect(() => registry.register(ACTION)).toThrow(/already registered/i);
  });

  it('exposes two current slots and context metadata without authority fields', () => {
    const registry = createSemanticActionRegistry();
    registry.register(ACTION);
    const registered = registry.action(ACTION.id);
    expect(registered.id).toBe(ACTION.id);
    expect(registered.contexts).toEqual(['captain']);
    expect(registered.bindings).toHaveLength(2);
    expect(registered.bindings[1]).toBeNull();
    expect(registered.labelId).toBe(ACTION.labelId);
    expect(registered.accessibilityLabelId).toBe(ACTION.accessibilityLabelId);
    expect(registered).not.toHaveProperty('station');
    expect(registered).not.toHaveProperty('authority');
  });
});

describe('context-scoped dispatch', () => {
  it('dispatches default and remapped bindings through the same identity', () => {
    const adapter = vi.fn(() => true);
    const registry = createSemanticActionRegistry();
    registry.register(ACTION, adapter);

    const original = key('KeyR');
    expect(registry.dispatchKeyboardEvent(original, 'captain')).toEqual({
      claimed: true, actionId: ACTION.id, handled: true,
    });
    expect(original.preventDefault).toHaveBeenCalledOnce();
    expect(adapter).toHaveBeenLastCalledWith(expect.objectContaining({
      actionId: ACTION.id, context: 'captain', source: 'keyboard',
    }));

    registry.setBinding(ACTION.id, 0, { code: 'KeyT', altKey: true });
    const stale = key('KeyR');
    expect(registry.dispatchKeyboardEvent(stale, 'captain').claimed).toBe(false);
    expect(stale.preventDefault).not.toHaveBeenCalled();

    const remapped = key('KeyT', { altKey: true });
    expect(registry.dispatchKeyboardEvent(remapped, 'captain').actionId).toBe(ACTION.id);
    expect(adapter).toHaveBeenCalledTimes(2);
  });

  it('matches KeyboardEvent.code and every modifier exactly', () => {
    const registry = createSemanticActionRegistry();
    registry.register(ACTION, () => true);
    registry.setBinding(ACTION.id, 1, { code: 'KeyR', shiftKey: true });
    expect(registry.dispatchKeyboardEvent(key('KeyR', { shiftKey: true }), 'captain').claimed)
      .toBe(true);
    expect(registry.dispatchKeyboardEvent(key('KeyR', { shiftKey: true, altKey: true }), 'captain').claimed)
      .toBe(false);
    expect(registry.dispatchKeyboardEvent(key('KeyR'), 'helm').claimed).toBe(false);
  });

  it('suppresses repeat, editable and remap-capture targets', () => {
    const adapter = vi.fn(() => true);
    const registry = createSemanticActionRegistry();
    registry.register(ACTION, adapter);

    expect(registry.dispatchKeyboardEvent(key('KeyR', { repeat: true }), 'captain').claimed)
      .toBe(false);
    expect(registry.dispatchKeyboardEvent(key('KeyR', { target: { tagName: 'INPUT' } }), 'captain').claimed)
      .toBe(false);
    const capture = {
      tagName: 'BUTTON',
      getAttribute: (name) => name === 'data-semantic-binding-capture' ? 'true' : null,
    };
    expect(isSemanticInputTarget(capture)).toBe(true);
    expect(registry.dispatchKeyboardEvent(key('KeyR', { target: capture }), 'captain').claimed)
      .toBe(false);
    expect(adapter).not.toHaveBeenCalled();
  });

  it('prevents default only after a binding is claimed', () => {
    const registry = createSemanticActionRegistry();
    registry.register(ACTION, () => false);
    const ignored = key('KeyQ');
    registry.dispatchKeyboardEvent(ignored, 'captain');
    expect(ignored.preventDefault).not.toHaveBeenCalled();
    const claimedButUnavailable = key('KeyR');
    expect(registry.dispatchKeyboardEvent(claimedButUnavailable, 'captain').handled).toBe(false);
    expect(claimedButUnavailable.preventDefault).toHaveBeenCalledOnce();
  });
});

describe('conflict-safe remapping', () => {
  const definition = (id, contexts, code, second = null) => ({
    id,
    contexts,
    labelId: `semantic_action.${id}.label`,
    accessibilityLabelId: `semantic_action.${id}.accessibility`,
    bindings: [{ code }, second && { code: second }],
  });

  it('conflicts only in intersecting contexts and preserves disjoint reuse', () => {
    const registry = createSemanticActionRegistry();
    registry.register(definition('bridge.primary', ['captain', 'bridge'], 'KeyA'));
    registry.register(definition('captain.secondary', ['captain'], 'KeyB'));
    registry.register(definition('helm.secondary', ['helm'], 'KeyC'));

    expect(registry.setBinding('bridge.primary', 0, { code: 'KeyY' }).status).toBe('applied');
    expect(registry.setBinding('helm.secondary', 0, { code: 'KeyY' }).status).toBe('applied');
    const before = registry.bindingProfile();
    const conflict = registry.setBinding('captain.secondary', 0, { code: 'KeyY' });
    expect(conflict).toMatchObject({
      status: 'conflict',
      conflicts: [{ actionId: 'bridge.primary', slot: 0 }],
    });
    expect(registry.bindingProfile()).toEqual(before);

    const replaced = registry.setBinding(
      'captain.secondary', 0, { code: 'KeyY' }, { replace: true },
    );
    expect(replaced.status).toBe('applied');
    expect(registry.action('bridge.primary').bindings[0]).toBeNull();
    expect(registry.action('captain.secondary').bindings[0].code).toBe('KeyY');
    expect(registry.action('helm.secondary').bindings[0].code).toBe('KeyY');
  });

  it('clears every overlapping conflict atomically and detects same-action slots', () => {
    const registry = createSemanticActionRegistry();
    registry.register(definition('multi.target', ['captain', 'bridge'], 'KeyA'));
    registry.register(definition('captain.source', ['captain'], 'KeyB'));
    registry.register(definition('bridge.source', ['bridge'], 'KeyC'));
    registry.setBinding('captain.source', 0, { code: 'KeyY' });
    registry.setBinding('bridge.source', 0, { code: 'KeyY' });

    const conflict = registry.setBinding('multi.target', 1, { code: 'KeyY' });
    expect(conflict.conflicts.map(({ actionId }) => actionId)).toEqual([
      'captain.source', 'bridge.source',
    ]);
    registry.setBinding('multi.target', 1, { code: 'KeyY' }, { replace: true });
    expect(registry.action('captain.source').bindings[0]).toBeNull();
    expect(registry.action('bridge.source').bindings[0]).toBeNull();

    const sameAction = registry.setBinding('multi.target', 0, { code: 'KeyY' });
    expect(sameAction).toMatchObject({
      status: 'conflict',
      conflicts: [{ actionId: 'multi.target', slot: 1 }],
    });
  });

  it('treats modifiers as binding identity', () => {
    const plain = normalizeKeyboardBinding({ code: 'KeyY' });
    const shifted = normalizeKeyboardBinding({ code: 'KeyY', shiftKey: true });
    expect(keyboardBindingsEqual(plain, shifted)).toBe(false);

    const registry = createSemanticActionRegistry();
    registry.register(definition('captain.one', ['captain'], 'KeyA'));
    registry.register(definition('captain.two', ['captain'], 'KeyB'));
    registry.setBinding('captain.one', 0, plain);
    expect(registry.setBinding('captain.two', 0, shifted).status).toBe('applied');
  });

  it('detects and replaces logical gamepad conflicts in overlapping contexts', () => {
    const registry = createSemanticActionRegistry();
    registry.register({
      ...ACTION,
      bindings: [{ code: 'KeyR' }, {
        type: 'gamepad', input: 'button', control: 'face-bottom',
      }],
    });
    registry.register({
      ...ACTION,
      id: 'captain.second',
      bindings: [{ code: 'KeyH' }, null],
    });
    const binding = { type: 'gamepad', input: 'button', control: 'face-bottom' };
    expect(registry.setBinding('captain.second', 1, binding)).toMatchObject({
      status: 'conflict', conflicts: [{ actionId: ACTION.id, slot: 1 }],
    });
    registry.setBinding('captain.second', 1, binding, { replace: true });
    expect(registry.action(ACTION.id).bindings[1]).toBeNull();
    expect(registry.action('captain.second').bindings[1]).toEqual(binding);
    registry.resetAction(ACTION.id);
    expect(registry.action(ACTION.id).bindings[1]).toEqual(binding);
    expect(registry.action('captain.second').bindings[1]).toBeNull();
  });
});

describe('reserved keyboard chords', () => {
  it('covers portable browser function keys and reviewer-reported misses', () => {
    expect(isReservedKeyboardBinding({ code: 'F4', ctrlKey: true })).toBe(true);
    expect(isReservedKeyboardBinding({ code: 'F1' })).toBe(true);
    expect(isReservedKeyboardBinding({ code: 'F6' })).toBe(true);
    expect(isReservedKeyboardBinding({ code: 'Home', altKey: true })).toBe(true);
    for (const code of ['F1', 'F3', 'F5', 'F6', 'F7', 'F10', 'F11', 'F12']) {
      expect(isReservedKeyboardBinding({ code })).toBe(true);
    }
  });

  it('covers tab selection, address-bar and every Meta chord', () => {
    expect(isReservedKeyboardBinding({ code: 'Digit1', ctrlKey: true })).toBe(true);
    expect(isReservedKeyboardBinding({ code: 'Digit9', ctrlKey: true })).toBe(true);
    expect(isReservedKeyboardBinding({ code: 'KeyD', altKey: true })).toBe(true);
    expect(isReservedKeyboardBinding({ code: 'KeyI', metaKey: true })).toBe(true);
    expect(isReservedKeyboardBinding({ code: 'KeyV', metaKey: true })).toBe(true);
    expect(isReservedKeyboardBinding({ code: 'ArrowLeft', metaKey: true })).toBe(true);
    for (let digit = 1; digit <= 9; digit++) {
      expect(isReservedKeyboardBinding({ code: `Digit${digit}`, ctrlKey: true })).toBe(true);
    }
    for (const code of ['KeyA', 'KeyV', 'ArrowUp', 'F2', 'Numpad1']) {
      expect(isReservedKeyboardBinding({ code, metaKey: true })).toBe(true);
    }
  });

  it('refuses dedicated browser and OS keys without applying them', () => {
    const codes = [
      'PrintScreen',
      'BrowserBack', 'BrowserForward', 'BrowserRefresh', 'BrowserHome',
      'BrowserSearch', 'BrowserFavorites', 'BrowserStop',
    ];
    const registry = createSemanticActionRegistry();
    registry.register(ACTION);
    const before = registry.bindingProfile();
    for (const code of codes) {
      expect(isReservedKeyboardBinding({ code })).toBe(true);
      expect(registry.setBinding(ACTION.id, 0, { code })).toMatchObject({
        status: 'reserved', actionId: ACTION.id, slot: 0,
      });
      expect(registry.bindingProfile()).toEqual(before);
    }
  });

  it('protects browser and OS chords even with optional Shift', () => {
    for (const binding of [
      { code: 'Escape' },
      { code: 'Tab', ctrlKey: true },
      { code: 'KeyR', ctrlKey: true, shiftKey: true },
      { code: 'KeyW', metaKey: true },
      { code: 'KeyD', ctrlKey: true },
      { code: 'KeyH', ctrlKey: true },
      { code: 'KeyJ', metaKey: true },
      { code: 'KeyU', ctrlKey: true },
      { code: 'KeyK', metaKey: true },
      { code: 'F4', metaKey: true, shiftKey: true },
      { code: 'PageUp', ctrlKey: true },
      { code: 'PageDown', metaKey: true },
      { code: 'Delete', ctrlKey: true, shiftKey: true },
      { code: 'Delete', ctrlKey: true, altKey: true },
      { code: 'KeyA', ctrlKey: true, shiftKey: true },
      { code: 'KeyB', ctrlKey: true, shiftKey: true },
      { code: 'KeyC', ctrlKey: true, shiftKey: true },
      { code: 'KeyI', ctrlKey: true, shiftKey: true },
      { code: 'KeyJ', ctrlKey: true, shiftKey: true },
      { code: 'KeyM', ctrlKey: true, shiftKey: true },
      { code: 'KeyQ', ctrlKey: true, shiftKey: true },
      { code: 'KeyD', altKey: true },
      { code: 'KeyE', altKey: true, shiftKey: true },
      { code: 'KeyF', altKey: true },
      { code: 'F4', altKey: true },
      { code: 'Tab', altKey: true, shiftKey: true },
      { code: 'Home', altKey: true, shiftKey: true },
      { code: 'ArrowLeft', altKey: true },
      { code: 'ArrowDown', altKey: true },
      { code: 'Space', altKey: true },
      { code: 'Enter', altKey: true },
      { code: 'KeyB', altKey: true, shiftKey: true },
      { code: 'KeyI', altKey: true, shiftKey: true },
      { code: 'KeyT', altKey: true, shiftKey: true },
      { code: 'Space', metaKey: true },
      { code: 'KeyQ', metaKey: true, shiftKey: true },
      { code: 'KeyM', metaKey: true },
      { code: 'BracketLeft', metaKey: true },
      { code: 'Backquote', metaKey: true },
      { code: 'AltLeft', altKey: true },
      { code: 'MetaRight', metaKey: true },
    ]) expect(isReservedKeyboardBinding(binding)).toBe(true);
  });

  it('keeps ordinary controls and standalone Control/Shift available', () => {
    for (const binding of [
      { code: 'Space' },
      { code: 'ArrowLeft' },
      { code: 'ArrowRight', shiftKey: true },
      { code: 'F2' },
      { code: 'F4' },
      { code: 'F8' },
      { code: 'F9' },
      { code: 'KeyR' },
      { code: 'KeyY', ctrlKey: true },
      { code: 'Delete', ctrlKey: true },
      { code: 'KeyX', altKey: true },
      { code: 'KeyR', altKey: true },
      { code: 'Digit1', altKey: true },
      { code: 'ControlLeft', ctrlKey: true },
      { code: 'ShiftRight', shiftKey: true },
    ]) expect(isReservedKeyboardBinding(binding)).toBe(false);
  });

  it('refuses a reserved proposal without mutating either slot', () => {
    const registry = createSemanticActionRegistry();
    registry.register(ACTION);
    const before = registry.bindingProfile();
    expect(registry.setBinding(ACTION.id, 0, { code: 'KeyR', ctrlKey: true }))
      .toMatchObject({ status: 'reserved' });
    expect(registry.bindingProfile()).toEqual(before);
  });
});

describe('authored binding resets', () => {
  const action = (id, contexts, first, second) => ({
    id,
    contexts,
    labelId: `semantic_action.${id}.label`,
    accessibilityLabelId: `semantic_action.${id}.accessibility`,
    bindings: [{ code: first }, { code: second }],
  });

  it('restores both action slots and clears remaps colliding with those defaults', () => {
    const registry = createSemanticActionRegistry();
    registry.register(action('captain.one', ['captain'], 'KeyA', 'Digit1'));
    registry.register(action('captain.two', ['captain'], 'KeyB', 'Digit2'));
    registry.setBinding('captain.one', 0, { code: 'KeyY' });
    registry.setBinding('captain.one', 1, { code: 'KeyU' });
    registry.setBinding('captain.two', 0, { code: 'KeyA' });
    registry.setBinding('captain.two', 1, { code: 'Digit1' });

    const result = registry.resetAction('captain.one');
    expect(result.cleared).toHaveLength(2);
    expect(registry.action('captain.one').bindings.map((binding) => binding.code))
      .toEqual(['KeyA', 'Digit1']);
    expect(registry.action('captain.two').bindings).toEqual([null, null]);
  });

  it('restores the complete two-slot authored profile globally', () => {
    const registry = createSemanticActionRegistry();
    registry.register(action('captain.one', ['captain'], 'KeyA', 'Digit1'));
    registry.register(action('captain.two', ['captain'], 'KeyB', 'Digit2'));
    registry.setBinding('captain.one', 0, { code: 'KeyY' });
    registry.setBinding('captain.one', 1, null);
    registry.setBinding('captain.two', 0, null);
    registry.setBinding('captain.two', 1, { code: 'KeyU' });

    expect(registry.resetAllBindings().status).toBe('applied');
    expect(registry.action('captain.one').bindings.map((binding) => binding.code))
      .toEqual(['KeyA', 'Digit1']);
    expect(registry.action('captain.two').bindings.map((binding) => binding.code))
      .toEqual(['KeyB', 'Digit2']);
  });
});

describe('binding profile seam and display tokens', () => {
  it('round-trips a serialisable two-slot profile and ignores unknown actions', () => {
    const registry = createSemanticActionRegistry();
    registry.register(ACTION);
    registry.updateBindings({
      [ACTION.id]: [null, { code: 'Digit7', metaKey: true }],
      'future.action': [{ code: 'KeyZ' }, null],
    });
    expect(registry.bindingProfile()[ACTION.id]).toEqual([
      null,
      {
        type: 'keyboard', code: 'Digit7', ctrlKey: false, shiftKey: false,
        altKey: false, metaKey: true,
      },
    ]);
  });

  it('normalizes events and returns localisable display parts', () => {
    const binding = keyboardBindingFromEvent({ code: 'KeyR', ctrlKey: true });
    expect(keyboardBindingDisplay(binding)).toEqual({
      modifiers: ['input.modifier.control'],
      code: 'R',
    });
  });

  it('serializes only logical gamepad controls and formats the binding union', () => {
    const registry = createSemanticActionRegistry();
    registry.register({
      ...ACTION,
      bindings: [{ code: 'KeyR' }, {
        type: 'gamepad', input: 'axis', control: 'left-stick-x',
        direction: 'positive', threshold: 0.75,
      }],
    });
    const profile = registry.bindingProfile();
    expect(profile[ACTION.id][1]).toEqual({
      type: 'gamepad', input: 'axis', control: 'left-stick-x',
      direction: 'positive', threshold: 0.75,
    });
    const encoded = JSON.stringify(profile);
    expect(encoded).not.toContain('id');
    expect(encoded).not.toContain('index');
    expect(encoded).not.toContain('hardware');
    expect(formatSemanticBinding(profile[ACTION.id][1], t))
      .toBe(t('input.gamepad.left_stick_right', { threshold: '0.75' }));
  });

  it('keeps gamepad axis and device labels free of encoding artifacts', () => {
    const labels = [
      'input.gamepad.left_stick_left',
      'input.gamepad.left_stick_right',
      'input.gamepad.left_stick_up',
      'input.gamepad.left_stick_down',
    ].map((id) => t(id, { threshold: '0.5' }));
    labels.push(
      t('settings.controls.gamepad.device_unsupported', { slot: '1' }),
      t('settings.controls.gamepad.device_disconnected', { slot: '1' }),
    );

    for (const label of labels) {
      expect(label).not.toMatch(/[\u00c2\u00c3\u00e2\ufffd]/u);
    }
  });
});

describe('continuous semantic actions', () => {
  const STEERING = {
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

  it('normalizes undirected axes with authored output metadata and separate tuning', () => {
    const registry = createSemanticActionRegistry();
    registry.register(STEERING);
    expect(registry.action(STEERING.id)).toMatchObject({
      continuous: { min: -1, max: 1, neutral: 0, cadenceMs: 100 },
      tuning: { deadzone: 0.1, inverted: false },
      bindings: [
        { type: 'gamepad', input: 'axis', control: 'left-stick-x' },
        null,
      ],
    });
    expect(formatSemanticBinding(registry.action(STEERING.id).bindings[0], t))
      .toBe(t('input.gamepad.left_stick_x'));
    expect(() => registry.setBinding(STEERING.id, 1, { code: 'KeyY' }))
      .toThrow(/gamepad axis/i);
  });

  it('passes only finite in-range scalars to the adapter', () => {
    const adapter = vi.fn(() => true);
    const registry = createSemanticActionRegistry();
    registry.register(STEERING, adapter);
    expect(registry.activate(STEERING.id, { context: 'helm', value: 0.4 }))
      .toMatchObject({ claimed: true, handled: true });
    expect(adapter).toHaveBeenCalledWith(expect.objectContaining({ value: 0.4 }));
    registry.activate(STEERING.id, { context: 'helm', value: 1.1 });
    registry.activate(STEERING.id, { context: 'helm', value: Number.NaN });
    expect(adapter).toHaveBeenCalledOnce();
  });

  it('exposes serialisable in-memory tuning and resets authored defaults', () => {
    const registry = createSemanticActionRegistry();
    registry.register(STEERING);
    expect(registry.setTuning(STEERING.id, {
      deadzone: 0.25, inverted: true,
    }).status).toBe('applied');
    expect(registry.tuningProfile()).toEqual({
      [STEERING.id]: { deadzone: 0.25, inverted: true },
    });
    expect(JSON.parse(JSON.stringify(registry.tuningProfile())))
      .toEqual(registry.tuningProfile());
    registry.resetAction(STEERING.id);
    expect(registry.tuningProfile()[STEERING.id]).toEqual({
      deadzone: 0.1, inverted: false,
    });
    expect(() => registry.setTuning(STEERING.id, { deadzone: 1 }))
      .toThrow(/deadzone/i);
  });

  it('treats a directed discrete axis as conflicting with its continuous axis', () => {
    const registry = createSemanticActionRegistry();
    registry.register(STEERING);
    expect(() => registry.register({
      ...ACTION,
      id: 'helm.discrete-right',
      contexts: ['helm'],
      bindings: [{
        type: 'gamepad', input: 'axis', control: 'left-stick-x',
        direction: 'positive', threshold: 0.5,
      }, null],
    })).toThrow(/conflict/i);
  });

  it('rejects discrete axis defaults that differ only by trigger threshold', () => {
    const registry = createSemanticActionRegistry();
    const axisAction = (id, threshold) => ({
      ...ACTION,
      id,
      contexts: ['helm'],
      bindings: [{
        type: 'gamepad', input: 'axis', control: 'left-stick-y',
        direction: 'negative', threshold,
      }, null],
    });
    registry.register(axisAction('helm.axis-low-threshold', 0.5));
    expect(() => registry.register(axisAction('helm.axis-high-threshold', 0.9)))
      .toThrow(/conflict/i);
  });
});
