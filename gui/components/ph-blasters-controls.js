import { phAdoptConsoleStyles } from './ph-console-styles.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { weaponReadinessView } from '../weapon-readiness.js';

export class PhBlastersControls extends HTMLElement {
  #state = null;
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .bank-row { display: flex; flex-direction: column; gap: 0.2rem; padding: 0.3rem 0; }
    .bank-top { display: flex; align-items: center; gap: 0.4rem; font-size: var(--text-xs); }
    .bank-top .lbl { min-width: 2.5rem; color: var(--ink-dim); }
    .bar-wrap { flex: 1; height: 0.5rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .bar-fill { height: 100%; transition: width 0.15s ease; }
    .bar-fill.charge { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .bar-fill.cooldown { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; margin-left: 0.3rem; }
    .status { font-size: var(--text-xs); letter-spacing: 0.15em; min-width: 4.5rem; text-align: right; color: var(--ink-dim); }
    .bank-row.blocked .status { color: var(--fire); }
    .bank-row.unavailable .status { color: var(--ink-faint); }
    .bank-row.ready .status { color: var(--loaded); }
    .bar-row { display: flex; align-items: center; gap: 0.4rem; padding-left: 2.9rem; }
    .bar-label { font-size: var(--text-xs); color: var(--ink-dim); min-width: 2rem; }
    .pattern-row { display: flex; gap: 0.5rem; padding-left: 2.9rem; font-size: var(--text-xs); letter-spacing: 0.15em; color: var(--reloading); }
    .pattern-row.idle { display: none; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header"><span>${t('component.blasters.title')}</span></div>
  <div id="banks"></div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    phAdoptConsoleStyles(this.shadowRoot);
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
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = t('component.blasters.empty'); container.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

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
        badge.textContent = t('console.common.auto');
        top.appendChild(badge);
        const status = document.createElement('span');
        status.className = 'status';
        top.appendChild(status);
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'btn';
        btn.innerHTML = '<span class="btn-bg"></span><span class="led"></span><span class="label">' + t('component.blasters.charge') + '</span>';
        btn.addEventListener('mousedown', () => {
          if (!btn.disabled && this.sendAction) {
            this.sendAction('charge_blaster_start', { bank: bank.id });
          }
        });
        btn.addEventListener('mouseup', () => {
          if (!btn.disabled && this.sendAction) {
            this.sendAction('fire_blaster', { bank: bank.id });
          }
        });
        btn.addEventListener('mouseleave', () => {
          if (!btn.disabled && this.sendAction && bank.state === 'charging') {
            this.sendAction('fire_blaster', { bank: bank.id });
          }
        });
        btn.addEventListener('touchstart', (e) => {
          e.preventDefault();
          if (!btn.disabled && this.sendAction) {
            this.sendAction('charge_blaster_start', { bank: bank.id });
          }
        }, { passive: false });
        btn.addEventListener('touchend', (e) => {
          e.preventDefault();
          if (!btn.disabled && this.sendAction) {
            this.sendAction('fire_blaster', { bank: bank.id });
          }
        }, { passive: false });
        btn.addEventListener('touchcancel', () => {
          if (!btn.disabled && this.sendAction && bank.state === 'charging') {
            this.sendAction('fire_blaster', { bank: bank.id });
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

        // Patterned-attack indicator (issue #765): current pattern step +
        // active barrels. Hidden unless the bank has a multi-barrel pattern.
        const patternRow = document.createElement('div');
        patternRow.className = 'pattern-row idle';
        const stepEl = document.createElement('span');
        stepEl.className = 'pattern-step';
        const barrelsEl = document.createElement('span');
        barrelsEl.className = 'pattern-barrels';
        patternRow.appendChild(stepEl);
        patternRow.appendChild(barrelsEl);
        row.appendChild(patternRow);

        if (idx < container.children.length) {
          container.insertBefore(row, container.children[idx]);
        } else {
          container.appendChild(row);
        }
      }

      row.querySelector('.lbl').textContent = bank.label || bank.id || t('component.blasters.bank_fallback');

      // Wire type BlasterBankState (core/messages.rs): fire_ready / on_cooldown
      // / cooldown_remaining / charge_progress / has_charge / pending_volley.
      // There is no per-bank `state`, `_pct`, or `auto` field — derive display
      // state from these. cooldown_remaining has no wire denominator, so the
      // cooldown bar shows full while cooling (mirrors ph-phasers-controls).
      row.querySelector('.auto-badge').style.display = 'none';

      const isCooling = !!bank.on_cooldown;
      const hasCharge = !!bank.has_charge;
      const chargeProgress = Number(bank.charge_progress || 0);
      const isCharging = !isCooling && chargeProgress > 0;

      const btn = row.querySelector('.btn');
      // Shared blocking-reason path (issue #764). When the server publishes a
      // `readiness` contract it drives the block label (in `.status`) and the
      // button enablement; otherwise fall back to the cooldown-only derivation.
      const rv = weaponReadinessView(bank.readiness);
      const status = row.querySelector('.status');
      if (rv.present) {
        status.textContent = rv.label;
        row.className = 'bank-row ' + (rv.unavailable ? 'unavailable' : rv.ready ? 'ready' : 'blocked');
        // A charge in progress is a valid mid-fire state, not a block — keep the
        // button live so mouseup/touchend can release the shot.
        btn.disabled = !rv.ready && !isCharging;
      } else {
        status.textContent = '';
        row.className = 'bank-row';
        btn.disabled = isCooling;
      }
      // charging → amber (tactical) pill, cooling → dimmed/disabled, else armed.
      btn.className = 'btn ' + (isCharging ? 'tactical' : btn.disabled ? 'disabled' : 'armed');
      btn.querySelector('.led').className = 'led' + (isCharging ? ' amber keep' : btn.disabled ? '' : ' on');
      btn.querySelector('.label').textContent = isCharging ? t('component.blasters.firing')
        : isCooling ? t('console.common.cooldown')
        : hasCharge ? t('component.blasters.charge')
        : t('console.common.fire');

      const fills = row.querySelectorAll('.bar-fill');
      const chargeFill = fills[0];
      const cooldownFill = fills[1];
      const chargePct = Math.max(0, Math.min(100, chargeProgress * 100));

      if (isCharging) {
        chargeFill.style.width = chargePct + '%';
        chargeFill.className = 'bar-fill charge';
        chargeFill.style.display = 'block';
        cooldownFill.style.display = 'none';
        row.querySelector('.bar-label').textContent = t('component.blasters.charge');
      } else if (isCooling) {
        cooldownFill.style.width = '100%';
        cooldownFill.className = 'bar-fill cooldown';
        cooldownFill.style.display = 'block';
        chargeFill.style.display = 'none';
        row.querySelector('.bar-label').textContent = t('console.common.cooldown');
      } else {
        chargeFill.style.display = 'none';
        cooldownFill.style.display = 'none';
        row.querySelector('.bar-label').textContent = '';
      }

      // Patterned-attack indicator (issue #765). `pattern_len > 0` marks a
      // multi-barrel patterned bank; show which step is active and which
      // barrels are currently firing.
      const patternRow = row.querySelector('.pattern-row');
      const patternLen = Number(bank.pattern_len || 0);
      const patternStep = Number(bank.pattern_step || 0);
      const activeBarrels = Array.isArray(bank.active_barrels) ? bank.active_barrels : [];
      if (patternLen > 0 && patternStep > 0) {
        patternRow.className = 'pattern-row';
        patternRow.querySelector('.pattern-step').textContent = t('component.blasters.pattern_step', {
          step: patternStep,
          total: patternLen,
        });
        patternRow.querySelector('.pattern-barrels').textContent = activeBarrels.length
          ? t('component.blasters.barrels', { barrels: activeBarrels.join(',') })
          : '';
      } else {
        patternRow.className = 'pattern-row idle';
        patternRow.querySelector('.pattern-step').textContent = '';
        patternRow.querySelector('.pattern-barrels').textContent = '';
      }
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-blasters-controls')) {
  customElements.define('ph-blasters-controls', PhBlastersControls);
}
