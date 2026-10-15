import { t } from './strings.js';
import { inspectFields, applyFieldsOperation } from '../editor/workshop-fields.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
export function mountWorkshopFields({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument, el = tag => doc.createElement(tag), controls = new Map();
  const node = el('section'); node.id = 'workshop-fields-panel';
  const label = (control, key) => { const label = el('label'); label.textContent = t('workshop.fields.' + key); label.htmlFor = control.id; label.append(control); return label; };
  const input = (key, type = 'number', initial = 0) => { const value = el('input'); value.id = 'workshop-fields-' + key; value.type = type; if (type === 'number') value.step = 'any'; value.value = initial; controls.set(key, value); return label(value, key); };
  const select = (key, values) => { const value = el('select'); value.id = 'workshop-fields-' + key; for (const id of values) { const option = el('option'); option.value = id; option.textContent = t('workshop.fields.' + id); value.append(option); } controls.set(key, value); return label(value, key); };
  const heading = el('h3'); heading.textContent = t('workshop.fields.title'); const hint = el('p'); hint.textContent = t('workshop.fields.hint');
  const template = el('select'); template.id = 'workshop-fields-template'; const status = el('p'); status.setAttribute('role', 'status');
  node.append(heading, hint, label(template, 'template'), input('materialise', 'checkbox'), select('shape', ['none', 'sphere', 'box', 'torus']), input('radius', 'number', 10),
    input('half-x', 'number', 10), input('half-y', 'number', 10), input('half-z', 'number', 10), input('yaw'), input('inner-radius', 'number', 5), input('outer-radius', 'number', 10),
    input('field', 'checkbox'), select('falloff', ['step', 'linear', 'smooth']), input('strength', 'number', 1), input('sensor-absorption'), input('comms-absorption'),
    input('evolution', 'checkbox'), input('duration', 'number', 10), input('translation-x'), input('translation-y'), input('translation-z'), input('final-scale', 'number', 1), input('final-strength', 'number', 1),
    input('material', 'checkbox'), input('material-sensor-absorption'), input('material-comms-absorption'), input('sensor-opaque', 'checkbox'), input('comms-opaque', 'checkbox'));
  const apply = el('button'); apply.id = 'workshop-fields-apply'; apply.type = 'button'; apply.textContent = t('workshop.fields.apply'); node.append(apply, status);
  let inspection = null, dependencies = null, loading = false, disposed = false, generation = 0, observed = null;
  const get = key => controls.get(key), number = key => get(key).valueAsNumber, checked = key => get(key).checked;
  const set = (key, value) => { const input = get(key); if (input.type === 'checkbox') input.checked = !!value; else input.value = value; };
  function disable() { for (const input of node.querySelectorAll('input,select,button')) input.disabled = disposed || loading || busy() || !inspection; }
  function paint() {
    const { shape, field, signal_material: material } = inspection.environment; set('shape', shape?.type || 'none'); set('radius', shape?.radius ?? 10);
    for (const [axis, index] of ['x','y','z'].map((axis, index) => [axis, index])) set('half-' + axis, shape?.half_extents?.[index] ?? 10);
    set('yaw', shape?.yaw ?? 0); set('inner-radius', shape?.inner_radius ?? 5); set('outer-radius', shape?.outer_radius ?? 10);
    set('field', !!field); set('falloff', field?.falloff || 'step'); set('strength', field?.strength ?? 1); set('sensor-absorption', field?.sensor_absorption_per_unit ?? 0); set('comms-absorption', field?.comms_absorption_per_unit ?? 0);
    set('evolution', !!field?.evolution); set('duration', field?.evolution?.duration_secs ?? 10);
    for (const [axis,index] of ['x','y','z'].map((axis,index) => [axis,index])) set('translation-' + axis, field?.evolution?.translation?.[index] ?? 0);
    set('final-scale', field?.evolution?.final_scale ?? 1); set('final-strength', field?.evolution?.final_strength ?? 1);
    set('material', !!material); set('material-sensor-absorption', material?.sensor_absorption_per_unit ?? 0); set('material-comms-absorption', material?.comms_absorption_per_unit ?? 0);
    set('sensor-opaque', material?.sensor_opaque); set('comms-opaque', material?.comms_opaque); set('materialise', false); disable();
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; disable(); return; }
    const previous = template.value; template.replaceChildren(); for (const path of draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort()) { const option = el('option'); option.value = path; option.textContent = path; template.append(option); }
    if ([...template.options].some(option => option.value === previous)) template.value = previous;
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && key === observed) { disable(); return; }
    try { inspection = inspectFields(draft, dependencies, template.value); observed = key; paint(); status.textContent = ''; } catch (error) { inspection = null; status.textContent = error.message; } disable();
  }
  apply.addEventListener('click', async () => {
    if (!inspection || busy()) return; const token = ++generation;
    const kind = get('shape').value, shape = kind === 'none' ? null : kind === 'sphere' ? { type: kind, radius: number('radius') } : kind === 'box' ? { type: kind, half_extents: ['x','y','z'].map(axis => number('half-' + axis)), yaw: number('yaw') } : { type: kind, inner_radius: number('inner-radius'), outer_radius: number('outer-radius') };
    const field = checked('field') ? { falloff: get('falloff').value, strength: number('strength'), sensor_absorption_per_unit: number('sensor-absorption'), comms_absorption_per_unit: number('comms-absorption'),
      ...(checked('evolution') ? { evolution: { duration_secs: number('duration'), translation: ['x','y','z'].map(axis => number('translation-' + axis)), final_scale: number('final-scale'), final_strength: number('final-strength') } } : {}) } : null;
    const signal_material = checked('material') ? { sensor_absorption_per_unit: number('material-sensor-absorption'), comms_absorption_per_unit: number('material-comms-absorption'), sensor_opaque: checked('sensor-opaque'), comms_opaque: checked('comms-opaque') } : null;
    const operation = { path: template.value, materialise: checked('materialise'), value: { shape, field, signal_material } };
    await runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: disable,
      invoke: () => applyFieldsOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.fields.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: disable });
  });
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return; loading = true; disable(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; } catch (error) { if (!disposed) status.textContent = error.message; }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); disable(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
