// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhCameraSelect extends HTMLElement {
  #state = null;
  #btnCache = new Map();
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
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    #container { display: grid; grid-template-columns: repeat(3, 1fr); grid-auto-rows: 1fr; gap: 0.35rem; }
    .cam-btn { background: var(--bg-card); border: 1px solid var(--line-faint); color: var(--ink-dim); font-family: 'Chakra Petch', sans-serif; font-size: var(--text-sm); font-weight: 600; padding: 0.5rem 0; letter-spacing: 0.15em; text-transform: uppercase; cursor: pointer; transition: all 0.15s ease; }
    .cam-btn:hover:not(:disabled) { background: var(--cyan-deep); color: var(--ink-dim); }
    .cam-btn.active { background: var(--surface-panel-up); border-color: var(--cyan); color: var(--cyan); }
    .cam-btn.active:hover { background: var(--cyan-dim); }
    .cam-btn:disabled { opacity: 0.4; cursor: default; }
    .placeholder { font-size: var(--text-sm); color: var(--ink-dim); letter-spacing: 0.2em; padding: 0.5rem 0; text-align: center; }
  </style>
  <div class="header">
    <span>${t('component.camera_select.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div id="container"></div>
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
    const root = this.shadowRoot;

    const views = s.views || [];
    const currentView = s.current_view || '';
    const auto = !!s.auto;

    root.getElementById('auto-badge').style.display = auto ? 'inline' : 'none';

    const container = root.getElementById('container');

    const live = new Set(views);
    for (const [key, btn] of this.#btnCache) {
      if (!live.has(key)) { btn.remove(); this.#btnCache.delete(key); }
    }

    if (views.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'placeholder'; this.#emptyEl.textContent = t('component.camera_select.empty'); container.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    // Arrange the four principal views in a cross (FWD top, PORT left, STBD
    // right, AFT bottom); any additional named cameras flow into free cells.
    let flowCell = 0;
    const freeCells = ['1 / 1', '1 / 3', '3 / 1', '3 / 3', '2 / 2'];
    views.forEach(v => {
      let btn = this.#btnCache.get(v);
      if (!btn) {
        btn = document.createElement('button');
        btn.className = 'cam-btn';
        btn.dataset.view = v;
        btn.addEventListener('click', () => {
          if (!btn.disabled && this.sendAction) {
            this.sendAction('set_view', { direction: btn.dataset.view });
          }
        });
        this.#btnCache.set(v, btn);
        container.appendChild(btn);
      }
      btn.className = 'cam-btn' + (v === currentView ? ' active' : '');
      btn.disabled = auto;
      btn.textContent = v;

      const n = String(v).toLowerCase();
      let area = null;
      if (/(^|[^a-z])(fore|forward|fwd|front|bow)([^a-z]|$)/.test(n)) area = '1 / 2';
      else if (/(^|[^a-z])(aft|rear|reverse|back|stern)([^a-z]|$)/.test(n)) area = '3 / 2';
      else if (/(^|[^a-z])(port|left)([^a-z]|$)/.test(n)) area = '2 / 1';
      else if (/(^|[^a-z])(starboard|stbd|right)([^a-z]|$)/.test(n)) area = '2 / 3';
      else { area = freeCells[flowCell % freeCells.length]; flowCell++; }
      btn.style.gridArea = area;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-camera-select')) {
  customElements.define('ph-camera-select', PhCameraSelect);
}
