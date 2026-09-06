/**
 * gui/comms-state.js — Pure JS port of src/client_comms.rs
 * (ClientCommsState + ThreadSummary helpers). Issue #460.
 *
 * `apply(msg)` takes an already-parsed ServerMessage `{ type, data }`.
 * Only the `CommsState` variant mutates state; all others are ignored.
 *
 * DOM-free; exposed on `window` as `window.commsState` (singleton).
 */

import {
  CHANGE_DOMAINS,
  REDUCER_EFFECTS,
  emptyReducerResult,
} from './reducer-result.js';

/**
 * Effective thread id for a message. Old wire payloads (pre-threading) have
 * `thread_id = ""` — treat those as their own thread (= message id).
 */
export function effectiveThreadId(msg) {
  return msg.thread_id ? msg.thread_id : msg.id;
}

/** Canonical client-side spellings of the authoritative wire priority. */
export const COMMS_PRIORITY = Object.freeze({
  ROUTINE: 'routine',
  URGENT: 'urgent',
  CRITICAL: 'critical',
});

/**
 * Normalise the Rust enum's wire spelling without coupling the pure client to
 * one serde casing. Unknown/missing values return null so the legacy boolean
 * can remain a decode fallback during a rolling upgrade.
 */
export function normalizeCommsPriority(value) {
  if (typeof value !== 'string') return null;
  const normalized = value.trim().toLowerCase();
  return Object.values(COMMS_PRIORITY).includes(normalized) ? normalized : null;
}

/**
 * Authoritative priority for one message. `is_urgent` is deliberately only a
 * compatibility fallback for payloads sent before CommsPriority existed.
 */
export function commsPriority(msg) {
  const priority = normalizeCommsPriority(msg && msg.priority);
  if (priority !== null) return priority;
  return msg && msg.is_urgent ? COMMS_PRIORITY.URGENT : COMMS_PRIORITY.ROUTINE;
}

/** A responded or invalidated dialogue no longer carries live importance. */
export function isLiveCommsMessage(msg) {
  return !!msg
    && (msg.selected_response === null || msg.selected_response === undefined)
    && !msg.is_orphaned;
}

/**
 * Priority of the latest live message in a thread. Looking only at the latest
 * message makes a newer hail an authoritative supersession; read/visit state
 * is intentionally absent, so Critical survives opening the thread.
 */
export function latestLiveThreadPriority(messages, threadId = null) {
  const raw = Array.isArray(messages) ? messages : [];
  const thread = threadId === null
    ? raw
    : raw.filter(m => effectiveThreadId(m) === threadId);
  const latest = thread[thread.length - 1];
  return isLiveCommsMessage(latest) ? commsPriority(latest) : COMMS_PRIORITY.ROUTINE;
}

/** True only for the latest, still-live Critical message in its thread. */
export function isLatestLiveCriticalMessage(msg, messages) {
  if (!msg) return false;
  const tid = effectiveThreadId(msg);
  const thread = (Array.isArray(messages) ? messages : [])
    .filter(candidate => effectiveThreadId(candidate) === tid);
  const latest = thread[thread.length - 1];
  return !!latest
    && latest.id === msg.id
    && latestLiveThreadPriority(thread) === COMMS_PRIORITY.CRITICAL;
}

/** Longest inbox/hail preview, in characters, before an ellipsis. */
export const COMMS_PREVIEW_CHARS = 64;

/**
 * A short, readable inbox/hail preview for a message.
 *
 * Derived from the RESOLVED body — `localiseTree` has already turned the body
 * id into words and applied `body_params` at the wire boundary, so a
 * parameterised body previews with its figures filled in. Falls back to the
 * (now equally resolvable) `subject` if a body is somehow absent. This is what
 * fixes the chopped-id preview: the old `subject` was the first forty
 * CHARACTERS OF THE ID, so any id past forty characters previewed as an
 * unresolvable fragment; here the source is real text, truncated on a word
 * boundary with an ellipsis.
 */
export function commsPreview(msg) {
  const text = String((msg && (msg.body || msg.subject)) || '').replace(/\s+/g, ' ').trim();
  if (text.length <= COMMS_PREVIEW_CHARS) return text;
  const cut = text.slice(0, COMMS_PREVIEW_CHARS);
  const lastSpace = cut.lastIndexOf(' ');
  const head = lastSpace > COMMS_PREVIEW_CHARS * 0.6 ? cut.slice(0, lastSpace) : cut;
  return head + '…';
}

