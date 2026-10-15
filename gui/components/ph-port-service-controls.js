import { t, wireText } from '../strings.js';
import { PORT_ACTION_IDS } from '../stations/port-actions.js';
import { activateEngineeringAction } from '../stations/engineering-action-control.js';
const labels = { torpedoes: 'coordination.weapon_family.torpedoes', coolant: 'energy.coolant', machinery_parts: 'damage.resource.machinery_parts',
  structural_parts: 'damage.resource.structural_parts', alloy: 'production.alloy', ore: 'recovery.resource.ore', scrap: 'recovery.resource.scrap', reserve_fuel: 'console.umbilical.reserve_fuel' };
const resourceLabel = value => labels[value] ? t(labels[value]) : wireText(value);
const storeLabel = store => store.id === 'hold-ammunition' ? t('logistics.hold-ammunition.label')
  : labels[store.resource] && store.id === 'hold-' + store.resource.replaceAll('_', '-') ? t('recovery.main-store', { resource: resourceLabel(store.resource) }) : wireText(store.id);
const safe = value => Number.isSafeInteger(value) && value >= 0;
const room = store => Math.max(0, Math.min(store.ceiling - store.level - store.incoming - (store.held || 0), store.headroom ?? 0));
export class PhPortServiceControls extends HTMLElement {
  constructor() {
    super(); const root = this.attachShadow({ mode: 'open' }), doc = this.ownerDocument;
    const el = (tag, key) => { const node = doc.createElement(tag); if (key) node.textContent = t('port.' + key); return node; };
    this._el = el; this._rows = new Map();
    this.maintenance = el('button'); this.start = el('button', 'start'); this.stop = el('button', 'stop');
    this.reset = el('button', 'reset-draft'); this.store = el('select'); this.offers = el('div'); this.policy = el('p'); this.progress = el('div');
    this.message = el('p'); this.message.setAttribute('role', 'status');
    const storeLabelNode = el('label', 'transfer-store'); storeLabelNode.append(this.store);
    const style = el('style'); style.textContent = ':host{display:block}fieldset{min-width:0}label{display:flex;flex-wrap:wrap;gap:.5em}button,select,input{min-height:44px}input{width:6em}p{overflow-wrap:anywhere}';
    for (const button of [this.maintenance, this.start, this.stop, this.reset]) button.type = 'button';
    root.append(style, el('h3', 'title'), this.policy, this.maintenance, storeLabelNode, this.offers, this.reset, this.start, this.stop, this.progress, this.message);
    const activate = (name, detail = {}) => activateEngineeringAction(null, PORT_ACTION_IDS[name], detail);
    this.maintenance.addEventListener('click', () => activate('maintenance', { enabled: !this._state.port_maintenance }));
    this.store.addEventListener('change', () => activate('store', { store: this.store.value || null }));
    this.stop.addEventListener('click', () => activate('stop'));
    this.reset.addEventListener('click', () => { for (const row of this._rows.values()) row.edited = false; this.paint(); });
    this.start.addEventListener('pointerdown', () => { this._pressIdentity = this._identity; });
    this.start.addEventListener('keydown', event => { if (['Enter', ' '].includes(event.key)) this._pressIdentity = this._identity; });
    this.start.addEventListener('click', () => {
      const pressed = this._pressIdentity; this._pressIdentity = null; if (pressed && pressed !== this._identity) return;
      const port = this._state.port;
      activate('start', { partner_uuid: port.partner_uuid, visit: port.visit,
        lines: [...this._rows.values()].filter(row => row.amount.valueAsNumber > 0).map(row => ({ offer: row.id, store: row.store.value, lots: row.amount.valueAsNumber })) });
    });
  }
  options(select, values, selected) {
    const signature = JSON.stringify(values.map(([id]) => id));
    if (select._signature !== signature) {
      select._signature = signature; select.replaceChildren();
      for (const [id, text] of values) { const option = this.ownerDocument.createElement('option'); option.value = id; option.textContent = text; select.append(option); }
    } else values.forEach(([, text], i) => { select.options[i].textContent = text; });
    select.value = values.some(([id]) => id === selected) ? selected : values[0]?.[0] || '';
  }
  makeOffer(offer) {
    const el = this._el, node = el('fieldset'), heading = el('legend'), summary = el('p'), store = el('select'), amount = el('input');
    amount.type = 'number'; amount.min = '0'; amount.step = '1';
    const storeLabelNode = el('label', 'receiving-store'); storeLabelNode.append(store);
    const amountLabel = el('label', 'lots'); amountLabel.append(amount);
    node.append(heading, summary, storeLabelNode, amountLabel); this.offers.append(node);
    const row = { id: offer.id, node, heading, summary, store, amount, edited: false };
    for (const control of [store, amount]) {
      control.addEventListener('input', () => { row.edited = true; this.paint(); });
      control.addEventListener('change', () => { row.edited = true; this.paint(); });
      control.addEventListener('keydown', event => event.stopPropagation());
    }
    this._rows.set(offer.id, row); return row;
  }
  paint() {
    const value = this._state; if (!value) return;
    const readonly = !!value.umbilical_auto || value.umbilical_controllable === false, port = value.port;
    const identity = JSON.stringify([value.system_id, port?.partner_uuid, port?.visit]);
    if (identity !== this._identity) { this._identity = identity; this.offers.replaceChildren(); this._rows.clear(); }
    this.maintenance.disabled = readonly;
    this.maintenance.textContent = t(value.port_maintenance ? 'port.pause-maintenance' : 'port.maintain-goals');
    this.policy.textContent = port ? t(port.policy === 'manual_offer' ? 'port.manual-policy' : 'port.free-policy') : t('port.no-provider');
    this.options(this.store, [['', t('port.named-service')], ...(port?.stores || []).filter(row => row.resource === value.selected_resource && row.role === 'main').map(row => [row.id, storeLabel(row)])], value.selected_store || '');
    this.store.disabled = readonly || !!value.running;
    const manual = port?.policy === 'manual_offer';
    this.offers.hidden = this.start.hidden = this.reset.hidden = !manual;
    this.stop.hidden = !port?.order?.active; this.stop.disabled = readonly;
    const offers = manual ? port.offers : [];
    for (const [id, row] of this._rows) if (!offers.some(offer => offer.id === id)) { row.node.remove(); this._rows.delete(id); }
    let any = false, valid = true;
    for (const offer of offers) {
      const row = this._rows.get(offer.id) || this.makeOffer(offer);
      row.heading.textContent = t('port.offer-heading', { resource: resourceLabel(offer.resource), amount: offer.unit_amount });
      row.summary.textContent = t('port.offer-summary', { quota: offer.quota_remaining, available: offer.available_lots,
        costs: offer.costs.length ? offer.costs.map(cost => t('port.cost', { resource: resourceLabel(cost.resource), amount: cost.amount })).join(', ') : t('port.free-cost') });
      const stores = port.stores.filter(store => store.role === 'main' && store.resource === offer.resource);
      this.options(row.store, stores.map(store => [store.id, storeLabel(store)]), row.store.value);
      const store = stores.find(store => store.id === row.store.value);
      const max = store ? Math.max(0, Math.min(offer.quota_remaining, offer.available_lots, Math.floor(room(store) / offer.unit_amount))) : 0;
      const demand = store ? Math.floor(Math.max(0, store.target - store.level - store.incoming - (store.held || 0)) / offer.unit_amount) : 0;
      if (!row.edited) row.amount.value = Math.min(max, demand); row.amount.max = max;
      row.store.disabled = row.amount.disabled = readonly || !port.eligible || !!port.order?.active;
      const lots = row.amount.valueAsNumber; any ||= lots > 0; valid &&= safe(lots) && lots <= max;
    }
    this.start.disabled = readonly || !port?.eligible || !!port.order?.active || !!value.running || !valid || !any;
    this.reset.disabled = readonly || !!port?.order?.active;
    this.progress.replaceChildren();
    for (const line of port?.order?.lines || []) {
      const row = this._el('p'), offer = port.offers.find(item => item.id === line.offer);
      row.textContent = t('port.delivered', { resource: offer ? resourceLabel(offer.resource) : wireText(line.offer), delivered: line.delivered_lots, requested: line.requested_lots });
      this.progress.append(row);
    }
    this.message.textContent = port?.order?.blocked ? t(port.order.blocked) : port && !port.eligible ? t('port.ineligible') : '';
  }
  set state(value) { this._state = value; this.hidden = !value?.system_id; if (!this.hidden) this.paint(); }
  get state() { return this._state; }
}
if (!customElements.get('ph-port-service-controls')) customElements.define('ph-port-service-controls', PhPortServiceControls);
