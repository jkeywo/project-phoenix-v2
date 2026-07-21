// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';

import './ph-damage-bar.js';
import './ph-damage-detail.js';

export class PhHullIntegrity extends HTMLElement {
  #state = null;
  #barEl = null;
  #detailEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .placeholder { font-size: 0.65rem; color: var(--ink-dim); letter-spacing: 0.2em; padding: 0.5rem 0; text-align: center; }
    .systems-label { font-size: 0.65rem; letter-spacing: 0.2em; color: var(--ink-dim); margin-top: 0.25rem; }
  </style>
  <div class="header"><span>${t('component.hull_integrity.title')}</span></div>
  <div id="bar-container"></div>
  <div class="placeholder" id="placeholder">${t('component.hull_integrity.no_data')}</div>
  <div class="systems-label" id="systems-label">${t('component.hull_integrity.systems')}</div>
  <div id="detail-container"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));

    this.#barEl = document.createElement('ph-damage-bar');
    this.#detailEl = document.createElement('ph-damage-detail');

    this.shadowRoot.getElementById('bar-container').appendChild(this.#barEl);
    this.shadowRoot.getElementById('detail-container').appendChild(this.#detailEl);
  }

  connectedCallback() {}

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state;
    const root = this.shadowRoot;
    const placeholder = root.getElementById('placeholder');
    const systemsLabel = root.getElementById('systems-label');
    const barContainer = root.getElementById('bar-container');

    if (s == null) {
      barContainer.style.display = 'none';
      this.#detailEl.style.display = 'none';
      systemsLabel.style.display = 'none';
      placeholder.style.display = 'block';
      return;
    }

    placeholder.style.display = 'none';
    barContainer.style.display = 'block';

    const barState = {};
    if (s.total_pct != null) {
      barState.pct = s.total_pct;
      if (Array.isArray(s.systems) && s.systems.length > 0) {
        barState.totalCurrent = s.systems.reduce((sum, sys) => sum + (sys.current || 0), 0);
        barState.totalMax = s.systems.reduce((sum, sys) => sum + (sys.max_hp || 0), 0);
      }
    } else if (s.pct != null) {
      barState.pct = s.pct;
      barState.totalCurrent = s.totalCurrent;
      barState.totalMax = s.totalMax;
    }

    this.#barEl.state = Object.keys(barState).length > 0 ? barState : null;

    const entries = Array.isArray(s.systems) ? s.systems
      : Array.isArray(s.entries) ? s.entries : [];

    if (entries.length > 0) {
      this.#detailEl.style.display = 'block';
      systemsLabel.style.display = 'block';
      this.#detailEl.state = { entries };
    } else {
      this.#detailEl.style.display = 'none';
      systemsLabel.style.display = 'none';
      this.#detailEl.state = null;
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-hull-integrity')) {
  customElements.define('ph-hull-integrity', PhHullIntegrity);
}