// ── Pure thread grouping (issue #1380) ──────────────────────────────────────
// Grouping, history and previews are functions of the message list, not of the
// store: the live Comms renderer holds no `ClientCommsState` at all — it is
// handed a console payload ten times a second — so the rules below have to be
// callable without one. `ClientCommsState`'s own methods are thin delegates to
// them, which is what keeps the store and the console showing one inbox rather
// than two spellings of it.

/** All messages belonging to `threadId`, in inbox (chronological) order. */
export function threadMessagesFrom(messages, threadId) {
  return (Array.isArray(messages) ? messages : [])
    .filter(m => effectiveThreadId(m) === threadId);
}

/**
 * The active message of `threadId`: the LAST message in it that still has
 * pending responses (non-empty responses, no selected_response, not orphaned,
 * sender in range). Null when the thread has none — a conversation answered
 * right through is still readable, it just cannot be replied to.
 */
export function activeThreadMessageFrom(messages, threadId) {
  const msgs = threadMessagesFrom(messages, threadId);
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i];
    if ((m.responses || []).length > 0
        && (m.selected_response === null || m.selected_response === undefined)
        && !m.is_orphaned
        && m.sender_in_range !== false) {
      return m;
    }
  }
  return null;
}

/**
 * Thread summaries sorted for display: live Critical first, urgent+unread,
 * plain unread, then read. Relative order within each group is preserved
 * (stable sort, inbox order). Each thread appears once and its metadata
 * reflects the LATEST message in the thread — which is what makes a
 * five-message conversation one inbox row previewing its newest line.
 *
 * `contacts` names the CHANNEL rather than the last speaker, so a multi-speaker
 * thread (an outpost whose science officer answers for it) stays filed under
 * the outpost; a synthetic broadcast with no contact falls back to the speaker.
 */
export function sortedThreadsFrom(messages, contacts) {
  const raw = Array.isArray(messages) ? messages : [];
  const roster = Array.isArray(contacts) ? contacts : [];

  // Unique thread ids in first-seen order (preserves inbox order).
  const seen = [];
  for (const m of raw) {
    const tid = effectiveThreadId(m);
    if (!seen.includes(tid)) seen.push(tid);
  }

  const summaries = seen.map(tid => {
    const threadMsgs = threadMessagesFrom(raw, tid);
    const latest = threadMsgs[threadMsgs.length - 1];
    // Only a real sender identity can match a contact. Without the guard an
    // `undefined` sender_uuid matches an `undefined` contact uuid — a fixture
    // or a synthetic broadcast would then be filed under the first nameless
    // contact in the roster and lose its sender's name entirely.
    const contact = latest.sender_uuid
      ? roster.find(c => c.uuid === latest.sender_uuid)
      : undefined;
    const anyUnread = threadMsgs.some(m => !m.is_read);
    const latestPriority = latestLiveThreadPriority(threadMsgs);
    // Preserve legacy Urgent's unread lifecycle. A historical Critical is
    // not allowed to leak through this compatibility field after a newer
    // message supersedes it.
    const anyUrgent = latestPriority === COMMS_PRIORITY.CRITICAL
      || threadMsgs.some(m => commsPriority(m) === COMMS_PRIORITY.URGENT && !m.is_read);
    return {
      thread_id: tid,
      sender_name: (contact && contact.name) || latest.sender_name,
      subject: commsPreview(latest),
      message_count: threadMsgs.length,
      any_unread: anyUnread,
      any_urgent: anyUrgent,
      latest_priority: latestPriority,
      latest_out_of_range: latest.sender_in_range === false,
      latest_orphaned: !!latest.is_orphaned,
    };
  });

  const priority = s => (
    s.latest_priority === COMMS_PRIORITY.CRITICAL ? 0
      : s.any_urgent ? 1
        : s.any_unread ? 2 : 3
  );
  // Array.prototype.sort is stable, matching Rust's sort_by.
  summaries.sort((a, b) => priority(a) - priority(b));
  return summaries;
}

/**
 * How many of `threads` carry unread traffic — the number the HAILS tab shows.
 * Takes summaries rather than raw messages so the caller that already has the
 * projection does not group the inbox twice per render.
 */
export function unreadThreadCount(threads) {
  return (Array.isArray(threads) ? threads : []).filter(s => s && s.any_unread).length;
}

/**
 * The client's view of the Comms console state.
 * Mirrors `ClientCommsState` in src/client_comms.rs.
 */
export class ClientCommsState {
  constructor() {
    this.reset();
  }

