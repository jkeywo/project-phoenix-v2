import {mountSharedLoaders} from './workshop-shared-loaders.js';
import {applySharedLoaders} from '../editor/workshop-shared-loaders.js';
import { mountGuidedProfile } from './workshop-guided-profile.js';
import { applyGuidedProfileOperation } from '../editor/workshop-guided-profile.js';
import { WEAPON_TARGET_TAGS } from '../editor/workshop-weapon-model.js';
import { t } from './strings.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
import { inspectWeapons, applyWeaponOperation, defaultWeapon, guidedWeaponRange, WEAPON_PARAMETERS, weaponSystemAddress } from '../editor/workshop-weapons.js';

/** Supported weapon data forms, accepted through authoritative validation. */
export function mountWorkshopWeapons({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, id, key, attrs = {}) => {
    const node = doc.createElement(tag); if (id) node.id = 'workshop-weapon-' + id;
    if (key) node.textContent = t(key);
    for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value);
    return node;
  };
  const label = (input, key) => { const node = el('label', null, key, { for: input.id }); node.append(input); return node; };
  const input = (id, type = 'text') => el('input', id, null, { type, ...(type === 'number' ? { step: 'any' } : {}) });
  const node = el('section', 'panel'), template = el('select', 'template'), existing = el('select', 'existing');
  const materialise = input('materialise', 'checkbox'), id = input('id'), strategy = input('strategy-id'), kind = el('select', 'kind');
  const marker = input('marker'), barrels = input('barrels'), status = el('p', 'status', null, { role: 'status' });
  const damageType = el('select', 'damage-type'), penetration = input('penetration', 'number');
  const empEnabled = input('emp-enabled', 'checkbox'), empDuration = input('emp-duration', 'number');
  const operation = el('select', 'operation'), roundsEnabled = input('rounds-enabled', 'checkbox'), roundStore = el('select', 'round-store'), roundAmount = input('round-amount', 'number');
  const targetMode = el('select', 'targets-mode'), targetTags = el('select', 'target-tags', null, { multiple: '', size: '5' });
  const console = el('select', 'console'), group = el('select', 'group'), compartment = el('select', 'compartment');
  for (const type of ['physical', 'laser']) { const option = el('option'); option.value = type; option.textContent = t('workshop.damage.type.' + type); damageType.append(option); }
  const parameters = el('fieldset', 'parameters'), pattern = el('fieldset', 'pattern'), color = input('color');
  const mount = Object.fromEntries(['facing_deg', 'fire_arc_deg', 'range'].map(key => [key, input(key, 'number')]));
  const save = el('button', 'save', 'workshop.weapon.save', { type: 'button' });
  const create = el('button', 'new', 'workshop.weapon.new', { type: 'button' });
  const addStep = el('button', 'add-step', 'workshop.weapon.add_step', { type: 'button' });
  let dependencies = null, inspection = null, selected = null, paramInputs = {}, steps = [], disposed = false, loading = false, generation = 0, observed = null, observedDraft = null, stepSerial = 0;
  const sharedProfile=mountGuidedProfile({root:node,apply: async operation=>{
    if(busy()||loading||!inspection)return;
    if(getDraft()!==observedDraft){inspect(true);return;}
    const token=++generation,draft=getDraft();
    return runWorkshopMutation({setBusy,current:()=>!disposed&&token===generation&&getDraft()===draft,start:controls,
      invoke:()=>applyGuidedProfileOperation({draft,provider,runtime,dependencies,operation,current:()=>!disposed&&token===generation&&getDraft()===draft}),
      success:()=>{changed(operation.path);inspect(true);status.textContent=t('workshop.changed');},
      error:error=>{status.textContent=t('workshop.weapon.refused',{reason:error.report?.findings?.[0]?.message||error.message});},release:controls});
  }});
  const sharedLoaders=mountSharedLoaders({root:node,apply: async operation=>{
    if(busy()||loading||!inspection)return;
    if(getDraft()!==observedDraft){inspect(true);return;}
    const token=++generation,draft=getDraft();
    return runWorkshopMutation({setBusy,current:()=>!disposed&&token===generation&&getDraft()===draft,start:controls,
      invoke:()=>applySharedLoaders({draft,provider,runtime,dependencies,operation,current:()=>!disposed&&token===generation&&getDraft()===draft}),
      success:()=>{changed(operation.path);inspect(true);status.textContent=t('workshop.changed');},
      error:error=>{status.textContent=t('workshop.weapon.refused',{reason:error.report?.findings?.[0]?.message||error.message});},release:controls});
  }});
  const option = (select, value, text) => { const item = el('option'); item.value = value; item.textContent = text; select.append(item); };
  for (const value of ['legacy','explicit']) option(targetMode, value, t('workshop.weapon.targets.' + value));
  for (const tag of WEAPON_TARGET_TAGS) option(targetTags, tag, tag);
  for (const key of Object.keys(WEAPON_PARAMETERS)) option(kind, key, t('workshop.weapon.' + key));
  for (const value of ['fixed','sustained']) option(operation, value, t('workshop.weapon.operation.' + value));
  node.append(el('h3', null, 'workshop.weapon.title'), el('p', null, 'workshop.weapon.hint'), label(template, 'workshop.weapon.template'),
    label(existing, 'workshop.weapon.existing'), create, label(materialise, 'workshop.weapon.materialise'), status,
    label(id, 'workshop.weapon.id'), label(strategy, 'workshop.weapon.strategy_id'), label(kind, 'workshop.weapon.delivery'),
    ...Object.entries(mount).map(([key, control]) => label(control, 'workshop.weapon.' + key)),
    label(damageType, 'workshop.damage.damage_type'), label(penetration, 'workshop.damage.penetration'),
    label(empEnabled, 'workshop.damage.emp_payload'), label(empDuration, 'workshop.damage.emp_duration'),
    label(targetMode, 'workshop.weapon.targets.mode'), label(targetTags, 'workshop.weapon.targets.tags'), el('p', null, 'workshop.weapon.targets.hint'),
    label(operation, 'workshop.weapon.operation'), label(roundsEnabled, 'workshop.weapon.rounds'), label(roundStore, 'workshop.weapon.round_store'), label(roundAmount, 'workshop.weapon.round_amount'),
    label(console, 'workshop.weapon.console'), label(group, 'workshop.weapon.group'), label(compartment, 'workshop.weapon.compartment'),
    label(marker, 'workshop.weapon.marker'), label(barrels, 'workshop.weapon.barrels'), label(color, 'workshop.weapon.color'), parameters, pattern, save);
  if (attach) root.append(node);
  function controls() {
    const disabled = disposed || loading || busy() || !inspection;
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disabled;
    id.disabled ||= !!selected; kind.disabled ||= !!selected;
    strategy.disabled ||= !!selected && selected.section !== 'weapon';
    mount.range.disabled ||= kind.value === 'guided';
    barrels.disabled ||= kind.value === 'beam';
    color.disabled ||= kind.value !== 'beam';
    empDuration.disabled ||= !empEnabled.checked;
    targetTags.disabled ||= targetMode.value !== 'explicit';
    operation.disabled ||= kind.value !== 'beam'; roundsEnabled.disabled ||= kind.value !== 'bolt';
    roundStore.disabled ||= kind.value !== 'bolt' || !roundsEnabled.checked; roundAmount.disabled ||= roundStore.disabled;
    if (kind.value === 'beam' && operation.value === 'sustained') for (const key of ['duration_secs','cycle_jitter']) if (paramInputs[key]) paramInputs[key].disabled = true;
    pattern.hidden = kind.value === 'beam';
    for (const control of [console, group, compartment]) control.disabled ||= !!selected;
    sharedProfile.refresh(inspection,disabled,getDraft());
    sharedLoaders.refresh(inspection,disabled,getDraft());
  }
  function addPatternStep(value = { offset_secs: 0, barrels: [0] }) {
    const index = stepSerial++, row = el('div'), offset = input('step-offset-' + index, 'number'), indices = input('step-barrels-' + index);
    offset.value = value.offset_secs ?? 0; indices.value = value.barrels.join(', ');
    const remove = el('button', null, 'workshop.weapon.remove_step', { type: 'button' });
    const step = { row, offset, indices };
    remove.addEventListener('click', () => { steps = steps.filter(item => item !== step); row.remove(); });
    row.append(label(offset, 'workshop.weapon.offset'), label(indices, 'workshop.weapon.indices'), remove);
    pattern.insertBefore(row, addStep); steps.push(step); controls();
  }
  function paint(value) {
    id.value = value.id; strategy.value = value.strategy_id || ''; kind.value = value.delivery.kind;
    damageType.value = value.impact?.damage_type || ''; penetration.value = value.impact?.penetration ?? '';
    empEnabled.checked = !!value.impact?.emp; empDuration.value = value.impact?.emp?.duration_secs ?? 6;
    const definition = inspection?.resolved.value || {};
    targetMode.value = Array.isArray(value.targets) ? 'explicit' : 'legacy';
    for (const option of targetTags.options) option.selected = Array.isArray(value.targets) && value.targets.includes(option.value);
    operation.value = value.delivery.operation || 'fixed';
    roundsEnabled.checked = !!value.delivery.rounds; roundAmount.value = value.delivery.rounds?.amount ?? 1;
    roundStore.replaceChildren();
    for (const row of definition.logistics?.store || []) if (row.role === 'local' && row.system === weaponSystemAddress(value).id && row.endpoint?.kind === 'capacity') option(roundStore, row.id, row.id);
    if (value.delivery.rounds?.store) roundStore.value = value.delivery.rounds.store;
    for (const [control, values] of [[console, (definition.console || []).map(row => row.id)], [group, (definition.energy?.group || []).map(row => row.id)], [compartment, (definition.interior?.compartment || []).map(row => row.id)]]) {
      const previous = control.value; control.replaceChildren(); for (const value of values) option(control, value, value); control.value = values.includes(previous) ? previous : values[0] || '';
    }
    if (selected) {
      const id = weaponSystemAddress(value).id, owner = definition.system?.find(system => system.id === id);
      console.value = owner?.console || ''; group.value = owner?.power_group || ''; compartment.value = definition.interior?.placements?.[id] || '';
    }
    for (const [key, control] of Object.entries(mount)) control.value = value.mount[key];
    marker.value = value.marker || ''; barrels.value = (value.barrels || []).join(', '); color.value = (value.delivery.color || []).join(', ');
    parameters.replaceChildren(el('legend', null, 'workshop.weapon.parameters')); paramInputs = {};
    for (const key of WEAPON_PARAMETERS[value.delivery.kind]) {
      const control = input('parameter-' + key, 'number'); control.value = value.delivery[key] ?? '';
      paramInputs[key] = control; parameters.append(label(control, 'workshop.weapon.param.' + key));
    }
    pattern.replaceChildren(el('legend', null, 'workshop.weapon.pattern'), el('p', null, 'workshop.weapon.pattern_hint'), addStep); steps = [];
    for (const step of value.pattern || []) addPatternStep(step);
    materialise.checked = false; controls();
  }
  function select() {
    selected = inspection?.weapons[Number(existing.value)] || null;
    paint(selected?.weapon || defaultWeapon(kind.value));
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const previous = template.value, paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    template.replaceChildren(); for (const path of paths) option(template, path, path);
    template.value = paths.includes(previous) ? previous : paths[0] || '';
    if (!template.value) { inspection = null; controls(); return; }
    const revision = JSON.stringify([draft.sourceRevision, template.value, dependencies]);
    if (!force && revision === observed && draft === observedDraft) { controls(); return; }
    try {
      if(draft!==observedDraft){generation+=1;selected=null;}
      inspection = inspectWeapons(draft, dependencies, template.value); observed = revision; observedDraft = draft;
      const previousWeapon = selected?.weapon.id, previousSection = selected?.section;
      existing.replaceChildren(); inspection.weapons.forEach((row, index) => option(existing, String(index), `${row.weapon.id} (${t('workshop.weapon.' + row.weapon.delivery.kind)})`));
      const index = inspection.weapons.findIndex(row => row.weapon.id === previousWeapon && row.section === previousSection);
      if (index >= 0) existing.value = String(index);
      select(); status.textContent = selected?.section || t('workshop.weapon.empty');
    } catch (error) { inspection = null; status.textContent = t('workshop.weapon.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const result = await runtime.dependencies(); if (!disposed && token === generation) dependencies = result; }
    catch (error) { if (!disposed) status.textContent = t('workshop.weapon.refused', { reason: error.message }); }
    finally { loading = false; if (!disposed) inspect(); }
  }
  const names = value => value.split(',').map(item => item.trim()).filter(Boolean);
  async function apply() {
    if (busy() || loading || !inspection) return;
    if(getDraft()!==observedDraft){inspect(true);return;}
    const value = structuredClone(selected?.weapon || defaultWeapon(kind.value));
    value.targets = targetMode.value === 'explicit' ? [...targetTags.selectedOptions].map(option => option.value) : false;
    value.id = id.value; value.mount = Object.fromEntries(Object.entries(mount).map(([key, control]) => [key, control.valueAsNumber]));
    value.impact = { damage_type: damageType.value, penetration: penetration.valueAsNumber,
      ...(empEnabled.checked ? { emp: { duration_secs: empDuration.valueAsNumber } } : {}) };
    if (strategy.value) value.strategy_id = strategy.value; else delete value.strategy_id;
    value.delivery = { kind: kind.value };
    for (const [key, control] of Object.entries(paramInputs)) if (control.value !== '') value.delivery[key] = control.valueAsNumber;
    if (kind.value === 'beam') value.delivery.operation = operation.value;
    if (kind.value === 'bolt' && roundsEnabled.checked) value.delivery.rounds = { store: roundStore.value, amount: roundAmount.valueAsNumber };
    if (kind.value === 'beam' && color.value.trim()) value.delivery.color = names(color.value).map(Number);
    delete value.marker; if (marker.value) value.marker = marker.value;
    value.barrels = kind.value === 'beam' ? [] : names(barrels.value);
    value.pattern = kind.value === 'beam' ? [] : steps.map(step => ({ offset_secs: step.offset.valueAsNumber, barrels: names(step.indices.value).map(Number) }));
    const token = ++generation, draft = getDraft(), editOperation = { path: template.value, section: selected?.section,
      console: console.value, power_group: group.value, compartment: compartment.value,
      ...(selected ? { originalId: selected.weapon.id } : {}), weapon: value, materialise: materialise.checked };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation && getDraft() === draft, start: controls,
      invoke: () => applyWeaponOperation({ draft, provider, runtime, dependencies, operation: editOperation, current: () => !disposed && token === generation && getDraft() === draft }),
      success: () => { selected = { section: editOperation.section || 'weapon', weapon: value }; changed(editOperation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.weapon.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  save.addEventListener('click', () => void apply());
  empEnabled.addEventListener('change', controls);
  roundsEnabled.addEventListener('change', controls);
  targetMode.addEventListener('change', controls);
  operation.addEventListener('change', () => { if (operation.value === 'sustained' && paramInputs.cycle_jitter) paramInputs.cycle_jitter.value = '0'; controls(); });
  create.addEventListener('click', () => { selected = null; existing.selectedIndex = -1; const value = defaultWeapon(kind.value);
    if (kind.value === 'guided') value.mount.range = guidedWeaponRange(inspection.resolved.value);
    paint(value); status.textContent = t('workshop.weapon.new'); });
  kind.addEventListener('change', () => { if (!selected) { const value = defaultWeapon(kind.value); value.id = id.value; value.strategy_id = strategy.value;
    if (kind.value === 'guided') value.mount.range = guidedWeaponRange(inspection.resolved.value);
    paint(value); } });
  template.addEventListener('change', () => { selected = null; inspect(true); }); existing.addEventListener('change', select);
  addStep.addEventListener('click', () => addPatternStep());
  inspect(); void load();
  return { node, refresh() { inspect(); void load(); }, dispose() { disposed = true; generation += 1; sharedProfile.dispose(); sharedLoaders.dispose(); node.remove(); } };
}
