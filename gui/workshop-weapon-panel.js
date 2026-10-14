import { t } from './strings.js';
import { runWorkshopMutation } from './workshop-edit-session.js';
import { inspectWeapons, applyWeaponOperation, defaultWeapon, guidedWeaponRange, WEAPON_PARAMETERS } from '../editor/workshop-weapons.js';

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
  const parameters = el('fieldset', 'parameters'), pattern = el('fieldset', 'pattern'), color = input('color');
  const mount = Object.fromEntries(['facing_deg', 'fire_arc_deg', 'range'].map(key => [key, input(key, 'number')]));
  const save = el('button', 'save', 'workshop.weapon.save', { type: 'button' });
  const create = el('button', 'new', 'workshop.weapon.new', { type: 'button' });
  const addStep = el('button', 'add-step', 'workshop.weapon.add_step', { type: 'button' });
  let dependencies = null, inspection = null, selected = null, paramInputs = {}, steps = [], disposed = false, loading = false, generation = 0, observed = null, stepSerial = 0;
  const option = (select, value, text) => { const item = el('option'); item.value = value; item.textContent = text; select.append(item); };
  for (const key of Object.keys(WEAPON_PARAMETERS)) option(kind, key, t('workshop.weapon.' + key));
  node.append(el('h3', null, 'workshop.weapon.title'), el('p', null, 'workshop.weapon.hint'), label(template, 'workshop.weapon.template'),
    label(existing, 'workshop.weapon.existing'), create, label(materialise, 'workshop.weapon.materialise'), status,
    label(id, 'workshop.weapon.id'), label(strategy, 'workshop.weapon.strategy_id'), label(kind, 'workshop.weapon.delivery'),
    ...Object.entries(mount).map(([key, control]) => label(control, 'workshop.weapon.' + key)),
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
    pattern.hidden = kind.value === 'beam';
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
    if (!force && revision === observed) { controls(); return; }
    try {
      inspection = inspectWeapons(draft, dependencies, template.value); observed = revision;
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
    const value = structuredClone(selected?.weapon || defaultWeapon(kind.value));
    value.id = id.value; value.mount = Object.fromEntries(Object.entries(mount).map(([key, control]) => [key, control.valueAsNumber]));
    if (strategy.value) value.strategy_id = strategy.value; else delete value.strategy_id;
    value.delivery = { kind: kind.value };
    for (const [key, control] of Object.entries(paramInputs)) if (control.value !== '') value.delivery[key] = control.valueAsNumber;
    if (kind.value === 'beam' && color.value.trim()) value.delivery.color = names(color.value).map(Number);
    delete value.marker; if (marker.value) value.marker = marker.value;
    value.barrels = kind.value === 'beam' ? [] : names(barrels.value);
    value.pattern = kind.value === 'beam' ? [] : steps.map(step => ({ offset_secs: step.offset.valueAsNumber, barrels: names(step.indices.value).map(Number) }));
    const token = ++generation, draft = getDraft(), operation = { path: template.value, section: selected?.section,
      ...(selected ? { originalId: selected.weapon.id } : {}), weapon: value, materialise: materialise.checked };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyWeaponOperation({ draft, provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { selected = { section: operation.section || 'weapon', weapon: value }; changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.weapon.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  save.addEventListener('click', () => void apply());
  create.addEventListener('click', () => { selected = null; existing.selectedIndex = -1; const value = defaultWeapon(kind.value);
    if (kind.value === 'guided') value.mount.range = guidedWeaponRange(inspection.resolved.value);
    paint(value); status.textContent = t('workshop.weapon.new'); });
  kind.addEventListener('change', () => { if (!selected) { const value = defaultWeapon(kind.value); value.id = id.value; value.strategy_id = strategy.value;
    if (kind.value === 'guided') value.mount.range = guidedWeaponRange(inspection.resolved.value);
    paint(value); } });
  template.addEventListener('change', () => { selected = null; inspect(true); }); existing.addEventListener('change', select);
  addStep.addEventListener('click', () => addPatternStep());
  inspect(); void load();
  return { node, refresh() { inspect(); void load(); }, dispose() { disposed = true; generation += 1; node.remove(); } };
}
