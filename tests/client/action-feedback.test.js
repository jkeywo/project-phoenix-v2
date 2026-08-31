// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
  ActionFeedbackRouter,
  emitActionFeedbackTransition,
  isValidActionCorrelation,
  MAX_ACTION_CORRELATION_BYTES,
} from '../../gui/action-feedback.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';

function lifecycleHarness(ids = ['corr-1']) {
  const transitions = [];
  let index = 0;
  const lifecycle = new ActionFeedbackLifecycle({
    now: () => 1_700_000_000_123.5,
    correlation: () => ids[index++],
    onTransition: (value) => transitions.push(value),
  });
  return { lifecycle, transitions };
}

describe('shared action feedback lifecycle', () => {
  it('drives Pressed, Pending and every terminal presentation from one transition', () => {
    for (const terminal of [
      ACTION_FEEDBACK_STATE.APPLIED,
      ACTION_FEEDBACK_STATE.REFUSED,
      ACTION_FEEDBACK_STATE.TIMED_OUT,
    ]) {
      const { lifecycle, transitions } = lifecycleHarness();
      const press = lifecycle.press('captain.red-alert');
      expect(press).toEqual({ correlation: 'corr-1', inputMs: 1_700_000_000_123.5 });
      expect(lifecycle.pending(press.correlation)).toBe(true);
      expect(lifecycle.settle(press.correlation, terminal)).toBe(true);
      expect(transitions.map((value) => value.state)).toEqual([
        ACTION_FEEDBACK_STATE.PRESSED,
        ACTION_FEEDBACK_STATE.PENDING,
        terminal,
      ]);
      const final = transitions.at(-1);
      expect(final).toMatchObject({
        actionId: 'captain.red-alert',
        correlation: 'corr-1',
        inputMs: 1_700_000_000_123.5,
        statusId: expect.stringMatching(/^action_feedback\./),
        cue: expect.stringMatching(/^action-/),
        isCurrent: true,
      });
      expect(lifecycle.settle(press.correlation, terminal)).toBe(false);
    }
  });

  it('keeps overlapping identities independent and marks only the newest visual result current', () => {
    const { lifecycle, transitions } = lifecycleHarness(['corr-old', 'corr-new']);
    const oldPress = lifecycle.press('captain.red-alert');
    lifecycle.pending(oldPress.correlation);
    const newPress = lifecycle.press('captain.red-alert');
    lifecycle.pending(newPress.correlation);

    lifecycle.settle(oldPress.correlation, ACTION_FEEDBACK_STATE.APPLIED);
    lifecycle.settle(newPress.correlation, ACTION_FEEDBACK_STATE.REFUSED);
    expect(transitions.find((value) => value.correlation === 'corr-old'
      && value.state === ACTION_FEEDBACK_STATE.APPLIED).isCurrent).toBe(false);
    expect(transitions.at(-1)).toMatchObject({ correlation: 'corr-new', isCurrent: true });
  });

  it('promotes an older pending occurrence when the newest occurrence is cancelled', () => {
    const { lifecycle, transitions } = lifecycleHarness(['corr-old', 'corr-new']);
    const oldPress = lifecycle.press('captain.red-alert');
    lifecycle.pending(oldPress.correlation);
    const newPress = lifecycle.press('captain.red-alert');
    lifecycle.pending(newPress.correlation);

    expect(lifecycle.cancel(newPress.correlation)).toBe(true);
    expect(transitions.slice(-2)).toEqual([
      expect.objectContaining({
        correlation: 'corr-new', state: null, cancelled: true, isCurrent: true,
      }),
      expect.objectContaining({
        correlation: 'corr-old',
        state: ACTION_FEEDBACK_STATE.PENDING,
        statusId: 'action_feedback.pending',
        isCurrent: true,
        lifecycleTransition: false,
        presentationRestored: true,
        cue: null,
        vibrationIntent: null,
      }),
    ]);
    expect(transitions.filter((value) => value.correlation === 'corr-old'
      && value.cue === 'action-pending')).toHaveLength(1);
    expect(transitions.filter((value) => value.correlation === 'corr-old'
      && value.lifecycleTransition).map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
    ]);
    expect(lifecycle.settle(oldPress.correlation, ACTION_FEEDBACK_STATE.APPLIED)).toBe(true);
    expect(transitions.at(-1)).toMatchObject({
      correlation: 'corr-old',
      state: ACTION_FEEDBACK_STATE.APPLIED,
      isCurrent: true,
    });
    expect(transitions.filter((value) => value.correlation === 'corr-old'
      && value.cue === 'action-applied')).toHaveLength(1);
    expect(transitions.filter((value) => value.correlation === 'corr-old'
      && value.lifecycleTransition).map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.APPLIED,
    ]);
  });

  it('cleans the provisional Pressed record when an adapter returns false', () => {
    const { lifecycle, transitions } = lifecycleHarness();
    const registry = createSemanticActionRegistry({ actionFeedback: lifecycle });
    registry.register({
      id: 'captain.red-alert',
      contexts: ['captain'],
      labelId: 'semantic_action.captain.red_alert.label',
      accessibilityLabelId: 'semantic_action.captain.red_alert.accessibility',
      authoritativeFeedback: true,
      bindings: [null, null],
    }, () => false);

    expect(registry.activate('captain.red-alert', { context: 'captain' }))
      .toMatchObject({ claimed: true, handled: false });
    expect(transitions.map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      null,
    ]);
    expect(transitions.at(-1)).toMatchObject({ cancelled: true, statusId: null });
    expect(lifecycle.size()).toBe(0);
  });

  it('lets an async local adapter settle through the same registry activation', () => {
    const { lifecycle, transitions } = lifecycleHarness();
    const registry = createSemanticActionRegistry({ actionFeedback: lifecycle });
    let settle;
    registry.register({
      id: 'editor.mod.import', contexts: ['editor'],
      labelId: 'semantic_action.captain.red_alert.label',
      accessibilityLabelId: 'semantic_action.captain.red_alert.accessibility',
      feedback: 'local', bindings: [null, null],
    }, ({ settleFeedback }) => { settle = settleFeedback; return true; });

    registry.activate('editor.mod.import', { context: 'editor' });
    settle(ACTION_FEEDBACK_STATE.REFUSED);
    expect(transitions.map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.REFUSED,
    ]);
  });

  it.each([
    ACTION_FEEDBACK_STATE.APPLIED,
    ACTION_FEEDBACK_STATE.REFUSED,
  ])('buffers synchronous local %s until Pending', (terminal) => {
    const { lifecycle, transitions } = lifecycleHarness();
    const registry = createSemanticActionRegistry({ actionFeedback: lifecycle });
    registry.register({
      id: 'editor.mod.import', contexts: ['editor'],
      labelId: 'semantic_action.captain.red_alert.label',
      accessibilityLabelId: 'semantic_action.captain.red_alert.accessibility',
      feedback: 'local', bindings: [null, null],
    }, ({ settleFeedback }) => {
      expect(settleFeedback(terminal)).toBe(true);
      return true;
    });

    expect(registry.activate('editor.mod.import', { context: 'editor' }))
      .toMatchObject({ claimed: true, handled: true });
    expect(transitions.map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      terminal,
    ]);
  });

  it('discards a synchronous settlement when the adapter returns handled=false', () => {
    const { lifecycle, transitions } = lifecycleHarness();
    const registry = createSemanticActionRegistry({ actionFeedback: lifecycle });
    registry.register({
      id: 'editor.mod.import', contexts: ['editor'],
      labelId: 'semantic_action.captain.red_alert.label',
      accessibilityLabelId: 'semantic_action.captain.red_alert.accessibility',
      feedback: 'local', bindings: [null, null],
    }, ({ settleFeedback }) => {
      settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
      return false;
    });

    registry.activate('editor.mod.import', { context: 'editor' });
    expect(transitions.map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      null,
    ]);
  });

  it('bounds and validates the opaque identity without interpreting it', () => {
    expect(isValidActionCorrelation('uuid-shaped_but-opaque')).toBe(true);
    expect(isValidActionCorrelation('')).toBe(false);
    expect(isValidActionCorrelation('has a space')).toBe(false);
    expect(isValidActionCorrelation('x'.repeat(MAX_ACTION_CORRELATION_BYTES + 1))).toBe(false);
  });

  it('bounds provisional records and times out the displaced identity', () => {
    const transitions = [];
    const ids = ['corr-old', 'corr-new'];
    const lifecycle = new ActionFeedbackLifecycle({
      capacity: 1,
      correlation: () => ids.shift(),
      onTransition: (value) => transitions.push(value),
    });
    lifecycle.pending(lifecycle.press('captain.red-alert').correlation);
    lifecycle.press('captain.red-alert');

    expect(lifecycle.size()).toBe(1);
    expect(transitions).toContainEqual(expect.objectContaining({
      correlation: 'corr-old', state: ACTION_FEEDBACK_STATE.TIMED_OUT,
    }));
  });
});

