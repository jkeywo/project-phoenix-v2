export class PhImpulseBtn extends HTMLElement {
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
    .btn.ready { background: #0e1117; border-color: #4ec870; color: #4ec870; }
    .btn.ready:hover:not(:disabled) { background: #16281d; }
    .btn.charging { background: #0e1117; border-color: #d8a040; color: #d8a040; }
    .btn.cooldown { background: #0e1117; border-color: #6a7178; color: #6a7178; }
    .btn:disabled { opacity: 0.4; cursor: default; }
    .progress-wrap { width: 100%; height: 0.4rem; background: #05080e; border: 1px solid #282c38; overflow: hidden; margin-bottom: 0.3rem; }
    .progress-fill { height: 100%; background: linear-gradient(90deg, #d8a040, #f0c040); transition: width 0.3s ease; }
  </style>
  <div class="header">
    <span>IMPULSE DRIVE</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
  </div>
  <div class="progress-wrap" id="progress-wrap" style="display:none">
    <div class="progress-fill" id="progress-fill" style="width:0%"></div>
  </div>
  <button class="btn ready" id="btn">IMPULSE</button>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    const btn = this.shadowRoot.getElementById('btn');
    btn.addEventListener('click', () => {
      if (this.sendAction && !btn.disabled) {
        this.sendAction('start_impulse_charge', {});
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
    const st = s.state || 'ready';
    const chargePct = s.charge_pct != null ? Math.max(0, Math.min(100, Number(s.charge_pct))) : 0;
    const auto = !!s.auto;

    const root = this.shadowRoot;
    const btn = root.getElementById('btn');
    const progressWrap = root.getElementById('progress-wrap');
    const progressFill = root.getElementById('progress-fill');
    const badge = root.getElementById('auto-badge');

    // Button text and classes
    if (st === 'ready') {
      btn.textContent = 'IMPULSE';
      btn.className = 'btn ready';
      btn.disabled = auto;
    } else if (st === 'charging') {
      btn.textContent = 'CHARGING ' + Math.round(chargePct) + '%';
      btn.className = 'btn charging';
      btn.disabled = true;
    } else if (st === 'cooldown') {
      btn.textContent = 'COOLDOWN';
      btn.className = 'btn cooldown';
      btn.disabled = true;
    }

    // Progress bar: visible during charging
    if (st === 'charging') {
      progressWrap.style.display = 'block';
      progressFill.style.width = chargePct + '%';
    } else {
      progressWrap.style.display = 'none';
    }

    badge.style.display = auto ? 'inline' : 'none';
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-impulse-btn')) {
  customElements.define('ph-impulse-btn', PhImpulseBtn);
}
