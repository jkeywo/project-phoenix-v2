/**
 * Shared semantic action adapters for every shipped Comms control.
 *
 * These adapters keep input identity and feedback above the legacy action-map
 * seam. They deliberately send the same Comms commands as the visible
 * controls did before #1283; reachability, dialogue and response validation
 * remain authoritative in the existing Rust consumers.
 */

import { defineStationAction, createCorrelatedActionSender, keyboard, gamepad } from './action-support.js';

import { ACTION_FEEDBACK_STATE } from '../action-feedback.js';
import { isLatestLiveCriticalMessage } from '../comms-state.js';
import { visibleCommsMessages, structuredComms, normalizeCommsRequest, mediaCallOrigin } from '../comms-requests.js';
import { familyView } from '../console-payload.js';
import { createSemanticActionRegistry } from '../semantic-action-registry.js';

export const COMMS_ACTION_CONTEXT = 'comms';
export const COMMS_HAIL_ACTION_ID = 'comms.hail';
export const COMMS_SELECT_MESSAGE_ACTION_ID = 'comms.select-message';
export const COMMS_RESPOND_ACTION_ID = 'comms.respond';
export const COMMS_CLEAR_ACTION_ID = 'comms.clear';
export const COMMS_SHOW_ON_SCREEN_ACTION_ID = 'comms.show-on-screen';
export const COMMS_SEND_TEXT_ACTION_ID = 'comms.send-text';
export const COMMS_SEND_REQUEST_ACTION_ID = 'comms.send-request';
export const COMMS_MARK_READ_ACTION_ID = 'comms.mark-read';
export const COMMS_SELECT_MEDIA_ACTION_ID = 'comms.select-media';

function action(id, label, keyboardBinding, gamepadBinding, feedback = 'authoritative') {
  return defineStationAction({ id, contexts: [COMMS_ACTION_CONTEXT], labelKey: `comms.${label}`,
    bindings: [keyboardBinding, gamepadBinding], feedback });
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
  ...[[COMMS_SEND_REQUEST_ACTION_ID, 'request.report_status'], [COMMS_MARK_READ_ACTION_ID, 'request.mark_read'],
    [COMMS_SELECT_MEDIA_ACTION_ID, 'media.select']]
    .map(([id, label]) => defineStationAction({id, contexts:[COMMS_ACTION_CONTEXT], labelKey:`comms.${label}`, bindings:[null,null], feedback:'authoritative'})),
  COMMS_SELECT_MESSAGE_ACTION,
  COMMS_RESPOND_ACTION,
  COMMS_CLEAR_ACTION,
  COMMS_SHOW_ON_SCREEN_ACTION,
  action(COMMS_SEND_TEXT_ACTION_ID, 'send_text', keyboard('Enter', { ctrlKey: true }), gamepad('button', 'face-top')),
]);

/** Resolve either the flat battleship payload or the cruiser's family slice. */
export function commsActionView(state) {
  if (!state || typeof state !== 'object') return null;
  const projected = familyView(state, COMMS_ACTION_CONTEXT);
  return Object.keys(projected).length > 0 ? projected : state;
}