  reset() {
    /** Inbox messages (CommsMessage), in server-determined order. */
    this.messages = [];
    /** Active objectives visible to the Comms operator. */
    this.objectives = [];
    /** Hailable contacts. */
    this.contacts = [];
    /** The thread the operator currently has open, or null. */
    this.selectedThreadId = null;
    /** Monotonically-increasing version, bumped on each state change. */
    this.version = 0;
    this._cleanVersion = 0;
  }

  /**
   * Apply a single inbound ServerMessage. Only CommsState is handled.
   * Mirrors `ClientCommsState::apply` and reports the semantic Comms change
   * plus its shell-render request from the reducer that owns the state.
   */
  apply(msg) {
    const changes = emptyReducerResult();
    if (!msg || msg.type !== 'CommsState') return changes;
    const d = msg.data || {};
    this.messages = d.messages || [];
    this.objectives = d.objectives || [];
    this.contacts = d.contacts || [];
    // Drop selected thread if no messages with that thread_id remain.
    if (this.selectedThreadId !== null
        && !this.messages.some(m => effectiveThreadId(m) === this.selectedThreadId)) {
      this.selectedThreadId = null;
    }
    this.version += 1;
    changes.changedDomains.add(CHANGE_DOMAINS.COMMS);
    changes.effects.push({ effect: REDUCER_EFFECTS.REQUEST_RENDER });
    return changes;
  }

  /** Open a thread in the chat view. No-op if the thread doesn't exist. */
  selectThread(threadId) {
    if (this.messages.some(m => effectiveThreadId(m) === threadId)) {
      this.selectedThreadId = threadId;
      this.version += 1;
    }
  }

  /** All messages belonging to `threadId`, in inbox (chronological) order. */
  threadMessages(threadId) {
    return threadMessagesFrom(this.messages, threadId);
  }

  /**
   * The active message for `threadId`: the LAST message in the thread that
   * still has pending responses (non-empty responses, no selected_response,
   * not orphaned, sender in range). Null when none.
   */
  activeMessageForThread(threadId) {
    return activeThreadMessageFrom(this.messages, threadId);
  }

  /**
   * Available response texts for `msg` — empty array if the operator has
   * already responded (selected_response set).
   */
  availableResponses(msg) {
    if (msg.selected_response !== null && msg.selected_response !== undefined) return [];
    return msg.responses || [];
  }

  /** True when the selected thread has an active message with pending responses. */
  responseButtonsEnabled() {
    if (this.selectedThreadId === null) return false;
    return this.activeMessageForThread(this.selectedThreadId) !== null;
  }

  /** True if a Hail click on `uuid` should produce an outbound message. */
  canHail(uuid) {
    return this.contacts.some(c => c.uuid === uuid && c.in_range !== false);
  }

  /** True if the state has changed since the last markClean(). */
  isDirty() {
    return this.version !== this._cleanVersion;
  }

  /** Mark the state as clean (no pending UI refresh needed). */
  markClean() {
    this._cleanVersion = this.version;
  }

  /** Clear the currently selected thread. */
  clearSelection() {
    if (this.selectedThreadId !== null) {
      this.selectedThreadId = null;
      this.version += 1;
    }
  }

  /**
   * Thread summaries sorted for display: live Critical first, urgent+unread,
   * plain unread, then read. Relative order within each group is preserved
   * (stable sort, inbox order). Each thread appears once; metadata reflects
   * the LATEST message in the thread. Mirrors `sorted_threads`.
   */
  sortedThreads() {
    return sortedThreadsFrom(this.messages, this.contacts);
  }
}

// ── Outbound ClientMessage builders ─────────────────────────────────────────
// Post-#822 (short-form shim retired): full ControlSystem envelopes targeting
// the `comms` system, matching gui/action-map.js.

export function hailMessage(targetUuid) {
  return {
    type: 'ControlSystem',
    data: { target: 'comms', payload: { type: 'Hail', data: { target_uuid: targetUuid } } },
  };
}

export function respondToMessage(messageId, responseIndex) {
  return {
    type: 'ControlSystem',
    data: {
      target: 'comms',
      payload: { type: 'RespondToMessage', data: { message_id: messageId, response_index: responseIndex } },
    },
  };
}

export function clearCommsMessage() {
  return {
    type: 'ControlSystem',
    data: { target: 'comms', payload: { type: 'ClearComms' } },
  };
}

/** Singleton used by client.html. */
export const commsState = new ClientCommsState();

if (typeof window !== 'undefined') {
  window.commsState = commsState;
}
