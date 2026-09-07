/**
 * Shared semantic action adapters for every shipped Comms control.
 *
 * These adapters keep input identity and feedback above the legacy action-map
 * seam. They deliberately send the same Comms commands as the visible
 * controls did before #1283; reachability, dialogue and response validation
 * remain authoritative in the existing Rust consumers.
 */

import { ACTION_FEEDBACK_STATE } from '../action-feedback.js';
import { isLatestLiveCriticalMessage } from '../comms-state.js';
import { familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const COMMS_ACTION_CONTEXT = 'comms';
export const COMMS_HAIL_ACTION_ID = 'comms.hail';
export const COMMS_SELECT_MESSAGE_ACTION_ID = 'comms.select-message';
export const COMMS_RESPOND_ACTION_ID = 'comms.respond';
export const COMMS_CLEAR_ACTION_ID = 'comms.clear';
export const COMMS_SHOW_ON_SCREEN_ACTION_ID = 'comms.show-on-screen';

function keyboard(code, modifiers = {}) {
  return Object.freeze({
    type: 'keyboard', code,
    ctrlKey: !!modifiers.ctrlKey,
    shiftKey: !!modifiers.shiftKey,
    altKey: !!modifiers.altKey,
    metaKey: !!modifiers.metaKey,
  });
}

function gamepad(input, control) {
  return Object.freeze({ type: 'gamepad', input, control });
}

function action(id, label, keyboardBinding, gamepadBinding, feedback = 'authoritative') {
  return Object.freeze({
    id,
    contexts: Object.freeze([COMMS_ACTION_CONTEXT]),
    labelId: `semantic_action.comms.${label}.label`,
    accessibilityLabelId: `semantic_action.comms.${label}.accessibility`,
    ...(feedback === 'local' ? { feedback: 'local' } : { authoritativeFeedback: true }),
    bindings: Object.freeze([keyboardBinding, gamepadBinding]),
  });
}

export const COMMS_HAIL_ACTION = action(
  COMMS_HAIL_ACTION_ID, 'hail', keyboard('KeyH'), gamepad('dpad', 'dpad-left'),
);
export const COMMS_SELECT_MESSAGE_ACTION = action(
  COMMS_SELECT_MESSAGE_ACTION_ID,
  'select_message',
  keyboard('KeyM'),
  gamepad('dpad', 'dpad-up'),
  'local',
);
export const COMMS_RESPOND_ACTION = action(
  COMMS_RESPOND_ACTION_ID, 'respond', keyboard('KeyR'), gamepad('button', 'face-bottom'),
);
export const COMMS_CLEAR_ACTION = action(
  COMMS_CLEAR_ACTION_ID,
  'clear',
  keyboard('KeyC', { shiftKey: true }),
  gamepad('dpad', 'dpad-down'),
);
export const COMMS_SHOW_ON_SCREEN_ACTION = action(
  COMMS_SHOW_ON_SCREEN_ACTION_ID,
  'show_on_screen',
  keyboard('KeyV'),
  gamepad('dpad', 'dpad-right'),
);

export const COMMS_ACTIONS = Object.freeze([
  COMMS_HAIL_ACTION,
  COMMS_SELECT_MESSAGE_ACTION,
  COMMS_RESPOND_ACTION,
  COMMS_CLEAR_ACTION,
  COMMS_SHOW_ON_SCREEN_ACTION,
]);

/** Resolve either the flat battleship payload or the cruiser's family slice. */
export function commsActionView(state) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, COMMS_ACTION_CONTEXT);
  return Object.keys(projected).length > 0 ? projected : state;
}

/** Keep semantic defaults identical to the shared Comms renderer's selection. */
export function currentCommsMessage(view) {
  const messages = Array.isArray(view && view.messages) ? view.messages : [];
  return [...messages].reverse().find((message) => isLatestLiveCriticalMessage(message, messages))
    || messages.find((message) => !message.is_read)
    || messages[messages.length - 1]
    || null;
}

function correlatedPayload(actionId, correlation, inputMs, payload) {
  if (typeof correlation !== 'string' || !correlation) return null;
  return {
    ...payload,
    correlation,
    semantic_action: actionId,
    __input_ms: inputMs,
  };
}

function contactId(contact) {
  // `uuid` is the wire identity. `id` remains a compatibility fallback for
  // older fixtures/payloads; it must never win over an authoritative UUID.
  return contact && (contact.uuid || contact.id || '');
}

