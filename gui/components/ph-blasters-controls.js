export class PhBlastersControls extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: #6a7178; text-transform: uppercase; }
    .bank-row { display: flex; flex-direction: column; gap: 0.2rem; padding: 0.3rem 0; }
    .bank-top { display: flex; align-items: center; gap: 0.4rem; font-size: 0.65rem; }
    .bank-top .lbl { min-width: 2.5rem; color: #6a7178; }
    .bar-wrap { flex: 1; height: 0.5rem; background: #05080e; border: 1px solid #282c38; overflow: hidden; }
    .bar-fill { height: 100%; transition: width 0.15s ease; }
    .bar-fill.charge { background: linear-gradient(90deg, #805818, #f0c040); }
    .bar-fill.cooldown { background: linear-gradient(90deg, #6a1a12, #e0402c); }
    .charge-btn { font-family: 'Chakra Petch', sans-serif; font-size: 0.6rem; font-weight: 700; padding: 0.3rem 0.8rem; letter-spacing: 0.15em; text-transform: uppercase; cursor: pointer; border: 2px solid #4ec870; color: #4ec870; background: #0e1117; transition: all 0.15s ease; touch-action: manipulation; }
    .charge-btn:hover:not(:disabled) { background: #16281d; }
    .charge-btn:disabled { opacity: 0.35; border-color: #6a7178; color: #6a7178; cursor: default; }
    .charge-btn.charging { background: #2a1a0a; border-color: #f0c040; color: #f0c040; }
    .auto-badge { font-size: 0.55rem; color: #f0c040; border: 1px solid #f0c040; padding: 0.05rem 0.3rem; letter-spacing: 0.2em; margin-left: 0.3rem; }
    .bar-row { display: flex; align-items: center; gap: 0.4rem; padding-left: 2.9rem; }
    .bar-label { font-size: 0.55rem; color: #6a7178; min-width: 2rem; }
    .empty { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header"><span>BLASTERS</span></div>
  <div id="banks"></div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
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
    const banks = Array.isArray(s.banks) ? s.banks : [];
    const container = this.shadowRoot.getElementById('banks');

    if (banks.length === 0) {
      container.innerHTML = '<div class="empty">NO BLASTER BANKS</div>';
      return;
    }

    const newIds = new Set(banks.map(b => b.id));
    Array.from(container.children).forEach(child => {
      if (!newIds.has(child.dataset.id)) {
        child.remove();
      }
    });

    banks.forEach((bank, idx) => {
      let row = container.querySelector(`[data-id="${bank.id}"]`);
      if (!row) {
        row = document.createElement('div');
        row.className = 'bank-row';
        row.dataset.id = bank.id;

        const top = document.createElement('div');
        top.className = 'bank-top';
        const lbl = document.createElement('span');
        lbl.className = 'lbl';
        top.appendChild(lbl);
        const wrap = document.createElement('div');
        wrap.className = 'bar-wrap';
        const fill = document.createElement('div');
        fill.className = 'bar-fill';
        wrap.appendChild(fill);
        top.appendChild(wrap);
        const badge = document.createElement('span');
        badge.className = 'auto-badge';
        badge.textContent = 'AUTO';
        top.appendChild(badge);
        const btn = document.createElement('button');
        btn.className = 'charge-btn';
        btn.textContent = 'CHARGE';
        btn.addEventListener('mousedown', () => {
          if (!btn.disabled && this.sendAction) {
            this.sendAction('charge_blaster_start', { bank_id: bank.id });
          }
        });
        btn.addEventListener('mouseup', () => {
          if (!btn.disabled && this.sendAction) {
            this.sendAction('fire_blaster', { bank_id: bank.id });
          }
        });
        btn.addEventListener('mouseleave', () => {
          if (!btn.disabled && this.sendAction && bank.state === 'charging') {
            this.sendAction('fire_blaster', { bank_id: bank.id });
          }
        });
        btn.addEventListener('touchstart', (e) => {
          e.preventDefault();
          if (!btn.disabled && this.sendAction) {
            this.sendAction('charge_blaster_start', { bank_id: bank.id });
          }
        }, { passive: false });
        btn.addEventListener('touchend', (e) => {
          e.preventDefault();
          if (!btn.disabled && this.sendAction) {
            this.sendAction('fire_blaster', { bank_id: bank.id });
          }
        }, { passive: false });
        btn.addEventListener('touchcancel', () => {
          if (!btn.disabled && this.sendAction && bank.state === 'charging') {
            this.sendAction('fire_blaster', { bank_id: bank.id });
          }
        });
        top.appendChild(btn);
        row.appendChild(top);

        const barRow = document.createElement('div');
        barRow.className = 'bar-row';
        const barLabel = document.createElement('span');
        barLabel.className = 'bar-label';
        barRow.appendChild(barLabel);
        const barWrap2 = document.createElement('div');
        barWrap2.className = 'bar-wrap';
        const barFill2 = document.createElement('div');
        barFill2.className = 'bar-fill';
        barWrap2.appendChild(barFill2);
        barRow.appendChild(barWrap2);
        row.appendChild(barRow);

        if (idx < container.children.length) {
          container.insertBefore(row, container.children[idx]);
        } else {
          container.appendChild(row);
        }
      }

      row.querySelector('.lbl').textContent = bank.label || bank.id;

      const auto = !!bank.auto;
      const badge = row.querySelector('.auto-badge');
      badge.style.display = auto ? 'inline' : 'none';

      const btn = row.querySelector('.charge-btn');
      const isCharging = bank.state === 'charging';
      const isCooling = bank.state === 'cooling';
      const disabled = auto || isCooling;
      btn.disabled = disabled;
      btn.className = 'charge-btn' + (isCharging ? ' charging' : '');
      btn.textContent = isCharging ? 'FIRING...' : isCooling ? 'COOLDOWN' : 'CHARGE';

      const fills = row.querySelectorAll('.bar-fill');
      const chargeFill = fills[0];
      const cooldownFill = fills[1];
      const chargePct = Math.max(0, Math.min(100, (bank.charge_pct || 0) * 100));
      const cooldownPct = Math.max(0, Math.min(100, (bank.cooldown_pct || 0) * 100));

      if (isCharging) {
        chargeFill.style.width = chargePct + '%';
        chargeFill.className = 'bar-fill charge';
        chargeFill.style.display = 'block';
        cooldownFill.style.display = 'none';
        row.querySelector('.bar-label').textContent = 'CHARGE';
      } else if (isCooling) {
        cooldownFill.style.width = cooldownPct + '%';
        cooldownFill.className = 'bar-fill cooldown';
        cooldownFill.style.display = 'block';
        chargeFill.style.display = 'none';
        row.querySelector('.bar-label').textContent = 'COOLDOWN';
      } else {
        chargeFill.style.display = 'none';
        cooldownFill.style.display = 'none';
        row.querySelector('.bar-label').textContent = '';
      }
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-blasters-controls')) {
  customElements.define('ph-blasters-controls', PhBlastersControls);
}
