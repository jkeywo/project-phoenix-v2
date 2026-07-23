// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';

export class PhRedAlert extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: 0.6rem; color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    .alert-btn { width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: 0.9rem; font-weight: 700; padding: 0.7rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 2px solid; transition: all 0.15s ease; }
    .alert-btn.standby { background: var(--bg-card); border-color: var(--line-faint); color: var(--ink-dim); }
    .alert-btn.standby:hover:not(:disabled) { background: #161b24; color: #aab; }
    .alert-btn.active { background: #3a0a0a; border-color: var(--fire); color: var(--fire); text-shadow: 0 0 8px rgba(224,64,44,0.5); }
    .alert-btn.active:hover:not(:disabled) { background: #4a0e0e; }
    .alert-btn:disabled { opacity: 0.4; cursor: default; }
  </style>
  <div class="header">
    <span>${t('component.red_alert.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <button class="alert-btn standby" id="alert-btn">${t('component.red_alert.standby')}</button>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    const btn = this.shadowRoot.getElementById('alert-btn');
    btn.addEventListener('click', () => {
      if (this.sendAction && !btn.disabled) {
        // Send the explicit desired state (issue #748): the opposite of what
        // is currently displayed. Assigning (not toggling) on the host makes a
        // stale / duplicated / retried command idempotent.
        const currentlyActive = !!(this.#state && this.#state.active);
        this.sendAction('set_red_alert', { active: !currentlyActive });
      }
    });
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const active = !!s.active;
    const auto = !!s.auto;
    const root = this.shadowRoot;
    const btn = root.getElementById('alert-btn');

    btn.textContent = active ? t('component.red_alert.active') : t('component.red_alert.standby');
    btn.className = 'alert-btn' + (active ? ' active' : ' standby');
    btn.disabled = auto;

    root.getElementById('auto-badge').style.display = auto ? 'inline' : 'none';
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-red-alert')) {
  customElements.define('ph-red-alert', PhRedAlert);
}
