import { describe, expect, it, vi } from 'vitest';
import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
} from '../../gui/action-feedback.js';
import {
  HOST_ACTION_CONTEXT,
  HOST_GM_ACTION_CONTEXT,
  HOST_QR_CODE_ACTION_ID,
  createHostActionRegistry,
} from '../../gui/host-actions.js';

describe('host semantic actions', () => {
  it('registers stable host-scoped QR identity with two safe binding slots', () => {
    const registry = createHostActionRegistry({ toggleQrCode: () => {} });
    expect(registry.list(HOST_ACTION_CONTEXT)).toEqual([
      expect.objectContaining({
        id: HOST_QR_CODE_ACTION_ID,
        contexts: [HOST_ACTION_CONTEXT, HOST_GM_ACTION_CONTEXT],
        feedback: 'local',
        bindings: [expect.objectContaining({ type: 'keyboard', code: 'KeyQ' }), null],
      }),
    ]);
  });

  it('toggles only through the supplied host seam and completes immediately', () => {
    const transitions = [];
    const feedback = new ActionFeedbackLifecycle({
      correlation: () => 'host-qr-1',
      now: () => 123,
      onTransition: (value) => transitions.push(value),
    });
    const toggleQrCode = vi.fn();
    const registry = createHostActionRegistry({ actionFeedback: feedback, toggleQrCode });

    expect(registry.activate(HOST_QR_CODE_ACTION_ID, { context: HOST_ACTION_CONTEXT }))
      .toMatchObject({ claimed: true, handled: true, correlation: 'host-qr-1', inputMs: 123 });
    expect(toggleQrCode).toHaveBeenCalledTimes(1);
    expect(transitions.map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.APPLIED,
    ]);
  });

  it('cleans the provisional press when the existing host seam is unavailable', () => {
    const transitions = [];
    const feedback = new ActionFeedbackLifecycle({
      correlation: () => 'host-qr-missing',
      onTransition: (value) => transitions.push(value),
    });
    const registry = createHostActionRegistry({ actionFeedback: feedback });

    expect(registry.activate(HOST_QR_CODE_ACTION_ID, { context: HOST_ACTION_CONTEXT }))
      .toMatchObject({ claimed: true, handled: false });
    expect(transitions.map((value) => value.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      null,
    ]);
  });
});
