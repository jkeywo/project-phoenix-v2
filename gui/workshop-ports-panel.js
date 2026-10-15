import { t } from './strings.js';
import { inspectPortServices, applyPortServiceOperation } from '../editor/workshop-ports.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
export function mountWorkshopPorts({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = 'workshop-ports-' + id; if (key) node.textContent = t('workshop.ports.' + key); return node; };
  const input = (id, type = 'text') => { const node = el('input', id); node.type = type; if (type === 'number') { node.min = '0'; node.step = '1'; } return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const button = (id, key, action) => { const node = el('button', id, key); node.type = 'button'; node.addEventListener('click', action); return node; };
  const fill = (select, values, previous = select.value) => { if (JSON.stringify([...select.options].map(option => option.value)) !== JSON.stringify(values)) { select.replaceChildren(); for (const value of values) { const option = el('option'); option.value = value; option.textContent = value; select.append(option); } } select.value = values.includes(previous) ? previous : values[0] || ''; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', 'checkbox'), policy = el('select', 'policy'), producer = el('select', 'producer');
  const factions = input('factions'), offers = el('div', 'offers'), status = el('p', 'status'); status.setAttribute('role', 'status');
  fill(policy, ['free_allied_replenishment', 'manual_offer']);
  let inspection = null, dependencies = null, loading = false, disposed = false, generation = 0, observed = null, observedDraft = null, serial = 0, rows = [];
  function addOffer(value = { id: '', resource: '', unit_amount: 1, quota: 0, cost: [] }) {
    const index = serial++, section = el('fieldset'), id = input('id-' + index), resource = input('resource-' + index), amount = input('amount-' + index, 'number'), quota = input('quota-' + index, 'number'), costs = el('div');
    id.value = value.id; resource.value = value.resource; amount.value = value.unit_amount; quota.value = value.quota;
    const row = { section, id, resource, amount, quota, costs: [] }; rows.push(row);
    function addCost(cost = { resource: '', amount: 1 }) {
      const costId = serial++, group = el('div'), name = input('cost-resource-' + costId), count = input('cost-amount-' + costId, 'number'); name.value = cost.resource; count.value = cost.amount;
      const entry = { group, name, count }; row.costs.push(entry); group.append(label(name, 'cost-resource'), label(count, 'cost-amount'), button(null, 'remove-cost', () => { row.costs = row.costs.filter(item => item !== entry); group.remove(); })); costs.append(group);
    }
    section.append(label(id, 'offer-id'), label(resource, 'resource'), label(amount, 'unit-amount'), label(quota, 'quota'), costs,
      button(null, 'add-cost', () => addCost()), button(null, 'remove-offer', () => { rows = rows.filter(item => item !== row); section.remove(); }));
    for (const cost of value.cost || []) addCost(cost); offers.append(section);
  }
  function controls() { for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection; }
  function paint() {
    const profile = inspection.port; policy.value = profile?.policy || 'free_allied_replenishment'; factions.value = (profile?.eligible_factions || []).join(', ');
    fill(producer, [...new Set(['', ...inspection.systems.filter(system => system.kind === 'dock').map(system => system.id), ...(profile?.system ? [profile.system] : [])])], profile?.system || '');
    producer.options[0].textContent = t('workshop.ports.producer-legacy');
    offers.replaceChildren(); rows = []; for (const offer of profile?.offer || []) addOffer(offer); controls();
  }
  const apply = button('apply', 'apply', async () => {
    if (!inspection || busy()) return;
    const token = ++generation, operation = { path: template.value, materialise: materialise.checked, value: { policy: policy.value,
      ...(producer.value ? { system: producer.value } : {}),
      eligible_factions: factions.value.split(',').map(id => id.trim()).filter(Boolean),
      offer: rows.map(row => ({ id: row.id.value, resource: row.resource.value, unit_amount: row.amount.valueAsNumber, quota: row.quota.valueAsNumber,
        cost: row.costs.map(cost => ({ resource: cost.name.value, amount: cost.count.valueAsNumber })) })) } };
    await runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyPortServiceOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.ports.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  });
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), label(policy, 'policy'), label(factions, 'factions'),
    label(producer, 'producer'), el('p', null, 'producer-hint'),
    el('p', null, 'factions-hint'), offers, button('add-offer', 'add-offer', () => addOffer()), apply, status);
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    fill(template, draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort());
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && observedDraft === draft && observed === key) { controls(); return; }
    try { inspection = inspectPortServices(draft, dependencies, template.value); observed = key; observedDraft = draft; paint(); materialise.checked = false; status.textContent = ''; }
    catch (error) { inspection = null; status.textContent = t('workshop.ports.refused', { reason: error.message }); } controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.ports.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
