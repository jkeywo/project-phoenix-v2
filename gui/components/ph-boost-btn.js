export class PhBoostBtn extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: #6a7178; text-transform: uppercase; margin-bottom: 0.4rem; }
    .auto-badge { font-size: 0.6rem; color: #f0c040; border: 1px solid #f0c040; padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    .btn { width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: 0.9rem; font-weight: 700; padding: 0.7rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 2px solid; transition: all 0.15s ease; }
    .btn.available { background: #0e1117; border-color: #4ec870; color: #4ec870; }
    .btn.available:hover:not(:disabled) { background: #16281d; }
    .btn.active { background: #0a2a1a; border-color: #4ec870; color: #4ec870; text-shadow: 0 0 8px rgba(78,200,112,0.5); }
    .btn.recharging { background: #0e1117; border-color: #6a7178; color: #6a7178; }
    .btn:disabled { opacity: 0.4; cursor: default; }
    .recharge-wrap { width: 100%; height: 0.4rem; background: #05080e; border: 1px solid #282c38; overflow: hidden; margin-bottom: 0.3rem; }
    .recharge-fill { height: 100%; background: linear-gradient(90deg, #2a6838, #4ec870); transition: width 0.3s ease; }
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
    btn.addEventListener('click', () => {
      if (this.sendAction && !btn.disabled) {
        this.sendAction('toggle_boost', {});
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
    const available = s.available !== false;
    const active = !!s.active;
    const rechargePct = s.recharge_pct != null ? Math.max(0, Math.min(100, Number(s.recharge_pct))) : 100;
    const auto = !!s.auto;

    const root = this.shadowRoot;
    const btn = root.getElementById('btn');
    const rechargeWrap = root.getElementById('recharge-wrap');
    const rechargeFill = root.getElementById('recharge-fill');
    const badge = root.getElementById('auto-badge');

    const recharging = !available || (rechargePct < 100 && !active);

    if (active) {
      btn.textContent = 'BOOSTING';
      btn.className = 'btn active';
      btn.disabled = false;
    } else if (recharging && rechargePct < 100) {
      btn.textContent = 'RECHARGING ' + Math.round(rechargePct) + '%';
      btn.className = 'btn recharging';
      btn.disabled = true;
    } else {
      btn.textContent = 'BOOST';
      btn.className = 'btn available';
      btn.disabled = auto;
    }

    // Recharge bar visible when recharging
    if (recharging && rechargePct < 100) {
      rechargeWrap.style.display = 'block';
      rechargeFill.style.width = rechargePct + '%';
    } else {
      rechargeWrap.style.display = 'none';
    }

    badge.style.display = auto ? 'inline' : 'none';
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-boost-btn')) {
  customElements.define('ph-boost-btn', PhBoostBtn);
}
