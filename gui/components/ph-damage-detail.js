// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhDamageDetail extends HTMLElement {
  #state = null;
  #rowCache = new Map();

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
    .list { display: flex; flex-direction: column; gap: 0.2rem; }
    .row { display: flex; align-items: center; gap: 0.4rem; font-size: var(--text-xs); }
    .row .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .row .bar-wrap { width: 4rem; height: 0.6rem; background: var(--bg-deep); border: 1px solid var(--line-faint); position: relative; overflow: hidden; flex-shrink: 0; }
    .row .bar-wrap .fill { position: absolute; top: 0; left: 0; height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); }
    .row .bar-wrap .fill.warn { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .row .bar-wrap .fill.crit { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .row .tier { font-size: var(--text-xs); color: var(--ink-dim); letter-spacing: 0.1em; min-width: 1.6rem; text-align: right; flex-shrink: 0; }
    .row.destroyed .name { color: var(--fire); letter-spacing: 0.15em; }
    .row.destroyed .bar-wrap .fill { background: var(--fire-dim); opacity: 0.5; }
    .destroyed-label { color: var(--fire); font-size: var(--text-xs); letter-spacing: 0.2em; flex-shrink: 0; }
  </style>
  <div class="list" id="list"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const d = this.#state || {};
    const entries = Array.isArray(d.entries) ? d.entries : [];
    const list = this.shadowRoot.getElementById('list');

    const live = new Set(entries.map(e => e.display_name || ''));
    for (const [key, el] of this.#rowCache) {
      if (!live.has(key)) { el.remove(); this.#rowCache.delete(key); }
    }

    if (entries.length === 0) { list.textContent = ''; return; }

    entries.forEach(e => {
      const key = e.display_name || '';
      const max = e.max_hp != null ? e.max_hp : 0;
      const cur = e.current != null ? e.current : 0;
      const pct = max > 0 ? cur / max : 0;
      const destroyed = cur === 0;
      const widthPct = Math.max(0, Math.min(1, pct)) * 100;

      let fillCls = 'fill';
      if (pct < 0.4) fillCls += ' crit';
      else if (pct < 0.75) fillCls += ' warn';

      const tierLabel = e.tier != null ? 'T' + e.tier : '';
      const nameLabel = e.display_name || '';

      let el = this.#rowCache.get(key);
      if (!el) {
        el = document.createElement('div');
        el.innerHTML = '<span class="name"></span><div class="bar-wrap"><div class="fill"></div></div><span class="tier"></span>';
        this.#rowCache.set(key, el);
        list.appendChild(el);
      }
      el.className = destroyed ? 'row destroyed' : 'row';
      el.children[0].textContent = nameLabel;
      el.children[1].firstChild.className = fillCls;
      el.children[1].firstChild.style.width = widthPct + '%';
      var tierEl = el.children[2];
      tierEl.textContent = tierLabel;
      var dEl = el.querySelector('.destroyed-label');
      if (destroyed) {
        if (!dEl) { dEl = document.createElement('span'); dEl.className = 'destroyed-label'; dEl.textContent = t('console.common.destroyed'); el.insertBefore(dEl, tierEl); }
      } else if (dEl) {
        dEl.remove();
      }
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-damage-detail')) {
  customElements.define('ph-damage-detail', PhDamageDetail);
}
