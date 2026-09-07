// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { isLatestLiveCriticalMessage } from '../comms-state.js';
import { PhElement, phDefine } from './ph-element.js';
import { COMMS_RESPOND_ACTION_ID } from '../stations/comms-actions.js';

/**
 * Normalise a wire response into `{ text, important, available }`.
 *
 * Post-#761 the server sends per-response objects; older payloads (and the
 * legacy captain/pilot overlays) may still pass bare strings. A bare string
 * is treated as an available, non-important response.
 */
function normalizeResponse(r) {
  if (typeof r === 'string') return { text: r, important: false, available: true };
  return {
    text: r && r.text != null ? r.text : '',
    important: !!(r && r.important),
    // Availability defaults to true when the field is absent (backward compat).
    available: !(r && r.available === false),
  };
}

/**
 * The open Comms thread: THE WHOLE CONVERSATION, responses pinned beneath
 * (issue #1380).
 *
 * `state.messages` is the thread in order, oldest first — what
 * `threadMessagesFrom` returns for the thread the inbox row opened. It scrolls
 * inside its own box, so a long exchange never scrolls the console out from
 * under the response buttons: those sit outside that box and stay on screen at
 * the bottom of the panel.
 *
 * `state.thread` is the ACTIVE message of that conversation — the one whose
 * responses are pinned and whose id a Respond names. `state.sender_name`, when
 * given, names the CHANNEL (the contact the thread belongs to) rather than the
 * last speaker; without it the header falls back to the active message's own
 * sender, which is what a single-message thread wants anyway.
 */
