import {renderPlannedRequests} from './planned-requests.js';
import { t, wireText } from '../strings.js';
import { LOGISTICS_ACTION_IDS } from '../stations/logistics-actions.js';
import { activateEngineeringAction } from '../stations/engineering-action-control.js';

/** Read-only stock facts plus admitted goals/jobs; no local inventory mutations. */
export class PhLogisticsControls extends HTMLElement {
  constructor() { super(); this.attachShadow({ mode: 'open' }); this._state = null; this._signature = ''; }
  set state(value) { this._state = value; this.render(); }
  get state() { return this._state; }
  render() {
    const state = this._state; this.hidden = !state?.system_id;
    if (!state?.system_id) return;
    const signature = JSON.stringify([state.system_id, state.enabled, state.logistics_auto, state.max_batch, state.max_active, state.stores, state.deliveries, state.refusal, state.planned_requests,state.planned_controllable]);
    if (signature === this._signature) return; this._signature = signature;
    // Preserve a currently edited input across snapshot paints of unchanged structure.
    const focused = this.shadowRoot.activeElement, selected = focused?.dataset?.key;
    const draft = selected ? { key: selected, value: focused.value } : null;
    const manualDraft = new Map([...this.shadowRoot.querySelectorAll('[data-manual]')].map(node => [node.dataset.key, node.value]));
    const doc = this.ownerDocument, root = this.shadowRoot; root.replaceChildren();
    const el = (tag, text) => { const node = doc.createElement(tag); if (text != null) node.textContent = text; return node; };
    const button = (key, action, detail) => { const node = el('button', t('logistics.' + key)); node.type = 'button'; node.disabled = !!state.logistics_auto;
      node.addEventListener('click', () => activateEngineeringAction(null, LOGISTICS_ACTION_IDS[action], typeof detail === 'function' ? detail() : detail)); return node; };
    const style = el('style', ':host{display:block;margin-top:1em}fieldset{margin:.5em 0;min-width:0}label{display:flex;gap:.5em;align-items:center;flex-wrap:wrap}input{min-width:4em;width:6em}button{min-height:44px;margin:.2em}p{overflow-wrap:anywhere}'); root.append(style, el('h2', t('logistics.title')));
    const planned=el('section');root.append(planned);
    renderPlannedRequests(planned,state,id=>activateEngineeringAction(null,LOGISTICS_ACTION_IDS.dispatch,{planned_request:id}));
    const toggle = button(state.enabled ? 'pause' : 'enable', 'enabled', { enabled: !state.enabled }); root.append(toggle);
    root.append(el('p', t('logistics.maintain-hint')));
    for (const store of state.stores || []) {
      const group = el('fieldset'); group.append(el('legend', wireText(store.id)));
      group.append(el('p', t('logistics.stock-summary', { level: store.level, available: store.available ?? store.level, held: store.held ?? 0, ceiling: store.ceiling, incoming: store.incoming,
        shortage: Math.max(0, store.target - store.level - (store.held || 0) - store.incoming), resource: wireText(store.resource), compartment: wireText(store.compartment) })));
      group.append(el('p', t('logistics.target-summary', { target: store.target, reserve: store.reserve })));
      // Inventory sees local demand, but only the local System can change it.
      if (store.role === 'main') for (const name of ['target', 'reserve']) {
        const label = el('label', t('logistics.' + name)), input = el('input'); input.type = 'number'; input.min = '0'; input.max = String(store.ceiling); input.step = '1';
        input.value = store[name]; input.dataset.key = store.id + '-' + name; input.disabled = !!state.logistics_auto;
        label.append(input, button('apply', name, () => ({ store: store.id, amount: input.valueAsNumber }))); group.append(label);
      }
      const priorityLabel = el('label', t('logistics.priority')), priority = el('input'); priority.type = 'number'; priority.min = '0'; priority.max = '65535'; priority.step = '1';
      priority.value = store.priority ?? 0; priority.dataset.key = store.id + '-priority'; priority.disabled = !!state.logistics_auto;
      priorityLabel.append(priority, button('apply', 'priority', () => ({ store: store.id, priority: priority.valueAsNumber }))); group.append(priorityLabel);
      root.append(group);
    }
    const dispatch = el('fieldset'); dispatch.append(el('legend', t('logistics.manual')));
    const source = el('select'), destination = el('select'), amount = el('input'); amount.type = 'number'; amount.min = '1'; amount.step = '1'; amount.value = state.next_delivery?.amount || 1;
    if (state.max_batch > 0) amount.max = String(state.max_batch);
    if (state.max_batch > 0 && state.max_active > 0) dispatch.append(el('p', t('logistics.handling-limits', { batch: state.max_batch, active: state.max_active })));
    for (const [control, key] of [[source, 'manual-source'], [destination, 'manual-destination'], [amount, 'manual-amount']]) {
      control.dataset.key = key; control.dataset.manual = 'true';
    }
    const row = (control, key) => { const label = el('label', t('logistics.' + key)); label.append(control); dispatch.append(label); control.disabled = !!state.logistics_auto; };
    for (const store of state.stores || []) { const option = el('option', wireText(store.id)); option.value = store.id; (store.role === 'main' ? source : destination).append(option); }
    if (state.next_delivery) { source.value = state.next_delivery.source; destination.value = state.next_delivery.destination; }
    for (const control of [source, destination, amount]) if (manualDraft.has(control.dataset.key)) {
      const value = manualDraft.get(control.dataset.key);
      if (control === amount || [...control.options].some(option => option.value === value)) control.value = value;
    }
    row(source, 'source'); row(destination, 'destination'); row(amount, 'amount');
    dispatch.append(button('dispatch', 'dispatch', () => ({ source: source.value, destination: destination.value, amount: amount.valueAsNumber }))); root.append(dispatch);
    for (const job of state.deliveries || []) {
      const row = el('p', t('logistics.delivery', { quantity: job.quantity, source: wireText(job.source), destination: wireText(job.destination), stage: ['loading', 'travelling', 'unloading'].includes(job.stage) ? t('logistics.stage.' + job.stage) : wireText(job.stage), compartment: wireText(job.compartment), progress: Math.round(job.progress * 100) }));
      if (job.blocked) row.append(el('span', ' ' + t(job.blocked)));
      if (!job.return_requested) row.append(button('return', 'return', { job: job.id })); root.append(row);
    }
    if (state.refusal) root.append(el('p', t(state.refusal)));
    if (draft) { const input = [...root.querySelectorAll('input,select')].find(node => node.dataset.key === draft.key); if (input) { input.value = draft.value; input.focus(); } }
  }
}
if (!customElements.get('ph-logistics-controls')) customElements.define('ph-logistics-controls', PhLogisticsControls);
