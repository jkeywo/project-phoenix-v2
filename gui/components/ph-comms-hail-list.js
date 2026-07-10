export class PhCommsHailList extends HTMLElement {
  #state = null;
  #rowCache = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.25rem; }
    .empty { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .row { display: flex; align-items: center; gap: 0.4rem; font-size: 0.7rem; padding: 0.35rem 0.4rem; cursor: pointer; border-radius: 2px; transition: background 0.15s ease; }
    .row:hover { background: #161b24; }
    .dot { width: 0.45rem; height: 0.45rem; border-radius: 50%; flex-shrink: 0; }
    .dot.unread { background: #4a8fd4; }
    .dot.read { background: transparent; }
    .sender { font-weight: 400; color: #cce; min-width: 4rem; }
    .sender.unread { font-weight: 700; }
    .preview { color: #6a7178; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: 1; min-width: 0; }
    .timestamp { color: #4a5060; font-size: 0.6rem; flex-shrink: 0; }
  </style>
  <div class="list" id="list"></div>
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
    const raw = Array.isArray(s.messages) ? s.messages : [];
    const list = this.shadowRoot.getElementById('list');

    const live = new Set(raw.map(t => t.id || ''));
    for (const [key, el] of this.#rowCache) {
      if (!live.has(key)) { el.remove(); this.#rowCache.delete(key); }
    }

    if (raw.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = 'NO MESSAGES'; list.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    raw.forEach(t => {
      const id = t.id || '';
      const sender = t.sender_name || '';
      const preview = t.subject || '';
      const unread = !t.is_read;
      let row = this.#rowCache.get(id);
      if (!row) {
        row = document.createElement('div');
        row.className = 'row';
        row.innerHTML = '<span class="dot"></span><span class="sender"></span><span class="preview"></span>';
        row.addEventListener('click', () => {
          if (this.sendAction) {
            this.sendAction('select_comms_message', { message_id: id });
          }
        });
        this.#rowCache.set(id, row);
        list.appendChild(row);
      }
      row.dataset.id = id;
      row.children[0].className = unread ? 'dot unread' : 'dot read';
      row.children[1].className = unread ? 'sender unread' : 'sender';
      row.children[1].textContent = sender;
      row.children[2].textContent = preview;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-comms-hail-list')) {
  customElements.define('ph-comms-hail-list', PhCommsHailList);
}
