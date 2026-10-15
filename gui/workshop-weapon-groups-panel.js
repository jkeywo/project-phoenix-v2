import { t } from './strings.js';
import { inspectWeaponGroups, applyWeaponGroupsOperation } from '../editor/workshop-weapon-groups.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

export function mountWorkshopWeaponGroups({ root, provider, runtime, draft: getDraft, busy, setBusy,
  changed = () => {}, preview = () => false, test = () => false, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-weapon-groups-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id; if (key) node.textContent = t('workshop.weapon_groups.' + key); return node; };
  const label = (node, key) => { const wrapper = el('label', null, key); wrapper.htmlFor = node.id; wrapper.append(node); return wrapper; };
  const node = el('section', 'panel'), template = el('select', 'template'), groups = el('select', 'group'), id = el('input', 'id'), name = el('input', 'label'), members = el('fieldset', 'members');
  const materialise = el('input', 'materialise'); materialise.type = 'checkbox';
  const fresh = el('button', 'new', 'new'), save = el('button', 'save', 'save'), remove = el('button', 'remove', 'remove'), show = el('button', 'preview'), fly = el('button', 'test');
  show.textContent = t('workshop.entity.preview'); fly.textContent = t('workshop.entity.test');
  for (const button of [fresh, save, remove, show, fly]) button.type = 'button';
  const status = el('p', 'status'); status.setAttribute('role', 'status');
  node.append(el('h3', null, 'title'), label(template, 'template'), label(groups, 'group'), fresh,
    label(id, 'id'), label(name, 'label'), members, label(materialise, 'materialise'), save, remove, show, fly, status);
  let dependencies = null, inspection = null, disposed = false, loading = false, generation = 0, observed = null, original = null, observedDraft = null;
  function controls() { const blocked = disposed || loading || busy() || !inspection;
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = blocked;
    save.disabled ||= !inspection?.capable; remove.disabled ||= !original;
  }
  function select(selected) {
    original = selected; const value = inspection?.groups.find(row => row.id === selected);
    groups.value = selected || ''; id.value = value?.id || ''; name.value = value?.label || '';
    members.replaceChildren(el('legend', null, 'members'));
    for (const system of inspection?.weapons || []) { const input = el('input'); input.type = 'checkbox'; input.value = system.id;
      input.checked = !!value?.members.includes(system.id); const text = el('label'); text.append(input, doc.createTextNode(system.id)); members.append(text); }
    materialise.checked = false; controls();
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    const selected = paths.includes(template.value) ? template.value : paths[0] || '';
    if ([...template.options].map(option => option.value).join('\n') !== paths.join('\n')) template.replaceChildren(...paths.map(path => { const option = el('option'); option.value = option.textContent = path; return option; }));
    template.value = selected; const key = JSON.stringify([draft.sourceRevision, selected, dependencies]);
    if (!force && draft === observedDraft && key === observed) { controls(); return; }
    try { inspection = inspectWeaponGroups(draft, dependencies, selected); observed = key; observedDraft = draft;
      groups.replaceChildren(...inspection.groups.map(row => { const option = el('option'); option.value = row.id; option.textContent = row.id; return option; }));
      select(inspection.groups.some(row => row.id === original) ? original : inspection.groups[0]?.id || null); status.textContent = '';
    } catch (error) { inspection = null; status.textContent = t('workshop.weapon_groups.refused', { reason: error.message }); } controls();
  }
  async function load() { if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.weapon_groups.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  function commit(deleting) {
    if (busy() || !inspection) return;
    const value = inspection.groups.filter(row => row.id !== original);
    if (!deleting) value.push({ id: id.value.trim(), label: name.value.trim(), members: [...members.querySelectorAll('input:checked')].map(input => input.value) });
    const operation = { path: template.value, value, materialise: materialise.checked }, token = ++generation;
    void runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyWeaponGroupsOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { original = deleting ? null : id.value.trim(); changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.weapon_groups.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  fresh.onclick = () => select(null); groups.onchange = () => select(groups.value); template.onchange = () => inspect(true);
  save.onclick = () => commit(false); remove.onclick = () => commit(true);
  for (const [button, invoke] of [[show, preview], [fly, test]]) button.onclick = () => { if (!busy() && inspection) invoke(template.value); };
  if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
