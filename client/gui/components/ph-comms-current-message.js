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

    const sender = thread.sender || '';
    const messages = Array.isArray(thread.messages) ? thread.messages : [];
    const responses = Array.isArray(thread.responses) ? thread.responses : [];

    const msgsHtml = messages.map(m => {
      const speaker = m.speaker || '';
      const text = m.text || '';
      return `<div class="msg"><span class="speaker">${speaker}:</span> <span class="text">${text}</span></div>`;
    }).join('');

    const respHtml = responses.map(r => {
      const id = r.id || '';
      const text = r.text || '';
      const available = r.available !== false;
      return `<button class="resp-btn" data-id="${id}"${available ? '' : ' disabled'}>${text}</button>`;
    }).join('');

    container.innerHTML = `
      <div class="thread">
        <div class="sender-label">${sender}</div>
        <div class="messages">${msgsHtml || '<div class="msg" style="color:#6a7178">(no messages)</div>'}</div>
        <div class="responses">${respHtml}</div>
      </div>
    `;

    container.querySelectorAll('.resp-btn').forEach(btn => {
      if (!btn.disabled) {
        btn.addEventListener('click', () => {
          if (this.sendAction) {
            this.sendAction('respond', { response_id: btn.dataset.id });
          }
        });
      }
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-comms-current-message')) {
  customElements.define('ph-comms-current-message', PhCommsCurrentMessage);
}
