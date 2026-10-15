import { t } from './strings.js';
import { inspectSensing, applySensingOperation, sensingRoles } from '../editor/workshop-sensing.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

/** Instruments edit the accepted source and retain ordinary Workshop history. */
export function mountWorkshopSensing({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-sensing-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id; if (key) node.textContent = t('workshop.sensing.' + key); return node; };
  const input = (id, type = 'text') => { const node = el('input', id); node.type = type; if (type === 'number') { node.min = '0'; node.step = 'any'; } return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const fill = (select, values, previous = select.value) => { select.replaceChildren(); for (const value of values) { const option = el('option'); option.value = value; option.textContent = value || t('workshop.sensing.none'); select.append(option); } select.value = values.includes(previous) ? previous : values[0] || ''; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', 'checkbox'), status = el('p', 'status'); status.setAttribute('role', 'status');
  const console = el('select', 'console'), group = el('select', 'group'), room = el('select', 'room'), newId = input('new-id'); newId.value = 'passive-receiver';
  const rows = {}; let inspection = null, dependencies = null, disposed = false, loading = false, observed = null, generation = 0;
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), status);
  for (const [role, spec] of Object.entries(sensingRoles)) {
    if (role === 'broad') continue;
    const fieldset = el('fieldset', role), system = el('select', role + '-system'), fields = {};
    fieldset.append(el('legend', null, role), label(system, 'system'));
    for (const field of [...spec.fields, ...(role === 'passive' ? ['facing_deg', 'arc_deg', 'range'] : [])]) {
      const control = input(role + '-' + field, 'number'); fields[field] = control;
      if (field === 'facing_deg') control.removeAttribute('min');
      if (field === 'history_limit') { control.step = '1'; control.min = '1'; control.max = '65535'; }
      fieldset.append(label(control, field));
    }
    rows[role] = { system, fields }; node.append(fieldset);
  }
  const broadHost = el('fieldset', 'broad'), broadRows = new Map(); let broadSequence = 0;
  broadHost.append(el('legend', null, 'broad')); node.append(broadHost);
  function addBroad(value = defaults.broad) {
    const index = broadSequence++, key = index === 0 ? 'broad' : 'broad-' + index;
    const container = el('fieldset'), system = el('select', key + '-system'), fields = {};
    container.append(label(system, 'system'));
    fill(system, ['', ...inspection.systems.filter(system => sensingRoles.broad.kinds.includes(system.kind)).map(system => system.id)], value.system || '');
    for (const field of sensingRoles.broad.fields) {
      const control = input(key + '-' + field, 'number'); control.value = value[field] ?? defaults.broad[field];
      fields[field] = control; container.append(label(control, field));
    }
    const remove = el('button', key + '-remove', 'remove-broad'); remove.type = 'button';
    remove.addEventListener('click', () => { broadRows.delete(key); container.remove(); });
    container.append(remove); broadRows.set(key, { system, fields }); broadHost.append(container);
  }
  const add = el('button', 'broad-add', 'add-broad'); add.type = 'button'; add.addEventListener('click', () => { addBroad(); controls(); }); node.append(add);
  const defaults = { focused: { work_secs: 1, history_limit: 32, unfinished_retention_secs: 0 }, broad: { sample_interval_secs: .5, identity_secs: 1, classification_secs: 3, assessment_secs: 6, position_error: 2, drift_error_per_sec: .5, track_retention_secs: 60 }, passive: { sample_interval_secs: .5, min_signal_strength: .01, bearing_error_rad: .15, mount: { facing_deg: 0, arc_deg: 360, range: 100 } } };
  function controls() { for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection; fit.disabled ||= !!inspection?.sensing?.passive; }
  function paint() {
    for (const [role, spec] of Object.entries(sensingRoles)) {
      if (role === 'broad') continue;
      const value = inspection.sensing?.[role] || defaults[role]; fill(rows[role].system, ['', ...inspection.systems.filter(system => system.kind === spec.kind).map(system => system.id)], inspection.sensing?.[role]?.system || '');
      for (const [field, control] of Object.entries(rows[role].fields)) control.value = value[field] ?? value.mount?.[field] ?? '';
    }
    for (const child of [...broadHost.children].slice(1)) child.remove(); broadRows.clear(); broadSequence = 0;
    for (const value of inspection.sensing?.broad || []) addBroad(value);
    fill(console, (inspection.resolved.value.console || []).map(row => row.id)); fill(group, (inspection.resolved.value.energy?.group || []).map(row => row.id)); fill(room, (inspection.resolved.value.interior?.compartment || []).map(row => row.id)); controls();
  }
  function read() { return { broad: [...broadRows.values()].filter(row => row.system.value).map(row => ({ system: row.system.value,
    ...Object.fromEntries(sensingRoles.broad.fields.map(field => [field, row.fields[field].valueAsNumber])) })),
    ...Object.fromEntries(Object.entries(sensingRoles).filter(([role]) => role !== 'broad' && rows[role].system.value).map(([role, spec]) => [role, {
    system: rows[role].system.value, ...Object.fromEntries(spec.fields.map(field => [field, rows[role].fields[field].valueAsNumber])),
    ...(role === 'passive' ? { mount: Object.fromEntries(['facing_deg', 'arc_deg', 'range'].map(field => [field, rows[role].fields[field].valueAsNumber])) } : {}),
  }])) }; }
  async function apply(extra = {}) {
    if (busy() || !inspection) return; const token = ++generation, value = read();
    if (extra.type) value.passive = { system: newId.value,
      ...Object.fromEntries(sensingRoles.passive.fields.map(field => [field, rows.passive.fields[field].valueAsNumber])),
      mount: Object.fromEntries(['facing_deg', 'arc_deg', 'range'].map(field => [field, rows.passive.fields[field].valueAsNumber])),
    };
    const operation = { path: template.value, value, materialise: materialise.checked, ...extra };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applySensingOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.sensing.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  const save = el('button', 'save', 'save'); save.type = 'button'; save.addEventListener('click', () => void apply());
  const fit = el('button', 'fit', 'fit'); fit.type = 'button'; fit.addEventListener('click', () => void apply({ type: 'sensing-fit-passive', console: console.value, group: group.value, compartment: room.value }));
  node.append(save, el('h4', null, 'fit'), label(newId, 'new-id'), label(console, 'console'), label(group, 'group'), label(room, 'room'), fit);
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    fill(template, draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort());
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && observed === key) { controls(); return; }
    try { inspection = inspectSensing(draft, dependencies, template.value); observed = key; paint(); materialise.checked = false; status.textContent = ''; }
    catch (error) { inspection = null; status.textContent = t('workshop.sensing.refused', { reason: error.message }); } controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.sensing.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
