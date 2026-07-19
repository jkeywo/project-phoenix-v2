// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';

export class PhCommsCurrentMessage extends HTMLElement {
  #state = null;
  #respCache = new Map();
  #prevThreadId = null;
  #placeholderEl = null;
  #threadEl = null;
  #senderEl = null;
  #messagesEl = null;
  #responsesEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .placeholder { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .thread { display: flex; flex-direction: column; gap: 0.5rem; }
    .sender-label { font-size: 0.6rem; color: #4a5060; letter-spacing: 0.15em; text-transform: uppercase; padding-bottom: 0.25rem; border-bottom: 1px solid var(--line-faint); }
    .messages { display: flex; flex-direction: column; gap: 0.35rem; max-height: 10rem; overflow-y: auto; }
    .msg { font-size: 0.7rem; line-height: 1.4; }
    .msg .speaker { font-weight: 700; color: #8ab; }
    .msg .text { color: var(--ink); }
    .responses { display: flex; flex-wrap: wrap; gap: 0.35rem; padding-top: 0.35rem; border-top: 1px solid var(--line-faint); }
    .resp-btn { background: var(--bg-card); border: 1px solid var(--line-faint); color: var(--ink); font-family: 'Chakra Petch', sans-serif; font-size: 0.65rem; font-weight: 600; padding: 0.35rem 0.6rem; cursor: pointer; letter-spacing: 0.1em; text-transform: uppercase; transition: all 0.15s ease; }
    .resp-btn:hover:not(:disabled) { background: #161b24; border-color: #4a5060; }
    .resp-btn:disabled { opacity: 0.35; cursor: default; }
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
    const responses = Array.isArray(thread.responses) ? thread.responses : [];
    const selectedIdx = thread.selected_response;

    this.#senderEl.textContent = sender;

    if (tid !== this.#prevThreadId) {
      this.#messagesEl.innerHTML = '<div class="msg"><span class="text"></span></div>';
      this.#respCache.clear();
      if (this.#responsesEl) { this.#responsesEl.remove(); this.#responsesEl = null; }
      this.#prevThreadId = tid;
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

      responses.forEach((text, idx) => {
        const key = String(idx);
        const chosen = selectedIdx != null && idx === selectedIdx;
        let btn = this.#respCache.get(key);
        if (!btn) {
          btn = document.createElement('button');
          btn.className = 'resp-btn';
          btn.dataset.idx = key;
          btn.addEventListener('click', () => {
            if (this.sendAction) {
              this.sendAction('respond_to_message', { message_id: tid, response_index: Number(btn.dataset.idx) });
            }
          });
          this.#respCache.set(key, btn);
          this.#responsesEl.appendChild(btn);
        }
        btn.disabled = chosen;
        btn.textContent = chosen ? '\u2713 ' + text : text;
      });
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-comms-current-message')) {
  customElements.define('ph-comms-current-message', PhCommsCurrentMessage);
}
