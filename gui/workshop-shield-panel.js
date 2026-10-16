import { t } from './strings.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
import { inspectShields, applyShieldOperation, applyShieldFrequencyOperation, defaultShieldArc, SHIELD_ENERGY_FIELDS } from '../editor/workshop-shields.js';
import { shieldArcSystemId } from '../editor/workshop-generated-systems.js';

/** Authored shield work and recovery bindings use the ordinary draft transaction. */
export function mountWorkshopShields({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, id, key, attrs = {}) => { const node = doc.createElement(tag); if (id) node.id = 'workshop-shield-' + id;
    if (key) node.textContent = t(key); for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value); return node; };
  const label = (control, key) => { const node = el('label', null, key, { for: control.id }); node.append(control); return node; };
  const input = (id, type = 'number') => el('input', id, null, { type, ...(type === 'number' ? { step: 'any' } : {}) });
  const option = (select, value, text = value) => { const node = el('option'); node.value = value; node.textContent = text; select.append(node); };
  const node = el('section', 'panel'), template = el('select', 'template'), existing = el('select', 'existing');
  const materialise = input('materialise', 'checkbox'), targets = el('select', 'targets', null, { multiple: '', size: '4' });
  const compartment = el('select', 'compartment');
  const resetAi = input('reset-ai', 'checkbox');
  const focusAvailable = input('focus-available', 'checkbox');
  const status = el('p', 'status', null, { role: 'status' }), create = el('button', 'new', 'workshop.shield.new', { type: 'button' });
  const save = el('button', 'save', 'workshop.shield.save', { type: 'button' }), remove = el('button', 'remove', 'workshop.shield.remove', { type: 'button' });
  const fields = Object.fromEntries(['id', 'label', 'center_deg', 'width_deg', 'max_hp', 'regen_per_sec', 'offline_duration']
    .map(key => [key, input(key, ['id', 'label'].includes(key) ? 'text' : 'number')]));
  const work = Object.fromEntries(SHIELD_ENERGY_FIELDS.map(key => [key, input(key)]));
  const loss=input('mismatch-loss'), cycling=input('cycling','checkbox'), frequencies=input('frequencies','text'), period=input('cycle-period');
  const frequencySave=el('button','frequency-save','workshop.shield.frequency_save',{type:'button'});
  node.append(el('h3', null, 'workshop.shield.title'), el('p', null, 'workshop.shield.hint'),
    label(template, 'workshop.shield.template'), label(existing, 'workshop.shield.existing'), create,
    label(materialise, 'workshop.shield.materialise'), status,
    ...Object.entries(fields).map(([key, control]) => label(control, 'workshop.shield.' + key)),
    label(focusAvailable, 'workshop.shield.focus_available'),
    ...Object.entries(work).map(([key, control]) => label(control, 'workshop.shield.' + key)),
    label(targets, 'workshop.shield.targets'), label(compartment, 'workshop.interior.placement'), label(resetAi, 'workshop.shield.reset_ai'), save, remove);
  node.append(el('h4',null,'workshop.shield.frequency_title'),label(loss,'workshop.shield.mismatch_loss'),label(cycling,'workshop.shield.cycling'),
    label(frequencies,'workshop.shield.frequencies'),label(period,'workshop.shield.cycle_period'),frequencySave);
  if (attach) root.append(node);
  let dependencies = null, inspection = null, selected = null, observed = null;
  let loading = false, disposed = false, generation = 0;
  function controls() {
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection;
    fields.id.disabled ||= !!selected; remove.disabled ||= !selected; frequencySave.disabled ||= !inspection?.arcs?.length;
  }
  function paint(arc = selected || defaultShieldArc()) {
    for (const [key, control] of Object.entries(fields)) control.value = arc[key] ?? '';
    for (const [key, control] of Object.entries(work)) control.value = arc.energy?.[key] ?? '';
    focusAvailable.checked = arc.focus_available !== false;
    targets.replaceChildren();
    compartment.replaceChildren();
    for (const room of inspection?.resolved.value.interior?.compartment || []) option(compartment, room.id, room.label || room.id);
    compartment.value = inspection?.resolved.value.interior?.placements?.[shieldArcSystemId(arc.id)] || compartment.options[0]?.value || '';
    for (const target of inspection?.arcs || []) { option(targets, shieldArcSystemId(target.id), target.label || target.id);
      targets.lastChild.selected = (arc.energy?.recovery_targets || []).includes(shieldArcSystemId(target.id)); }
    materialise.checked = false; resetAi.checked = false; controls();
  }
  function select() { selected = inspection?.arcs.find(arc => arc.id === existing.value) || null; paint(); }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const previous = template.value, paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    template.replaceChildren(); for (const path of paths) option(template, path);
    template.value = paths.includes(previous) ? previous : paths[0] || '';
    const revision = JSON.stringify([draft.sourceRevision, template.value, dependencies]);
    if (!force && revision === observed) { controls(); return; }
    if (!template.value) { inspection = null; controls(); return; }
    try {
      inspection = inspectShields(draft, dependencies, template.value); observed = revision;
      const policy=inspection.resolved.value.shields_console;
      loss.value=policy?.frequency_response?.mismatch_loss ?? .5; cycling.checked=!!policy?.frequency_cycle;
      frequencies.value=policy?.frequency_cycle?.frequencies?.join(', ') || ''; period.value=policy?.frequency_cycle?.period_secs ?? '';
      const previousArc = selected?.id; existing.replaceChildren();
      for (const arc of inspection.arcs) option(existing, arc.id, arc.label || arc.id);
      if (inspection.arcs.some(arc => arc.id === previousArc)) existing.value = previousArc;
      select(); status.textContent = t(selected ? 'workshop.shield.ready' : 'workshop.shield.empty');
    } catch (error) { inspection = null; status.textContent = t('workshop.shield.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const result = await runtime.dependencies(); if (!disposed && token === generation) dependencies = result; }
    catch (error) { if (!disposed) status.textContent = t('workshop.shield.refused', { reason: error.message }); }
    finally { loading = false; if (!disposed) inspect(); }
  }
  async function apply(deleting = false) {
    if (busy() || loading || !inspection || (deleting && !selected)) return;
    const arc = structuredClone(selected || defaultShieldArc());
    for (const [key, control] of Object.entries(fields)) {
      if (['id', 'label'].includes(key)) arc[key] = control.value;
      else if (control.value === '') delete arc[key]; else arc[key] = control.valueAsNumber;
    }
    arc.focus_available = focusAvailable.checked;
    arc.energy = Object.fromEntries(Object.entries(work).map(([key, control]) => [key, control.valueAsNumber]));
    arc.energy.recovery_targets = [...targets.selectedOptions].map(option => option.value);
    const token = ++generation, draft = getDraft(), operation = { path: template.value, arc, remove: deleting,
      ...(selected ? { originalId: selected.id } : {}), materialise: materialise.checked, resetAi: resetAi.checked, compartment: compartment.value };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyShieldOperation({ draft, provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { selected = deleting ? null : arc; changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.shield.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  save.addEventListener('click', () => void apply()); remove.addEventListener('click', () => void apply(true));
  frequencySave.onclick=()=>{
    if(busy() || !inspection)return;
    const token=++generation,operation={path:template.value,materialise:materialise.checked,value:{response:{mismatch_loss:loss.valueAsNumber},
      cycle:cycling.checked?{frequencies:frequencies.value.split(',').map(value=>value.trim()===''?NaN:Number(value)),period_secs:period.valueAsNumber}:false}};
    void runWorkshopMutation({setBusy,current:()=>!disposed&&token===generation,start:controls,
      invoke:()=>applyShieldFrequencyOperation({draft:getDraft(),provider,runtime,dependencies,operation,current:()=>!disposed&&token===generation}),
      success:()=>{changed(operation.path);inspect(true);status.textContent=t('workshop.changed');},
      error:error=>{status.textContent=t('workshop.shield.refused',{reason:error.report?.findings?.[0]?.message||error.message});},release:controls});
  };
  create.addEventListener('click', () => { selected = null; existing.selectedIndex = -1; paint(); status.textContent = t('workshop.shield.new'); });
  existing.addEventListener('change', select); template.addEventListener('change', () => { selected = null; inspect(true); });
  inspect(); void load();
  return { node, refresh() { inspect(); void load(); }, dispose() { disposed = true; generation += 1; node.remove(); } };
}