describe('originating iframe router', () => {
  function routerHarness() {
    const scheduled = [];
    const delivered = [];
    const router = new ActionFeedbackRouter({
      schedule: (fn) => { scheduled.push(fn); return scheduled.length; },
      cancelSchedule: vi.fn(),
      deliver: (value) => delivered.push(value),
    });
    return { router, scheduled, delivered };
  }

  it('routes accepted and refused results only to their exact originating correlations', () => {
    const { router, delivered } = routerHarness();
    router.track({ correlation: 'corr-a', actionId: 'captain.red-alert', console: 'captain', inputMs: 10 });
    router.track({ correlation: 'corr-b', actionId: 'captain.red-alert', console: 'visiting-captain', inputMs: 20 });

    expect(router.resolve({ correlation: 'corr-b', outcome: 'Refused' })).toBe(true);
    expect(router.resolve({ correlation: 'corr-a', outcome: 'Applied' })).toBe(true);
    expect(delivered).toEqual([
      expect.objectContaining({ correlation: 'corr-b', console: 'visiting-captain', state: 'Refused' }),
      expect.objectContaining({ correlation: 'corr-a', console: 'captain', state: 'Applied' }),
    ]);
    expect(router.resolve({ correlation: 'corr-a', outcome: 'Applied' })).toBe(false);
  });

  it('times out a missing acknowledgement and ignores a late response', () => {
    const { router, scheduled, delivered } = routerHarness();
    router.track({ correlation: 'corr-timeout', actionId: 'captain.red-alert', console: 'captain', inputMs: 10 });
    scheduled[0]();
    expect(delivered).toEqual([
      expect.objectContaining({ correlation: 'corr-timeout', state: 'TimedOut' }),
    ]);
    expect(router.resolve({ correlation: 'corr-timeout', outcome: 'Applied' })).toBe(false);
  });

  it('bounds outstanding origins by timing out the oldest correlation', () => {
    const delivered = [];
    const router = new ActionFeedbackRouter({
      capacity: 1,
      schedule: () => 1,
      cancelSchedule: () => {},
      deliver: (value) => delivered.push(value),
    });
    router.track({ correlation: 'corr-old', actionId: 'captain.red-alert', console: 'a', inputMs: 1 });
    router.track({ correlation: 'corr-new', actionId: 'captain.red-alert', console: 'b', inputMs: 2 });
    expect(router.size()).toBe(1);
    expect(delivered).toEqual([
      expect.objectContaining({ correlation: 'corr-old', state: 'TimedOut', console: 'a' }),
    ]);
  });
});

