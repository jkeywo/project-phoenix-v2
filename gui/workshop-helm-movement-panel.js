import { t } from './strings.js';
import { inspectHelmMovement, applyHelmMovementOperation } from '../editor/workshop-helm-movement.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

export function mountWorkshopHelmMovement({ root, provider, runtime, draft: getDraft, busy = () => false, setBusy = () => {},
  changed = () => {}, preview = () => false, test = () => false, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-helm-movement-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id;
    if (key) node.textContent = t('workshop.helm_movement.' + key); return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const node = el('section', 'panel'), template = el('select', 'template'), enabled = el('input', 'enabled'), materialise = el('input', 'materialise');
  enabled.type = materialise.type = 'checkbox';
  const axes = Object.fromEntries(['thrust', 'steering', 'lateral'].map(role => [role, el('select', role)]));
  const status = el('p', 'status'); status.setAttribute('role', 'status');
  const save = el('button', 'save', 'save'), show = el('button', 'preview'), fly = el('button', 'test');
  for (const button of [save, show, fly]) button.type = 'button';
  show.textContent = t('workshop.entity.preview'); fly.textContent = t('workshop.entity.test');
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(enabled, 'enabled'),
    ...Object.entries(axes).map(([role, control]) => label(control, role)), label(materialise, 'materialise'), save, show, fly, status);
  let dependencies = null, inspection = null, disposed = false, loading = false, generation = 0, observed = null, observedDraft = null;
  function controls() {
    const blocked = disposed || loading || busy() || !inspection;
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = blocked;
    template.disabled = disposed || loading || busy() || !dependencies;
    for (const control of Object.values(axes)) control.disabled ||= !enabled.checked;
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    const selected = paths.includes(template.value) ? template.value : paths[0] || '';
    if ([...template.options].map(option => option.value).join('\n') !== paths.join('\n')) template.replaceChildren(...paths.map(path => {
      const option = el('option'); option.value = option.textContent = path; return option;
    }));
    template.value = selected;
    const key = JSON.stringify([draft.sourceRevision, selected]);
    if (!force && key === observed && draft === observedDraft) { controls(); return; }
    try {
      inspection = inspectHelmMovement(draft, dependencies, selected); observed = key; observedDraft = draft;
      enabled.checked = inspection.value !== false;
      for (const [role, select] of Object.entries(axes)) {
        const ids = inspection.systems.filter(system => system.kind === (role === 'lateral' ? 'lateral_thrust' : 'helm_' + role)).map(system => system.id);
        if (role === 'lateral') ids.unshift('');
        select.replaceChildren(...ids.map(id => { const option = el('option'); option.value = id; option.textContent = id || t('workshop.helm_movement.none'); return option; }));
        select.value = inspection.value?.[role] ?? ids[0] ?? '';
      }
      materialise.checked = false; status.textContent = '';
    } catch (error) { inspection = null; status.textContent = t('workshop.helm_movement.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.helm_movement.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  save.addEventListener('click', () => {
    if (busy() || !inspection) return;
    const value = enabled.checked ? { thrust: axes.thrust.value, steering: axes.steering.value,
      ...(axes.lateral.value ? { lateral: axes.lateral.value } : {}) } : false;
    const token = ++generation, operation = { path: template.value, materialise: materialise.checked, value };
    void runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyHelmMovementOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.helm_movement.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  });
  for (const [button, invoke, kind] of [[show, preview, 'preview'], [fly, test, 'test']]) button.addEventListener('click', () => {
    if (busy() || !inspection) return;
    status.textContent = invoke(template.value) ? t(kind === 'preview' ? 'workshop.entity.previewing' : 'workshop.entity.testing', { path: template.value })
      : t(kind === 'preview' ? 'workshop.entity.preview_unavailable' : 'workshop.entity.test_unavailable', { detail: template.value });
  });
  enabled.addEventListener('change', controls); template.addEventListener('change', () => inspect(true));
  if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } },
    dispose() { disposed = true; generation++; node.remove(); } };
}
