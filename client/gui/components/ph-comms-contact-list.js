// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';

export class PhCommsContactList extends PhElement {
  #pillCache = new Map();
  #emptyEl = null;

  template() {
    return `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.25rem; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    .pill { display: flex; align-items: center; gap: 0.5rem; font-size: var(--text-sm); padding: 0.35rem 0.4rem; border: 1px solid var(--line-faint); border-radius: 3px; }
    .pill.out-of-range { opacity: 0.45; }
    .name { flex: 1; min-width: 0; }
    .badge { font-size: var(--text-xs); padding: 0.1rem 0.35rem; border-radius: 2px; letter-spacing: 0.1em; text-transform: uppercase; }
    .badge.hostile { background: var(--tactical-deep); color: var(--fire); }
    .badge.friendly { background: var(--loaded-deep); color: var(--loaded); }
    .badge.neutral { background: var(--surface-panel-up); color: var(--science); }
    .badge.allied { background: var(--loaded-deep); color: var(--loaded); }
    .hail-btn { background: var(--bg-card); border: 1px solid var(--line-faint); color: var(--ink); font-family: 'Chakra Petch', sans-serif; font-size: var(--text-xs); font-weight: 600; padding: 0.25rem 0.5rem; cursor: pointer; letter-spacing: 0.1em; text-transform: uppercase; transition: all 0.15s ease; min-height: var(--control-hit-min); }
    .hail-btn:hover:not(:disabled) { background: var(--cyan-deep); border-color: var(--edge); }
    .hail-btn:disabled { opacity: 0.35; cursor: default; }
  </style>
  <div class="list" id="list"></div>
`;
  }

  render(state) {
    const s = state || {};
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

phDefine('ph-comms-contact-list', PhCommsContactList);
