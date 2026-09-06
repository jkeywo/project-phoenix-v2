// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import {
  CAPTAIN_ACTION_CONTEXT,
  CAPTAIN_RED_ALERT_ACTION_ID,
} from '../stations/captain-actions.js';
import { PhElement, phDefine } from './ph-element.js';

// Red Alert, and Red Alert only.
//
// It carried a second button from issue #1041 to #1398 — the captain's Weapons
// Hold. That lever is retired: restraint is expressed by POWER now, so the
// control that expresses it is Engineering's `ph-power-controls` (a `weapons`
// group commanded to LVL 0, tagged COLD), and the Captain's console has one
// posture lever rather than two that had to be read together.
export class PhRedAlert extends PhElement {
  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    /* The round alert control (issue #1377). A captain finds Red Alert by
       shape as much as by reading it — the one circle on a bridge of
       rectangles — so it stops being a full-width bar like every other
       control here. .alert-row centres it in the host's width rather than
       stretching it, and the diameter is min(host width, 7.5rem) via
       width:100%/max-width plus aspect-ratio, floored at the
       --control-hit-min custom property so a squeezed rail never drops it
       below the touch minimum. STAND DOWN / RED ALERT wrap onto two lines
       inside it — white-space:normal, a smaller face size than the old bar
       carried — because a single line of either string does not fit the
       circle's narrower waist; the strings themselves are untouched. */
    .alert-row { display: flex; justify-content: center; padding: 0.15rem 0; }
    .alert-btn {
      width: 100%;
      max-width: 7.5rem;
      min-width: var(--control-hit-min);
      /* Redundant with aspect-ratio (a circle's height already tracks its
         floored width) but declared explicitly: the control-floors touch-
         floor test reads the CSS text statically, on both axes, and does not
         evaluate aspect-ratio to infer one axis from the other. */
      min-height: var(--control-hit-min);
      aspect-ratio: 1 / 1;
      border-radius: 50%;
      display: flex;
      align-items: center;
      justify-content: center;
      text-align: center;
      padding: 0.6rem;
      font-family: 'Chakra Petch', sans-serif;
      font-size: var(--text-xs);
      font-weight: 700;
      line-height: 1.2;
      letter-spacing: 0.1em;
      text-transform: uppercase;
      white-space: normal;
      cursor: pointer;
      border: 3px solid;
      transition: all 0.15s ease;
    }
    .alert-btn.standby { background: var(--bg-card); border-color: var(--line-faint); color: var(--ink-dim); }
    .alert-btn.standby:hover:not(:disabled) { background: var(--cyan-deep); color: var(--ink-dim); }
    .alert-btn.active { background: var(--fire-deep); border-color: var(--fire); color: var(--fire); text-shadow: 0 0 8px rgba(var(--rgb-fire), 0.5); }
    .alert-btn.active:hover:not(:disabled) { background: var(--tactical-deep); }
    .alert-btn:disabled { opacity: 0.4; cursor: default; }
    .feedback-status { min-height: 1.2em; color: var(--ink); font-size: var(--text-xs); letter-spacing: 0.12em; text-transform: uppercase; }
    .feedback-status[data-state="Refused"], .feedback-status[data-state="TimedOut"] { color: var(--fire); }
  </style>
  <div class="header">
    <span>${t('component.red_alert.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div class="alert-row">
    <button class="alert-btn standby" id="alert-btn">${t('component.red_alert.standby')}</button>
  </div>
  <span class="feedback-status" id="feedback-status" role="status" aria-live="polite" aria-atomic="true"></span>
`;
  }

  connectedCallback() {
    super.connectedCallback();
    // One action, one lane. The per-action map this used to keep existed only
    // because Weapons Hold shared the component (issue #1041); #1398 retired it.
    this._feedback = null;
    this._onFeedback = (event) => {
      const value = event && event.detail;
      if (!value
          || value.actionId !== CAPTAIN_RED_ALERT_ACTION_ID
          || value.isCurrent === false) return;
      this._feedback = value.cancelled || !value.statusId ? null : value;
      this._renderFeedback();
    };
    if (typeof window !== 'undefined') {
      window.addEventListener('phoenix-action-feedback', this._onFeedback);
    }
    const btn = this.shadowRoot.getElementById('alert-btn');
    btn.addEventListener('click', () => {
      if (btn.disabled) return;
      const activate = typeof window !== 'undefined' && window.activateSemanticAction;
      if (typeof activate === 'function') {
        activate(CAPTAIN_RED_ALERT_ACTION_ID, {
          context: CAPTAIN_ACTION_CONTEXT,
          source: 'control',
        });
      }
    });
  }

  disconnectedCallback() {
    if (typeof window !== 'undefined' && this._onFeedback) {
      window.removeEventListener('phoenix-action-feedback', this._onFeedback);
    }
    this._onFeedback = null;
  }

  _renderFeedback() {
    const status = this.shadowRoot.getElementById('feedback-status');
    const btn = this.shadowRoot.getElementById('alert-btn');
    if (!status || !btn) return;
    const value = this._feedback;
    status.textContent = value && value.statusId ? t(value.statusId) : '';
    status.dataset.state = value && value.state ? value.state : '';
    if (value && value.state === 'Pending') {
      btn.setAttribute('aria-busy', 'true');
    } else btn.removeAttribute('aria-busy');
  }

  render(state) {
    const s = state || {};
    const active = !!s.active;
    const auto = !!s.auto;
    const root = this.shadowRoot;
    const btn = root.getElementById('alert-btn');

    btn.textContent = active ? t('component.red_alert.active') : t('component.red_alert.standby');
    btn.className = 'alert-btn' + (active ? ' active' : ' standby');
    btn.disabled = auto;
    this._renderFeedback();

    root.getElementById('auto-badge').style.display = auto ? 'inline' : 'none';
  }
}

phDefine('ph-red-alert', PhRedAlert);
