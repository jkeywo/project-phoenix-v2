import { t } from './strings.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
import { inspectLogistics, applyLogisticsOperation, emptyLogistics } from '../editor/workshop-logistics.js';

/** Located stores edit the same exact source document used by Preview and Test. */
export function mountWorkshopLogistics({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument, clone = value => structuredClone(value);
  const el = (tag, id, key, attrs = {}) => { const node = doc.createElement(tag); if (id) node.id = 'workshop-logistics-' + id;
    if (key) node.textContent = t('workshop.logistics.' + key); for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value); return node; };
  const input = (id, type = 'text') => el('input', id, null, { type, ...(type === 'number' ? { min: '0', step: '1' } : {}) });
  const label = (control, key) => { const node = el('label', null, key, { for: control.id }); node.append(control); return node; };
  const option = (select, value, text = value) => { const node = el('option'); node.value = value; node.textContent = text; select.append(node); };
  const fill = (select, values, previous = select.value) => { select.replaceChildren(); for (const value of values) option(select, value); select.value = values.includes(previous) ? previous : values[0] || ''; };
  const button = (id, key, action) => { const node = el('button', id, key, { type: 'button' }); node.addEventListener('click', action); return node; };
  const fieldset = (key, ...children) => { const node = el('fieldset'); node.append(el('legend', null, key), ...children); return node; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', 'checkbox'), status = el('p', 'status', null, { role: 'status' });
  const stores = el('select', 'stores'), storeId = input('store-id'), resource = input('resource'), room = el('select', 'compartment'), role = el('select', 'role');
  const endpoint = el('select', 'endpoint'), owner = el('select', 'system'), source = el('select', 'source'), target = input('target', 'number'), reserve = input('reserve', 'number');
  const bays = el('select', 'storage'), bayRoom = el('select', 'storage-compartment'), space = input('space', 'number'), costs = el('div', 'unit-space');
  const handling = el('select', 'handling-system'), slots = input('max-active', 'number'), batch = input('max-batch', 'number'), rate = input('units-per-sec', 'number'); rate.step = 'any';
  const fitId = input('fit-id'), fitConsole = el('select', 'fit-console'), fitGroup = el('select', 'fit-group'), fitRoom = el('select', 'fit-room');
  const fitCapacity = input('fit-capacity'), fitAmount = input('fit-amount', 'number'), fitCeiling = input('fit-ceiling', 'number');
  fitId.value = 'logistics'; fitCapacity.value = 'hold-ammunition'; fitAmount.value = 0; fitCeiling.value = 6;
  let inspection = null, dependencies = null, disposed = false, loading = false, generation = 0, observed = null, costRows = [], serial = 0;
  const profile = () => clone(inspection?.logistics || emptyLogistics());
  const selectedStore = () => inspection?.logistics?.store.find(row => row.id === stores.value);
  const selectedBay = () => inspection?.logistics?.storage.find(row => row.compartment === bays.value);
  const mutate = async transform => {
    if (busy() || loading || !inspection) return;
    try { const value = profile(); transform(value); return await apply(value); }
    catch (error) { status.textContent = t('workshop.logistics.refused', { reason: error.message }); }
  };
  function controls() {
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection;
    create.disabled ||= !!inspection?.logistics;
    owner.disabled ||= role.value !== 'local'; source.disabled ||= role.value !== 'local';
  }
  function paintStore(value = selectedStore()) {
    storeId.value = value?.id || ''; resource.value = value?.resource || 'torpedoes'; role.value = value?.role || 'main';
    room.value = value?.compartment || inspection?.compartments[0]?.id || '';
    endpoint.value = value?.endpoint.kind === 'torpedo_magazine' ? 'torpedo_magazine' : 'capacity:' + (value?.endpoint.id || inspection?.capacities[0]?.id || '');
    owner.value = value?.system || ''; source.value = value?.source || ''; target.value = value?.target ?? 0; reserve.value = value?.reserve ?? 0; controls();
  }
  function addCost(resourceValue = '', amount = 1) {
    const id = serial++, row = el('div'), resource = input('space-resource-' + id), cost = input('space-cost-' + id, 'number');
    resource.value = resourceValue; cost.value = amount;
    const item = { row, resource, cost }; costRows.push(item);
    row.append(label(resource, 'resource'), label(cost, 'unit-cost'), button(null, 'remove', () => { costRows = costRows.filter(other => other !== item); row.remove(); })); costs.append(row);
  }
  function paintBay(value = selectedBay()) {
    bayRoom.value = value?.compartment || inspection?.compartments[0]?.id || ''; space.value = value?.space ?? 10;
    costs.replaceChildren(); costRows = []; for (const [resource, amount] of Object.entries(value?.unit_space || {})) addCost(resource, amount); controls();
  }
  function paint() {
    const value = inspection?.logistics, rooms = inspection?.compartments.map(row => row.id) || [];
    fill(stores, (value?.store || []).map(row => row.id)); fill(bays, (value?.storage || []).map(row => row.compartment));
    fill(room, rooms); fill(bayRoom, rooms); fill(owner, inspection?.systems.map(row => row.id) || []);
    fill(source, (value?.store || []).filter(row => row.role === 'main').map(row => row.id));
    fill(endpoint, [...(inspection?.capacities || []).map(row => 'capacity:' + row.id), ...(inspection?.magazine ? ['torpedo_magazine'] : [])]);
    fill(handling, (inspection?.systems || []).filter(row => row.kind === 'logistics').map(row => row.id));
    fill(fitConsole, (inspection?.resolved.value.console || []).map(row => row.id));
    fill(fitGroup, (inspection?.resolved.value.energy?.group || []).map(row => row.id)); fill(fitRoom, rooms);
    const handler = value?.handling[0]; handling.value = handler?.system || handling.value;
    slots.value = handler?.max_active ?? 1; batch.value = handler?.max_batch ?? 1; rate.value = handler?.units_per_sec ?? 1;
    paintStore(); paintBay(); controls();
  }
  async function apply(logistics, extra = {}) {
    const token = ++generation, operation = { path: template.value, logistics, materialise: materialise.checked, ...extra };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyLogisticsOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.logistics.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  const saveStore = button('save-store', 'save-store', () => void mutate(value => {
    const row = { id: storeId.value, resource: resource.value, compartment: room.value, role: role.value,
      endpoint: endpoint.value === 'torpedo_magazine' ? { kind: 'torpedo_magazine' } : { kind: 'capacity', id: endpoint.value.slice(9) },
      target: target.valueAsNumber, reserve: reserve.valueAsNumber,
      ...(role.value === 'local' ? { system: owner.value, source: source.value } : {}) };
    const old = stores.value; value.store = value.store.filter(other => other.id !== old && other.id !== row.id); value.store.push(row);
    for (const local of value.store) if (local.source === old && old !== row.id) local.source = row.id;
  }));
  const removeStore = button('remove-store', 'remove', () => void mutate(value => {
    if (value.store.some(row => row.source === stores.value)) throw new Error(t('workshop.logistics.source-in-use'));
    value.store = value.store.filter(row => row.id !== stores.value);
  }));
  const saveBay = button('save-storage', 'save-storage', () => void mutate(value => {
    const unit_space = Object.create(null); for (const row of costRows) {
      if (Object.hasOwn(unit_space, row.resource.value)) throw new Error(t('workshop.logistics.duplicate-resource'));
      unit_space[row.resource.value] = row.cost.valueAsNumber;
    }
    value.storage = value.storage.filter(row => row.compartment !== bays.value && row.compartment !== bayRoom.value);
    value.storage.push({ compartment: bayRoom.value, space: space.valueAsNumber, unit_space });
  }));
  const saveHandling = button('save-handling', 'save-handling', () => void mutate(value => {
    value.handling = [{ system: handling.value, max_active: slots.valueAsNumber, max_batch: batch.valueAsNumber, units_per_sec: rate.valueAsNumber }];
  }));
  const create = button('create', 'create', () => {
    if (busy() || loading || !inspection) return;
    void apply(null, { type: 'logistics-create', system: fitId.value, console: fitConsole.value, group: fitGroup.value,
      compartment: fitRoom.value, capacity: fitCapacity.value, amount: fitAmount.valueAsNumber, ceiling: fitCeiling.valueAsNumber });
  });
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), status,
    fieldset('create', label(fitId, 'system'), label(fitConsole, 'console'), label(fitGroup, 'power-group'), label(fitRoom, 'compartment'),
      label(fitCapacity, 'capacity-id'), label(fitAmount, 'initial-hold'), label(fitCeiling, 'hold-ceiling'), create),
    fieldset('stores', label(stores, 'existing'), button('new-store', 'new', () => { stores.value = ''; paintStore(null); }),
      label(storeId, 'id'), label(endpoint, 'endpoint'), label(resource, 'resource'), label(room, 'compartment'), label(role, 'role'),
      label(owner, 'system'), label(source, 'source'), label(target, 'target'), label(reserve, 'reserve'), saveStore, removeStore),
    fieldset('storage-title', label(bays, 'existing'), button('new-storage', 'new', () => { bays.value = ''; paintBay(null); }),
      label(bayRoom, 'compartment'), label(space, 'space'), costs, button('add-cost', 'add-cost', () => addCost()), saveBay,
      button('remove-storage', 'remove', () => void mutate(value => { value.storage = value.storage.filter(row => row.compartment !== bays.value); }))),
    fieldset('handling', label(handling, 'system'), label(slots, 'max-active'), label(batch, 'max-batch'), label(rate, 'units-per-sec'), saveHandling));
  option(role, 'main', t('workshop.logistics.main')); option(role, 'local', t('workshop.logistics.local'));
  if (attach) root.append(node);
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    fill(template, draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort());
    if (!template.value) { inspection = null; controls(); return; }
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]);
    if (!force && observed === key) { controls(); return; }
    try { inspection = inspectLogistics(draft, dependencies, template.value); observed = key; paint(); materialise.checked = false;
      status.textContent = inspection.logistics ? '' : t('workshop.logistics.create-hint'); }
    catch (error) { inspection = null; status.textContent = t('workshop.logistics.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.logistics.refused', { reason: error.message }); }
    finally { loading = false; if (!disposed) inspect(); }
  }
  template.addEventListener('change', () => inspect(true)); stores.addEventListener('change', () => paintStore());
  bays.addEventListener('change', () => paintBay()); role.addEventListener('change', controls); inspect(); void load();
  return { node, refresh() { inspect(); void load(); }, dispose() { disposed = true; generation += 1; node.remove(); } };
}
