import { t } from './strings.js';
import { inspectTransit, applyTransitOperation } from '../editor/workshop-transit.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

export function mountWorkshopTransit({ root, provider, runtime, draft: getDraft, busy = () => false, setBusy = () => {},
  changed = () => {}, preview = () => false, test = () => false, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-transit-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id;
    if (key) node.textContent = t('workshop.transit.' + key); return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const node = el('section', 'panel'), template = el('select', 'template'), owner = el('select', 'owner'), nav = el('select', 'navigation'), fuel = el('select', 'fuel');
  const materialise = el('input', 'materialise'); materialise.type = 'checkbox';
  const numbers = Object.fromEntries(['preparation_work_secs', 'max_distance', 'scatter_radius', 'amount', 'reading_ticks'].map(key => {
    const input = el('input', key); input.type = 'number'; input.step = ['amount', 'reading_ticks'].includes(key) ? '1' : 'any'; input.min = ['scatter_radius', 'reading_ticks'].includes(key) ? '0' : '0.000001'; return [key, input];
  }));
  const gravity = el('input', 'gravity'), radius = el('input', 'radius'); gravity.type = 'checkbox'; radius.type = 'number'; radius.step = 'any'; radius.min = '0.000001';
  const save = el('button', 'save', 'save'), saveGravity = el('button', 'save-gravity', 'save_gravity');
  const show = el('button', 'preview'), fly = el('button', 'test'), status = el('p', 'status'); status.setAttribute('role', 'status');
  for (const button of [save, saveGravity, show, fly]) button.type = 'button';
  show.textContent = t('workshop.entity.preview'); fly.textContent = t('workshop.entity.test');
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(owner, 'owner'), label(nav, 'navigation'), label(fuel, 'fuel'),
    ...Object.entries(numbers).map(([key, input]) => label(input, key)), label(materialise, 'materialise'), save,
    label(gravity, 'gravity'), label(radius, 'radius'), saveGravity, show, fly, status);
  let dependencies = null, inspection = null, disposed = false, loading = false, generation = 0, observed = null, observedDraft = null;
  const options = (select, values) => {
    select.replaceChildren(...values.map(value => { const option = el('option'); option.value = option.textContent = value; return option; }));
  };
  function controls() {
    const blocked = disposed || loading || busy() || !inspection;
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = blocked;
    template.disabled = disposed || loading || busy() || !dependencies;
    for (const control of [owner, nav, fuel, ...Object.values(numbers), save]) control.disabled ||= !inspection?.drives.length;
    radius.disabled ||= !gravity.checked;
  }
  function selectDrive() {
    const drive = inspection?.drives.find(row => row.id === owner.value), component = drive?.component.find(row => row.kind === 'transit_drive');
    const entity = inspection?.resolved.value;
    options(nav, (entity?.system || []).filter(row => row.kind === 'navigation').map(row => row.id));
    options(fuel, (entity?.logistics?.store || []).filter(row => row.system === owner.value && row.role === 'local' && row.endpoint?.kind === 'capacity').map(row => row.id));
    nav.value = component?.navigation_source || ''; fuel.value = component?.config.fuel.store || '';
    for (const [key, input] of Object.entries(numbers)) input.value = component ? String(key === 'amount' ? component.config.fuel.amount : component.config[key] ?? 0) : '';
    controls();
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    const selected = paths.includes(template.value) ? template.value : paths[0] || '';
    if ([...template.options].map(option => option.value).join('\n') !== paths.join('\n')) options(template, paths);
    template.value = selected;
    const key = JSON.stringify([draft.sourceRevision, selected]);
    if (!force && key === observed && draft === observedDraft) { controls(); return; }
    try {
      const selectedOwner = owner.value; inspection = inspectTransit(draft, dependencies, selected); observed = key; observedDraft = draft;
      options(owner, inspection.drives.map(row => row.id)); if (inspection.drives.some(row => row.id === selectedOwner)) owner.value = selectedOwner;
      gravity.checked = !!inspection.resolved.value.arrival_gravity;
      radius.value = String(inspection.resolved.value.arrival_gravity?.exclusion_radius ?? 8);
      materialise.checked = false; status.textContent = ''; selectDrive();
    } catch (error) { inspection = null; status.textContent = t('workshop.transit.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.transit.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  function commit(operation) {
    if (busy() || !inspection) return;
    const token = ++generation, draft = getDraft();
    const current = () => !disposed && token === generation && getDraft() === draft;
    void runWorkshopMutation({ setBusy, current, start: controls,
      invoke: () => applyTransitOperation({ draft, provider, runtime, dependencies, operation, current }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.transit.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  save.addEventListener('click', () => {
    if (!inspection?.drives.length) return;
    commit({ path: template.value, owner: owner.value, navigation_source: nav.value, materialise: materialise.checked,
      config: { preparation_work_secs: Number(numbers.preparation_work_secs.value), max_distance: Number(numbers.max_distance.value),
        scatter_radius: Number(numbers.scatter_radius.value), reading_ticks: Number(numbers.reading_ticks.value), fuel: { store: fuel.value, amount: Number(numbers.amount.value) } } });
  });
  saveGravity.addEventListener('click', () => commit({ type: 'gravity', path: template.value, materialise: materialise.checked,
    value: gravity.checked ? { exclusion_radius: Number(radius.value) } : false }));
  for (const [button, invoke, kind] of [[show, preview, 'preview'], [fly, test, 'test']]) button.addEventListener('click', () => {
    if (busy() || !inspection) return;
    status.textContent = invoke(template.value) ? t(kind === 'preview' ? 'workshop.entity.previewing' : 'workshop.entity.testing', { path: template.value })
      : t(kind === 'preview' ? 'workshop.entity.preview_unavailable' : 'workshop.entity.test_unavailable', { detail: template.value });
  });
  owner.addEventListener('change', selectDrive); gravity.addEventListener('change', controls); template.addEventListener('change', () => inspect(true));
  if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } },
    dispose() { disposed = true; generation++; node.remove(); } };
}
