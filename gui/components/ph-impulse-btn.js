import { observeGamepadButton, GAMEPAD_BUTTON } from '../gamepad-button.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhImpulseBtn extends HTMLElement {
  #state = null;
  #stopGamepad = null;

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
    .header { display: flex; justify-content: space-between; align-items: center; gap: 0.4rem; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; margin-bottom: 0.4rem; }
    .binding { margin-left: auto; font-size: 0.55rem; letter-spacing: 0.15em; color: var(--ink-faint); }
    .auto-badge { font-size: 0.6rem; color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    .btn { --charge: 0; width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: 0.9rem; font-weight: 700; padding: 0.7rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 2px solid; transition: background 0.3s ease; }
    .btn.ready { background: var(--bg-card); border-color: var(--loaded); color: var(--loaded); }
    .btn.ready:hover:not(:disabled) { background: var(--loaded-dim); }
    .btn.charging { background: linear-gradient(90deg, var(--reloading) calc(var(--charge) * 100%), var(--bg-card) calc(var(--charge) * 100%)); border-color: var(--reloading); color: var(--reloading); }
    .btn.cooldown { background: var(--bg-card); border-color: var(--ink-dim); color: var(--ink-dim); }
    .btn:disabled { opacity: 0.4; cursor: default; }
  </style>
  <div class="header">
    <span>${t('component.impulse.title')}</span>
    <span class="binding" id="binding">${t('component.impulse.binding')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <button class="btn ready" id="btn">${t('component.impulse.ready')}</button>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    this.shadowRoot.getElementById('btn').addEventListener('click', this.#press);
    // Ctrl and gamepad B fire the same press as the on-screen button, so the
    // helm keeps impulse under thumb while the other hand flies the stick.
    if (typeof document !== 'undefined') document.addEventListener('keydown', this.#onKeyDown);
    this.#stopGamepad = observeGamepadButton(GAMEPAD_BUTTON.B, (pressed) => {
      if (pressed) this.#press();
    });
  }

  disconnectedCallback() {
    if (typeof document !== 'undefined') document.removeEventListener('keydown', this.#onKeyDown);
    if (this.#stopGamepad) { this.#stopGamepad(); this.#stopGamepad = null; }
  }

  #onKeyDown = (e) => {
    const tag = e.target && e.target.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return;
    if (e.code !== 'ControlLeft' && e.code !== 'ControlRight') return;
    // Held Ctrl auto-repeats; impulse is a discrete press, and a repeat would
    // otherwise start and immediately cancel the charge over and over.
    if (e.repeat) return;
    e.preventDefault();
    this.#press();
  };

  #press = () => {
    const btn = this.shadowRoot.getElementById('btn');
    if (!this.sendAction || btn.disabled) return;
    const s = this.#state || {};
    const st = s.state || 'ready';
    // Pressing IMPULSE again while it is charging cancels the charge.
    if (st === 'charging') {
      this.sendAction('cancel_impulse', {});
    } else if (st === 'ready') {
      this.sendAction('start_impulse_charge', {});
    }
  };

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const st = s.state || 'ready';
    const chargePct = s.charge_pct != null ? Math.max(0, Math.min(100, Number(s.charge_pct))) : 0;
    const auto = !!s.auto;

    const root = this.shadowRoot;
    const btn = root.getElementById('btn');
    const badge = root.getElementById('auto-badge');

    // Button text and classes
    if (st === 'ready') {
      btn.textContent = t('component.impulse.ready');
      btn.className = 'btn ready';
      btn.disabled = auto;
    } else if (st === 'charging') {
      // Keep the button enabled during charging so a second press cancels it
      // (disabled only under AUTO, where the operator has no manual control).
      btn.textContent = t('component.impulse.cancel', { pct: Math.round(chargePct) });
      btn.className = 'btn charging';
      btn.disabled = auto;
    } else if (st === 'cooldown') {
      btn.textContent = t('console.common.cooldown');
      btn.className = 'btn cooldown';
      btn.disabled = true;
    }

    // Fill the button itself left-to-right as it charges.
    btn.style.setProperty('--charge', chargePct / 100);

    badge.style.display = auto ? 'inline' : 'none';
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-impulse-btn')) {
  customElements.define('ph-impulse-btn', PhImpulseBtn);
}
