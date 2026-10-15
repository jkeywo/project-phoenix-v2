import { t } from './strings.js';
import { inspectPersonnel, applyPersonnelOperation } from '../editor/workshop-personnel.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

export function mountWorkshopPersonnel({ root, provider, runtime, draft: getDraft, busy, setBusy,
  changed = () => {}, preview = () => false, test = () => false, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-personnel-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id; if (key) node.textContent = t('workshop.personnel.' + key); return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const input = (id, value, type = 'text') => { const node = el('input', id); node.type = type; node.value = value; if (type === 'number') node.step = 'any'; return node; };
  const button = (id, key, action) => { const node = el('button', id, key); node.type = 'button'; node.addEventListener('click', action); return node; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', '', 'checkbox');
  const teams = el('div', 'teams'), officers = el('div', 'officers'), status = el('p', 'status'); status.setAttribute('role', 'status');
  let dependencies = null, inspection = null, loading = false, disposed = false, generation = 0, observed = null, observedDraft = null, serial = 0, teamRows = [], officerRows = [];
  function addRow(kind, value) {
    const section = el('fieldset'), index = serial++, rows = kind === 'team' ? teamRows : officerRows;
    const fields = kind === 'team' ? ['id', 'label', 'team_idx'] : ['id', 'label', 'home_compartment', 'repair_work_multiplier'];
    const row = { section, fields: {} }; rows.push(row);
    for (const field of fields) {
      const control = input(kind + '-' + field + '-' + index, value[field], ['team_idx', 'repair_work_multiplier'].includes(field) ? 'number' : 'text');
      if (field === 'team_idx') { control.min = '0'; control.step = '1'; }
      if (field === 'repair_work_multiplier') control.min = '1';
      row.fields[field] = control; section.append(label(control, field));
    }
    section.append(el('p', null, 'competency'), button(null, 'remove', () => { rows.splice(rows.indexOf(row), 1); section.remove(); }));
    (kind === 'team' ? teams : officers).append(section); controls();
  }
  function controls() { for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection; }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    const selected = paths.includes(template.value) ? template.value : paths[0] || '';
    if ([...template.options].map(option => option.value).join('\n') !== paths.join('\n')) template.replaceChildren(...paths.map(path => { const option = el('option'); option.value = option.textContent = path; return option; }));
    template.value = selected;
    const key = JSON.stringify([draft.sourceRevision, selected, dependencies]); if (!force && observedDraft === draft && observed === key) { controls(); return; }
    try {
      inspection = inspectPersonnel(draft, dependencies, selected); observed = key; observedDraft = draft; materialise.checked = false;
      teams.replaceChildren(); officers.replaceChildren(); teamRows = []; officerRows = [];
      for (const value of inspection.personnel.repair_team || []) addRow('team', value);
      for (const value of inspection.personnel.officer || []) addRow('officer', value);
      status.textContent = '';
    } catch (error) { inspection = null; status.textContent = t('workshop.personnel.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.personnel.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  function values(rows) { return rows.map(row => ({ ...Object.fromEntries(Object.entries(row.fields).map(([key, control]) => [key, control.type === 'number' ? control.valueAsNumber : control.value])), competencies: ['mechanical_repair'] })); }
  const save = button('save', 'save', () => {
    if (!inspection || busy()) return;
    const token = ++generation, operation = { path: template.value, materialise: materialise.checked, value: { repair_team: values(teamRows), officer: values(officerRows) } };
    void runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyPersonnelOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.personnel.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  });
  const show = button('preview', null, () => { if (!busy() && inspection) preview(template.value); }); show.textContent = t('workshop.entity.preview');
  const fly = button('test', null, () => { if (!busy() && inspection) test(template.value); }); fly.textContent = t('workshop.entity.test');
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), teams,
    button('add-team', 'add_team', () => addRow('team', { id: '', label: '', team_idx: 0 })), officers,
    button('add-officer', 'add_officer', () => addRow('officer', { id: '', label: '', home_compartment: '', repair_work_multiplier: 1 })), save, show, fly, status);
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
