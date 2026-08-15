// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

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

export class PhCommsCurrentMessage extends HTMLElement {
  #state = null;
  #respCache = new Map();
  #prevThreadId = null;
  #placeholderEl = null;
  #threadEl = null;
  #senderEl = null;
  #messagesEl = null;
  #responsesEl = null;
  // Index of the important response currently armed (awaiting a confirm click),
  // or null when nothing is armed. Reset whenever the thread changes.
  #armedIdx = null;
  // Timestamp of the last rejection this element flashed, so a repeated
  // rejection for the same button re-triggers the animation.
  #lastRejectionTs = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .placeholder { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .thread { display: flex; flex-direction: column; gap: 0.5rem; }
    .sender-label { font-size: var(--text-xs); color: var(--edge); letter-spacing: 0.15em; text-transform: uppercase; padding-bottom: 0.25rem; border-bottom: 1px solid var(--line-faint); }
    .messages { display: flex; flex-direction: column; gap: 0.35rem; max-height: 10rem; overflow-y: auto; }
    .msg { font-size: var(--text-sm); line-height: 1.4; }
    .msg .speaker { font-weight: 700; color: var(--ink-dim); }
    .msg .text { color: var(--ink); }
    .responses { display: flex; flex-wrap: wrap; gap: 0.35rem; padding-top: 0.35rem; border-top: 1px solid var(--line-faint); }
    .resp-btn { background: var(--bg-card); border: 1px solid var(--line-faint); color: var(--ink); font-family: 'Chakra Petch', sans-serif; font-size: var(--text-xs); font-weight: 600; padding: 0.35rem 0.6rem; cursor: pointer; letter-spacing: 0.1em; text-transform: uppercase; transition: all 0.15s ease; }
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
      <div class="sender-label" id="sender-label"></div>
      <div class="messages" id="messages"><div class="msg"><span class="text"></span></div></div>
    </div>
  </div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const root = this.shadowRoot;
    const s = this.#state || {};
    const thread = s.thread;

    if (!this.#placeholderEl) this.#placeholderEl = root.getElementById('placeholder');
    if (!this.#threadEl) this.#threadEl = root.getElementById('thread');
    if (!this.#senderEl) this.#senderEl = root.getElementById('sender-label');
    if (!this.#messagesEl) this.#messagesEl = root.getElementById('messages');

    if (!thread) {
      this.#placeholderEl.style.display = '';
      this.#threadEl.style.display = 'none';
      this.#prevThreadId = null;
      return;
    }

    this.#placeholderEl.style.display = 'none';
    this.#threadEl.style.display = '';

    const tid = thread.id;
    const sender = thread.sender_name || '';
    const body = thread.body || '';
    const responses = (Array.isArray(thread.responses) ? thread.responses : [])
      .map(normalizeResponse);
    const selectedIdx = thread.selected_response;
    // Rejection targeting THIS thread (#761 AC3): the attempted control flashes.
    const rejection = s.rejection && s.rejection.message_id === tid ? s.rejection : null;

    this.#senderEl.textContent = sender;

    if (tid !== this.#prevThreadId) {
      this.#messagesEl.innerHTML = '<div class="msg"><span class="text"></span></div>';
      this.#respCache.clear();
      if (this.#responsesEl) { this.#responsesEl.remove(); this.#responsesEl = null; }
      this.#prevThreadId = tid;
      this.#armedIdx = null;
    }

    this.#messagesEl.firstChild.firstChild.textContent = body || '(empty)';

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
      this.#render();
      return;
    }
    // Non-important, or a confirmed important response: submit and disarm.
    this.#armedIdx = null;
    if (this.sendAction) {
      this.sendAction('respond_to_message', { message_id: tid, response_index: idx });
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-comms-current-message')) {
  customElements.define('ph-comms-current-message', PhCommsCurrentMessage);
}