/** Keep semantic defaults identical to the shared Comms renderer's selection. */
export function currentCommsMessage(view) {
  const messages = visibleCommsMessages(view?.messages);
  return [...messages].reverse().find((message) => isLatestLiveCriticalMessage(message, messages))
    || messages.find((message) => !message.is_read)
    || messages[messages.length - 1]
    || null;
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
  const getRadioDraft = typeof options.getRadioDraft === 'function' ? options.getRadioDraft : () => null;
  const sendAction = typeof options.sendAction === 'function' ? options.sendAction : null;
  const send = createCorrelatedActionSender(sendAction);
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
  registry.register(COMMS_ACTIONS.find(entry => entry.id === COMMS_SELECT_MEDIA_ACTION_ID),
    ({actionId, correlation, inputMs, detail} = {}) => {
      const view = commsActionView(getState());
      if (!view || view.comms_auto || !sendAction) return false;
      const origin = detail?.origin_id === null ? null : detail?.origin_id ?? mediaCallOrigin(activeMessage(view));
      if (origin === null && detail?.origin_id !== null) return false;
      if (origin !== null && !(view.messages || []).some(message => mediaCallOrigin(message) === origin)) return false;
      return send(actionId, correlation, inputMs, 'set_comms_media_call', {origin_id: origin});
    });

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
    return send(actionId, correlation, inputMs, 'hail', {
      target_uuid: targetUuid,
    });
  });

  registry.register(COMMS_ACTIONS.find(entry => entry.id === COMMS_SEND_TEXT_ACTION_ID), ({ actionId, correlation, inputMs, detail } = {}) => {
    const view = commsActionView(getState());
    const draft = detail ?? getRadioDraft();
    if (!view || view.comms_auto || !sendAction || !draft
        || !(view.channels || []).some(channel => channel.id === draft.channel)
        || typeof draft.body !== 'string' || !draft.body.trim()
        || new TextEncoder().encode(draft.body).length > 4096
        || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u.test(draft.body)) return false;
    const audience = draft.audience;
    if (!audience || !['all', 'fleet', 'contact'].includes(audience.kind)) return false;
    if (audience.kind === 'contact' && !(view.contacts || []).some(contact => contactId(contact) === audience.target_uuid)) return false;
    return send(actionId, correlation, inputMs, 'send_comms_text', {
      channel: draft.channel,
      audience: audience.kind === 'contact' ? {kind: 'contact', target_uuid: audience.target_uuid} : {kind: audience.kind},
      body: draft.body,
    });
  });

  registry.register(COMMS_ACTIONS.find(entry => entry.id === COMMS_SEND_REQUEST_ACTION_ID), ({actionId, correlation, inputMs, detail} = {}) => {
    const view = commsActionView(getState());
    const draft = detail ?? getRadioDraft();
    const request = normalizeCommsRequest(draft?.request);
    if (!view || view.comms_auto || !sendAction || !draft || !request
      || !(view.channels || []).some(channel => channel.id === draft.channel)) return false;
    const audience = draft.audience;
    if (!audience || !['all', 'fleet', 'contact'].includes(audience.kind) || (typeof request === 'object' && audience.kind !== 'contact')) return false;
    if (audience.kind === 'contact' && !(view.contacts || []).some(contact => contactId(contact) === audience.target_uuid)
      && !(view.messages || []).some(message => message.sender_uuid === audience.target_uuid && message.transmission?.received_tick != null)) return false;
    if (draft.planned_request !== undefined) {
      if (typeof draft.planned_request !== 'string' || new TextEncoder().encode(draft.planned_request).length > 2048
        || !draft.planned_request.trim() || /[\u0000-\u001f\u007f-\u009f]/u.test(draft.planned_request)
        || audience.kind !== 'contact' || !(view.contacts || []).some(contact => contactId(contact) === audience.target_uuid)
        || !(view.planned_requests || []).some(row => row.id === draft.planned_request
          && row.recipient_uuid === audience.target_uuid && row.channel === draft.channel && row.request === draft.request)) return false;
    }
    return send(actionId, correlation, inputMs, 'send_comms_request', {
      channel:draft.channel, audience:audience.kind === 'contact' ? {kind:'contact',target_uuid:audience.target_uuid} : {kind:audience.kind}, request,
      ...(draft.planned_request !== undefined ? {planned_request:draft.planned_request} : {}),
    });
  });
  registry.register(COMMS_ACTIONS.find(entry => entry.id === COMMS_MARK_READ_ACTION_ID), ({actionId, correlation, inputMs, detail} = {}) => {
    const view = commsActionView(getState());
    const message = (view?.messages || []).find(message => message.id === detail?.message_id);
    if (!view || view.comms_auto || !sendAction || !message || !structuredComms(message)
      || message.transmission.received_tick == null || message.is_read) return false;
    return send(actionId, correlation, inputMs, 'mark_comms_read', {message_id:message.id});
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
    const addressed = (view.messages || []).find(message => message.id === messageId)
      || (active?.id === messageId ? active : null);
    if (view.comms_auto && structuredComms(addressed)) return false;
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
    return send(actionId, correlation, inputMs, 'respond_to_message', {
      message_id: messageId,
      response_index: responseIndex,
    });
  });

  registry.register(COMMS_CLEAR_ACTION, ({ actionId, correlation, inputMs } = {}) => {
    return send(actionId, correlation, inputMs, 'clear_comms', {});
  });

  registry.register(COMMS_SHOW_ON_SCREEN_ACTION, ({
    actionId, correlation, inputMs, detail,
  } = {}) => {
    const view = commsActionView(getState());
    if (!view || !sendAction) return false;
    const messageId = detail && typeof detail.message_id === 'string'
      ? detail.message_id : activeMessage(view)?.id;
    if (!messageId) return false;
    return send(actionId, correlation, inputMs, 'show_on_screen', { message_id: messageId });
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
