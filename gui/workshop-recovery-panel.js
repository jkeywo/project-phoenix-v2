import { t } from './strings.js';
import { inspectRecovery, applyRecoveryOperation } from '../editor/workshop-resource-recovery.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

/** Closed tool and source declarations in the ordinary accepted-source workflow. */
export function mountWorkshopRecovery({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = 'workshop-resource-recovery-' + id;
    if (key) node.textContent = t('workshop.recovery.' + key); return node; };
  const input = (id, type = 'text') => { const node = el('input', id); node.type = type; if (type === 'number') { node.min = '0'; node.step = 'any'; } return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const button = (id, key, action) => { const node = el('button', id, key); node.type = 'button'; node.addEventListener('click', action); return node; };
  const fill = (select, values, previous = select.value) => { select.replaceChildren(); for (const value of values) { const option = el('option'); option.value = value; option.textContent = value; select.append(option); }
    select.value = values.includes(previous) ? previous : values[0] || ''; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', 'checkbox'), status = el('p', 'status'); status.setAttribute('role', 'status');
  const system = input('system'), console = el('select', 'console'), group = el('select', 'group'), room = el('select', 'room');
  const range = input('range', 'number'), rate = input('units-per-sec', 'number'), resources = input('resources');
  const availability = el('select', 'availability'), manifest = el('fieldset', 'manifest');
  fill(availability, ['always', 'disabled']);
  let inspection = null, dependencies = null, disposed = false, loading = false, observed = null, generation = 0, serial = 0, rows = [];
  const endpoints = () => [...(inspection?.capacities || []).map(row => 'capacity:' + row.id), ...(inspection?.resolved.value.torpedoes ? ['torpedo_magazine'] : [])];
  function addRow(resource = '', endpoint = null) {
    const id = serial++, row = el('div'), name = input('resource-' + id), stock = el('select', 'endpoint-' + id); name.value = resource;
    fill(stock, endpoints(), endpoint?.kind === 'capacity' ? 'capacity:' + endpoint.id : endpoint?.kind);
    const entry = { row, name, stock }; rows.push(entry);
    row.append(label(name, 'resource'), label(stock, 'endpoint'), button(null, 'remove', () => { rows = rows.filter(item => item !== entry); row.remove(); })); manifest.append(row);
  }
  function controls() {
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection;
    create.disabled ||= !!inspection?.recovery; saveTool.disabled ||= !inspection?.recovery;
    for (const control of [system, console, group, room]) control.disabled ||= !!inspection?.recovery;
  }
  function paint() {
    const tool = inspection.recovery;
    system.value = tool?.system || 'resource-recovery'; range.value = tool?.range ?? 8; rate.value = tool?.units_per_sec ?? 2;
    resources.value = (tool?.resources || ['torpedoes', 'coolant', 'machinery_parts', 'structural_parts', 'alloy', 'ore', 'scrap']).join(', ');
    fill(console, (inspection.resolved.value.console || []).map(row => row.id)); fill(group, (inspection.resolved.value.energy?.group || []).map(row => row.id));
    fill(room, inspection.compartments.map(row => row.id)); availability.value = inspection.recoverable?.availability || 'always';
    manifest.replaceChildren(el('legend', null, 'manifest')); rows = [];
    for (const [resource, endpoint] of Object.entries(inspection.recoverable?.resources || {})) addRow(resource, endpoint);
    controls();
  }
  async function apply(section, value, extra = {}) {
    const token = ++generation, operation = { path: template.value, section, value, materialise: materialise.checked, ...extra };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyRecoveryOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.recovery.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  const readTool = () => ({ system: system.value, range: range.valueAsNumber, units_per_sec: rate.valueAsNumber,
    resources: resources.value.split(',').map(name => name.trim()).filter(Boolean) });
  const create = button('create', 'create', () => { if (!busy() && inspection) void apply('recovery', readTool(),
    { type: 'recovery-create', console: console.value, group: group.value, compartment: room.value }); });
  const saveTool = button('save-tool', 'save-tool', () => { if (!busy() && inspection?.recovery) void apply('recovery', readTool()); });
  const saveSource = button('save-source', 'save-source', () => {
    if (busy() || !inspection) return;
    const resources = Object.create(null);
    for (const row of rows) {
      if (Object.hasOwn(resources, row.name.value)) { status.textContent = t('workshop.recovery.duplicate-resource'); return; }
      resources[row.name.value] = row.stock.value === 'torpedo_magazine' ? { kind: 'torpedo_magazine' } : { kind: 'capacity', id: row.stock.value.slice(9) };
    }
    void apply('recoverable', { availability: availability.value, resources });
  });
  node.append(el('h3', null, 'title'), label(template, 'template'), label(materialise, 'materialise'), status,
    el('h4', null, 'tool'), el('p', null, 'tool-hint'), label(system, 'system'), label(console, 'console'), label(group, 'group'), label(room, 'room'),
    label(range, 'range'), label(rate, 'units-per-sec'), label(resources, 'resources'), create, saveTool,
    el('h4', null, 'source'), el('p', null, 'source-hint'), label(availability, 'availability'), manifest,
    button('add-resource', 'add-resource', () => addRow()), saveSource);
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    fill(template, draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort());
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && observed === key) { controls(); return; }
    try { inspection = inspectRecovery(draft, dependencies, template.value); observed = key; paint(); materialise.checked = false; status.textContent = ''; }
    catch (error) { inspection = null; status.textContent = t('workshop.recovery.refused', { reason: error.message }); } controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.recovery.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