/** Register the complete shipped Comms action family on an isolated registry. */
export function registerCommsActions(registry, options = {}) {
  if (!registry || typeof registry.register !== 'function') {
    throw new TypeError('comms action registration requires a registry');
  }
  const getState = typeof options.getState === 'function' ? options.getState : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;
  const selectMessage = typeof options.selectMessage === 'function'
    ? options.selectMessage : null;
  const selectThread = typeof options.selectThread === 'function'
    ? options.selectThread : null;
  const getCurrentMessage = typeof options.getCurrentMessage === 'function'
    ? options.getCurrentMessage : null;

  const activeMessage = (view) => {
    const selected = getCurrentMessage ? getCurrentMessage() : null;
    return selected || currentCommsMessage(view);
  };

  registry.register(COMMS_HAIL_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = commsActionView(getState());
    if (!view || !sendAction) return false;
    const targetUuid = detail && typeof detail.target_uuid === 'string'
      ? detail.target_uuid
      : (Array.isArray(view.contacts)
        ? contactId(view.contacts.find((contact) => contact && contact.in_range !== false))
        : '');
    if (!targetUuid) return false;
    const payload = correlatedPayload(actionId, correlation, inputMs, {
      target_uuid: targetUuid,
    });
    if (!payload) return false;
    sendAction('hail', payload);
    return true;
  });

  // One selection identity for both grains (issue #1380). The inbox lists
  // THREADS, so a HAILS row supplies `thread_id`; the parameter-free
  // keyboard/gamepad binding and any control naming an exact message still
  // supply `message_id`. Both are the same operation to the operator — "show
  // me that" — so they share one action, one key and one feedback lifecycle,
  // and the renderer keeps its two picks consistent underneath.
  registry.register(COMMS_SELECT_MESSAGE_ACTION, ({ detail, settleFeedback } = {}) => {
    const view = commsActionView(getState());
    if (!view) return false;
    const thread = detail && typeof detail.thread_id === 'string'
      ? detail.thread_id : null;
    const selected = detail && typeof detail.message_id === 'string'
      ? detail.message_id : null;
    // The renderer owns this local state. A parameter-free keyboard/gamepad
    // activation asks it to choose/cycle; a row supplies its exact id. Never
    // claim Applied for a missing/stale thread or message, or emit the
    // unconsumed legacy SelectCommsMessage host command.
    if (thread !== null) {
      if (!selectThread || selectThread(thread) !== true) return false;
    } else {
      if (!selectMessage) return false;
      if (selectMessage(selected) !== true) return false;
    }
    if (typeof settleFeedback === 'function') {
      settleFeedback(ACTION_FEEDBACK_STATE.APPLIED);
    }
    return true;
  });

  registry.register(COMMS_RESPOND_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = commsActionView(getState());
    if (!view || !sendAction) return false;
    const active = activeMessage(view);
    const explicitMessageId = detail && typeof detail.message_id === 'string'
      ? detail.message_id : null;
    const messageId = explicitMessageId || (active && active.id);
    let responseIndex = detail && Number.isInteger(detail.response_index)
      ? detail.response_index : null;
    if (responseIndex == null && active) {
      const responses = Array.isArray(active.responses) ? active.responses : [];
      responseIndex = responses.findIndex((response) => {
        const normalized = typeof response === 'string'
          ? { important: false, available: true }
          : response || {};
        return normalized.available !== false && normalized.important !== true;
      });
    }
    if (!messageId || responseIndex == null || responseIndex < 0) return false;

    // Pointer controls have already performed the two-click confirmation. A
    // forged activation naming a live important response must prove that same
    // confirmation; stale/unknown details still go to the host so its exact
    // authoritative refusal semantics remain observable.
    if (active && active.id === messageId) {
      const response = Array.isArray(active.responses) ? active.responses[responseIndex] : null;
      if (response && typeof response === 'object' && response.important === true
          && !(detail && detail.confirmed === true)) return false;
    }
    const payload = correlatedPayload(actionId, correlation, inputMs, {
      message_id: messageId,
      response_index: responseIndex,
    });
    if (!payload) return false;
    sendAction('respond_to_message', payload);
    return true;
  });

  registry.register(COMMS_CLEAR_ACTION, ({ actionId, correlation, inputMs } = {}) => {
    const payload = correlatedPayload(actionId, correlation, inputMs, {});
    if (!payload || !sendAction) return false;
    sendAction('clear_comms', payload);
    return true;
  });

  registry.register(COMMS_SHOW_ON_SCREEN_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = commsActionView(getState());
    if (!view || !sendAction) return false;
    const messageId = detail && typeof detail.message_id === 'string'
      ? detail.message_id : activeMessage(view)?.id;
    if (!messageId) return false;
    const payload = correlatedPayload(actionId, correlation, inputMs, { message_id: messageId });
    if (!payload) return false;
    sendAction('show_on_screen', payload);
    return true;
  });
  return registry;
}

export function createCommsActionRegistry(options = {}) {
  return registerCommsActions(createSemanticActionRegistry({
    actionFeedback: options.actionFeedback,
  }), options);
}

if (typeof window !== 'undefined') {
  window.createCommsActionRegistry = createCommsActionRegistry;
}
