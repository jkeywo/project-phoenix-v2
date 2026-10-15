import { t } from './strings.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
import { inspectDamage, applyDamageOperation } from '../editor/workshop-damage.js';
import { DAMAGE_TYPES, DAMAGE_BINDINGS, defaultDamageComponent, defaultEmpResistance } from '../editor/workshop-damage-model.js';

/** Supported physical definitions, edited through the existing accepted source history. */
export function mountWorkshopDamage({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument, clone = value => structuredClone(value);
  const el = (tag, id, key, attrs = {}) => { const node = doc.createElement(tag); if (id) node.id = 'workshop-damage-' + id;
    if (key) node.textContent = t('workshop.damage.' + key); for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value); return node; };
  const input = (id, type = 'text') => el('input', id, null, { type, ...(type === 'number' ? { step: 'any' } : {}) });
  const label = (control, key) => { const node = el('label', null, key, { for: control.id }); node.append(control); return node; };
  const option = (select, value, text = value) => { const node = el('option'); node.value = value; node.textContent = text; select.append(node); };
  const button = (id, key, action) => { const node = el('button', id, key, { type: 'button' }); node.addEventListener('click', action); return node; };
  const fill = (select, values, previous) => { select.replaceChildren(); for (const value of values) option(select, value); select.value = values.includes(previous) ? previous : values[0] || ''; };
  const node = el('section', 'panel'), template = el('select', 'template'), status = el('p', 'status', null, { role: 'status' });
  const materialise = input('materialise', 'checkbox'), materials = el('select', 'materials'), materialId = input('material-id');
  const empEnabled = input('emp-enabled', 'checkbox'), empResistance = input('emp-resistance', 'number'), empStacks = input('emp-stacks', 'number'), empDecay = input('emp-decay', 'number');
  const empBindings = Object.fromEntries(DAMAGE_BINDINGS.map(binding => [binding, input('emp-binding-' + binding, 'checkbox')]));
  const resistance = Object.fromEntries(DAMAGE_TYPES.map(type => [type, input('resistance-' + type, 'number')])), residual = input('residual', 'number');
  const remedies = el('select', 'remedies'), remedyId = input('remedy-id'), work = input('work-secs', 'number'), manual = input('manual-work-fraction', 'number'), hp = input('structural-hp', 'number');
  const outcome = el('select', 'outcome'), patchFactor = input('patch-factor', 'number'), requirements = el('div', 'requirements');
  const systems = el('select', 'systems'), components = el('select', 'components'), componentId = input('component-id'), binding = el('select', 'binding');
  const susceptibility = Object.fromEntries(DAMAGE_TYPES.map(type => [type, input('susceptibility-' + type, 'number')]));
  const threshold = input('threshold', 'number'), degraded = input('degraded', 'number'), full = el('select', 'full-remedy'), patch = el('select', 'patch-remedy'), field = input('field-replaceable', 'checkbox');
  for (const kind of ['structural', 'full', 'patch']) option(outcome, kind, t('workshop.damage.outcome.' + kind));
  for (const kind of DAMAGE_BINDINGS) option(binding, kind, t('workshop.damage.binding.' + kind));
  let inspection = null, dependencies = null, loading = false, disposed = false, generation = 0, observed = null, partRows = [], partSerial = 0;
  const selectedSystem = () => inspection?.systems.find(system => system.id === systems.value);
  const material = () => inspection?.damage?.materials[materials.value];
  const remedy = () => inspection?.damage?.remedies[remedies.value];
  const selectedComponent = () => selectedSystem()?.damage_component?.find(component => component.id === components.value);
  const create = button('create', 'create', () => void apply({ type: 'damage-create' }));
  function controls() {
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection;
    create.disabled ||= !!inspection?.damage;
    for (const section of node.querySelectorAll('fieldset')) section.disabled = !inspection?.damage || disposed || loading || busy();
    hp.disabled ||= outcome.value !== 'structural'; patchFactor.disabled ||= outcome.value !== 'patch';
    for (const control of [empResistance, empStacks, empDecay, ...Object.values(empBindings)]) control.disabled ||= !empEnabled.checked;
  }
  function paintMaterial(value = material()) {
    materialId.value = materials.value; for (const [type, control] of Object.entries(resistance)) control.value = value?.resistance[type] ?? 0;
    residual.value = value?.residual_fraction ?? 0.1;
  }
  function addRequirement(value = { resource: 'machinery_parts', service: 'machinery_parts', amount: 1 }) {
    const index = partSerial++, row = el('div'), resource = input('part-resource-' + index), service = input('part-service-' + index), amount = input('part-amount-' + index, 'number');
    resource.value = value.resource; service.value = value.service; amount.value = value.amount;
    const state = { row, resource, service, amount };
    row.append(label(resource, 'resource'), label(service, 'service'), label(amount, 'amount'), button(null, 'remove', () => { partRows = partRows.filter(item => item !== state); row.remove(); }));
    requirements.append(row); partRows.push(state);
  }
  function paintRemedy(value = remedy()) {
    remedyId.value = remedies.value; work.value = value?.work_secs ?? 4; manual.value = value?.manual_work_fraction ?? 0.25; hp.value = value?.structural_hp ?? ''; outcome.value = value?.outcome.kind || 'full';
    patchFactor.value = value?.outcome.factor ?? 0.5; requirements.replaceChildren(); partRows = [];
    for (const requirement of value?.requirements || []) addRequirement(requirement); controls();
  }
  function paintComponent(value = selectedComponent() || defaultDamageComponent()) {
    componentId.value = value.id; binding.value = value.binding; threshold.value = value.fault_dose_threshold; degraded.value = value.degraded_factor;
    for (const [type, control] of Object.entries(susceptibility)) control.value = value.susceptibility[type];
    const recipes = inspection?.damage?.remedies || {};
    fill(full, Object.keys(recipes).filter(id => recipes[id].outcome.kind === 'full'), value.full_remedy);
    fill(patch, ['', ...Object.keys(recipes).filter(id => recipes[id].outcome.kind === 'patch')], value.patch_remedy || ''); field.checked = value.field_replaceable;
  }
  function paintComponents() { fill(components, (selectedSystem()?.damage_component || []).map(component => component.id), components.value); paintComponent(); }
  function paint() {
    const emp = inspection?.damage?.emp || defaultEmpResistance(); empEnabled.checked = !!inspection?.damage?.emp;
    empResistance.value = emp.resistance_per_stack; empStacks.value = emp.max_stacks; empDecay.value = emp.decay_secs;
    for (const [binding, control] of Object.entries(empBindings)) control.checked = emp.bindings.includes(binding);
    fill(materials, Object.keys(inspection?.damage?.materials || {}), materials.value); paintMaterial();
    fill(remedies, Object.keys(inspection?.damage?.remedies || {}), remedies.value); paintRemedy();
    fill(systems, (inspection?.systems || []).map(system => system.id), systems.value); paintComponents(); controls();
  }
  async function apply(operation) {
    if (busy() || loading || !inspection) return;
    const token = ++generation, selected = { ...operation, path: template.value, materialise: materialise.checked };
    return runWorkshopMutation({ setBusy, current: () => !disposed && generation === token, start: controls,
      invoke: () => applyDamageOperation({ draft: getDraft(), provider, runtime, dependencies, operation: selected, current: () => !disposed && generation === token }),
      success: () => { changed(selected.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.damage.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  const saveMaterial = button('save-material', 'save_material', () => {
    const damage = clone(inspection.damage); if (materialId.value !== materials.value) delete damage.materials[materials.value];
    damage.materials[materialId.value] = { resistance: Object.fromEntries(Object.entries(resistance).map(([type, control]) => [type, control.valueAsNumber])), residual_fraction: residual.valueAsNumber };
    void apply({ type: 'damage-profile', damage });
  });
  const saveRemedy = button('save-remedy', 'save_remedy', () => {
    const damage = clone(inspection.damage); if (remedyId.value !== remedies.value) delete damage.remedies[remedies.value];
    damage.remedies[remedyId.value] = { work_secs: work.valueAsNumber, manual_work_fraction: manual.valueAsNumber, ...(outcome.value === 'structural' ? { structural_hp: hp.valueAsNumber } : {}),
      outcome: { kind: outcome.value, ...(outcome.value === 'patch' ? { factor: patchFactor.valueAsNumber } : {}) },
      requirements: partRows.map(row => ({ resource: row.resource.value, service: row.service.value, amount: row.amount.valueAsNumber })) };
    void apply({ type: 'damage-profile', damage });
  });
  const saveComponent = button('save-component', 'save_component', () => {
    const rows = clone(selectedSystem().damage_component || []), value = { id: componentId.value, binding: binding.value,
      susceptibility: Object.fromEntries(Object.entries(susceptibility).map(([type, control]) => [type, control.valueAsNumber])), fault_dose_threshold: threshold.valueAsNumber,
      degraded_factor: degraded.valueAsNumber, full_remedy: full.value, ...(patch.value ? { patch_remedy: patch.value } : {}), field_replaceable: field.checked };
    const index = rows.findIndex(row => row.id === components.value); if (index >= 0) rows[index] = value; else rows.push(value);
    void apply({ type: 'damage-components', system: systems.value, components: rows });
  });
  const materialSection = el('fieldset'), remedySection = el('fieldset'), componentSection = el('fieldset');
  const empSection = el('fieldset');
  const saveEmp = button('save-emp', 'save_emp', () => {
    const damage = clone(inspection.damage);
    if (empEnabled.checked) damage.emp = { resistance_per_stack: empResistance.valueAsNumber, max_stacks: empStacks.valueAsNumber,
      decay_secs: empDecay.valueAsNumber, bindings: Object.entries(empBindings).filter(([, control]) => control.checked).map(([binding]) => binding) };
    else delete damage.emp;
    void apply({ type: 'damage-profile', damage });
  });
  empSection.append(el('legend', null, 'emp_receiver'), label(empEnabled, 'emp_enabled'), label(empResistance, 'emp_resistance'), label(empStacks, 'emp_stacks'), label(empDecay, 'emp_decay'),
    ...Object.entries(empBindings).map(([binding, control]) => label(control, 'binding.' + binding)), saveEmp);
  empEnabled.addEventListener('change', controls);
  materialSection.append(el('legend', null, 'materials'), label(materials, 'material'), label(materialId, 'id'), ...Object.entries(resistance).map(([type, control]) => label(control, 'resistance.' + type)), label(residual, 'residual'),
    button('new-material', 'new', () => { materials.selectedIndex = -1; paintMaterial({ resistance: { physical: 1, laser: 1 }, residual_fraction: 0.1 }); materialId.value = ''; }), saveMaterial,
    button('remove-material', 'remove', () => { const damage = clone(inspection.damage); delete damage.materials[materials.value]; void apply({ type: 'damage-profile', damage }); }));
  remedySection.append(el('legend', null, 'remedies'), label(remedies, 'remedy'), label(remedyId, 'id'), label(outcome, 'outcome'), label(work, 'work_secs'), label(manual, 'manual_work_fraction'), label(hp, 'structural_hp'), label(patchFactor, 'patch_factor'), requirements,
    button('add-part', 'add_part', () => addRequirement()), button('new-remedy', 'new', () => { remedies.selectedIndex = -1; paintRemedy(); remedyId.value = ''; }), saveRemedy,
    button('remove-remedy', 'remove', () => { const damage = clone(inspection.damage); delete damage.remedies[remedies.value]; void apply({ type: 'damage-profile', damage }); }));
  componentSection.append(el('legend', null, 'components'), label(systems, 'system'), label(components, 'component'), label(componentId, 'id'), label(binding, 'binding'),
    ...Object.entries(susceptibility).map(([type, control]) => label(control, 'susceptibility.' + type)), label(threshold, 'threshold'), label(degraded, 'degraded'), label(full, 'full_remedy'), label(patch, 'patch_remedy'), label(field, 'field_replaceable'),
    button('new-component', 'new', () => { components.selectedIndex = -1; paintComponent(); }), saveComponent,
    button('remove-component', 'remove', () => void apply({ type: 'damage-components', system: systems.value, components: (selectedSystem().damage_component || []).filter(row => row.id !== components.value) })));
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), create, status, materialSection, remedySection, componentSection, empSection);
  if (attach) root.append(node);
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const previous = template.value, paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    fill(template, paths, previous);
    const revision = JSON.stringify([draft.sourceRevision, template.value, dependencies]);
    if (!force && revision === observed) { controls(); return; }
    try { inspection = inspectDamage(draft, dependencies, template.value); observed = revision; paint(); status.textContent = ''; }
    catch (error) { inspection = null; status.textContent = t('workshop.damage.refused', { reason: error.message }); controls(); }
  }
  async function load() { if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try {
      const [result, schema] = await Promise.all([runtime.dependencies(), typeof runtime.shipSchema === 'function' ? runtime.shipSchema() : null]);
      if (!disposed && token === generation) {
        dependencies = result;
        if (schema?.damage) {
          const vocabulary = schema.damage;
          outcome.replaceChildren(); binding.replaceChildren();
          for (const kind of vocabulary.outcomes.filter(kind => ['structural', 'full', 'patch'].includes(kind))) option(outcome, kind, t('workshop.damage.outcome.' + kind));
          for (const kind of vocabulary.bindings.filter(kind => DAMAGE_BINDINGS.includes(kind))) option(binding, kind, t('workshop.damage.binding.' + kind));
        }
      }
    }
    catch (error) { if (!disposed) status.textContent = t('workshop.damage.refused', { reason: error.message }); }
    finally { loading = false; if (!disposed) inspect(); }
  }
  template.addEventListener('change', () => inspect(true)); materials.addEventListener('change', () => paintMaterial()); remedies.addEventListener('change', () => paintRemedy());
  systems.addEventListener('change', paintComponents); components.addEventListener('change', () => paintComponent()); outcome.addEventListener('change', controls);
  inspect(); void load();
  return { node, refresh() { inspect(); void load(); }, dispose() { disposed = true; generation++; node.remove(); } };
}
