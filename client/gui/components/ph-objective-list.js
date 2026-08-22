// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';
import { installRovingTabindex, syncRovingTabindex } from '../roving-tabindex.js';

export class PhObjectiveList extends PhElement {
  #rowCache = new Map();
  #emptyEl = null;
  #roving = null;

  template() {
    return `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.35rem; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    /* The row is a native <button role="option"> (issue #1178): focusable,
       named by its objective text, activating on Enter/Space through the SAME
       set_objective_priority the pointer click sends. The reset strips the
       browser button chrome so it still reads as a plain list row. */
    .row { display: flex; align-items: flex-start; gap: 0.4rem; width: 100%; margin: 0; font: inherit; text-align: left; background: none; border: 0; color: inherit; font-size: var(--text-sm); line-height: 1.3; min-height: var(--control-hit-min); }
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
  }

  connectedCallback() {
    super.connectedCallback();
    // Role + accessible name + keyboard operation (issue #1178). The objectives
    // were clickable <div>s; the list is now a listbox — one Tab stop, arrows
    // roving over the option rows — with the boosted objective marked selected.
    this.setAttribute('role', 'listbox');
    this.setAttribute('aria-orientation', 'vertical');
    this.setAttribute('aria-label', t('component.objectives.label'));
    this.#roving ??= installRovingTabindex(this, {
      getItems: () => this.#rovingItems(),
      orientation: 'vertical',
    });
    this.#syncRoving();
  }

  /** The list's rovable option rows, in document order. */
  #rovingItems() {
    return Array.from(this.shadowRoot.querySelectorAll('.row'));
  }

  /** Re-establish the single tab stop after a render adds/removes rows. */
  #syncRoving() {
    syncRovingTabindex(this.#rovingItems());
  }

  render(state) {
    const s = state || {};
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
        el = document.createElement('button');
        el.type = 'button';
        el.setAttribute('role', 'option');
        el.innerHTML = '<span class="indicator"></span><span class="text"></span>';
        // Enter/Space (native to the button) and a pointer tap alike run this
        // one handler, dispatching the SAME set_objective_priority action.
        el.addEventListener('click', () => {
          if (this.sendAction && key) {
            this.sendAction('set_objective_priority', { id: key });
          }
        });
        this.#rowCache.set(key, el);
        list.appendChild(el);
      }
      el.className = ['row', done && 'done', boosted && 'boosted'].filter(Boolean).join(' ');
      // The boosted objective is the listbox's selected option.
      el.setAttribute('aria-selected', String(boosted));
      el.firstChild.className = done ? 'indicator done' : 'indicator pending';
      el.lastChild.textContent = text;
    });
    this.#syncRoving();
  }
}

phDefine('ph-objective-list', PhObjectiveList);
