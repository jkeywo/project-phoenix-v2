import { describe, it, expect, vi } from 'vitest';
import {
  createSemanticActionRegistry,
  isSemanticInputTarget,
  keyboardBindingDisplay,
  keyboardBindingFromEvent,
  normalizeBindingSlots,
  normalizeKeyboardBinding,
} from '../../gui/semantic-action-registry.js';

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
    registry.setBinding(ACTION.id, 1, { code: 'KeyR', ctrlKey: true });
    expect(registry.dispatchKeyboardEvent(key('KeyR', { ctrlKey: true }), 'captain').claimed)
      .toBe(true);
    expect(registry.dispatchKeyboardEvent(key('KeyR', { ctrlKey: true, shiftKey: true }), 'captain').claimed)
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
});