export class PhCommsCurrentMessage extends PhElement {
  #respCache = new Map();
  #msgCache = new Map();
  #prevThreadId = null;
  #prevConversationKey = null;
  #placeholderEl = null;
  #threadEl = null;
  #senderEl = null;
  #priorityEl = null;
  #messagesEl = null;
  #responsesEl = null;
  // Index of the important response currently armed (awaiting a confirm click),
  // or null when nothing is armed. Reset whenever the thread changes.
  #armedIdx = null;
  // Timestamp of the last rejection this element flashed, so a repeated
  // rejection for the same button re-triggers the animation.
  #lastRejectionTs = null;

  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; min-height: 0; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    #container { display: flex; flex-direction: column; flex: 1; min-height: 0; }
    .placeholder { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .thread { display: flex; flex-direction: column; gap: 0.5rem; flex: 1; min-height: 0; }
    .sender-label { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; flex-shrink: 0; font-size: var(--text-xs); color: var(--edge); letter-spacing: 0.15em; text-transform: uppercase; padding-bottom: 0.25rem; border-bottom: 1px solid var(--line-faint); }
    .priority-cue { display: inline-flex; align-items: center; gap: 0.25rem; flex-shrink: 0; border: 1px solid var(--fire-bright); background: var(--fire-deep); color: var(--fire-bright); padding: 0.12rem 0.35rem; font-weight: 700; letter-spacing: 0.1em; }
    .priority-cue[hidden] { display: none; }
    .priority-shape { line-height: 1; }
    /* The conversation scrolls in HERE and nowhere else (issue #1380): the
       history takes the panel's leftover height and the responses below it
       are outside the scroller, so a long exchange never pushes them off the
       console — the console itself never scrolls. Where the panel has no
       definite height of its own (a fixture, a plain document flow) the
       max-height still bounds the box. */
    .messages { display: flex; flex-direction: column; gap: 0.35rem; flex: 1; min-height: 0; max-height: 100%; overflow-y: auto; }
    .msg { font-size: var(--text-sm); line-height: 1.4; }
    .msg .speaker { font-weight: 700; color: var(--ink-dim); margin-right: 0.4rem; }
    .msg .speaker:empty { display: none; }
    .msg .text { color: var(--ink); }
    .responses { display: flex; flex-wrap: wrap; gap: 0.35rem; flex-shrink: 0; padding-top: 0.35rem; border-top: 1px solid var(--line-faint); }
    .resp-btn { background: var(--bg-card); border: 1px solid var(--line-faint); color: var(--ink); font-family: 'Chakra Petch', sans-serif; font-size: var(--text-xs); font-weight: 600; padding: 0.35rem 0.6rem; cursor: pointer; letter-spacing: 0.1em; text-transform: uppercase; transition: all 0.15s ease; min-height: var(--control-hit-min); }
    .resp-btn:hover:not(:disabled) { background: var(--cyan-deep); border-color: var(--edge); }
    .resp-btn:disabled { opacity: 0.35; cursor: default; }
    /* Unavailable (sender out of range): visible but greyed and disabled,
       mirroring ph-comms-contact-list's .out-of-range. */
    .resp-btn.unavailable { opacity: 0.45; }
    /* An armed important response awaiting a confirm click. */
    .resp-btn.important { border-color: var(--reloading); color: var(--gold-bright); }
    /* Red flash when the host rejects an attempted submission (#761 AC3). */
    .resp-btn.rejected { animation: resp-reject-flash 0.6s ease; }
    @keyframes resp-reject-flash {
      0%   { background: var(--tactical-deep); border-color: var(--fire); color: var(--tactical-bright); }
      100% { background: var(--bg-card); border-color: var(--line-faint); color: var(--ink); }
    }
  </style>
  <div id="container">
    <div class="placeholder" id="placeholder">${t('component.comms_message.no_active_hail')}</div>
    <div class="thread" id="thread" style="display:none">
      <div class="sender-label"><span id="sender-label"></span><span class="priority-cue" id="priority-cue" hidden></span></div>
      <div class="messages" id="messages"></div>
    </div>
  </div>
`;
  }

  render(state) {
    const root = this.shadowRoot;
    const s = state || {};
    const thread = s.thread;

    if (!this.#placeholderEl) this.#placeholderEl = root.getElementById('placeholder');
    if (!this.#threadEl) this.#threadEl = root.getElementById('thread');
    if (!this.#senderEl) this.#senderEl = root.getElementById('sender-label');
    if (!this.#priorityEl) this.#priorityEl = root.getElementById('priority-cue');
    if (!this.#messagesEl) this.#messagesEl = root.getElementById('messages');

    if (!thread) {
      this.#placeholderEl.style.display = '';
      this.#threadEl.style.display = 'none';
      this.#prevThreadId = null;
      this.#prevConversationKey = null;
      return;
    }

    this.#placeholderEl.style.display = 'none';
    this.#threadEl.style.display = '';

    const tid = thread.id;
    const responses = (Array.isArray(thread.responses) ? thread.responses : [])
      .map(normalizeResponse);
    const selectedIdx = thread.selected_response;
    // The whole conversation, oldest first. A caller with only the one active
    // message (a fixture, or a surface that has not grouped its inbox) still
    // gets a one-message thread rather than an empty box.
    const messages = Array.isArray(s.messages) && s.messages.length > 0
      ? s.messages : [thread];
    // The channel's name when the projection supplied one, else this message's.
    const sender = (typeof s.sender_name === 'string' && s.sender_name)
      || thread.sender_name || '';
    const critical = isLatestLiveCriticalMessage(thread, messages);
    // Rejection targeting THIS thread (#761 AC3): the attempted control flashes.
    const rejection = s.rejection && s.rejection.message_id === tid ? s.rejection : null;

    this.#senderEl.textContent = sender;
    this.#priorityEl.hidden = !critical;
    this.#priorityEl.replaceChildren();
    if (critical) {
      const shape = document.createElement('span');
      shape.className = 'priority-shape';
      shape.setAttribute('aria-hidden', 'true');
      shape.textContent = '◆';
      const text = document.createElement('span');
      text.textContent = t('component.comms.priority.critical');
      this.#priorityEl.append(shape, text);
    }

    if (tid !== this.#prevThreadId) {
      this.#respCache.clear();
      if (this.#responsesEl) { this.#responsesEl.remove(); this.#responsesEl = null; }
      this.#prevThreadId = tid;
      this.#armedIdx = null;
    }

    // ── The conversation, in order ───────────────────────────────────────
    // Rows are cached by message id and moved ONLY where the order actually
    // differs from the projection, so a thread that gains a reply grows by one
    // node instead of being rebuilt — the scroll position of the exchange the
    // operator is reading, and any text they have selected inside a hail,
    // survive the ten-times-a-second repaint. (`appendChild` on a node that is
    // already a child is a remove + re-insert, so appending unconditionally
    // would tear the whole conversation out every 100ms; the sibling
    // ph-comms-hail-list carries the same index compare for the same reason.)
    const conversationKey = messages.map((m) => (m && m.id) || '').join(' ');
    const liveMessages = new Set(messages.map((m) => (m && m.id) || ''));
    for (const [key, node] of this.#msgCache) {
      if (!liveMessages.has(key)) { node.remove(); this.#msgCache.delete(key); }
    }
    const multiSpeaker = messages.length > 1;
    messages.forEach((m, index) => {
      const key = (m && m.id) || '';
      let node = this.#msgCache.get(key);
      if (!node) {
        node = document.createElement('div');
        node.className = 'msg';
        node.innerHTML = '<span class="speaker"></span><span class="text"></span>';
        this.#msgCache.set(key, node);
      }
      // The order has to follow the projection, but ONLY where it differs:
      // dead nodes were pruned above, so `children` holds message rows alone
      // and an index-for-index comparison is exact. An unchanged thread
      // moves nothing.
      const at = this.#messagesEl.children[index];
      if (at !== node) this.#messagesEl.insertBefore(node, at || null);
      // Who spoke matters only once a thread has more than one line in it; on
      // a single message the header above already says it.
      node.querySelector('.speaker').textContent = multiSpeaker
        ? ((m && m.sender_name) || '') : '';
      node.querySelector('.text').textContent = (m && m.body) || '(empty)';
    });

    if (responses.length === 0) {
      if (this.#responsesEl) { this.#responsesEl.style.display = 'none'; }
    } else {
      if (!this.#responsesEl) {
        this.#responsesEl = document.createElement('div');
        this.#responsesEl.className = 'responses';
        this.#threadEl.appendChild(this.#responsesEl);
      }
      this.#responsesEl.style.display = '';

      const live = new Set(responses.map((_, i) => String(i)));
      for (const [key, btn] of this.#respCache) {
        if (!live.has(key)) { btn.remove(); this.#respCache.delete(key); }
      }

      responses.forEach((r, idx) => {
        const key = String(idx);
        const chosen = selectedIdx != null && idx === selectedIdx;
        let btn = this.#respCache.get(key);
        if (!btn) {
          btn = document.createElement('button');
          btn.className = 'resp-btn';
          btn.dataset.idx = key;
          btn.addEventListener('click', () => this.#onResponseClick(btn, tid));
          this.#respCache.set(key, btn);
          this.#responsesEl.appendChild(btn);
        }
        // Stash per-response flags on the element so the (persistent) click
        // handler always reads the current render's values.
        btn.dataset.important = r.important ? 'true' : 'false';
        btn.dataset.available = r.available ? 'true' : 'false';

        const armed = this.#armedIdx === idx;
        // A greyed unavailable response is disabled; a chosen one is disabled.
        btn.disabled = chosen || !r.available;
        btn.classList.toggle('unavailable', !r.available && !chosen);
        btn.classList.toggle('important', r.important && armed && r.available && !chosen);

        let label = r.text;
        if (chosen) {
          label = '\u2713 ' + r.text;
          btn.removeAttribute('title');
        } else if (!r.available) {
          btn.title = t('component.comms_message.unavailable');
        } else if (armed) {
          label = t('component.comms_message.confirm_important');
          btn.removeAttribute('title');
        } else {
          btn.removeAttribute('title');
        }
        btn.textContent = label;
      });

      // Apply the red-flash to the attempted control. Re-trigger on a fresh
      // rejection even when the button element is reused across renders.
      if (rejection && rejection.ts !== this.#lastRejectionTs) {
        const btn = this.#respCache.get(String(rejection.response_index));
        if (btn) {
          btn.title = t('component.comms_message.rejected');
          btn.classList.remove('rejected');
          // Force reflow so re-adding the class restarts the animation.
          void btn.offsetWidth;
          btn.classList.add('rejected');
        }
        this.#lastRejectionTs = rejection.ts;
      }
    }

    // Open on the newest line: a conversation is read from its latest turn.
    //
    // Deliberately the LAST thing render() does, and deliberately gated on the
    // box having layout:
    //   * the responses row above may only just have been created, and it
    //     takes height out of the scroller — measuring before it exists clamps
    //     scrollTop short and cuts off the very message whose responses are
    //     pinned beneath it;
    //   * on a phone the thread opens as an OVERLAY, so the render that first
    //     fills it happens while the panel is still `display:none` and
    //     scrollHeight is 0. Recording the key there would spend the one shot
    //     this conversation gets on a box that cannot scroll, and the operator
    //     would open the thread at its oldest line. Unshown renders therefore
    //     leave the key alone, and the first render that is actually visible
    //     performs the scroll.
    // The key is tested FIRST so the short-circuit keeps the steady-state
    // repaint from reading clientHeight at all: that read forces a layout
    // flush, and this component repaints ten times a second.
    if (conversationKey !== this.#prevConversationKey
        && this.#messagesEl.clientHeight > 0) {
      this.#prevConversationKey = conversationKey;
      this.#messagesEl.scrollTop = this.#messagesEl.scrollHeight;
    }
  }

  /**
   * Handle a click on a response button. Non-important responses submit
   * immediately (unchanged behaviour). An important response arms on the first
   * click (showing a confirm prompt) and submits on the second \u2014 a two-step
   * confirm so exceptional irreversible choices are not committed accidentally
   * (#761 AC1). Unavailable responses never submit.
   */
  #onResponseClick(btn, tid) {
    if (btn.dataset.available === 'false') return; // greyed: never submit
    const idx = Number(btn.dataset.idx);
    const important = btn.dataset.important === 'true';
    if (important && this.#armedIdx !== idx) {
      // First click on an important response: arm and re-render to show the
      // confirm prompt. Nothing is sent yet.
      this.#armedIdx = idx;
      this.render(this.state);
      return;
    }
    // Non-important, or a confirmed important response: submit and disarm.
    this.#armedIdx = null;
    if (typeof window.activateSemanticAction === 'function') {
      window.activateSemanticAction(COMMS_RESPOND_ACTION_ID, {
        source: 'control',
        detail: { message_id: tid, response_index: idx, confirmed: important },
      });
    }
  }
}

phDefine('ph-comms-current-message', PhCommsCurrentMessage);
