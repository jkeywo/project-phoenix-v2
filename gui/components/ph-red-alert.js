// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import {
  CAPTAIN_ACTION_CONTEXT,
  CAPTAIN_RED_ALERT_ACTION_ID,
  CAPTAIN_WEAPONS_HOLD_ACTION_ID,
} from '../stations/captain-actions.js';
import { PhElement, phDefine } from './ph-element.js';

export class PhRedAlert extends PhElement {
  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    .alert-btn { width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: var(--text-md); font-weight: 700; padding: 0.7rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 2px solid; transition: all 0.15s ease; min-height: var(--control-hit-min); }
    .alert-btn.standby { background: var(--bg-card); border-color: var(--line-faint); color: var(--ink-dim); }
    .alert-btn.standby:hover:not(:disabled) { background: var(--cyan-deep); color: var(--ink-dim); }
    .alert-btn.active { background: var(--fire-deep); border-color: var(--fire); color: var(--fire); text-shadow: 0 0 8px rgba(var(--rgb-fire), 0.5); }
    .alert-btn.active:hover:not(:disabled) { background: var(--tactical-deep); }
    .alert-btn:disabled { opacity: 0.4; cursor: default; }
    /* The restraint lever (issue #1041). Deliberately quieter than the alert
       button above it: holding fire is a posture, not an emergency. */
    .hold-btn { width: 100%; font-family: 'Chakra Petch', sans-serif; font-size: var(--text-sm); font-weight: 700; padding: 0.4rem 0; letter-spacing: 0.2em; text-transform: uppercase; cursor: pointer; border: 1px solid; transition: all 0.15s ease; min-height: var(--control-hit-min); }
    .hold-btn.free { background: var(--bg-card); border-color: var(--line-faint); color: var(--ink-dim); }
    .hold-btn.free:hover:not(:disabled) { background: var(--cyan-deep); color: var(--ink-dim); }
    .hold-btn.held { background: var(--reloading-deep); border-color: var(--reloading); color: var(--reloading); }
    .hold-btn.held:hover:not(:disabled) { background: var(--reloading-deep); }
    .hold-btn:disabled { opacity: 0.4; cursor: default; }
    .feedback-status { min-height: 1.2em; color: var(--ink); font-size: var(--text-xs); letter-spacing: 0.12em; text-transform: uppercase; }
    .feedback-status[data-state="Refused"], .feedback-status[data-state="TimedOut"] { color: var(--fire); }
  </style>
  <div class="header">
    <span>${t('component.red_alert.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <button class="alert-btn standby" id="alert-btn">${t('component.red_alert.standby')}</button>
  <span class="feedback-status" id="feedback-status" role="status" aria-live="polite" aria-atomic="true"></span>
  <button class="hold-btn free" id="hold-btn">${t('component.weapons_hold.free')}</button>
`;
  }

  connectedCallback() {
    super.connectedCallback();
    this._feedback = null;
    this._feedbackByAction = new Map();
    this._onFeedback = (event) => {
      const value = event && event.detail;
      if (!value
          || ![CAPTAIN_RED_ALERT_ACTION_ID, CAPTAIN_WEAPONS_HOLD_ACTION_ID].includes(value.actionId)
          || value.isCurrent === false) return;
      if (value.cancelled || !value.statusId) {
        this._feedbackByAction.delete(value.actionId);
      } else {
        this._feedbackByAction.set(value.actionId, value);
      }
      const current = [...this._feedbackByAction.values()];
      this._feedback = current.slice().reverse()
        .find((feedback) => feedback.state === 'Pending') || current.at(-1) || null;
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
    // The weapons hold (issue #1041). Its own button beside the alert, not a
    // third state of it: the two are independent, and a captain can be at
    // stations with the guns cold.
    const holdBtn = this.shadowRoot.getElementById('hold-btn');
    holdBtn.addEventListener('click', () => {
      if (holdBtn.disabled) return;
      const activate = typeof window !== 'undefined' && window.activateSemanticAction;
      if (typeof activate === 'function') {
        activate(CAPTAIN_WEAPONS_HOLD_ACTION_ID, {
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
    const holdBtn = this.shadowRoot.getElementById('hold-btn');
    if (!status || !btn || !holdBtn) return;
    const value = this._feedback;
    status.textContent = value && value.statusId ? t(value.statusId) : '';
    status.dataset.state = value && value.state ? value.state : '';
    const redAlertFeedback = this._feedbackByAction
      && this._feedbackByAction.get(CAPTAIN_RED_ALERT_ACTION_ID);
    const weaponsHoldFeedback = this._feedbackByAction
      && this._feedbackByAction.get(CAPTAIN_WEAPONS_HOLD_ACTION_ID);
    if (redAlertFeedback && redAlertFeedback.state === 'Pending') {
      btn.setAttribute('aria-busy', 'true');
    } else btn.removeAttribute('aria-busy');
    if (weaponsHoldFeedback && weaponsHoldFeedback.state === 'Pending') {
      holdBtn.setAttribute('aria-busy', 'true');
    } else holdBtn.removeAttribute('aria-busy');
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

    // The hold reads off the same control source as the alert — one console
    // owns the ship's firing posture — so it greys out together with it.
    const held = !!s.hold;
    const holdBtn = root.getElementById('hold-btn');
    holdBtn.textContent = held
      ? t('component.weapons_hold.held')
      : t('component.weapons_hold.free');
    holdBtn.className = 'hold-btn' + (held ? ' held' : ' free');
    holdBtn.disabled = auto;

    root.getElementById('auto-badge').style.display = auto ? 'inline' : 'none';
  }
}

phDefine('ph-red-alert', PhRedAlert);
