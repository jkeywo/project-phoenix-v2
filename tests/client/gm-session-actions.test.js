import { describe, expect, it, vi } from 'vitest';
import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
} from '../../gui/action-feedback.js';
import {
  GM_ACTION_CONTEXT,
  GM_PAUSE_ACTION_ID,
  GM_RESUME_ACTION_ID,
  GM_SESSION_CONFIRMATION_CATEGORY,
  createGmSessionActionRegistry,
} from '../../gui/gm-session-actions.js';

describe('GM session semantic actions', () => {
  it('registers explicit pause and resume metadata with exactly two slots', () => {
    const registry = createGmSessionActionRegistry({ submitSessionPaused: () => true });

    expect(registry.list(GM_ACTION_CONTEXT)).toEqual([
      expect.objectContaining({
        id: GM_PAUSE_ACTION_ID,
        contexts: [GM_ACTION_CONTEXT],
        labelId: 'semantic_action.gm.session_pause.label',
        accessibilityLabelId: 'semantic_action.gm.session_pause.accessibility',
        feedback: 'authoritative',
        authoritativeFeedback: true,
        confirmationCategory: GM_SESSION_CONFIRMATION_CATEGORY,
        bindings: [expect.objectContaining({ type: 'keyboard', code: 'KeyP' }), null],
      }),
      expect.objectContaining({
        id: GM_RESUME_ACTION_ID,
        contexts: [GM_ACTION_CONTEXT],
        labelId: 'semantic_action.gm.session_resume.label',
        accessibilityLabelId: 'semantic_action.gm.session_resume.accessibility',
        feedback: 'authoritative',
        authoritativeFeedback: true,
        confirmationCategory: GM_SESSION_CONFIRMATION_CATEGORY,
        bindings: [expect.objectContaining({ type: 'keyboard', code: 'KeyR' }), null],
      }),
    ]);
    expect(registry.list(GM_ACTION_CONTEXT).every(({ bindings }) => bindings.length === 2))
      .toBe(true);
  });

  it('submits absolute literal booleans with feedback correlations', () => {
    const correlations = ['gm-pause-1', 'gm-resume-1'];
    const transitions = [];
    const feedback = new ActionFeedbackLifecycle({
      correlation: () => correlations.shift(),
      now: () => 123,
      onTransition: (value) => transitions.push(value),
    });
    const submitSessionPaused = vi.fn(() => true);
    const registry = createGmSessionActionRegistry({ actionFeedback: feedback, submitSessionPaused });

    expect(registry.activate(GM_PAUSE_ACTION_ID, { context: GM_ACTION_CONTEXT }))
      .toMatchObject({ claimed: true, handled: true, correlation: 'gm-pause-1' });
    expect(registry.activate(GM_RESUME_ACTION_ID, { context: GM_ACTION_CONTEXT }))
      .toMatchObject({ claimed: true, handled: true, correlation: 'gm-resume-1' });
    expect(submitSessionPaused.mock.calls).toEqual([
      [true, 'gm-pause-1'],
      [false, 'gm-resume-1'],
    ]);
    expect(transitions.map(({ state }) => state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
    ]);
  });

  it('dispatches both keyboard defaults through the same authoritative adapters', () => {
    const submitSessionPaused = vi.fn(() => true);
    let serial = 0;
    const feedback = new ActionFeedbackLifecycle({ correlation: () => `gm-key-${++serial}` });
    const registry = createGmSessionActionRegistry({ actionFeedback: feedback, submitSessionPaused });
    const event = (code) => ({ type: 'keydown', code, preventDefault: vi.fn() });

    expect(registry.dispatchKeyboardEvent(event('KeyP'), GM_ACTION_CONTEXT))
      .toMatchObject({ claimed: true, handled: true, actionId: GM_PAUSE_ACTION_ID });
    expect(registry.dispatchKeyboardEvent(event('KeyR'), GM_ACTION_CONTEXT))
      .toMatchObject({ claimed: true, handled: true, actionId: GM_RESUME_ACTION_ID });
    expect(submitSessionPaused.mock.calls).toEqual([
      [true, 'gm-key-1'],
      [false, 'gm-key-2'],
    ]);
  });

  it('cancels provisional feedback when the host seam refuses submission', () => {
    const transitions = [];
    const feedback = new ActionFeedbackLifecycle({
      correlation: () => 'gm-refused-locally',
      onTransition: (value) => transitions.push(value),
    });
    const registry = createGmSessionActionRegistry({
      actionFeedback: feedback,
      submitSessionPaused: () => false,
    });

    expect(registry.activate(GM_PAUSE_ACTION_ID, { context: GM_ACTION_CONTEXT }))
      .toMatchObject({ claimed: true, handled: false });
    expect(transitions.map(({ state }) => state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      null,
    ]);
  });
});
