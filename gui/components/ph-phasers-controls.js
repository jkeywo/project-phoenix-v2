export class PhPhasersControls extends HTMLElement {
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
    .bank-row { display: flex; align-items: center; gap: 0.4rem; font-size: 0.65rem; padding: 0.3rem 0; }
    .bank-row .lbl { min-width: 2.5rem; color: #6a7178; }
    .cooldown-wrap { flex: 1; height: 0.5rem; background: #05080e; border: 1px solid #282c38; overflow: hidden; }
    .cooldown-fill { height: 100%; background: linear-gradient(90deg, #2a6838, #4ec870); transition: width 0.3s ease; }
    .cooldown-fill.cooling { background: linear-gradient(90deg, #6a1a12, #e0402c); }
    .fire-btn { font-family: 'Chakra Petch', sans-serif; font-size: 0.6rem; font-weight: 700; padding: 0.25rem 0.6rem; letter-spacing: 0.15em; text-transform: uppercase; cursor: pointer; border: 2px solid #4ec870; color: #4ec870; background: #0e1117; transition: all 0.15s ease; }
    .fire-btn:hover:not(:disabled) { background: #16281d; }
    .fire-btn:disabled { opacity: 0.35; border-color: #6a7178; color: #6a7178; cursor: default; }
    .auto-badge { font-size: 0.55rem; color: #f0c040; border: 1px solid #f0c040; padding: 0.05rem 0.3rem; letter-spacing: 0.2em; margin-left: 0.3rem; }
    .mode-toggle { font-family: 'Chakra Petch', sans-serif; font-size: 0.55rem; font-weight: 700; padding: 0.1rem 0.5rem; letter-spacing: 0.15em; text-transform: uppercase; cursor: pointer; border: 1px solid #6a7178; color: #6a7178; background: #0e1117; transition: all 0.15s ease; }
    .mode-toggle:hover { border-color: #aab; color: #aab; }
    .mode-toggle.auto { border-color: #f0c040; color: #f0c040; }
    .empty { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header"><span>PHASERS</span><button class="mode-toggle" id="mode-toggle" type="button">MANUAL</button></div>
  <div id="banks"></div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    const toggle = this.shadowRoot.getElementById('mode-toggle');
    toggle.addEventListener('click', () => {
      if (!this.sendAction) return;
      const mode = (this.#state && this.#state.mode) || 'Auto';
      // Flip between the two operator modes; Auto = banks fire themselves.
      this.sendAction('set_phaser_mode', { mode: mode === 'Auto' ? 'Manual' : 'Auto' });
    });
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const banks = Array.isArray(s.banks) ? s.banks : [];
    const targetValid = s.target_valid !== false;
    const mode = s.mode || 'Auto';
    const auto = mode === 'Auto';
    const container = this.shadowRoot.getElementById('banks');

    const toggle = this.shadowRoot.getElementById('mode-toggle');
    toggle.textContent = auto ? 'AUTO' : 'MANUAL';
    toggle.className = 'mode-toggle' + (auto ? ' auto' : '');

    if (banks.length === 0) {
      container.innerHTML = '<div class="empty">NO PHASER BANKS</div>';
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
        const lbl = document.createElement('span');
        lbl.className = 'lbl';
        row.appendChild(lbl);
        const wrap = document.createElement('div');
        wrap.className = 'cooldown-wrap';
        const fill = document.createElement('div');
        fill.className = 'cooldown-fill';
        wrap.appendChild(fill);
        row.appendChild(wrap);
        const badge = document.createElement('span');
        badge.className = 'auto-badge';
        badge.textContent = 'AUTO';
        row.appendChild(badge);
        const btn = document.createElement('button');
        btn.className = 'fire-btn';
        btn.textContent = 'FIRE';
        btn.addEventListener('click', () => {
          if (this.sendAction && !btn.disabled) {
            this.sendAction('fire_phaser', { bank: bank.id });
          }
        });
        row.appendChild(btn);
        if (idx < container.children.length) {
          container.insertBefore(row, container.children[idx]);
        } else {
          container.appendChild(row);
        }
      }

      row.querySelector('.lbl').textContent = bank.label || bank.id || 'BANK';

      // Wire type is `PhaserBankState` (core/messages.rs): fire_ready /
      // on_cooldown / cooldown_remaining — there is no per-bank `cooldown_pct`
      // or `state`. Auto/Manual is a ship-level mode, shown via the header
      // toggle rather than a per-bank badge.
      const onCooldown = !!bank.on_cooldown;
      const fireReady = !!bank.fire_ready;
      const fill = row.querySelector('.cooldown-fill');
      fill.style.width = (onCooldown ? 100 : (fireReady ? 100 : 0)) + '%';
      fill.className = 'cooldown-fill' + (onCooldown ? ' cooling' : '');

      row.querySelector('.auto-badge').style.display = 'none';

      const btn = row.querySelector('.fire-btn');
      btn.disabled = auto || !targetValid || !fireReady || onCooldown;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-phasers-controls')) {
  customElements.define('ph-phasers-controls', PhPhasersControls);
}
