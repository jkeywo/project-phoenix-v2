export class PhObjectiveList extends HTMLElement {
  #state = null;
  #rowCache = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.35rem; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .row { display: flex; align-items: flex-start; gap: 0.4rem; font-size: 0.7rem; line-height: 1.3; }
    .row .indicator { flex-shrink: 0; width: 0.7rem; height: 0.7rem; margin-top: 0.2rem; border: 1px solid #4a5060; border-radius: 50%; display: flex; align-items: center; justify-content: center; }
    .row .indicator.done { background: var(--loaded-dim); border-color: var(--loaded); }
    .row .indicator.done::after { content: '\\2713'; font-size: 0.5rem; color: var(--loaded); }
    .row .indicator.pending { background: transparent; border-color: #4a5060; }
    .row .text { flex: 1; min-width: 0; }
    .row.done .text { text-decoration: line-through; color: var(--ink-dim); }
    .row { cursor: pointer; border-radius: 2px; padding: 0.1rem 0.2rem; }
    .row.boosted { background: #1a2a3a; border-left: 2px solid var(--cyan); }
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
    const raw = Array.isArray(s.objectives) ? s.objectives : [];
    const list = this.shadowRoot.getElementById('list');

    const live = new Set(raw.map(o => o.id || o.text || ''));
    for (const [key, el] of this.#rowCache) {
      if (!live.has(key)) { el.remove(); this.#rowCache.delete(key); }
    }

    if (raw.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = 'NO OBJECTIVES'; list.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    const boostedId = s.boosted_objective_id ?? null;

    raw.forEach(o => {
      const key = o.id || o.text || '';
      const done = o.done != null ? o.done : (o.status === 'Completed');
      const text = o.text || '';
      const boosted = key !== '' && boostedId === key;
      let el = this.#rowCache.get(key);
      if (!el) {
        el = document.createElement('div');
        el.innerHTML = '<span class="indicator"></span><span class="text"></span>';
        el.addEventListener('click', () => {
          if (this.sendAction && key) {
            this.sendAction('set_objective_priority', { id: key });
          }
        });
        this.#rowCache.set(key, el);
        list.appendChild(el);
      }
      el.className = ['row', done && 'done', boosted && 'boosted'].filter(Boolean).join(' ');
      el.firstChild.className = done ? 'indicator done' : 'indicator pending';
      el.lastChild.textContent = text;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-objective-list')) {
  customElements.define('ph-objective-list', PhObjectiveList);
}
