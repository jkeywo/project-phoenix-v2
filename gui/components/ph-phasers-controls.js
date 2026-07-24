import { phAdoptConsoleStyles } from './ph-console-styles.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { weaponReadinessView } from '../weapon-readiness.js';

export class PhPhasersControls extends HTMLElement {
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
    .bank-row { display: flex; align-items: center; gap: 0.4rem; font-size: 0.65rem; padding: 0.3rem 0; }
    .bank-row .lbl { min-width: 2.5rem; color: var(--ink-dim); }
    .cooldown-wrap { flex: 1; height: 0.5rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .cooldown-fill { height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.3s ease; }
    .cooldown-fill.cooling { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .auto-badge { font-size: 0.55rem; color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; margin-left: 0.3rem; }
    .status { font-size: 0.5rem; letter-spacing: 0.15em; min-width: 4.5rem; text-align: right; color: var(--ink-dim); }
    .bank-row.blocked .status { color: var(--fire); }
    .bank-row.unavailable .status { color: var(--ink-faint); }
    .bank-row.ready .status { color: var(--loaded); }
    .mode-toggle { font-family: 'Chakra Petch', sans-serif; font-size: 0.55rem; font-weight: 700; padding: 0.1rem 0.5rem; letter-spacing: 0.15em; text-transform: uppercase; cursor: pointer; border: 1px solid var(--ink-dim); color: var(--ink-dim); background: var(--bg-card); transition: all 0.15s ease; }
    .mode-toggle:hover { border-color: var(--ink); color: var(--ink); }
    .mode-toggle.auto { border-color: var(--reloading); color: var(--reloading); }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header"><span>${t('component.phasers.title')}</span><button class="mode-toggle" id="mode-toggle" type="button">${t('component.phasers.manual')}</button></div>
  <div id="banks"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    phAdoptConsoleStyles(this.shadowRoot);
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
    toggle.textContent = auto ? t('console.common.auto') : t('component.phasers.manual');
    toggle.className = 'mode-toggle' + (auto ? ' auto' : '');

    if (banks.length === 0) {
      container.innerHTML = '<div class="empty">' + t('component.phasers.empty') + '</div>';
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
        badge.textContent = t('console.common.auto');
        row.appendChild(badge);
        const status = document.createElement('span');
        status.className = 'status';
        row.appendChild(status);
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'btn';
        btn.innerHTML = '<span class="btn-bg"></span><span class="led"></span><span class="label">' + t('console.common.fire') + '</span>';
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

      row.querySelector('.lbl').textContent = bank.label || bank.id || t('component.phasers.bank_fallback');

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

      // Shared blocking-reason path (issue #764). When the server publishes a
      // `readiness` contract, it is authoritative for the block label + button
      // enablement; without it (legacy states) fall back to the old derivation.
      const rv = weaponReadinessView(bank.readiness);
      const btn = row.querySelector('.btn');
      const status = row.querySelector('.status');
      let ready;
      if (rv.present) {
        ready = !auto && rv.ready;
        // In Auto mode the bank fires itself; show AUTO rather than a block.
        status.textContent = auto ? t('console.common.auto') : rv.label;
        row.className = 'bank-row ' + (auto ? '' : rv.unavailable ? 'unavailable' : rv.ready ? 'ready' : 'blocked');
      } else {
        ready = !auto && targetValid && fireReady && !onCooldown;
        status.textContent = '';
        row.className = 'bank-row';
      }
      btn.disabled = !ready;
      btn.className = 'btn' + (ready ? ' armed' : ' disabled');
      btn.querySelector('.led').className = 'led' + (ready ? ' on' : '');
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-phasers-controls')) {
  customElements.define('ph-phasers-controls', PhPhasersControls);
}