describe('feedback event hooks', () => {
  it('emits live presentation, semantic cue and optional vibration intent from one value', () => {
    const seen = { feedback: [], cue: [], vibration: [] };
    window.addEventListener('phoenix-action-feedback', (event) => seen.feedback.push(event.detail));
    window.addEventListener('phoenix-semantic-cue', (event) => seen.cue.push(event.detail));
    window.addEventListener('phoenix-vibration-intent', (event) => seen.vibration.push(event.detail));
    const value = {
      actionId: 'captain.red-alert', correlation: 'corr-1', state: 'Applied',
      statusId: 'action_feedback.applied', cue: 'action-applied', vibrationIntent: 'confirm',
    };
    emitActionFeedbackTransition(window, value);
    const restored = {
      actionId: 'captain.red-alert', correlation: 'corr-old', state: 'Pending',
      statusId: 'action_feedback.pending', cue: null, vibrationIntent: null,
      lifecycleTransition: false, presentationRestored: true,
    };
    emitActionFeedbackTransition(window, restored);
    expect(seen.feedback).toEqual([value, restored]);
    expect(seen.cue).toEqual([value]);
    expect(seen.vibration).toEqual([value]);
  });

  it('keeps mandatory status while profile preferences suppress optional outputs', () => {
    const seen = { feedback: [], cue: [], vibration: [] };
    window.addEventListener('phoenix-action-feedback', (event) => seen.feedback.push(event.detail));
    window.addEventListener('phoenix-semantic-cue', (event) => seen.cue.push(event.detail));
    window.addEventListener('phoenix-vibration-intent', (event) => seen.vibration.push(event.detail));
    const value = {
      actionId: 'captain.red-alert', correlation: 'corr-private', state: 'Applied',
      statusId: 'action_feedback.applied', cue: 'action-applied', vibrationIntent: 'confirm',
    };
    emitActionFeedbackTransition(window, value, {
      semanticCues: false,
      vibration: false,
    });
    expect(seen.feedback).toEqual([value]);
    expect(seen.cue).toEqual([]);
    expect(seen.vibration).toEqual([]);
  });
});
