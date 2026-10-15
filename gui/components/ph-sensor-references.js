import '../strings-boot.js';
import { t, wireText } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';
import { SENSORS_ATTACH_REFERENCE_ACTION_ID } from '../stations/sensors-actions.js';

/** Stable native controls: snapshots never replace focused search/picker nodes. */
export class PhSensorReferences extends PhElement {
  #rows = new Map();
  template() { return `<section><h4>${t('sensing.reference.title')}</h4><p>${t('sensing.reference.hypothesis')}</p><div id="owners"></div><div id="attached"></div></section>`; }
  render(state = {}) {
    const owners = this.shadowRoot.getElementById('owners'), present = new Set();
    owners.onkeydown = event => event.stopPropagation();
    for (const control of state.reference_controls || []) {
      present.add(control.system_id);
      let row = this.#rows.get(control.system_id);
      if (!row) {
        const node = document.createElement('fieldset'), legend = document.createElement('legend'), search = document.createElement('input'), picker = document.createElement('select'), detail = document.createElement('p'), attach = document.createElement('button');
        search.type = 'search'; search.placeholder = t('sensing.reference.search'); search.setAttribute('aria-label', t('sensing.reference.search'));
        picker.setAttribute('aria-label', t('sensing.reference.choose')); attach.type = 'button'; attach.textContent = t('sensing.reference.attach');
        row = { node, legend, search, picker, detail, attach, records: [] };
        search.addEventListener('input', () => this.#paint(row)); picker.addEventListener('change', () => this.#detail(row));
        attach.addEventListener('click', () => window.activateSemanticAction?.(SENSORS_ATTACH_REFERENCE_ACTION_ID,
          { source: 'control', detail: { system_id: control.system_id, contact: this.state?.target_uuid, reference: picker.value } }));
        node.append(legend, search, picker, detail, attach); owners.append(node); this.#rows.set(control.system_id, row);
      }
      row.legend.textContent = control.system_id; row.records = control.records || []; row.control = control;
      this.#paint(row);
      row.attach.disabled = !control.controllable || !control.contacts.includes(state.target_uuid) || !row.picker.value;
    }
    for (const [id, row] of this.#rows) if (!present.has(id)) { row.node.remove(); this.#rows.delete(id); }
    const attached = this.shadowRoot.getElementById('attached'); attached.replaceChildren();
    for (const value of state.target_references || []) {
      const row = document.createElement('p');
      row.textContent = `${wireText(value.text)} · ${wireText(value.source_name)} · ${value.provenance} · ${t('sensing.reference.sample_tick', { tick: value.gathered_at_tick })}`;
      attached.append(row);
    }
    this.hidden = !(state.reference_controls?.length || state.target_references?.length);
  }
  #paint(row) {
    const selected = row.picker.value, query = row.search.value.trim().toLocaleLowerCase();
    const records = row.records.filter(value => [value.reference.id, wireText(value.text), wireText(value.source_name)].some(text => text.toLocaleLowerCase().includes(query)));
    const ids = new Set(records.map(value => value.reference.id));
    for (const option of [...row.picker.options]) if (!ids.has(option.value)) option.remove();
    for (const record of records) {
      let option = [...row.picker.options].find(option => option.value === record.reference.id);
      if (!option) { option = document.createElement('option'); option.value = record.reference.id; row.picker.append(option); }
      option.textContent = `${record.reference.unusual ? t('sensing.reference.unusual') + ' · ' : ''}${wireText(record.text)}`;
    }
    if (ids.has(selected)) row.picker.value = selected;
    this.#detail(row);
  }
  #detail(row) {
    const record = row.records.find(value => value.reference.id === row.picker.value);
    row.detail.textContent = record ? `${wireText(record.source_name)} · ${record.provenance} · ${t('sensing.reference.sample_tick', { tick: record.gathered_at_tick })}` : '';
    row.attach.disabled = !row.control?.controllable || !row.control?.contacts.includes(this.state?.target_uuid) || !record;
  }
}
phDefine('ph-sensor-references', PhSensorReferences);
