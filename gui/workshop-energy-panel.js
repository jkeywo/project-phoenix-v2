import { t } from './strings.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
import { inspectEnergy, applyEnergyOperation, createEnergyModel, defaultEnergyConsumer, defaultEnergyGroup,
  ENERGY_RESERVE_FIELDS, ENERGY_REACTOR_FIELDS, ENERGY_CONSUMER_FIELDS, ENERGY_GROUP_FIELDS } from '../editor/workshop-energy.js';

/** Closed local hardware forms; runtime validation owns supported contracts. */
export function mountWorkshopEnergy({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, id, text, attrs = {}) => { const node = doc.createElement(tag); if (id) node.id = 'workshop-energy-' + id;
    if (text) node.textContent = t(text); for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value); return node; };
  const label = (control, text) => { const node = el('label', null, text, { for: control.id }); node.append(control); return node; };
  const option = (select, value, text = value) => { const node = el('option'); node.value = value; node.textContent = text; select.append(node); };
  const input = (id, type = 'number') => el('input', id, null, { type, ...(type === 'number' ? { step: 'any', min: '0' } : {}) });
  const node = el('section', 'panel'), template = el('select', 'template'), subject = el('select', 'subject'), parent = el('select', 'parent');
  const status = el('p', 'status', null, { role: 'status' }), materialise = input('materialise', 'checkbox');
  const create = el('button', 'create', 'workshop.energy.create', { type: 'button' }), apply = el('button', 'apply', 'workshop.energy.apply', { type: 'button' });
  const values = el('fieldset', 'values'), storage = el('fieldset', 'storage'), cooling = el('fieldset', 'cooling');
  const endpointKind = el('select', 'endpoint-kind'), endpoint = el('select', 'endpoint');
  const parentRow = label(parent, 'workshop.energy.parent');
  let dependencies = null, inspection = null, energy = null, controlsByName = {}, reserveControls = {}, selected = null;
  let disposed = false, loading = false, generation = 0, observed = null;
  const subjectParts = value => { const split = value.indexOf(':'); return [value.slice(0, split), value.slice(split + 1)]; };
  for (const kind of ['service', 'capacity']) option(endpointKind, kind, t('workshop.energy.' + kind));
  cooling.append(el('legend', null, 'workshop.energy.coolant'), label(endpointKind, 'workshop.energy.endpoint_kind'), label(endpoint, 'workshop.energy.endpoint'));
  node.append(el('h3', null, 'workshop.energy.title'), el('p', null, 'workshop.energy.hint'), label(template, 'workshop.energy.template'),
    create, label(subject, 'workshop.energy.subject'), label(materialise, 'workshop.energy.materialise'), status, parentRow, values, storage, cooling, apply);
  if (attach) root.append(node);
  function controls() {
    const disabled = disposed || loading || busy() || !inspection;
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disabled;
    subject.disabled ||= !energy; apply.disabled ||= !selected;
    create.disabled ||= !!energy;
  }
  function endpoints(preferred) {
    endpoint.replaceChildren();
    const rows = endpointKind.value === 'capacity' ? inspection?.capacities.map(item => item.id) : Object.keys(inspection?.services || {});
    for (const id of rows || []) option(endpoint, id);
    endpoint.value = (rows || []).includes(preferred) ? preferred : (rows || [])[0] || '';
  }
  function paint() {
    const [kind, id] = subjectParts(subject.value);
    selected = !energy ? null : kind === 'reactor' ? energy.reactor : kind === 'consumer'
      ? energy.consumer.find(item => item.system === id) || defaultEnergyConsumer(id)
      : energy.group.find(item => item.id === id) || defaultEnergyGroup(id);
    values.replaceChildren(el('legend', null, 'workshop.energy.hardware')); storage.replaceChildren(el('legend', null, 'workshop.energy.reserve'));
    controlsByName = {}; reserveControls = {};
    if (!selected) { controls(); return; }
    for (const field of kind === 'reactor' ? ENERGY_REACTOR_FIELDS : kind === 'consumer' ? ENERGY_CONSUMER_FIELDS : ENERGY_GROUP_FIELDS) {
      const control = input('field-' + field); control.value = selected[field]; controlsByName[field] = control;
      values.append(label(control, 'workshop.energy.field.' + field));
    }
    for (const field of kind === 'group' ? [] : ENERGY_RESERVE_FIELDS) {
      const control = input('reserve-' + field); control.value = selected.reserve[field]; reserveControls[field] = control;
      storage.append(label(control, 'workshop.energy.field.' + field));
    }
    parent.replaceChildren(); option(parent, '', t('workshop.energy.no_parent'));
    for (const group of inspection.groups.filter(group => group !== id)) option(parent, group);
    parent.value = selected.parent || ''; parentRow.hidden = kind !== 'group'; storage.hidden = cooling.hidden = kind === 'group';
    endpointKind.value = selected.coolant?.provider?.kind === 'capacity' ? 'capacity' : 'service';
    endpoints(selected.coolant?.provider?.id || selected.coolant?.service || 'coolant');
    materialise.checked = false; controls();
  }
  function subjects(preferred = subject.value) {
    subject.replaceChildren(); if (!energy) { selected = null; controls(); return; }
    option(subject, 'reactor:', t('workshop.energy.reactor'));
    for (const item of inspection.systems) option(subject, 'consumer:' + item.id, t('workshop.energy.consumer', { id: item.id }));
    for (const id of inspection.groups) option(subject, 'group:' + id, t('workshop.energy.group', { id }));
    subject.value = [...subject.options].some(item => item.value === preferred) ? preferred : 'reactor:'; paint();
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const previous = template.value, paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    template.replaceChildren(); for (const path of paths) option(template, path);
    template.value = paths.includes(previous) ? previous : paths[0] || '';
    if (!template.value) { inspection = null; controls(); return; }
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && observed === key) { controls(); return; }
    try { inspection = inspectEnergy(draft, dependencies, template.value); energy = structuredClone(inspection.energy); observed = key;
      subjects(); status.textContent = energy ? t('workshop.energy.ready') : t('workshop.energy.empty'); }
    catch (error) { inspection = null; status.textContent = t('workshop.energy.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const result = await runtime.dependencies(); if (!disposed && token === generation) dependencies = result; }
    catch (error) { if (!disposed) status.textContent = t('workshop.energy.refused', { reason: error.message }); }
    finally { loading = false; if (!disposed) inspect(); }
  }
  async function save(initial = false) {
    if (busy() || loading || !inspection || (!initial && !selected)) return;
    const model = structuredClone(initial ? createEnergyModel(inspection.resolved.value) : energy);
    if (!initial) {
      const [kind, id] = subjectParts(subject.value), row = structuredClone(selected);
      for (const [key, control] of Object.entries(controlsByName)) row[key] = control.valueAsNumber;
      if (kind === 'group') { delete row.parent; if (parent.value) row.parent = parent.value; }
      else { row.reserve = Object.fromEntries(Object.entries(reserveControls).map(([key, control]) => [key, control.valueAsNumber]));
        row.coolant = endpointKind.value === 'service' ? { service: endpoint.value } : { provider: { kind: 'capacity', id: endpoint.value } }; }
      if (kind === 'reactor') model.reactor = row;
      else { const rows = model[kind], identity = kind === 'consumer' ? 'system' : 'id', index = rows.findIndex(item => item[identity] === id);
        if (index < 0) rows.push(row); else rows[index] = row; }
    }
    const token = ++generation, draft = getDraft(), operation = { path: template.value, energy: model, materialise: materialise.checked };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyEnergyOperation({ draft, provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.energy.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  create.addEventListener('click', () => void save(true)); apply.addEventListener('click', () => void save()); subject.addEventListener('change', paint);
  template.addEventListener('change', () => { observed = null; inspect(true); }); endpointKind.addEventListener('change', () => endpoints());
  inspect(); void load();
  return { node, refresh() { inspect(); void load(); }, dispose() { disposed = true; generation += 1; node.remove(); } };
}
