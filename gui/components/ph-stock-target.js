import { t } from '../strings.js';
import { LOGISTICS_ACTION_IDS } from '../stations/logistics-actions.js';
import { activateTacticalAction } from '../stations/tactical-action-control.js';
export class PhStockTarget extends (globalThis.HTMLElement || class {}) {
  constructor() { super(); const root = this.attachShadow({ mode: 'open' }), doc = this.ownerDocument;
    const label = doc.createElement('label'); label.textContent = t('logistics.target');
    this.input = doc.createElement('input'); this.input.type = 'number'; this.input.min = '0'; this.input.step = '1';
    this.button = doc.createElement('button'); this.button.type = 'button'; this.button.textContent = t('logistics.apply');
    this.summary = doc.createElement('p'); label.append(this.input); root.append(label, this.button, this.summary);
    this.input.addEventListener('keydown', event => event.stopPropagation());
    this.button.addEventListener('click', () => {
      if (this._state?.generic && this._state?.system_id && typeof window !== 'undefined' && typeof window.activateSemanticAction === 'function')
        window.activateSemanticAction(LOGISTICS_ACTION_IDS.target, { context: 'stock', source: 'control',
          detail: { system: this._state.system_id, store: this._state.store, amount: this.input.valueAsNumber } });
      else activateTacticalAction(null, LOGISTICS_ACTION_IDS.target, { store: this._state?.store, amount: this.input.valueAsNumber });
    });
  }
  set state(value) { this._state = value; this.hidden = !value;
    if (!value) return; if (this.shadowRoot.activeElement !== this.input) this.input.value = value.target;
    this.input.max = value.ceiling; this.input.disabled = this.button.disabled = !!value.auto;
    this.summary.hidden = !!value.generic;
    this.summary.textContent = t('logistics.local-summary', { incoming: value.incoming, reserved: value.reserved, held: value.held });
  }
  get state() { return this._state; }
}
if (globalThis.customElements && !customElements.get('ph-stock-target')) customElements.define('ph-stock-target', PhStockTarget);
