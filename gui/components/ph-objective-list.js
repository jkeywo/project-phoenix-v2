export class PhObjectiveList extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.35rem; }
    .empty { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .row { display: flex; align-items: flex-start; gap: 0.4rem; font-size: 0.7rem; line-height: 1.3; }
    .row .indicator { flex-shrink: 0; width: 0.7rem; height: 0.7rem; margin-top: 0.2rem; border: 1px solid #4a5060; border-radius: 50%; display: flex; align-items: center; justify-content: center; }
    .row .indicator.done { background: #2a6838; border-color: #4ec870; }
    .row .indicator.done::after { content: '\\2713'; font-size: 0.5rem; color: #4ec870; }
    .row .indicator.pending { background: transparent; border-color: #4a5060; }
    .row .text { flex: 1; min-width: 0; }
    .row.done .text { text-decoration: line-through; color: #6a7178; }
  </style>
  <div class="list" id="list"></div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
  }

  connectedCallback() {}

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const raw = Array.isArray(s.objectives) ? s.objectives : [];
    const list = this.shadowRoot.getElementById('list');

    if (raw.length === 0) {
      list.innerHTML = '<div class="empty">NO OBJECTIVES</div>';
      return;
    }

    list.innerHTML = raw.map(o => {
      const done = o.done != null ? o.done : (o.status === 'Completed');
      const text = o.text || '';
      const rowCls = done ? 'row done' : 'row';
      const indicatorCls = done ? 'indicator done' : 'indicator pending';
      return `<div class="${rowCls}"><span class="${indicatorCls}"></span><span class="text">${text}</span></div>`;
    }).join('');
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-objective-list')) {
  customElements.define('ph-objective-list', PhObjectiveList);
}
