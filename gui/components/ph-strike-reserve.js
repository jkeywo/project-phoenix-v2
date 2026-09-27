import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';
import './ph-battery-bar.js';

/** Shared factual reserve status; only Gunnery receives the toggle attribute. */
export class PhStrikeReserve extends PhElement {
  template() {
    return `<style>
      :host { display:block; color:var(--ink); }
      h3 { font-size:var(--text-sm); margin:0 0 .5rem; }
      button { min-height:var(--control-hit-min); min-width:var(--control-hit-min); width:100%; margin:.5rem 0; padding:.5rem; color:var(--ink); background:var(--surface-panel); border:2px solid var(--edge); }
      button[aria-pressed="true"] { min-height:var(--control-hit-min); min-width:var(--control-hit-min); border-color:var(--loaded); }
      button:focus-visible { outline:3px solid var(--cyan); outline-offset:2px; }
      p { font-size:var(--text-sm); line-height:1.4; }
    </style>
    <h3>${t('dynasty.reserve.title')}</h3>
    <ph-battery-bar id="charge" reserve></ph-battery-bar>
    <button id="toggle" type="button" aria-pressed="false">${t('dynasty.boost.toggle')}</button>
    <p id="status" role="status" aria-live="polite" aria-atomic="true"></p>
    <p id="feedback" role="status" aria-live="polite" aria-atomic="true"></p>`;
  }

  onTemplate() {
    this.shadowRoot.getElementById('toggle').addEventListener('click', () => {
      const activate = typeof window !== 'undefined' && window.activateSemanticAction;
      if (this.hasAttribute('toggle') && typeof activate === 'function') {
        activate('tactical.strike-boost', { context: 'tactical' });
      }
    });
  }

  connectedCallback() {
    super.connectedCallback();
    this._onFeedback = (event) => {
      const value = event && event.detail;
      if (!value || value.actionId !== 'tactical.strike-boost' || value.isCurrent === false) return;
      this.shadowRoot.getElementById('feedback').textContent =
        value.cancelled || !value.statusId ? '' : t(value.statusId);
      const button = this.shadowRoot.getElementById('toggle');
      if (value.state === 'Pending') button.setAttribute('aria-busy', 'true');
      else button.removeAttribute('aria-busy');
    };
    window.addEventListener('phoenix-action-feedback', this._onFeedback);
  }

  disconnectedCallback() {
    window.removeEventListener('phoenix-action-feedback', this._onFeedback);
  }

  render(state) {
    const reserve = state || {};
    const button = this.shadowRoot.getElementById('toggle');
    button.hidden = !this.hasAttribute('toggle');
    button.disabled = !state;
    button.setAttribute('aria-pressed', String(!!reserve.enabled));
    this.shadowRoot.getElementById('charge').state = {
      charge: reserve.charge || 0,
      capacity: reserve.capacity || 1,
      charging: !!reserve.charging,
    };
    this.shadowRoot.getElementById('status').textContent = t(
      reserve.depleted ? 'dynasty.boost.depleted'
        : reserve.enabled ? 'dynasty.boost.enabled' : 'dynasty.boost.disabled',
    );
  }
}
phDefine('ph-strike-reserve', PhStrikeReserve);
