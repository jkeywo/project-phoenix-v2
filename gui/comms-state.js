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
   * Thread summaries sorted for display: urgent+unread first, then plain
   * unread, then read. Relative order within each group preserved (stable
   * sort, inbox order). Each thread appears once; metadata reflects the
   * LATEST message in the thread. Mirrors `sorted_threads`.
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
      const anyUnread = threadMsgs.some(m => !m.is_read);
      const anyUrgent = threadMsgs.some(m => m.is_urgent && !m.is_read);
      return {
        thread_id: tid,
        sender_name: latest.sender_name,
        subject: latest.subject,
        any_unread: anyUnread,
        any_urgent: anyUrgent,
        latest_out_of_range: latest.sender_in_range === false,
        latest_orphaned: !!latest.is_orphaned,
      };
    });

    const priority = s => (s.any_urgent ? 0 : s.any_unread ? 1 : 2);
    // Array.prototype.sort is stable, matching Rust's sort_by.
    summaries.sort((a, b) => priority(a) - priority(b));
    return summaries;
  }
}

// ── Outbound ClientMessage builders ─────────────────────────────────────────

export function hailMessage(targetUuid) {
  return { type: 'Hail', data: { target_uuid: targetUuid } };
}

export function selectCommsMessage(messageId) {
  return { type: 'SelectCommsMessage', data: { message_id: messageId } };
}

export function respondToMessage(messageId, responseIndex) {
  return { type: 'RespondToMessage', data: { message_id: messageId, response_index: responseIndex } };
}

export function clearCommsMessage() {
  return { type: 'ClearComms' };
}

/** Singleton used by client.html. */
export const commsState = new ClientCommsState();

if (typeof window !== 'undefined') {
  window.commsState = commsState;
}
