export class PhCommsCurrentMessage extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .placeholder { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .thread { display: flex; flex-direction: column; gap: 0.5rem; }
    .sender-label { font-size: 0.6rem; color: #4a5060; letter-spacing: 0.15em; text-transform: uppercase; padding-bottom: 0.25rem; border-bottom: 1px solid #282c38; }
    .messages { display: flex; flex-direction: column; gap: 0.35rem; max-height: 10rem; overflow-y: auto; }
    .msg { font-size: 0.7rem; line-height: 1.4; }
    .msg .speaker { font-weight: 700; color: #8ab; }
    .msg .text { color: #cce; }
    .responses { display: flex; flex-wrap: wrap; gap: 0.35rem; padding-top: 0.35rem; border-top: 1px solid #282c38; }
    .resp-btn { background: #0e1117; border: 1px solid #282c38; color: #cce; font-family: 'Chakra Petch', sans-serif; font-size: 0.65rem; font-weight: 600; padding: 0.35rem 0.6rem; cursor: pointer; letter-spacing: 0.1em; text-transform: uppercase; transition: all 0.15s ease; }
    .resp-btn:hover:not(:disabled) { background: #161b24; border-color: #4a5060; }
    .resp-btn:disabled { opacity: 0.35; cursor: default; }
  </style>
  <div id="container"></div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
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
    const s = this.#state || {};
    const thread = s.thread;
    const container = this.shadowRoot.getElementById('container');

    if (!thread) {
      container.innerHTML = '<div class="placeholder">NO ACTIVE HAIL</div>';
      return;
    }

    const sender = thread.sender_name || '';
    const body = thread.body || '';
    const responses = Array.isArray(thread.responses) ? thread.responses : [];
    const selectedIdx = thread.selected_response;

    // Single message body displayed as the current message text.
    const bodyHtml = body
      ? `<div class="msg"><span class="text">${body}</span></div>`
      : '<div class="msg" style="color:#6a7178">(empty)</div>';

    // Responses are plain strings from Vec<String>.
    const respHtml = responses.map((text, idx) => {
      const chosen = selectedIdx != null && idx === selectedIdx;
      return `<button class="resp-btn" data-idx="${idx}"${chosen ? ' disabled' : ''}>${chosen ? '\u2713 ' : ''}${text}</button>`;
    }).join('');

    container.innerHTML = `
      <div class="thread">
        <div class="sender-label">${sender}</div>
        <div class="messages">${bodyHtml}</div>
        ${respHtml ? `<div class="responses">${respHtml}</div>` : ''}
      </div>
    `;

    container.querySelectorAll('.resp-btn:not([disabled])').forEach(btn => {
      btn.addEventListener('click', () => {
        if (this.sendAction) {
          // Use the thread message id as the target for respond action.
          this.sendAction('respond_to_message', { message_id: thread.id, response_index: Number(btn.dataset.idx) });
        }
      });
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-comms-current-message')) {
  customElements.define('ph-comms-current-message', PhCommsCurrentMessage);
}
