// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { weaponReadinessView } from '../weapon-readiness.js';
import { cooldownRemainingPercent } from '../weapon-cooldown.js';
import { installRovingTabindex, syncRovingTabindex } from '../roving-tabindex.js';
import { PhElement, phDefine } from './ph-element.js';
import {
  TACTICAL_PHASER_FIRE_ACTION_ID,
  TACTICAL_PHASER_MODE_ACTION_ID,
} from '../stations/tactical-actions.js';
import { activateTacticalAction } from '../stations/tactical-action-control.js';

export class PhPhasersControls extends PhElement {
  #roving = null;

  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    /* flex-wrap (issue #1378): the codebase convention for a row that does
       not fit its column at phone width (see gui/components/ph-navigation-map.js,
       issue #1379) rather than a width-based breakpoint, which no other
       console in the repo uses. The row's natural wrap point cannot be left
       to the browser, though: with the label+bar+badge+status+button all as
       flat siblings, the browser breaks wherever the running width happens
       to overflow, which — with any non-empty blocking-reason label, i.e.
       every non-Ready bank including the default no-lock state — lands
       after '.status' rather than before it, producing three lines instead
       of two. '.bank-line-top' (label + bar) and '.bank-line-bottom' (badge
       + status + button) each force a full flex line via 'flex: 1 1 100%',
       so the row always wraps to exactly two lines regardless of label
       length. '.cooldown-wrap' keeps a real minimum width so it stays a
       visible bar rather than collapsing to nothing before the row wraps;
       'row-gap' gives the second line the same breathing room as the row's
       own horizontal gap. '.btn' (the FIRE button) keeps its 44px floor from
       the shared control family on either line. */
    .bank-row { display: flex; flex-wrap: wrap; row-gap: 0.3rem; gap: 0.4rem; font-size: var(--text-xs); padding: 0.3rem 0; }
    .bank-line-top, .bank-line-bottom { display: flex; align-items: center; gap: 0.4rem; flex: 1 1 100%; min-width: 0; }
    .bank-line-bottom { justify-content: space-between; }
    .bank-row .lbl { min-width: 2.5rem; color: var(--ink-dim); }
    .cooldown-wrap { flex: 1 1 4rem; min-width: 4rem; height: 0.5rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .cooldown-fill { height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: width 0.3s ease; }
    .cooldown-fill.cooling { background: linear-gradient(90deg, var(--fire-dim), var(--fire)); }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; margin-left: 0.3rem; }
    .status { font-size: var(--text-xs); letter-spacing: 0.15em; min-width: 4.5rem; text-align: right; color: var(--ink-dim); }
    .bank-row.blocked .status { color: var(--fire); }
    .bank-row.unavailable .status { color: var(--ink-faint); }
    .bank-row.ready .status { color: var(--loaded); }
    .mode-toggle { font-family: 'Chakra Petch', sans-serif; font-size: var(--text-xs); font-weight: 700; padding: 0.1rem 0.5rem; letter-spacing: 0.15em; text-transform: uppercase; cursor: pointer; border: 1px solid var(--ink-dim); color: var(--ink-dim); background: var(--bg-card); transition: all 0.15s ease; min-height: var(--control-hit-min); }
    .mode-toggle:hover { border-color: var(--ink); color: var(--ink); }
    .mode-toggle.auto { border-color: var(--reloading); color: var(--reloading); }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header"><span>${t('component.phasers.title')}</span><button class="mode-toggle" id="mode-toggle" type="button">${t('component.phasers.manual')}</button></div>
  <div id="banks"></div>
`;
  }

  connectedCallback() {
    super.connectedCallback();
    // Role + accessible name (issue #1170). The panel is a toolbar — a group of
    // command buttons the arrow keys rove between; its name is the same string
    // its visible heading already shows, so the two never drift.
    this.setAttribute('role', 'toolbar');
    this.setAttribute('aria-orientation', 'vertical');
    this.setAttribute('aria-label', t('component.phasers.title'));
    const toggle = this.shadowRoot.getElementById('mode-toggle');
    toggle.addEventListener('click', () => {
      const mode = (this.state && this.state.mode) || 'Auto';
      // Flip between the two operator modes; Auto = banks fire themselves.
      activateTacticalAction(this, TACTICAL_PHASER_MODE_ACTION_ID,
        { mode: mode === 'Auto' ? 'Manual' : 'Auto' }, 'set_phaser_mode');
    });
    // Roving tabindex over the mode toggle + each bank's FIRE button (issue
    // #1170): one Tab stop for the whole toolbar, arrows between its controls.
    // Native buttons keep their own Enter/Space activation — no fork.
    this.#roving ??= installRovingTabindex(this, {
      getItems: () => this.#rovingItems(),
      orientation: 'vertical',
    });
    this.#syncRoving();
  }

  /** The toolbar's rovable controls, in visual order. */
  #rovingItems() {
    const toggle = this.shadowRoot.getElementById('mode-toggle');
    const fires = Array.from(this.shadowRoot.querySelectorAll('#banks .btn'));
    return [toggle, ...fires].filter(Boolean);
  }

  /** Re-establish the single tab stop after a render adds/removes banks. */
  #syncRoving() {
    syncRovingTabindex(this.#rovingItems());
  }

  render(state) {
    const s = state || {};
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
      this.#syncRoving();
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

        const lineTop = document.createElement('div');
        lineTop.className = 'bank-line-top';
        const lbl = document.createElement('span');
        lbl.className = 'lbl';
        lineTop.appendChild(lbl);
        const wrap = document.createElement('div');
        wrap.className = 'cooldown-wrap';
        const fill = document.createElement('div');
        fill.className = 'cooldown-fill';
        wrap.appendChild(fill);
        lineTop.appendChild(wrap);
        row.appendChild(lineTop);

        const lineBottom = document.createElement('div');
        lineBottom.className = 'bank-line-bottom';
        const badge = document.createElement('span');
        badge.className = 'auto-badge';
        badge.textContent = t('console.common.auto');
        lineBottom.appendChild(badge);
        const status = document.createElement('span');
        status.className = 'status';
        lineBottom.appendChild(status);
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'btn';
        btn.innerHTML = '<span class="btn-bg"></span><span class="led"></span><span class="label">' + t('console.common.fire') + '</span>';
        btn.addEventListener('click', () => {
          if (!btn.disabled) {
            activateTacticalAction(this, TACTICAL_PHASER_FIRE_ACTION_ID,
              { bank: bank.id }, 'fire_phaser');
          }
        });
        lineBottom.appendChild(btn);
        row.appendChild(lineBottom);

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
      // A legacy payload without an authored duration keeps the binary cue;
      // otherwise the existing CSS transition follows each server fraction.
      fill.style.width = (onCooldown ? (cooldownRemainingPercent(bank) ?? 100)
        : (fireReady ? 100 : 0)) + '%';
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

    // Banks were reconciled above; keep the toolbar's single tab stop over the
    // new button set (issue #1170).
    this.#syncRoving();
  }
}

phDefine('ph-phasers-controls', PhPhasersControls);
