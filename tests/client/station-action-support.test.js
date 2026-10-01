import { describe, expect, it, vi } from 'vitest';
import {
  axis, keyboard, defineStationAction, createCorrelatedActionSender,
} from '../../gui/stations/action-support.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';

describe('Station action support', () => {
  it.each(['authoritative', 'local'])('registers immutable two-slot metadata with %s feedback', (feedback) => {
    const contexts = ['helm'];
    const bindings = [keyboard('KeyR', { shiftKey: true }), axis('left-stick-x', 'negative')];
    const definition = defineStationAction({
      id: 'helm.test', contexts, labelKey: 'helm.test', bindings, feedback,
    });
    const registry = createSemanticActionRegistry();
    expect(() => registry.register(definition, () => true)).not.toThrow();
    contexts.push('captain');
    bindings[0] = null;
    expect(definition.contexts).toEqual(['helm']);
    expect(definition.bindings).toHaveLength(2);
    expect(definition.bindings[0]).toMatchObject({ code: 'KeyR', shiftKey: true, ctrlKey: false });
    expect(definition.bindings[1]).toMatchObject({ direction: 'negative', threshold: 0.5 });
    for (const value of [definition, definition.contexts, definition.bindings, ...definition.bindings]) {
      expect(Object.isFrozen(value)).toBe(true);
    }
    expect(definition.authoritativeFeedback).toBe(feedback === 'authoritative' ? true : undefined);
    expect(definition.feedback).toBe(feedback === 'local' ? 'local' : undefined);
  });

  it.each([undefined, null, '', 123])('refuses absent or invalid correlation %s', (correlation) => {
    const sendAction = vi.fn();
    expect(createCorrelatedActionSender(sendAction)('helm.test', correlation, 12, 'command')).toBe(false);
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('refuses a missing callback and preserves the authoritative envelope over payload fields', () => {
    expect(createCorrelatedActionSender(null)('helm.test', 'press', 12, 'command')).toBe(false);
    const sendAction = vi.fn();
    const payload = { active: true, correlation: 'stale', semantic_action: 'stale', __input_ms: 0 };
    expect(createCorrelatedActionSender(sendAction)('helm.test', 'press', 12, 'command', payload)).toBe(true);
    expect(sendAction).toHaveBeenCalledExactlyOnceWith('command', {
      active: true, correlation: 'press', semantic_action: 'helm.test', __input_ms: 12,
    });
    expect(payload.correlation).toBe('stale');
  });
});
