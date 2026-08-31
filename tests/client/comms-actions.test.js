import { describe, expect, it, vi } from 'vitest';
import { ActionFeedbackLifecycle, ACTION_FEEDBACK_STATE } from '../../gui/action-feedback.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import {
  COMMS_ACTIONS,
  COMMS_ACTION_CONTEXT,
  COMMS_CLEAR_ACTION_ID,
  COMMS_HAIL_ACTION_ID,
  COMMS_RESPOND_ACTION_ID,
  COMMS_SELECT_MESSAGE_ACTION_ID,
  COMMS_SHOW_ON_SCREEN_ACTION_ID,
  createCommsActionRegistry,
  currentCommsMessage,
} from '../../gui/stations/comms-actions.js';

let sequence = 0;

function registry(options = {}, transitions = []) {
  return createCommsActionRegistry({
    ...options,
    actionFeedback: new ActionFeedbackLifecycle({
      correlation: () => `comms-test-${++sequence}`,
      now: () => 456,
      onTransition: (value) => transitions.push(value),
    }),
  });
}

function key(code, overrides = {}) {
  return {
    type: 'keydown', code, cancelable: true, preventDefault: vi.fn(), ...overrides,
  };
}

describe('Comms semantic action family', () => {
  it('publishes every shipped action with stable context and two remappable device slots', () => {
    expect(COMMS_ACTIONS.map((entry) => entry.id)).toEqual([
      COMMS_HAIL_ACTION_ID,
      COMMS_SELECT_MESSAGE_ACTION_ID,
      COMMS_RESPOND_ACTION_ID,
      COMMS_CLEAR_ACTION_ID,
      COMMS_SHOW_ON_SCREEN_ACTION_ID,
    ]);
    for (const action of COMMS_ACTIONS) {
      expect(action.contexts).toEqual([COMMS_ACTION_CONTEXT]);
      expect(action.bindings).toHaveLength(2);
      expect(action.bindings[0]).toMatchObject({ type: 'keyboard' });
      expect(action.bindings[1]).toMatchObject({ type: 'gamepad' });
    }
    expect(COMMS_ACTIONS.find((entry) => entry.id === COMMS_SELECT_MESSAGE_ACTION_ID).feedback)
      .toBe('local');
    expect(COMMS_ACTIONS.filter((entry) => entry.id !== COMMS_SELECT_MESSAGE_ACTION_ID)
      .every((entry) => entry.authoritativeFeedback === true)).toBe(true);

    const settings = createClientSemanticActionRegistry();
    expect(settings.list(COMMS_ACTION_CONTEXT).map((entry) => entry.id))
      .toEqual(COMMS_ACTIONS.map((entry) => entry.id));
  });

  it('hails the authoritative UUID through one correlated adapter for keys and controls', () => {
    const sendAction = vi.fn();
    const actions = registry({
      getState: () => ({ contacts: [{ uuid: 'wire-uuid', id: 'legacy-id', in_range: true }] }),
      sendAction,
    });
    expect(actions.dispatchKeyboardEvent(key('KeyH'), COMMS_ACTION_CONTEXT)).toMatchObject({
      claimed: true, handled: true, actionId: COMMS_HAIL_ACTION_ID,
    });
    expect(sendAction).toHaveBeenLastCalledWith('hail', expect.objectContaining({
      target_uuid: 'wire-uuid',
      correlation: expect.any(String),
      semantic_action: COMMS_HAIL_ACTION_ID,
      __input_ms: 456,
    }));

    actions.activate(COMMS_HAIL_ACTION_ID, {
      context: COMMS_ACTION_CONTEXT,
      source: 'control',
      detail: { target_uuid: 'explicit-stale-target' },
    });
    expect(sendAction).toHaveBeenLastCalledWith('hail', expect.objectContaining({
      target_uuid: 'explicit-stale-target',
    }));
  });

  it("resolves the cruiser's keyed Comms family without inventing a System id", () => {
    const sendAction = vi.fn();
    const actions = registry({
      getState: () => ({
        systems: { radio_port: { contacts: [{ uuid: 'radio-contact', in_range: true }] } },
        system_ids: ['radio_port'],
        system_families: { radio_port: 'comms' },
      }),
      sendAction,
    });
    actions.activate(COMMS_HAIL_ACTION_ID, { context: COMMS_ACTION_CONTEXT });
    expect(sendAction).toHaveBeenCalledWith('hail', expect.objectContaining({
      target_uuid: 'radio-contact',
    }));
  });

  it('keeps selection local while still using the shared Pressed/Pending/Applied lifecycle', () => {
    const transitions = [];
    const sendAction = vi.fn();
    const selectMessage = vi.fn(() => true);
    const actions = registry({
      getState: () => ({ messages: [{ id: 'message-1', is_read: false }] }),
      sendAction,
      selectMessage,
    }, transitions);
    expect(actions.activate(COMMS_SELECT_MESSAGE_ACTION_ID, {
      context: COMMS_ACTION_CONTEXT,
      detail: { message_id: 'message-1' },
    })).toMatchObject({ claimed: true, handled: true });
    expect(selectMessage).toHaveBeenCalledWith('message-1');
    expect(sendAction).not.toHaveBeenCalled();
    expect(transitions.map((entry) => entry.state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.APPLIED,
    ]);
  });

  it('does not claim Applied when the local selection owner rejects stale work', () => {
    const transitions = [];
    const actions = registry({
      getState: () => ({ messages: [{ id: 'message-1', is_read: false }] }),
      selectMessage: () => false,
      sendAction: vi.fn(),
    }, transitions);
    expect(actions.activate(COMMS_SELECT_MESSAGE_ACTION_ID, {
      context: COMMS_ACTION_CONTEXT,
      detail: { message_id: 'missing' },
    })).toMatchObject({ claimed: true, handled: false });
    expect(transitions.some((entry) => entry.state === ACTION_FEEDBACK_STATE.APPLIED)).toBe(false);
  });

  it('chooses only an available routine response for a key or gamepad activation', () => {
    const sendAction = vi.fn();
    const actions = registry({
      getState: () => ({ messages: [{
        id: 'dialogue-1', is_read: false,
        responses: [
          { text: 'Unavailable', available: false, important: false },
          { text: 'Important', available: true, important: true },
          { text: 'Routine', available: true, important: false },
        ],
      }] }),
      sendAction,
    });
    actions.dispatchKeyboardEvent(key('KeyR'), COMMS_ACTION_CONTEXT);
    expect(sendAction).toHaveBeenCalledWith('respond_to_message', expect.objectContaining({
      message_id: 'dialogue-1', response_index: 2,
      semantic_action: COMMS_RESPOND_ACTION_ID,
    }));
  });

  it('preserves important confirmation and leaves exact stale/availability refusal to the host', () => {
    const sendAction = vi.fn();
    const actions = registry({
      getState: () => ({ messages: [{
        id: 'dialogue-1', is_read: false,
        responses: [{ text: 'Important', available: true, important: true }],
      }] }),
      sendAction,
    });
    expect(actions.activate(COMMS_RESPOND_ACTION_ID, {
      context: COMMS_ACTION_CONTEXT,
      detail: { message_id: 'dialogue-1', response_index: 0 },
    }).handled).toBe(false);
    expect(sendAction).not.toHaveBeenCalled();

    actions.activate(COMMS_RESPOND_ACTION_ID, {
      context: COMMS_ACTION_CONTEXT,
      detail: { message_id: 'dialogue-1', response_index: 0, confirmed: true },
    });
    expect(sendAction).toHaveBeenLastCalledWith('respond_to_message', expect.objectContaining({
      message_id: 'dialogue-1', response_index: 0,
    }));

    actions.activate(COMMS_RESPOND_ACTION_ID, {
      context: COMMS_ACTION_CONTEXT,
      detail: { message_id: 'stale-message', response_index: 99 },
    });
    expect(sendAction).toHaveBeenLastCalledWith('respond_to_message', expect.objectContaining({
      message_id: 'stale-message', response_index: 99,
    }));
  });

  it('routes clear and current-message viewscreen actions through correlated adapters', () => {
    const sendAction = vi.fn();
    const messages = [
      { id: 'read', is_read: true },
      { id: 'critical', is_read: true, priority: 'Critical', selected_response: null },
    ];
    const actions = registry({ getState: () => ({ messages }), sendAction });
    expect(currentCommsMessage({ messages })).toBe(messages[1]);

    actions.dispatchKeyboardEvent(
      key('KeyC', { shiftKey: true }), COMMS_ACTION_CONTEXT,
    );
    expect(sendAction).toHaveBeenLastCalledWith('clear_comms', expect.objectContaining({
      correlation: expect.any(String), semantic_action: COMMS_CLEAR_ACTION_ID,
    }));

    actions.dispatchKeyboardEvent(key('KeyV'), COMMS_ACTION_CONTEXT);
    expect(sendAction).toHaveBeenLastCalledWith('show_on_screen', expect.objectContaining({
      message_id: 'critical', semantic_action: COMMS_SHOW_ON_SCREEN_ACTION_ID,
    }));
  });

  it('allows a safe remap without changing semantic identity', () => {
    const sendAction = vi.fn();
    const actions = registry({
      getState: () => ({ contacts: [{ uuid: 'target', in_range: true }] }), sendAction,
    });
    actions.setBinding(COMMS_HAIL_ACTION_ID, 0, { code: 'KeyJ', shiftKey: true });
    const result = actions.dispatchKeyboardEvent(
      key('KeyJ', { shiftKey: true }), COMMS_ACTION_CONTEXT,
    );
    expect(result.actionId).toBe(COMMS_HAIL_ACTION_ID);
    expect(sendAction).toHaveBeenCalledWith('hail', expect.objectContaining({
      semantic_action: COMMS_HAIL_ACTION_ID,
    }));
  });
});
