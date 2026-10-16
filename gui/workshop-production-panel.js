import { t } from './strings.js';
import { inspectProduction, applyProductionOperation } from '../editor/workshop-production.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

/** Recipes reference real located stores; source/history remains Workshop's. */
export function mountWorkshopProduction({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument;
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = 'workshop-production-' + id;
    if (key) node.textContent = t('workshop.production.' + key); return node; };
  const input = (id, type = 'text') => { const node = el('input', id); node.type = type; if (type === 'number') { node.min = '0'; node.step = '1'; } return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const button = (id, key, action) => { const node = el('button', id, key); node.type = 'button'; node.addEventListener('click', action); return node; };
  const fill = (select, values, previous = select.value) => { select.replaceChildren(); for (const value of values) { const option = el('option'); option.value = value; option.textContent = value || t('workshop.production.none'); select.append(option); }
    select.value = values.includes(previous) ? previous : values[0] || ''; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', 'checkbox'), status = el('p', 'status'); status.setAttribute('role', 'status');
  const system = input('system'), console = el('select', 'console'), group = el('select', 'group'), room = el('select', 'room'); system.value = 'fabrication';
  const recipes = el('select', 'recipes'), recipeId = input('recipe-id'), work = input('work-secs', 'number'), inputs = el('fieldset', 'inputs'), outputs = el('fieldset', 'outputs'); work.step = 'any';
  const initial = el('select', 'initial-recipe'), target = input('initial-target', 'number');
  let inspection = null, dependencies = null, disposed = false, loading = false, observed = null, generation = 0, serial = 0, amountRows = { inputs: [], outputs: [] };
  const profile = () => structuredClone(inspection.production || { system: system.value, initial_recipe: null, initial_target: 0, recipe: [] });
  const stores = () => inspection?.stores.filter(store => store.compartment === (inspection.production ? inspection.resolved.value.interior.placements[inspection.production.system] : room.value)).map(store => store.id) || [];
  function addRow(kind, value = { store: '', amount: 1 }) {
    const id = serial++, row = el('div'), store = el('select', kind + '-store-' + id), amount = input(kind + '-amount-' + id, 'number'); amount.min = '1'; amount.value = value.amount;
    fill(store, stores(), value.store); const entry = { row, store, amount }; amountRows[kind].push(entry);
    const remove = button(null, 'remove-row', () => { amountRows[kind] = amountRows[kind].filter(other => other !== entry); row.remove(); });
    row.append(label(store, 'store'), label(amount, 'amount'), remove);
    if (kind === 'outputs') for (const [key, direction] of [['earlier', -1], ['later', 1]]) row.append(button(null, key, () => {
      const list = amountRows[kind], index = list.indexOf(entry), next = index + direction;
      if (next < 0 || next >= list.length) return;
      [list[index], list[next]] = [list[next], list[index]]; for (const item of list) outputs.append(item.row);
    }));
    (kind === 'inputs' ? inputs : outputs).append(row);
  }
  function paintRecipe(value = inspection?.production?.recipe.find(row => row.id === recipes.value)) {
    recipeId.value = value?.id || 'torpedoes'; work.value = value?.work_secs ?? 10;
    inputs.replaceChildren(el('legend', null, 'inputs')); outputs.replaceChildren(el('legend', null, 'outputs')); amountRows = { inputs: [], outputs: [] };
    for (const kind of ['inputs', 'outputs']) for (const row of value?.[kind] || []) addRow(kind, row);
  }
  function controls() {
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = disposed || loading || busy() || !inspection;
    create.disabled ||= !!inspection?.production; saveRecipe.disabled ||= !inspection?.production;
    saveInitial.disabled ||= !inspection?.production; removeRecipe.disabled ||= !inspection?.production;
    system.disabled ||= !!inspection?.production; console.disabled ||= !!inspection?.production; group.disabled ||= !!inspection?.production; room.disabled ||= !!inspection?.production;
  }
  function paint() {
    const value = inspection.production;
    fill(console, (inspection.resolved.value.console || []).map(row => row.id));
    fill(group, (inspection.resolved.value.energy?.group || []).map(row => row.id)); fill(room, inspection.compartments.map(row => row.id));
    fill(recipes, (value?.recipe || []).map(row => row.id)); fill(initial, ['', ...(value?.recipe || []).map(row => row.id)], value?.initial_recipe || '');
    system.value = value?.system || 'fabrication'; target.value = value?.initial_target ?? 0; paintRecipe(); controls();
  }
  async function apply(production, extra = {}) {
    const token = ++generation, operation = { path: template.value, production, materialise: materialise.checked, ...extra };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applyProductionOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.production.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  const readRecipe = () => ({ id: recipeId.value, work_secs: work.valueAsNumber,
    ...Object.fromEntries(['inputs', 'outputs'].map(kind => [kind, amountRows[kind].map(row => ({ store: row.store.value, amount: row.amount.valueAsNumber }))])) });
  const create = button('create', 'create', () => {
    if (busy() || !inspection) return;
    const recipe = readRecipe(); void apply({ system: system.value, initial_recipe: recipe.id, initial_target: 0, recipe: [recipe] },
      { type: 'production-create', console: console.value, group: group.value, compartment: room.value });
  });
  const saveRecipe = button('save-recipe', 'save-recipe', () => {
    if (busy() || !inspection?.production) return;
    const value = profile(), recipe = readRecipe(), old = recipes.value;
    const index = value.recipe.findIndex(row => row.id === old);
    value.recipe = value.recipe.filter(row => row.id !== old && row.id !== recipe.id);
    value.recipe.splice(index < 0 ? value.recipe.length : index, 0, recipe);
    if (value.initial_recipe === old) value.initial_recipe = recipe.id;
    void apply(value);
  });
  const removeRecipe = button('remove-recipe', 'remove-recipe', () => {
    if (busy() || !inspection?.production) return; const value = profile(); value.recipe = value.recipe.filter(row => row.id !== recipes.value);
    if (value.initial_recipe === recipes.value) { value.initial_recipe = null; value.initial_target = 0; } void apply(value);
  });
  const saveInitial = button('save-initial', 'save-initial', () => { if (busy() || !inspection?.production) return;
    void apply({ ...profile(), initial_recipe: initial.value || null, initial_target: target.valueAsNumber }); });
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), status,
    label(system, 'system'), label(console, 'console'), label(group, 'group'), label(room, 'room'), create,
    label(recipes, 'recipes'), button('new-recipe', 'new-recipe', () => { recipes.value = ''; paintRecipe(null); }),
    label(recipeId, 'recipe-id'), label(work, 'work-secs'), inputs, button('add-input', 'add-input', () => addRow('inputs')),
    outputs, el('p', null, 'primary-hint'), button('add-output', 'add-output', () => addRow('outputs')), saveRecipe, removeRecipe,
    label(initial, 'initial-recipe'), label(target, 'initial-target'), saveInitial);
  recipes.addEventListener('change', () => paintRecipe()); room.addEventListener('change', () => paintRecipe(null));
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    fill(template, draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort());
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && observed === key) { controls(); return; }
    try { inspection = inspectProduction(draft, dependencies, template.value); observed = key; paint(); materialise.checked = false; status.textContent = ''; }
    catch (error) { inspection = null; status.textContent = t('workshop.production.refused', { reason: error.message }); } controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.production.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
