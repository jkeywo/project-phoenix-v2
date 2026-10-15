import { t } from './strings.js';
import { inspectHelmProjection, applyHelmProjectionOperation } from '../editor/workshop-helm-projection.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

export function mountWorkshopHelmProjection({ root, provider, runtime, draft: getDraft, busy, setBusy,
  changed = () => {}, preview = () => false, test = () => false, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-helm-projection-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id;
    if (key) node.textContent = t('workshop.helm_projection.' + key); return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const node = el('section', 'panel'), template = el('select', 'template'), enabled = el('input', 'enabled'), materialise = el('input', 'materialise'), horizon = el('input', 'horizon');
  enabled.type = materialise.type = 'checkbox'; horizon.type = 'number'; horizon.min = '0'; horizon.step = 'any';
  const status = el('p', 'status'); status.setAttribute('role', 'status');
  const save = el('button', 'save', 'save'), show = el('button', 'preview'), fly = el('button', 'test');
  for (const button of [save, show, fly]) button.type = 'button';
  show.textContent = t('workshop.entity.preview'); fly.textContent = t('workshop.entity.test');
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'),
    label(enabled, 'enabled'), label(horizon, 'horizon'), label(materialise, 'materialise'), save, show, fly, status);
  let dependencies = null, inspection = null, disposed = false, loading = false, generation = 0, observed = null;
  function controls() {
    const blocked = disposed || loading || busy() || !inspection;
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = blocked;
    enabled.disabled ||= !inspection?.capable; save.disabled ||= !inspection?.capable;
    horizon.disabled ||= !inspection?.capable || !enabled.checked;
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    const selected = paths.includes(template.value) ? template.value : paths[0] || '';
    if ([...template.options].map(option => option.value).join('\n') !== paths.join('\n')) {
      template.replaceChildren(...paths.map(path => { const option = el('option'); option.value = option.textContent = path; return option; }));
    }
    template.value = selected;
    const key = JSON.stringify([draft.sourceRevision, selected, dependencies]);
    if (!force && key === observed) { controls(); return; }
    try {
      inspection = inspectHelmProjection(draft, dependencies, selected); observed = key;
      enabled.checked = inspection.projection !== false; horizon.value = inspection.projection?.horizon_secs ?? 30;
      materialise.checked = false; status.textContent = '';
    } catch (error) { inspection = null; status.textContent = t('workshop.helm_projection.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.helm_projection.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  save.addEventListener('click', () => {
    if (busy() || !inspection?.capable) return;
    const token = ++generation, operation = { path: template.value, materialise: materialise.checked,
      value: enabled.checked ? { horizon_secs: horizon.valueAsNumber } : false };
    void runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyHelmProjectionOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.helm_projection.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
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
