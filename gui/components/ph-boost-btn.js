import { observeGamepadButton, GAMEPAD_BUTTON } from '../gamepad-button.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhBoostBtn extends HTMLElement {
  #state = null;
  // Boost is a hold, and pointer / Shift / gamepad A can hold it at the same
  // time. Tracking the live sources means `set_boost` is sent once when the
  // first one engages and once when the last one lets go — releasing one
  // source never cuts boost while another is still held.
  #holds = new Set();
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
    .header { display: flex; justify-content: space-between; align-items: center; gap: 0.4rem; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; margin-bottom: 0.4rem; }
    .binding { margin-left: auto; font-size: var(--text-xs); letter-spacing: 0.15em; color: var(--ink-faint); }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    .btn { width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: var(--text-md); font-weight: 700; padding: 0.7rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 2px solid; transition: all 0.15s ease; }
    .btn.available { background: var(--bg-card); border-color: var(--loaded); color: var(--loaded); }
    .btn.available:hover:not(:disabled) { background: var(--loaded-dim); }
    .btn.active { background: var(--loaded-deep); border-color: var(--loaded); color: var(--loaded); text-shadow: 0 0 8px rgba(var(--rgb-loaded), 0.5); }
    .btn.recharging { background: var(--bg-card); border-color: var(--ink-dim); color: var(--ink-dim); }
    .btn:disabled { opacity: 0.4; cursor: default; }
    .recharge-wrap { width: 100%; height: 0.4rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; margin-bottom: 0.3rem; }
    .recharge-fill { height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.3s ease; }
    .recharge-fill.draining { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
  </style>
  <div class="header">
    <span>${t('component.boost.title')}</span>
    <span class="binding" id="binding">${t('component.boost.binding')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div class="recharge-wrap" id="recharge-wrap" style="display:none">
    <div class="recharge-fill" id="recharge-fill" style="width:100%"></div>
  </div>
  <button class="btn available" id="btn">${t('component.boost.ready')}</button>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  #pointerId = null;

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    const btn = this.shadowRoot.getElementById('btn');
    btn.addEventListener('pointerdown', (e) => {
      if (this.#pointerId !== null) return;
      if (!this.#hold('pointer')) return;
      this.#pointerId = e.pointerId;
      if (btn.setPointerCapture) btn.setPointerCapture(e.pointerId);
      e.preventDefault();
    });
    const release = (e) => {
      if (e.pointerId !== this.#pointerId) return;
      this.#pointerId = null;
      try { if (btn.releasePointerCapture) btn.releasePointerCapture(e.pointerId); } catch (_) {}
      this.#release('pointer');
    };
    btn.addEventListener('pointerup', release);
    btn.addEventListener('pointercancel', release);
    // Safety net: a silently revoked pointer capture must not latch boost on.
    btn.addEventListener('lostpointercapture', release);

    // Hold Shift or gamepad A to boost, mirroring the on-screen hold.
    if (typeof document !== 'undefined') {
      document.addEventListener('keydown', this.#onKeyDown);
      document.addEventListener('keyup', this.#onKeyUp);
    }
    // A focus loss eats the keyup, so drop the key hold rather than boost on
    // forever with the battery draining behind a hidden tab.
    if (typeof window !== 'undefined') window.addEventListener('blur', this.#onBlur);
    this.#stopGamepad = observeGamepadButton(GAMEPAD_BUTTON.A, (pressed) => {
      if (pressed) this.#hold('gamepad'); else this.#release('gamepad');
    });
  }

  disconnectedCallback() {
    if (typeof document !== 'undefined') {
      document.removeEventListener('keydown', this.#onKeyDown);
      document.removeEventListener('keyup', this.#onKeyUp);
    }
    if (typeof window !== 'undefined') window.removeEventListener('blur', this.#onBlur);
    // Stopping the observer reports a held pad button as released, which
    // clears the 'gamepad' hold through the callback above.
    if (this.#stopGamepad) { this.#stopGamepad(); this.#stopGamepad = null; }
    this.#release('pointer');
    this.#release('key');
  }

  #onKeyDown = (e) => {
    const tag = e.target && e.target.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return;
    if (e.code !== 'ShiftLeft' && e.code !== 'ShiftRight') return;
    e.preventDefault();
    this.#hold('key');
  };

  #onKeyUp = (e) => {
    if (e.code !== 'ShiftLeft' && e.code !== 'ShiftRight') return;
    this.#release('key');
  };

  #onBlur = () => this.#release('key');

  /**
   * Engage boost from `source`. Returns false when the press was rejected
   * because boost is unavailable (recharging, or AUTO holds the controls).
   */
  #hold(source) {
    if (this.#holds.has(source)) return true;
    const btn = this.shadowRoot.getElementById('btn');
    if (this.#holds.size === 0 && (!this.sendAction || btn.disabled)) return false;
    const wasIdle = this.#holds.size === 0;
    this.#holds.add(source);
    if (wasIdle && this.sendAction) this.sendAction('set_boost', { active: true });
    return true;
  }

  /** Release `source`; sends the stop only once the last source lets go. */
  #release(source) {
    if (!this.#holds.delete(source)) return;
    if (this.#holds.size === 0 && this.sendAction) this.sendAction('set_boost', { active: false });
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const available = s.available !== false;
    const active = !!s.active;
    const batteryPct = s.recharge_pct != null ? Math.max(0, Math.min(100, Number(s.recharge_pct))) : 100;
    const auto = !!s.auto;

    const root = this.shadowRoot;
    const btn = root.getElementById('btn');
    const rechargeWrap = root.getElementById('recharge-wrap');
    const rechargeFill = root.getElementById('recharge-fill');
    const badge = root.getElementById('auto-badge');

    const draining = active && batteryPct < 100;
    const recharging = !active && batteryPct < 100;

    if (active) {
      btn.textContent = batteryPct < 100
        ? t('component.boost.boosting', { pct: Math.round(batteryPct) })
        : t('component.boost.boosting_full');
      btn.className = 'btn active';
      btn.disabled = false;
    } else if (recharging) {
      btn.textContent = t('component.boost.recharging', { pct: Math.round(batteryPct) });
      btn.className = 'btn recharging';
      btn.disabled = true;
    } else {
      btn.textContent = t('component.boost.ready');
      btn.className = 'btn available';
      btn.disabled = auto;
    }

    // Battery bar visible when not full (shows drain during boost, fill during recharge)
    if (draining || recharging) {
      rechargeWrap.style.display = 'block';
      rechargeFill.style.width = batteryPct + '%';
      rechargeFill.className = 'recharge-fill' + (draining ? ' draining' : '');
    } else {
      rechargeWrap.style.display = 'none';
    }

    badge.style.display = auto ? 'inline' : 'none';
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-boost-btn')) {
  customElements.define('ph-boost-btn', PhBoostBtn);
}
