// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';

export class PhCommsContactList extends HTMLElement {
  #state = null;
  #pillCache = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.25rem; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .pill { display: flex; align-items: center; gap: 0.5rem; font-size: 0.7rem; padding: 0.35rem 0.4rem; border: 1px solid var(--line-faint); border-radius: 3px; }
    .pill.out-of-range { opacity: 0.45; }
    .name { flex: 1; min-width: 0; }
    .badge { font-size: 0.55rem; padding: 0.1rem 0.35rem; border-radius: 2px; letter-spacing: 0.1em; text-transform: uppercase; }
    .badge.hostile { background: #3a1515; color: #e05555; }
    .badge.friendly { background: #153a1e; color: #55e070; }
    .badge.neutral { background: #15283a; color: #5590e0; }
    .badge.allied { background: #153a1e; color: #55e070; }
    .hail-btn { background: var(--bg-card); border: 1px solid var(--line-faint); color: var(--ink); font-family: 'Chakra Petch', sans-serif; font-size: 0.6rem; font-weight: 600; padding: 0.25rem 0.5rem; cursor: pointer; letter-spacing: 0.1em; text-transform: uppercase; transition: all 0.15s ease; }
    .hail-btn:hover:not(:disabled) { background: #161b24; border-color: #4a5060; }
    .hail-btn:disabled { opacity: 0.35; cursor: default; }
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
    const raw = Array.isArray(s.contacts) ? s.contacts : [];
    const list = this.shadowRoot.getElementById('list');

    const live = new Set(raw.map(c => c.id || ''));
    for (const [key, el] of this.#pillCache) {
      if (!live.has(key)) { el.remove(); this.#pillCache.delete(key); }
    }

    if (raw.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = t('component.comms_contacts.empty'); list.appendChild(this.#emptyEl); }
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
        pill.innerHTML = '<span class="name"></span><span class="badge"></span><button class="hail-btn">' + t('component.comms_contacts.hail') + '</button>';
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
      // stance is a wire token ('friendly'/'hostile'/'neutral') — keep it as
      // the CSS class, localise only the visible badge text.
      pill.children[1].textContent = t('console.stance.' + stance);
      pill.children[2].disabled = !inRange;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-comms-contact-list')) {
  customElements.define('ph-comms-contact-list', PhCommsContactList);
}
