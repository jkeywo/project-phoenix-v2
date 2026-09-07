// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import {
  HELM_ACTION_CONTEXT,
  HELM_BOOST_ACTION_ID,
} from '../stations/helm-actions.js';
import { PhElement, phDefine } from './ph-element.js';

export class PhBoostBtn extends PhElement {
  // Boost is a hold, and pointer / Shift / gamepad A can hold it at the same
  // time. Tracking the live sources means `set_boost` is sent once when the
  // first one engages and once when the last one lets go — releasing one
  // source never cuts boost while another is still held.
  #holds = new Set();
  template() {
    return `
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
    /* Key-binding hints are meaningless on a phone (issue #1376): a touch
       screen has no SHIFT to hold. Same phone query the Station Bar already
       decides its own label mode on (gui/hero-bar.js's HERO_BAR_CODE_QUERY) —
       repeated here as a literal because a shadow-root <style> cannot import
       a JS string, not because this is a second convention. */
    @media (orientation: portrait) and (max-width: 599px), (orientation: landscape) and (max-height: 500px) {
      .binding { display: none; }
    }
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
  }

  #pointerId = null;

  connectedCallback() {
    super.connectedCallback();
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

    // Keyboard hold on the FOCUSED button (issue #1176): boost is a hold, not a
    // click, so a native <button> under focus would do nothing on Enter/Space.
    // A keydown/keyup pair mirrors the pointer hold onto the SAME set_boost
    // action (its own hold source, so it composes with Shift / pointer / pad
    // rather than fighting them), exactly as the blaster's hold-to-fire does.
    // `repeat` is ignored so autorepeat does not re-engage every tick, and a
    // blur releases a held key that would otherwise latch boost on.
    btn.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter' && e.key !== ' ' && e.key !== 'Spacebar') return;
      e.preventDefault();
      if (e.repeat) return;
      this.#hold('enter');
    });
    btn.addEventListener('keyup', (e) => {
      if (e.key !== 'Enter' && e.key !== ' ' && e.key !== 'Spacebar') return;
      e.preventDefault();
      this.#release('enter');
    });
    btn.addEventListener('blur', () => this.#release('enter'));

    // Shift and gamepad A are matched by the parent semantic input runtime.
    // This component owns only its pointer and focused-button hold sources.
  }

  disconnectedCallback() {
    this.#release('pointer');
    this.#release('enter');
  }

  /**
   * Engage boost from `source`. Returns false when the press was rejected
   * because boost is unavailable (recharging, or AUTO holds the controls).
   */
  #hold(source) {
    if (this.#holds.has(source)) return true;
    const btn = this.shadowRoot.getElementById('btn');
    const wasIdle = this.#holds.size === 0;
    if (wasIdle) {
      const activate = typeof window !== 'undefined' && window.activateSemanticAction;
      if (btn.disabled || typeof activate !== 'function') return false;
      const result = activate(HELM_BOOST_ACTION_ID, {
        context: HELM_ACTION_CONTEXT,
        source: 'control', detail: { holdSource: 'boost-button' }, pressed: true,
      });
      if (!result || result.handled !== true) return false;
    }
    this.#holds.add(source);
    return true;
  }

  /** Release `source`; sends the stop only once the last source lets go. */
  #release(source) {
    if (!this.#holds.delete(source)) return;
    if (this.#holds.size !== 0) return;
    const activate = typeof window !== 'undefined' && window.activateSemanticAction;
    if (typeof activate === 'function') {
      activate(HELM_BOOST_ACTION_ID, {
        context: HELM_ACTION_CONTEXT,
        source: 'control', detail: { holdSource: 'boost-button' }, pressed: false,
      });
    }
  }

  render(state) {
    const s = state || {};
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

phDefine('ph-boost-btn', PhBoostBtn);
