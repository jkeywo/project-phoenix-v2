import { t } from './strings.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
import { inspectResources, applyResourceOperation } from '../editor/workshop-resources.js';

/** Typed resource editing through the ordinary exact-source Workshop owner. */
export function mountWorkshopResources({ root, provider, runtime, draft: getDraft, busy, setBusy,
  changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, id, text, attrs = {}) => {
    const node = doc.createElement(tag); if (id) node.id = 'workshop-resource-' + id;
    if (text) node.textContent = t(text);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    return node;
  };
  const node = el('section', 'panel'), template = el('select', 'template');
  const status = el('p', 'status', null, { role: 'status' });
  const materialise = el('input', 'materialise', null, { type: 'checkbox' });
  const capacities = el('select', 'capacities', null, { size: '4' });
  const capacityId = el('input', 'capacity-id'), amount = el('input', 'amount', null, { type: 'number', min: '0', step: '1' });
  const ceiling = el('input', 'ceiling', null, { type: 'number', min: '0', step: '1' }), label = el('input', 'label');
  const count = el('input', 'count', null, { type: 'number', min: '0', step: '1' });
  const initialCount = el('input', 'initial-count', null, { type: 'number', min: '0', step: '1' });
  const services = el('select', 'services', null, { size: '4' }), serviceName = el('input', 'service-name');
  const serviceKind = el('select', 'service-kind'), serviceCapacity = el('select', 'service-capacity');
  const linkKind = el('select', 'link-kind'), linkService = el('select', 'link-service'), linkCapacity = el('select', 'link-capacity');
  let dependencies = null, inspection = null, disposed = false, generation = 0, loading = false, observedSource = null;
  const inputs = [template, materialise, capacities, capacityId, amount, ceiling, label, count, initialCount, services,
    serviceName, serviceKind, serviceCapacity, linkKind, linkService, linkCapacity];
  const buttons = [];
  const row = (control, text) => { const wrapper = el('label', null, text, { for: control.id }); wrapper.append(control); return wrapper; };
  const button = (id, text, operation) => {
    const control = el('button', id, text, { type: 'button' });
    control.addEventListener('click', () => void apply(operation)); buttons.push(control); return control;
  };
  const fieldset = (text, ...children) => { const field = el('fieldset'); field.append(el('legend', null, text), ...children); return field; };
  const capacityApply = button('capacity-apply', 'workshop.resource.apply_capacity', () => ({ type: 'capacity-set',
    id: capacityId.value, amount: amount.valueAsNumber, ceiling: ceiling.valueAsNumber, ...(label.value ? { label: label.value } : {}) }));
  const magazineApply = button('magazine-apply', 'workshop.resource.apply_magazine', () => ({ type: 'magazine-set', count: count.valueAsNumber, initial_count: initialCount.valueAsNumber }));
  const serviceApply = button('service-apply', 'workshop.resource.apply_service', () => ({ type: 'service-set', name: serviceName.value,
    endpoint: serviceKind.value === 'torpedo_magazine' ? { kind: 'torpedo_magazine' } : { kind: 'capacity', id: serviceCapacity.value } }));
  const linkApply = button('link-apply', 'workshop.resource.apply_link', () => ({ type: 'link-set',
    ...(linkKind.value === 'service' ? { service: linkService.value }
      : { provider: linkKind.value === 'torpedo_magazine' ? { kind: 'torpedo_magazine' } : { kind: 'capacity', id: linkCapacity.value } }) }));
  const magazineGroup = fieldset('workshop.resource.magazine', row(count, 'workshop.resource.count'), row(initialCount, 'workshop.resource.initial_count'), magazineApply);
  const linkGroup = fieldset('workshop.resource.link', row(linkKind, 'workshop.resource.endpoint'), row(linkService, 'workshop.resource.service_name'),
    row(linkCapacity, 'workshop.resource.capacity_id'), linkApply);
  node.append(el('h3', null, 'workshop.resource.title'), el('p', null, 'workshop.resource.hint'), row(template, 'workshop.resource.template'),
    row(materialise, 'workshop.resource.materialise'), status,
    fieldset('workshop.resource.capacities', row(capacities, 'workshop.resource.existing'), row(capacityId, 'workshop.resource.capacity_id'),
      row(amount, 'workshop.resource.amount'), row(ceiling, 'workshop.resource.ceiling'), row(label, 'workshop.resource.label'), capacityApply),
    magazineGroup, fieldset('workshop.resource.services', row(services, 'workshop.resource.existing'), row(serviceName, 'workshop.resource.service_name'),
      row(serviceKind, 'workshop.resource.endpoint'), row(serviceCapacity, 'workshop.resource.capacity_id'), serviceApply), linkGroup);
  if (attach) root.append(node);
  const option = (select, value, text = value) => { const item = doc.createElement('option'); item.value = value; item.textContent = text; select.append(item); };
  const choose = (select, rows, preferred = select.value) => {
    select.replaceChildren(); for (const [value, text] of rows) option(select, value, text);
    select.value = [...select.options].some(item => item.value === preferred) ? preferred : rows[0]?.[0] || '';
  };
  function controls() {
    const disabled = disposed || loading || busy() || !inspection;
    for (const control of [...inputs, ...buttons]) control.disabled = disabled;
    magazineApply.disabled ||= !inspection?.magazine;
    linkApply.disabled ||= !inspection?.umbilical;
    serviceCapacity.disabled ||= serviceKind.value !== 'capacity';
    linkCapacity.disabled ||= linkKind.value !== 'capacity';
    linkService.disabled ||= linkKind.value !== 'service';
  }
  function paintCapacity() {
    const capacity = inspection?.capacities.find(item => item.id === capacities.value);
    capacityId.value = capacity?.id || ''; amount.value = capacity?.amount ?? 0;
    ceiling.value = capacity?.ceiling ?? capacity?.amount ?? 0; label.value = capacity?.label || '';
  }
  function paintService() {
    const service = inspection?.services[services.value];
    serviceName.value = services.value; serviceKind.value = service?.kind || 'capacity'; serviceCapacity.value = service?.id || ''; controls();
  }
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    const paths = draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort();
    choose(template, paths.map(path => [path, path]));
    if (!template.value) { inspection = null; controls(); return; }
    const sourceKey = JSON.stringify([draft.sourceRevision, template.value, dependencies]);
    if (!force && observedSource === sourceKey) { controls(); return; }
    try {
      inspection = inspectResources(draft, dependencies, template.value); observedSource = sourceKey;
      choose(capacities, inspection.capacities.map(item => [item.id, item.id])); paintCapacity();
      choose(services, Object.keys(inspection.services).map(name => [name, name]));
      const kinds = [['capacity', t('workshop.resource.capacity_endpoint')], ...(inspection.magazine ? [['torpedo_magazine', t('workshop.resource.magazine_endpoint')]] : [])];
      choose(serviceKind, kinds); choose(linkKind, [['service', t('workshop.resource.named_endpoint')], ...kinds]);
      const caps = inspection.capacities.map(item => [item.id, item.id]); choose(serviceCapacity, caps); choose(linkCapacity, caps);
      choose(linkService, Object.keys(inspection.services).map(name => [name, name])); paintService();
      count.value = inspection.magazine?.count ?? ''; initialCount.value = inspection.magazine?.initial_count ?? inspection.magazine?.count ?? '';
      magazineGroup.hidden = !inspection.magazine; linkGroup.hidden = !inspection.umbilical;
      const link = inspection.umbilical?.link;
      linkKind.value = link?.provider?.kind || 'service'; linkCapacity.value = link?.provider?.id || '';
      linkService.value = link?.service || inspection.umbilical?.capacity || '';
      const owners = [...new Set(inspection.fields.filter(item => /^(resource_services|infrastructure\.capacity|torpedoes\.count|umbilical\.link)/.test(item.path)).map(item => item.owner))];
      status.textContent = owners.join(', '); materialise.checked = false;
    } catch (error) { inspection = null; status.textContent = t('workshop.resource.refused', { reason: error.message }); }
    controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const result = await runtime.dependencies(); if (!disposed && token === generation) dependencies = result; }
    catch (error) { if (!disposed) status.textContent = t('workshop.resource.refused', { reason: error.message }); }
    finally { loading = false; if (!disposed) inspect(); }
  }
  async function apply(factory) {
    if (busy() || loading || !inspection) return;
    const draft = getDraft(), token = ++generation, operation = { ...factory(), path: template.value, materialise: materialise.checked };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation,
      start: controls,
      invoke: () => applyResourceOperation({ draft, provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.resource.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  template.addEventListener('change', () => inspect(true)); capacities.addEventListener('change', paintCapacity);
  services.addEventListener('change', paintService); serviceKind.addEventListener('change', controls); linkKind.addEventListener('change', controls);
  inspect(); void load();
  return { node, refresh() { inspect(); void load(); }, dispose() { disposed = true; generation += 1; node.remove(); } };
}
