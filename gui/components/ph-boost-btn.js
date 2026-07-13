export class PhBoostBtn extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; margin-bottom: 0.4rem; }
    .auto-badge { font-size: 0.6rem; color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    .btn { width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: 0.9rem; font-weight: 700; padding: 0.7rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 2px solid; transition: all 0.15s ease; }
    .btn.available { background: var(--bg-card); border-color: var(--loaded); color: var(--loaded); }
    .btn.available:hover:not(:disabled) { background: var(--loaded-dim); }
    .btn.active { background: #0a2a1a; border-color: var(--loaded); color: var(--loaded); text-shadow: 0 0 8px rgba(78,200,112,0.5); }
    .btn.recharging { background: var(--bg-card); border-color: var(--ink-dim); color: var(--ink-dim); }
    .btn:disabled { opacity: 0.4; cursor: default; }
    .recharge-wrap { width: 100%; height: 0.4rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; margin-bottom: 0.3rem; }
    .recharge-fill { height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.3s ease; }
    .recharge-fill.draining { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
  </style>
  <div class="header">
    <span>BOOST</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
  </div>
  <div class="recharge-wrap" id="recharge-wrap" style="display:none">
    <div class="recharge-fill" id="recharge-fill" style="width:100%"></div>
  </div>
  <button class="btn available" id="btn">BOOST</button>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    const btn = this.shadowRoot.getElementById('btn');
    let pointerId = null;
    btn.addEventListener('pointerdown', (e) => {
      if (pointerId !== null || !this.sendAction || btn.disabled) return;
      pointerId = e.pointerId;
      if (btn.setPointerCapture) btn.setPointerCapture(e.pointerId);
      this.sendAction('set_boost', { active: true });
      e.preventDefault();
    });
    const release = (e) => {
      if (e.pointerId !== pointerId) return;
      pointerId = null;
      try { if (btn.releasePointerCapture) btn.releasePointerCapture(e.pointerId); } catch (_) {}
      if (this.sendAction) this.sendAction('set_boost', { active: false });
    };
    btn.addEventListener('pointerup', release);
    btn.addEventListener('pointercancel', release);
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
      btn.textContent = batteryPct < 100 ? 'BOOSTING ' + Math.round(batteryPct) + '%' : 'BOOSTING';
      btn.className = 'btn active';
      btn.disabled = false;
    } else if (recharging) {
      btn.textContent = 'RECHARGING ' + Math.round(batteryPct) + '%';
      btn.className = 'btn recharging';
      btn.disabled = true;
    } else {
      btn.textContent = 'BOOST';
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
