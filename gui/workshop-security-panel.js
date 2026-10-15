import { t } from './strings.js';
import { inspectSecurity, applySecurityOperation, LOCAL_SECURITY_ACTIONS, SECURITY_PRIORITIES } from '../editor/workshop-security.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

export function mountWorkshopSecurity({ root, provider, runtime, draft: getDraft, busy, setBusy,
  changed = () => {}, preview = () => false, test = () => false, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-security-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id; if (key) node.textContent = t('workshop.security.' + key); return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const input = (id, value, type = 'text') => { const node = el('input', id); node.type = type; node.value = value; if (type === 'number') node.step = 'any'; return node; };
  const button = (id, key, action) => { const node = el('button', id, key); node.type = 'button'; node.addEventListener('click', action); return node; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', '', 'checkbox'), enabled = input('local', '', 'checkbox');
  const homes = el('div', 'homes'), actions = el('div', 'actions'), status = el('p', 'status'); status.setAttribute('role', 'status');
  let dependencies = null, inspection = null, loading = false, disposed = false, generation = 0, observed = null, observedDraft = null, serial = 0, homeInputs = [], rows = [];
  const actionText = { secure_contain: 'component.security_teams.action.secure_contain', assist_evacuation: 'component.security_teams.action.assist_evacuation' };
  const priorityText = { life_safety: 'component.security_teams.priority.life_safety', evacuation_underway: 'component.security_teams.priority.evacuation_underway', urgent_objective: 'component.security_teams.priority.urgent_objective', threat_containment: 'component.security_teams.priority.threat_containment', optional: 'component.security_teams.priority.optional' };
  function select(id, options, value) { const control = el('select', id); for (const [key, text] of options) { const option = el('option'); option.value = key; option.textContent = text; control.append(option); } control.value = value; return control; }
  const rooms = () => (inspection?.rooms || []).map(room => [room.id, t(room.label)]);
  function addRow(value = {}) {
    const section = el('fieldset'), index = serial++, fields = {};
    fields.compartment = select('room-' + index, rooms(), value.compartment || inspection?.rooms[0]?.id || '');
    fields.action = select('action-' + index, LOCAL_SECURITY_ACTIONS.map(id => [id, t(actionText[id])]), value.action || 'secure_contain');
    fields.priority = select('priority-' + index, SECURITY_PRIORITIES.map(id => [id, t(priorityText[id])]), value.priority || 'optional');
    for (const key of ['duration_secs', 'risk', 'outcome_flag', 'outcome_value', 'warning']) fields[key] = input(key + '-' + index, value[key] ?? '', ['duration_secs', 'risk', 'outcome_value'].includes(key) ? 'number' : 'text');
    const row = { section, fields }; rows.push(row);
    for (const [key, control] of Object.entries(fields)) section.append(label(control, key));
    section.append(button(null, 'remove', () => { rows.splice(rows.indexOf(row), 1); section.remove(); }));
    actions.append(section); controls();
  }
  function controls() {
    const held = disposed || loading || busy();
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = held || !inspection?.security;
    template.disabled = held || !template.options.length;
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    const selected = paths.includes(template.value) ? template.value : paths[0] || '';
    if ([...template.options].map(option => option.value).join('\n') !== paths.join('\n')) template.replaceChildren(...paths.map(path => { const option = el('option'); option.value = option.textContent = path; return option; }));
    template.value = selected;
    const key = JSON.stringify([draft.sourceRevision, selected, dependencies]); if (!force && observedDraft === draft && observed === key) { controls(); return; }
    try {
      inspection = inspectSecurity(draft, dependencies, selected); observed = key; observedDraft = draft; materialise.checked = false;
      enabled.checked = !!inspection.security?.local; homes.replaceChildren(); actions.replaceChildren(); homeInputs = []; rows = [];
      for (let index = 0; index < (inspection.security?.team_count || 0); index++) {
        const control = select('home-' + index, rooms(), inspection.security?.local?.team_homes?.[index] || inspection.rooms[0]?.id || '');
        homeInputs.push(control); const wrapper = label(control, 'home'); wrapper.prepend(t('component.security_teams.team', { n: String(index + 1) }) + ' '); homes.append(wrapper);
      }
      for (const target of inspection.security?.local?.target || []) for (const action of target.action) addRow({ compartment: target.compartment, ...action });
      status.textContent = '';
    } catch (error) { inspection = null; status.textContent = t('workshop.security.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.security.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  function value() {
    const profile = structuredClone(inspection.security);
    if (!enabled.checked) { profile.local = false; return profile; }
    const target = [];
    for (const row of rows) {
      const fields = row.fields, action = { action: fields.action.value, priority: fields.priority.value,
        duration_secs: fields.duration_secs.valueAsNumber, risk: fields.risk.valueAsNumber };
      for (const key of ['outcome_flag', 'warning']) if (fields[key].value) action[key] = fields[key].value;
      if (fields.outcome_value.value !== '') action.outcome_value = fields.outcome_value.valueAsNumber;
      let group = target.find(group => group.compartment === fields.compartment.value);
      if (!group) { group = { compartment: fields.compartment.value, action: [] }; target.push(group); }
      group.action.push(action);
    }
    profile.local = { team_homes: homeInputs.map(control => control.value), target }; return profile;
  }
  const save = button('save', 'save', () => {
    if (!inspection?.security || busy()) return;
    const capturedDraft = getDraft();
    const token = ++generation, operation = { path: template.value, materialise: materialise.checked, value: value() };
    const current = () => !disposed && token === generation && getDraft() === capturedDraft && template.value === operation.path;
    void runWorkshopMutation({ setBusy, current, start: controls,
      invoke: () => applySecurityOperation({ draft: capturedDraft, provider, runtime, dependencies, operation, current }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.security.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  });
  const show = button('preview', null, () => { if (!busy() && inspection?.security) preview(template.value); }); show.textContent = t('workshop.entity.preview');
  const fly = button('test', null, () => { if (!busy() && inspection?.security) test(template.value); }); fly.textContent = t('workshop.entity.test');
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), label(enabled, 'local'), homes, actions,
    button('add-action', 'add_action', () => addRow()), save, show, fly, status);
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
