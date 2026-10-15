import { t } from './strings.js';
import { inspectFitting, fittingHardware, emptyFitting, applyFittingOperation } from '../editor/workshop-fitting.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

/** Closed equipment records edit the existing exact-source Workshop document. */
export function mountWorkshopFitting({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument, text = key => t(key.startsWith('workshop.') ? key : 'workshop.fitting.' + key);
  const el = (tag, key) => { const node = doc.createElement(tag); if (key) node.textContent = text(key); return node; };
  let serial = 0;
  const input = (key, type = 'text') => { const node = el('input'); node.id = 'workshop-fitting-' + key + '-' + serial++; node.type = type;
    if (type === 'number') { node.min = '0'; node.step = 'any'; } return node; };
  const select = key => { const node = el('select'); node.id = 'workshop-fitting-' + key + '-' + serial++; return node; };
  const label = (control, key) => { const node = el('label', key); node.htmlFor = control.id; node.append(control); return node; };
  const button = (key, action) => { const node = el('button', key); node.type = 'button'; node.dataset.fittingAction = key; node.addEventListener('click', action); return node; };
  const fill = (control, ids, previous = control.value) => { control.replaceChildren(); for (const id of ids) { const option = el('option'); option.value = id; option.textContent = !id ? text('none') : [adapter, workClass, condition].includes(control) || control.id.includes('damage-type') ? text('option.' + id) : id; control.append(option); }
    control.value = ids.includes(previous) ? previous : ids[0] || ''; };
  const section = key => { const node = el('fieldset'); node.append(el('legend', key)); return node; };
  const node = el('section'), template = select('template'), materialise = input('materialise', 'checkbox'), status = el('p'); node.id = 'workshop-fitting-panel'; status.setAttribute('role', 'status');
  const slots = select('slots'), slotId = input('slot-id'), system = select('system'), adapter = select('adapter'), slotInterface = input('slot-interface'), slotSize = input('slot-size', 'number'), defaultEquipment = input('default'), operation = select('operation');
  const definitions = select('definitions'), definitionId = input('definition-id'), definitionInterface = input('definition-interface'), definitionSize = input('definition-size', 'number'), storage = input('storage-space', 'number'), workClass = select('work-class'), workSecs = input('work-secs', 'number'), manual = input('manual', 'number');
  const hardware = section('hardware'), requirements = section('requirements');
  const lockers = select('lockers'), lockerId = input('locker-id'), room = select('room'), space = input('locker-space', 'number');
  const assemblies = select('assemblies'), assemblyId = input('assembly-id'), assemblyDefinition = select('assembly-definition'), locker = select('assembly-locker'), condition = select('condition'), dose = input('dose', 'number'), source = input('source'), patch = input('patch-factor', 'number');
  let inspection = null, dependencies = null, observed = null, disposed = false, loading = false, generation = 0, hardwareInputs = new Map(), requirementRows = [];
  const profile = () => ({ ...emptyFitting(), ...structuredClone(inspection?.fitting || {}) });
  const value = () => inspection?.resolved.value;
  const currentSlot = () => profile().slot.find(row => row.id === slots.value);
  const currentDefinition = () => profile().equipment.find(row => row.id === definitions.value);
  const eligible = () => inspection?.systems.filter(row => ['phaser_bank', 'power_reactor'].includes(row.kind)) || [];
  const replace = (list, old, row) => [...list.filter(item => item.id !== old && item.id !== row.id), row];
  fill(adapter, ['beam', 'reactor']); fill(workClass, ['field', 'facility']); fill(condition, ['healthy', 'damaged', 'destroyed', 'patched']);

  function paintSlot(row = currentSlot()) {
    fill(system, eligible().map(row => row.id), row?.system); const actual = eligible().find(row => row.id === system.value);
    slotId.value = row?.id || 'equipment-slot'; adapter.value = row?.family || (actual?.kind === 'power_reactor' ? 'reactor' : 'beam');
    slotInterface.value = row?.interface || adapter.value; slotSize.value = row?.size || 1; defaultEquipment.value = row?.default_equipment || slotId.value + '-standard';
    fill(operation, actual?.damage_component?.filter(row => row.binding === 'operation').map(row => row.id) || [], row?.operation_component); adapter.disabled = true;
  }
  function addRequirement(part = { resource: '', service: '', amount: 1 }) {
    const row = el('div'), resource = input('part-resource'), service = select('part-service'), amount = input('part-amount', 'number');
    resource.value = part.resource; fill(service, Object.keys(inspection?.services || {}), part.service); amount.value = part.amount; amount.step = '1'; amount.min = '1';
    const entry = { row, resource, service, amount }; requirementRows.push(entry);
    row.append(label(resource, 'resource'), label(service, 'service'), label(amount, 'amount'), button('remove', () => { row.remove(); requirementRows = requirementRows.filter(other => other !== entry); })); requirements.append(row);
  }
  function paintDefinition(row = currentDefinition()) {
    const slot = currentSlot() || profile().slot[0], actual = inspection?.systems.find(row => row.id === slot?.system) || eligible()[0];
    let base; try { base = actual && fittingHardware(value(), actual); } catch { base = null; }
    const record = row || { id: 'equipment-sidegrade', interface: slot?.interface || 'beam', size: slot?.size || 1, storage_space: 1,
      work: { class: base?.kind === 'reactor' ? 'facility' : 'field', work_secs: 20, manual_work_fraction: 0.25, requirements: [] }, hardware: base };
    definitionId.value = record.id; definitionInterface.value = record.interface; definitionSize.value = record.size; storage.value = record.storage_space;
    workClass.value = record.work.class; workSecs.value = record.work.work_secs; manual.value = record.work.manual_work_fraction;
    hardware.replaceChildren(el('legend', 'hardware')); hardwareInputs = new Map();
    if (record.hardware) for (const [key, amount] of Object.entries(record.hardware.kind === 'beam' ? { operation: 'fixed', ...record.hardware } : record.hardware)) {
      if (key === 'kind') { hardware.dataset.kind = amount; continue; }
      if (key === 'operation') {
        const control = select('beam-operation');
        for (const value of ['fixed','sustained']) { const option = el('option'); option.value = value; option.textContent = t('workshop.weapon.operation.' + value); control.append(option); }
        control.value = amount; hardwareInputs.set(key, control); hardware.append(label(control, 'workshop.weapon.operation'));
      } else if (key === 'impact') {
        const type = select('damage-type'), penetration = input('penetration', 'number'); fill(type, ['laser', 'physical'], amount.damage_type); penetration.value = amount.penetration;
        const emp = input('emp-enabled', 'checkbox'), duration = input('emp-duration', 'number'); emp.checked = !!amount.emp; duration.value = amount.emp?.duration_secs ?? 6; duration.disabled = !emp.checked;
        emp.addEventListener('change', () => { duration.disabled = !emp.checked; });
        hardware.append(label(type, 'damage_type'), label(penetration, 'penetration'), label(emp, 'workshop.damage.emp_payload'), label(duration, 'workshop.damage.emp_duration')); hardwareInputs.set('impact', { type, penetration, emp, duration });
      } else { const control = input(key, 'number'); control.value = amount; hardwareInputs.set(key, control); hardware.append(label(control, key)); }
    }
    requirements.replaceChildren(el('legend', 'requirements')); requirementRows = []; for (const part of record.work.requirements || []) addRequirement(part);
  }
  function paintLocker(row = profile().locker.find(row => row.id === lockers.value)) { lockerId.value = row?.id || 'equipment-locker'; fill(room, (inspection?.rooms || []).map(row => row.id), row?.compartment); space.value = row?.space || 10; }
  function paintAssembly(row = profile().assembly.find(row => row.id === assemblies.value)) {
    assemblyId.value = row?.id || 'equipment-spare'; fill(assemblyDefinition, profile().equipment.map(row => row.id), row?.definition); fill(locker, profile().locker.map(row => row.id), row?.location?.locker);
    condition.value = row?.condition?.condition.kind || 'healthy'; dose.value = row?.condition?.dose || 0; source.value = row?.condition?.last_source || ''; patch.value = row?.condition?.condition.factor || 0.5;
    const installed = row?.location.kind === 'installed'; for (const control of [assemblyId, assemblyDefinition, locker, condition, dose, source, patch, saveAssembly, removeAssembly]) control.disabled = installed || busy();
  }
  function controls() {
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection;
    adapter.disabled = true;
    if (inspection) paintAssemblyDisabled();
  }
  function paintAssemblyDisabled() { const installed = profile().assembly.find(row => row.id === assemblies.value)?.location.kind === 'installed';
    if (installed) for (const control of [assemblyId, assemblyDefinition, locker, condition, dose, source, patch, saveAssembly, removeAssembly]) control.disabled = true; }
  function paint() {
    const current = profile(); fill(slots, ['', ...current.slot.map(row => row.id)]); fill(definitions, ['', ...current.equipment.map(row => row.id)]);
    fill(lockers, ['', ...current.locker.map(row => row.id)]); fill(assemblies, ['', ...current.assembly.map(row => row.id)]);
    paintSlot(); paintDefinition(); paintLocker(); paintAssembly(); controls();
  }
  async function apply(next) {
    if (!inspection || busy()) return; const token = ++generation, operation = { path: template.value, value: next, materialise: materialise.checked };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyFittingOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.fitting.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  const saveSlot = button('save-slot', () => {
    const next = profile(), old = currentSlot(), actual = inspection?.systems.find(row => row.id === system.value); if (!actual) return;
    const row = { id: slotId.value, system: actual.id, family: adapter.value, interface: slotInterface.value, size: slotSize.valueAsNumber, default_equipment: defaultEquipment.value, operation_component: operation.value };
    next.slot = replace(next.slot, old?.id, row);
    if (!old) {
      let base; try { base = fittingHardware(value(), actual); } catch (error) { status.textContent = error.message; return; }
      if (!next.equipment.some(item => item.id === row.default_equipment)) next.equipment.push({ id: row.default_equipment, interface: row.interface, size: row.size, storage_space: 1,
        work: { class: row.family === 'reactor' ? 'facility' : 'field', work_secs: 20, manual_work_fraction: 0.25, requirements: [] }, hardware: base });
      next.assembly.push({ id: row.id + '-installed', definition: row.default_equipment, location: { kind: 'installed', slot: row.id } });
    } else {
      for (const item of next.assembly) if (item.location.kind === 'installed' && item.location.slot === old.id) { item.location.slot = row.id; item.definition = row.default_equipment; }
    }
    void apply(next);
  });
  const removeSlot = button('remove-slot', () => { const next = profile(); next.slot = next.slot.filter(row => row.id !== slots.value); next.assembly = next.assembly.filter(item => item.location.kind !== 'installed' || item.location.slot !== slots.value); void apply(next); });
  const saveDefinition = button('save-definition', () => { const next = profile(), values = { kind: hardware.dataset.kind };
    for (const [key, control] of hardwareInputs) values[key] = key === 'impact' ? { damage_type: control.type.value, penetration: control.penetration.valueAsNumber,
      ...(control.emp.checked ? { emp: { duration_secs: control.duration.valueAsNumber } } : {}) } : key === 'operation' ? control.value : control.valueAsNumber;
    const row = { id: definitionId.value, interface: definitionInterface.value, size: definitionSize.valueAsNumber, storage_space: storage.valueAsNumber,
      work: { class: workClass.value, work_secs: workSecs.valueAsNumber, manual_work_fraction: manual.valueAsNumber,
        requirements: requirementRows.map(part => ({ resource: part.resource.value, service: part.service.value, amount: part.amount.valueAsNumber })) }, hardware: values };
    next.equipment = replace(next.equipment, definitions.value, row); void apply(next);
  });
  const removeDefinition = button('remove-definition', () => { const next = profile(); next.equipment = next.equipment.filter(row => row.id !== definitions.value); void apply(next); });
  const saveLocker = button('save-locker', () => { const next = profile(); next.locker = replace(next.locker, lockers.value, { id: lockerId.value, compartment: room.value, space: space.valueAsNumber }); void apply(next); });
  const removeLocker = button('remove-locker', () => { const next = profile(); next.locker = next.locker.filter(row => row.id !== lockers.value); void apply(next); });
  const saveAssembly = button('save-assembly', () => { const next = profile(), state = { condition: { kind: condition.value, ...(condition.value === 'patched' ? { factor: patch.valueAsNumber } : {}) }, dose: dose.valueAsNumber, ...(source.value ? { last_source: source.value } : {}) };
    next.assembly = replace(next.assembly, assemblies.value, { id: assemblyId.value, definition: assemblyDefinition.value, location: { kind: 'stored', locker: locker.value }, condition: state }); void apply(next); });
  const removeAssembly = button('remove-assembly', () => { const next = profile(); next.assembly = next.assembly.filter(row => row.id !== assemblies.value); void apply(next); });
  const slotPanel = section('slots'), definitionPanel = section('definitions'), lockerPanel = section('lockers'), assemblyPanel = section('assemblies');
  slotPanel.append(label(slots, 'select-slot'), label(slotId, 'id'), label(system, 'system'), label(adapter, 'family'), label(slotInterface, 'interface'), label(slotSize, 'size'), label(defaultEquipment, 'default'), label(operation, 'operation'), saveSlot, removeSlot);
  definitionPanel.append(label(definitions, 'select-definition'), label(definitionId, 'id'), label(definitionInterface, 'interface'), label(definitionSize, 'size'), label(storage, 'storage-space'), label(workClass, 'work-class'), label(workSecs, 'work-secs'), label(manual, 'manual'), requirements, button('add-requirement', () => addRequirement()), hardware, saveDefinition, removeDefinition);
  lockerPanel.append(label(lockers, 'select-locker'), label(lockerId, 'id'), label(room, 'room'), label(space, 'space'), saveLocker, removeLocker);
  assemblyPanel.append(label(assemblies, 'select-assembly'), el('p', 'installed-hint'), label(assemblyId, 'id'), label(assemblyDefinition, 'definition'), label(locker, 'locker'), label(condition, 'condition'), label(dose, 'dose'), label(source, 'source'), label(patch, 'patch-factor'), saveAssembly, removeAssembly);
  node.append(el('h3', 'title'), el('p', 'hint'), label(template, 'template'), label(materialise, 'materialise'), status, slotPanel, definitionPanel, lockerPanel, assemblyPanel,
    button('clear', () => void apply(emptyFitting())));
  slots.addEventListener('change', () => { paintSlot(); paintDefinition(); }); system.addEventListener('change', () => { paintSlot(null); paintDefinition(null); });
  definitions.addEventListener('change', () => paintDefinition()); lockers.addEventListener('change', () => paintLocker()); assemblies.addEventListener('change', () => paintAssembly());
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    fill(template, draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort());
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && observed === key) { controls(); return; }
    try { inspection = inspectFitting(draft, dependencies, template.value); observed = key; paint(); materialise.checked = false; status.textContent = ''; }
    catch (error) { inspection = null; status.textContent = t('workshop.fitting.refused', { reason: error.message }); } controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const next = await runtime.dependencies(); if (!disposed && token === generation) dependencies = next; }
    catch (error) { if (!disposed) status.textContent = t('workshop.fitting.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
