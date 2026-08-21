// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { commsPreview } from '../comms-state.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';
import { installRovingTabindex, syncRovingTabindex } from '../roving-tabindex.js';

export class PhCommsHailList extends HTMLElement {
  #state = null;
  #rowCache = new Map();
  #emptyEl = null;
  #roving = null;
  #selectedId = null;

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
    .list { display: flex; flex-direction: column; gap: 0.25rem; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    /* The row is a native <button role="option"> (issue #1178): focusable,
       named by its own text, and activating on Enter/Space through the SAME
       click handler the pointer uses. The reset strips the browser chrome so it
       still reads as a list row. */
    .row { display: flex; align-items: center; gap: 0.4rem; width: 100%; margin: 0; font: inherit; text-align: left; background: none; border: 0; color: var(--ink); font-size: var(--text-sm); padding: 0.35rem 0.4rem; cursor: pointer; border-radius: 2px; transition: background 0.15s ease; min-height: var(--control-hit-min); }
    .row:hover { background: var(--cyan-deep); }
    .row[aria-selected="true"] { background: var(--cyan-deep); }
    .dot { width: 0.45rem; height: 0.45rem; border-radius: 50%; flex-shrink: 0; }
    .dot.unread { background: var(--science); }
    .dot.read { background: transparent; }
    .sender { font-weight: 400; color: var(--ink); min-width: 4rem; }
    .sender.unread { font-weight: 700; }
    .preview { color: var(--ink-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: 1; min-width: 0; }
    .timestamp { color: var(--edge); font-size: var(--text-xs); flex-shrink: 0; }
  </style>
  <div class="list" id="list"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    // Role + accessible name + keyboard operation (issue #1178). The hails were
    // clickable <div>s the keyboard could not land on; the list is now a proper
    // listbox — one Tab stop, arrows roving over the option rows — named from
    // the same string its console heading already shows.
    this.setAttribute('role', 'listbox');
    this.setAttribute('aria-orientation', 'vertical');
    this.setAttribute('aria-label', t('component.comms_hails.label'));
    // One Tab stop for the whole list; arrows move between the option rows.
    // Native <button>s keep their own Enter/Space activation — no fork.
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

  /** Paint aria-selected across the cached rows from the current selection. */
  #reflectSelection() {
    for (const [id, row] of this.#rowCache) {
      row.setAttribute('aria-selected', String(id === this.#selectedId));
    }
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

    const live = new Set(raw.map(h => h.id || ''));
    for (const [key, el] of this.#rowCache) {
      if (!live.has(key)) { el.remove(); this.#rowCache.delete(key); }
    }

    if (raw.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = t('component.comms_hails.empty'); list.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    raw.forEach(h => {
      const id = h.id || '';
      const sender = h.sender_name || '';
      const preview = commsPreview(h);
      const unread = !h.is_read;
      let row = this.#rowCache.get(id);
      if (!row) {
        row = document.createElement('button');
        row.type = 'button';
        row.className = 'row';
        row.setAttribute('role', 'option');
        row.innerHTML = '<span class="dot"></span><span class="sender"></span><span class="preview"></span>';
        // Enter/Space (native to the button) and a pointer tap alike run this
        // one handler, dispatching the SAME select_comms_message action.
        row.addEventListener('click', () => {
          this.#selectedId = id;
          this.#reflectSelection();
          if (this.sendAction) {
            this.sendAction('select_comms_message', { message_id: id });
          }
        });
        this.#rowCache.set(id, row);
        list.appendChild(row);
      }
      row.dataset.id = id;
      row.setAttribute('aria-selected', String(id === this.#selectedId));
      row.children[0].className = unread ? 'dot unread' : 'dot read';
      row.children[1].className = unread ? 'sender unread' : 'sender';
      row.children[1].textContent = sender;
      row.children[2].textContent = preview;
    });
    this.#syncRoving();
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-comms-hail-list')) {
  customElements.define('ph-comms-hail-list', PhCommsHailList);
}
