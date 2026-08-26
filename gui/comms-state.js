/**
 * gui/comms-state.js — Pure JS port of src/client_comms.rs
 * (ClientCommsState + ThreadSummary helpers). Issue #460.
 *
 * `apply(msg)` takes an already-parsed ServerMessage `{ type, data }`.
 * Only the `CommsState` variant mutates state; all others are ignored.
 *
 * DOM-free; exposed on `window` as `window.commsState` (singleton).
 */

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
   * Mirrors `ClientCommsState::apply`.
   */
  apply(msg) {
    if (!msg || msg.type !== 'CommsState') return;
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
    return this.messages.filter(m => effectiveThreadId(m) === threadId);
  }

  /**
   * The active message for `threadId`: the LAST message in the thread that
   * still has pending responses (non-empty responses, no selected_response,
   * not orphaned, sender in range). Null when none.
   */
  activeMessageForThread(threadId) {
    const msgs = this.threadMessages(threadId);
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
    // Unique thread ids in first-seen order (preserves inbox order).
    const seen = [];
    for (const m of this.messages) {
      const tid = effectiveThreadId(m);
      if (!seen.includes(tid)) seen.push(tid);
    }

    const summaries = seen.map(tid => {
      const threadMsgs = this.threadMessages(tid);
      const latest = threadMsgs[threadMsgs.length - 1];
      const contact = this.contacts.find(c => c.uuid === latest.sender_uuid);
      const anyUnread = threadMsgs.some(m => !m.is_read);
      const latestPriority = latestLiveThreadPriority(threadMsgs);
      // Preserve legacy Urgent's unread lifecycle. A historical Critical is
      // not allowed to leak through this compatibility field after a newer
      // message supersedes it.
      const anyUrgent = latestPriority === COMMS_PRIORITY.CRITICAL
        || threadMsgs.some(m => commsPriority(m) === COMMS_PRIORITY.URGENT && !m.is_read);
      return {
        thread_id: tid,
        sender_name: contact ? contact.name : latest.sender_name,
        subject: commsPreview(latest),
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

export function selectCommsMessage(messageId) {
  return {
    type: 'ControlSystem',
    data: { target: 'comms', payload: { type: 'SelectCommsMessage', data: { message_id: messageId } } },
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
