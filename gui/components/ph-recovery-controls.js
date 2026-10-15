import { t, wireText } from '../strings.js';
import { RECOVERY_ACTION_IDS } from '../stations/recovery-actions.js';
import { activateEngineeringAction } from '../stations/engineering-action-control.js';
const resourceKeys = { torpedoes: 'coordination.weapon_family.torpedoes', coolant: 'energy.coolant', machinery_parts: 'damage.resource.machinery_parts',
  structural_parts: 'damage.resource.structural_parts', alloy: 'production.alloy', ore: 'recovery.resource.ore', scrap: 'recovery.resource.scrap' };
const resourceLabel = resource => resourceKeys[resource] ? t(resourceKeys[resource]) : wireText(resource);
const storeLabel = store => store.id === 'hold-ammunition' ? t('logistics.hold-ammunition.label')
  : resourceKeys[store.resource] && store.id === 'hold-' + store.resource.replaceAll('_', '-')
    ? t('recovery.main-store', { resource: resourceLabel(store.resource) }) : wireText(store.id);
/** Known contact manifests and actual receiving stock; no source quantities. */
export class PhRecoveryControls extends HTMLElement {
  constructor() {
    super(); const root = this.attachShadow({ mode: 'open' }), doc = this.ownerDocument;
    const el = (tag, key) => { const node = doc.createElement(tag); if (key) node.textContent = t('recovery.' + key); return node; };
    this.toggle = el('button'); this.target = el('select'); this.resource = el('select'); this.destination = el('select');
    this.applySelection = el('button', 'set-selection'); this.clearSelection = el('button', 'clear-selection');
    for (const button of [this.toggle, this.applySelection, this.clearSelection]) button.type = 'button';
    this.hint = el('p', 'science-hint'); this.stock = el('p'); this.progress = el('p'); this.blocked = el('p'); this.blocked.setAttribute('role', 'status');
    const label = (control, key) => { const node = el('label', key); node.append(control); return node; };
    const style = el('style'); style.textContent = ':host{display:block}label{display:flex;gap:.5em;flex-wrap:wrap}button,select{min-height:44px}p{overflow-wrap:anywhere}';
    root.append(style, el('h2', 'title'), this.hint, this.toggle, label(this.target, 'target'),
      label(this.resource, 'resource'), label(this.destination, 'destination'), this.applySelection, this.clearSelection, this.stock, this.progress, this.blocked);
    const activate = (name, detail) => activateEngineeringAction(null, RECOVERY_ACTION_IDS[name], detail);
    this.toggle.addEventListener('click', () => activate('enabled', { enabled: !this._state.enabled }));
    this.applySelection.addEventListener('click', () => activate('selection', { target_uuid: this.target.value, resource: this.resource.value, destination_store: this.destination.value }));
    this.clearSelection.addEventListener('click', () => activate('selection', { target_uuid: null, resource: null, destination_store: null }));
    this._draft = new Set();
    for (const [control, key] of [[this.target, 'target_uuid'], [this.resource, 'resource'], [this.destination, 'destination_store']]) {
      control.addEventListener('change', () => { this._draft.add(key); this.paint(); }); control.addEventListener('keydown', event => event.stopPropagation());
    }
  }
  options(select, values, selected, key) {
    const signature = JSON.stringify(values.map(([id]) => id)), current = select.value;
    if (select._options !== signature) { select._options = signature; select.replaceChildren();
      for (const [id, text] of values) { const option = this.ownerDocument.createElement('option'); option.value = id; option.textContent = text; select.append(option); }
    } else values.forEach(([, text], index) => { select.options[index].textContent = text; });
    if (this._draft.has(key) && values.some(([id]) => id === current) && current !== (selected || '')) select.value = current;
    else { this._draft.delete(key); select.value = values.some(([id]) => id === (selected || '')) ? selected || '' : values[0]?.[0] || ''; }
  }
  paint() {
    const value = this._state; if (!value) return;
    const readonly = !!value.recovery_auto || value.recovery_controllable === false;
    const targets = [['', t('recovery.none')], ...(value.candidates || []).map(row => [row.uuid, t('recovery.contact', { name: wireText(row.name || row.uuid), distance: row.distance.toFixed(1) })])];
    if (value.target_uuid && !targets.some(([id]) => id === value.target_uuid)) targets.push([value.target_uuid, t('recovery.unavailable-target')]);
    this.options(this.target, targets, value.target_uuid, 'target_uuid');
    const contact = value.candidates?.find(row => row.uuid === this.target.value);
    this.options(this.resource, (contact?.resources || []).map(resource => [resource, resourceLabel(resource)]), value.resource, 'resource');
    const destinations = (value.destinations || []).filter(row => row.resource === this.resource.value && row.role === 'main');
    this.options(this.destination, destinations.map(row => [row.id, storeLabel(row)]), value.destination_store, 'destination_store');
    for (const control of [this.toggle, this.target, this.resource, this.destination, this.clearSelection]) control.disabled = readonly;
    this.applySelection.disabled = readonly || !contact || !this.resource.value || !this.destination.value;
    this.toggle.textContent = t(value.enabled ? 'recovery.pause' : 'recovery.enable');
    const store = destinations.find(row => row.id === this.destination.value);
    this.stock.textContent = store ? t('recovery.receiving', { level: store.level, incoming: store.incoming, held: store.held, target: store.target, headroom: store.headroom ?? 0 }) : '';
    this.progress.textContent = t('recovery.progress', { moved: value.last_moved ?? 0, fraction: Math.round((value.fraction || 0) * 100) });
    this.blocked.textContent = value.blocked ? t(value.blocked) : '';
  }
  set state(value) {
    if (this._state?.system_id !== value?.system_id) this._draft.clear();
    this._state = value; this.hidden = !value?.system_id; if (!this.hidden) this.paint();
  }
  get state() { return this._state; }
}
if (!customElements.get('ph-recovery-controls')) customElements.define('ph-recovery-controls', PhRecoveryControls);
