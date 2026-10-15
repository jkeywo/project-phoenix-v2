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
  const bindings = Object.fromEntries(["combat_lock","science_target"].map(key=>[key,el("select",key)]));
  const rows = {}; let inspection = null, dependencies = null, disposed = false, loading = false, observed = null, generation = 0;
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), ...Object.entries(bindings).map(([key,control])=>label(control,key)), status);
  const defaults = { focused: { work_secs: 1, history_limit: 32, unfinished_retention_secs: 0 }, broad: { sample_interval_secs: .5, identity_secs: 1, classification_secs: 3, assessment_secs: 6, position_error: 2, drift_error_per_sec: .5, track_retention_secs: 60 }, passive: { sample_interval_secs: .5, min_signal_strength: .01, bearing_error_rad: .15, mount: { facing_deg: 0, arc_deg: 360, range: 100 } } };
  const hosts = {}, sequences = {};
  function addInstrument(role, value = defaults[role]) {
    const spec = sensingRoles[role], index = sequences[role]++, key = index === 0 ? role : role + '-' + index;
    const container = el('fieldset'), system = el('select', key + '-system'), fields = {};
    container.append(label(system, 'system'));
    fill(system, ['', ...inspection.systems.filter(system => (spec.kinds || [spec.kind]).includes(system.kind)).map(system => system.id)], value.system || '');
    for (const field of [...spec.fields, ...(role === 'passive' ? ['facing_deg', 'arc_deg', 'range'] : [])]) {
      const control = input(key + '-' + field, 'number'); control.value = value[field] ?? value.mount?.[field] ?? defaults[role][field] ?? defaults[role].mount?.[field];
      if (field === 'facing_deg') control.removeAttribute('min');
      if (field === 'history_limit') { control.step = '1'; control.min = '1'; control.max = '65535'; }
      fields[field] = control; container.append(label(control, field));
    }
    if (role === 'broad') {
      const initial = inspection.sensing?.sharing?.find(row => row.system === value.system);
      for (const [key, checked] of [['share_own_faction', initial?.share_own_faction ?? false], ['receive', initial?.receive ?? true]]) {
        fields[key] = input(key + '-' + index, 'checkbox'); fields[key].checked = checked;
        container.append(label(fields[key], key));
      }
      container.append(el('p', null, 'sharing-hint'));
    }
    if (role === 'focused') {
      fields.forecast_enabled = input(key + '-forecast-enabled', 'checkbox'); fields.forecast_enabled.checked = !!value.forecast;
      fields.forecast_horizon_secs = input(key + '-forecast-horizon', 'number'); fields.forecast_horizon_secs.value = value.forecast?.horizon_secs ?? 1;
      container.append(label(fields.forecast_enabled, 'forecast-enabled'), label(fields.forecast_horizon_secs, 'forecast-horizon'));
    }
    const remove = el('button', key + '-remove', 'remove-instrument'); remove.type = 'button';
    remove.addEventListener('click', () => { rows[role].delete(key); container.remove(); });
    container.append(remove); rows[role].set(key, { system, fields }); hosts[role].append(container);
    system.addEventListener('change',controls);
  }
  for (const role of Object.keys(sensingRoles)) {
    hosts[role] = el('fieldset', role); hosts[role].append(el('legend', null, role)); rows[role] = new Map(); sequences[role] = 0;
    const add = el('button', role + '-add', 'add-instrument'); add.type = 'button'; add.addEventListener('click', () => { addInstrument(role); controls(); }); node.append(hosts[role], add);
  }
  function controls() {
    const blocked=disposed || loading || busy() || !inspection;
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = blocked;
    for(const row of rows.broad?.values() || []) {
      const eligible=inspection?.systems.some(system=>system.id===row.system.value && ['sensor_radar','tactical_radar'].includes(system.kind));
      for(const key of ['share_own_faction','receive']) { row.fields[key].parentElement.hidden=!eligible;row.fields[key].disabled=blocked||!eligible; }
    }
  }
  function paint() {
    for(const [binding,kind] of [["combat_lock","tactical_radar"],["science_target","sensors"]]) fill(bindings[binding],["",...inspection.systems.filter(system=>system.kind===kind).map(system=>system.id)],inspection.sensing?.control?.[binding] || "");
    for (const role of Object.keys(sensingRoles)) {
      for (const child of [...hosts[role].children].slice(1)) child.remove(); rows[role].clear(); sequences[role] = 0;
      for (const value of inspection.sensing?.[role] || []) addInstrument(role, value);
      // An empty editable row also provides the visible values for fitting new hardware.
      if (!rows[role].size) addInstrument(role);
    }
    fill(console, (inspection.resolved.value.console || []).map(row => row.id)); fill(group, (inspection.resolved.value.energy?.group || []).map(row => row.id)); fill(room, (inspection.resolved.value.interior?.compartment || []).map(row => row.id)); controls();
  }
  function readRow(role, row, system = row.system.value) { return { system,
    ...Object.fromEntries(sensingRoles[role].fields.map(field => [field, row.fields[field].valueAsNumber])),
    ...(role === 'passive' ? { mount: Object.fromEntries(['facing_deg', 'arc_deg', 'range'].map(field => [field, row.fields[field].valueAsNumber])) } : {}),
    ...(role === 'focused' && row.fields.forecast_enabled.checked ? {forecast:{horizon_secs:row.fields.forecast_horizon_secs.valueAsNumber}} : {}),
  }; }
  function read() { return {
    control:Object.fromEntries(Object.entries(bindings).filter(([,control])=>control.value).map(([key,control])=>[key,control.value])),
    sharing:[...rows.broad.values()].filter(row=>inspection.systems.some(system=>system.id===row.system.value && ['sensor_radar','tactical_radar'].includes(system.kind)))
      .map(row=>({system:row.system.value,share_own_faction:row.fields.share_own_faction.checked,receive:row.fields.receive.checked})),
    ...Object.fromEntries(Object.keys(sensingRoles).map(role => [role, [...rows[role].values()].filter(row => row.system.value).map(row => readRow(role, row))]))}; }
  async function apply(extra = {}) {
    if (busy() || !inspection) return; const token = ++generation, value = read();
    if (extra.type) {
      const editable = [...rows.passive.values()], values = editable.find(row => !row.system.value) || editable.at(-1);
      if (!values) return;
      value.passive.push(readRow('passive', values, newId.value));
    }
    const operation = { path: template.value, value, materialise: materialise.checked, ...extra };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applySensingOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.sensing.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  const save = el('button', 'save', 'save'); save.type = 'button'; save.addEventListener('click', () => void apply());
  const fit = el('button', 'fit', 'fit'); fit.type = 'button'; fit.addEventListener('click', () => void apply({ type: 'sensing-fit-passive', new_system: newId.value, console: console.value, group: group.value, compartment: room.value }));
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
