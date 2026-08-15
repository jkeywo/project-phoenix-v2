// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhObjectiveList extends HTMLElement {
  #state = null;
  #rowCache = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.35rem; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .row { display: flex; align-items: flex-start; gap: 0.4rem; font-size: var(--text-sm); line-height: 1.3; }
    .row .indicator { flex-shrink: 0; width: 0.7rem; height: 0.7rem; margin-top: 0.2rem; border: 1px solid var(--edge); border-radius: 50%; display: flex; align-items: center; justify-content: center; }
    .row .indicator.done { background: var(--loaded-dim); border-color: var(--loaded); }
    .row .indicator.done::after { content: '\\2713'; font-size: var(--text-xs); color: var(--loaded); }
    .row .indicator.pending { background: transparent; border-color: var(--edge); }
    .row .text { flex: 1; min-width: 0; }
    .row.done .text { text-decoration: line-through; color: var(--ink-dim); }
    .row { cursor: pointer; border-radius: 2px; padding: 0.1rem 0.2rem; }
    .row.boosted { background: var(--surface-panel-up); border-left: 2px solid var(--cyan); }
  </style>
  <div class="list" id="list"></div>
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
    const s = this.#state || {};
    const raw = Array.isArray(s.objectives) ? s.objectives : [];
    const list = this.shadowRoot.getElementById('list');

    const live = new Set(raw.map(o => o.id || o.text || ''));
    for (const [key, el] of this.#rowCache) {
      if (!live.has(key)) { el.remove(); this.#rowCache.delete(key); }
    }

    if (raw.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = t('component.objectives.empty'); list.appendChild(this.#emptyEl); }
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
