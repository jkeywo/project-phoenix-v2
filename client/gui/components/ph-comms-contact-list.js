export class PhCommsContactList extends HTMLElement {
  #state = null;
  #pillCache = new Map();
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
    .pill { display: flex; align-items: center; gap: 0.5rem; font-size: 0.7rem; padding: 0.35rem 0.4rem; border: 1px solid #282c38; border-radius: 3px; }
    .pill.out-of-range { opacity: 0.45; }
    .name { flex: 1; min-width: 0; }
    .badge { font-size: 0.55rem; padding: 0.1rem 0.35rem; border-radius: 2px; letter-spacing: 0.1em; text-transform: uppercase; }
    .badge.hostile { background: #3a1515; color: #e05555; }
    .badge.friendly { background: #153a1e; color: #55e070; }
    .badge.neutral { background: #15283a; color: #5590e0; }
    .badge.allied { background: #153a1e; color: #55e070; }
    .hail-btn { background: #0e1117; border: 1px solid #282c38; color: #cce; font-family: 'Chakra Petch', sans-serif; font-size: 0.6rem; font-weight: 600; padding: 0.25rem 0.5rem; cursor: pointer; letter-spacing: 0.1em; text-transform: uppercase; transition: all 0.15s ease; }
    .hail-btn:hover:not(:disabled) { background: #161b24; border-color: #4a5060; }
    .hail-btn:disabled { opacity: 0.35; cursor: default; }
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
    const raw = Array.isArray(s.contacts) ? s.contacts : [];
    const list = this.shadowRoot.getElementById('list');

    const live = new Set(raw.map(c => c.id || ''));
    for (const [key, el] of this.#pillCache) {
      if (!live.has(key)) { el.remove(); this.#pillCache.delete(key); }
    }

    if (raw.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = 'NO CONTACTS'; list.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    raw.forEach(c => {
      const id = c.id || '';
      const name = c.name || '';
      const stance = c.stance || 'neutral';
      const inRange = !!c.in_range;
      let pill = this.#pillCache.get(id);
      if (!pill) {
        pill = document.createElement('div');
        pill.className = 'pill';
        pill.innerHTML = '<span class="name"></span><span class="badge"></span><button class="hail-btn">HAIL</button>';
        pill.lastChild.addEventListener('click', (e) => {
          e.stopPropagation();
          if (this.sendAction) {
            this.sendAction('hail', { target_uuid: id });
          }
        });
        this.#pillCache.set(id, pill);
        list.appendChild(pill);
      }
      pill.dataset.id = id;
      pill.className = inRange ? 'pill' : 'pill out-of-range';
      pill.children[0].textContent = name;
      pill.children[1].className = 'badge ' + stance;
      pill.children[1].textContent = stance;
      pill.children[2].disabled = !inRange;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-comms-contact-list')) {
  customElements.define('ph-comms-contact-list', PhCommsContactList);
}
