// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhRedAlert extends HTMLElement {
  #state = null;

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
    .alert-btn { width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: var(--text-md); font-weight: 700; padding: 0.7rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 2px solid; transition: all 0.15s ease; min-height: var(--control-hit-min); }
    .alert-btn.standby { background: var(--bg-card); border-color: var(--line-faint); color: var(--ink-dim); }
    .alert-btn.standby:hover:not(:disabled) { background: var(--cyan-deep); color: var(--ink-dim); }
    .alert-btn.active { background: var(--fire-deep); border-color: var(--fire); color: var(--fire); text-shadow: 0 0 8px rgba(var(--rgb-fire), 0.5); }
    .alert-btn.active:hover:not(:disabled) { background: var(--tactical-deep); }
    .alert-btn:disabled { opacity: 0.4; cursor: default; }
    /* The restraint lever (issue #1041). Deliberately quieter than the alert
       button above it: holding fire is a posture, not an emergency. */
    .hold-btn { width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: var(--text-sm); font-weight: 700; padding: 0.4rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 1px solid; transition: all 0.15s ease; min-height: var(--control-hit-min); }
    .hold-btn.free { background: var(--bg-card); border-color: var(--line-faint); color: var(--ink-dim); }
    .hold-btn.free:hover:not(:disabled) { background: var(--cyan-deep); color: var(--ink-dim); }
    .hold-btn.held { background: var(--reloading-deep); border-color: var(--reloading); color: var(--reloading); }
    .hold-btn.held:hover:not(:disabled) { background: var(--reloading-deep); }
    .hold-btn:disabled { opacity: 0.4; cursor: default; }
  </style>
  <div class="header">
    <span>${t('component.red_alert.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <button class="alert-btn standby" id="alert-btn">${t('component.red_alert.standby')}</button>
  <button class="hold-btn free" id="hold-btn">${t('component.weapons_hold.free')}</button>
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
    // The weapons hold (issue #1041). Its own button beside the alert, not a
    // third state of it: the two are independent, and a captain can be at
    // stations with the guns cold.
    const holdBtn = this.shadowRoot.getElementById('hold-btn');
    holdBtn.addEventListener('click', () => {
      if (this.sendAction && !holdBtn.disabled) {
        const currentlyHeld = !!(this.#state && this.#state.hold);
        this.sendAction('set_weapons_hold', { held: !currentlyHeld });
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

    // The hold reads off the same control source as the alert — one console
    // owns the ship's firing posture — so it greys out together with it.
    const held = !!s.hold;
    const holdBtn = root.getElementById('hold-btn');
    holdBtn.textContent = held
      ? t('component.weapons_hold.held')
      : t('component.weapons_hold.free');
    holdBtn.className = 'hold-btn' + (held ? ' held' : ' free');
    holdBtn.disabled = auto;

    root.getElementById('auto-badge').style.display = auto ? 'inline' : 'none';
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-red-alert')) {
  customElements.define('ph-red-alert', PhRedAlert);
}
