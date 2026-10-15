import { t } from './strings.js';
import { resolveStateTree, idleStateTree } from '../editor/workshop-state-trees.js';
import { initialAccessGateTree } from '../editor/workshop-interiors.js';

/** Host vocabulary plus actual authored hardware bounds; never a simulation. */
export function systemTreeVocabulary(owner, schema) {
  if (!schema) return schema;
  const host = schema.hosts?.[owner?.kind], vocabulary = { ...schema, ...host };
  const profile = owner?.energy_profile;
  if (['illumination','jamming'].includes(owner?.kind)) { const emitter=owner.component?.find(row=>row.kind===`${owner.kind}_emitter`); vocabulary.channels=[emitter?.enabled_channel || 'enabled',emitter?.target_channel || 'target']; }
  if (owner?.kind === 'identity-emitter') vocabulary.channels = [owner.component?.find(row => row.kind === 'identity_emitter')?.enabled_channel || 'enabled'];
  if (owner?.kind === 'signal-protection') vocabulary.channels = [owner.component?.find(row => row.kind === 'signal_protection')?.enabled_channel || 'enabled'];
  if (owner?.kind === 'access-gate') vocabulary.channels = [owner.component?.find(row => row.kind === 'access_boundary')?.sealed_channel || 'sealed'];
  if (['power_reactor', 'power_battery'].includes(owner?.kind)) {
    vocabulary.numeric_facts = []; vocabulary.numeric_channels = {};
  }
  if (['power_reactor', 'power_battery'].includes(owner?.kind) && profile?.reactor) {
    const reactor = profile.reactor;
    vocabulary.channels = [...new Set([...(vocabulary.channels || []), 'reactor-generation', 'priority-allocation'])];
    vocabulary.numeric_facts = ['reactor_reserve_fraction', 'reactor_heat_fraction', 'output_actual', 'strike_boost_enabled'];
    vocabulary.numeric_channels = {
      'reactor-output': { min: 0, max: Math.fround(reactor.max_output_per_sec) },
      'reactor-cooling': { min: 0, max: Math.fround(reactor.max_cooling_per_sec) },
    };
    for (const group of profile.group || []) {
      vocabulary.numeric_facts.push('reserve-' + group.id, 'heat-' + group.id);
      vocabulary.numeric_channels['flow-' + group.id] = { min: 0, max: Math.fround(group.max_flow_per_sec) };
      vocabulary.numeric_channels['cooling-' + group.id] = { min: 0, max: Math.fround(group.max_cooling_per_sec) };
    }
  }
  if (owner?.kind === 'shield_arc') {
    vocabulary.facts = [...(host?.facts || [])];
    vocabulary.channels = [...(host?.channels || [])];
    vocabulary.numeric_facts = [...(host?.numeric_facts || [])];
    vocabulary.numeric_channels = {};
    const prefixes = schema.shield_recovery_bindings || {};
    for (const target of owner.shield_energy?.recovery_targets || []) {
      for (const [destination, key] of [['facts', 'facts'], ['channels', 'channels'], ['numeric_facts', 'numeric_facts']]) {
        for (const prefix of prefixes[key] || []) vocabulary[destination].push(`${prefix}-${target}`);
      }
    }
  }
  if (!vocabulary.numeric_facts?.length) vocabulary.sources = (vocabulary.sources || []).filter(source => source !== 'number_compare');
  if (!Object.keys(vocabulary.numeric_channels || {}).length) vocabulary.actions = (vocabulary.actions || []).filter(action => action !== 'write_number');
  return vocabulary;
}
export const systemTreeContext = {
  id: 'workshop-ai', schemaKey: 'ai_state_tree',
  value: owner => owner?.config?.ai, defaultValue: (_, owner) => owner?.kind === 'access-gate'
    ? initialAccessGateTree(owner.component?.find(row => row.kind === 'access_boundary')?.sealed_channel || 'sealed') : idleStateTree(),
  channels: (owner, schema) => [...new Set([...(schema?.channels || []), ...(owner?.component || []).filter(row => row.kind === 'dock_mating').map(row => row.enabled_channel)])],
  vocabulary: systemTreeVocabulary,
  inherited: owner => owner?.ai_inherited === true || (owner?.local === false && !owner?.shield_arc_id),
  available: owner => !!owner?.id, supported: (owner, schema) => !!schema.hosts?.[owner?.kind] || schema.system_kinds?.includes(owner?.kind),
  operation: (owner, ai, materialise) => ({ type: 'system-ai-set', id: owner.id, ai, materialise }),
};
export const repairJobTreeContext = {
  id: 'workshop-repair-workflow', schemaKey: 'job_workflow',
  labels: { title: 'workshop.ship.job.title', rebinding: 'workshop.ship.job.context' },
  value: owner => owner?.workflow, defaultValue: schema => schema?.default_tree || idleStateTree(),
  channels: (_, schema) => schema?.channels || [],
  inherited: owner => owner?.ai_inherited === true, available: owner => !!owner,
  supported: owner => !!owner,
  operation: (_, workflow, materialise) => ({ type: 'repair-workflow-set', workflow, materialise }),
};

/** Closed, typed visual authoring. The local draft may be incomplete; Apply is atomic. */
export function mountStateTrees({ root, apply, context = systemTreeContext }) {
  const doc = root.ownerDocument, selections = new Map();
  let owner = null, schema = null, draft = { root: '', tree: [] }, stamp = '', inherited = false, materialise = false, error = '';
  const section = doc.createElement('fieldset'); section.id = context.id; root.append(section);
  const word = key => t(context.labels?.[key] || `workshop.ship.ai.${key}`);
  const node = (tag, text) => { const el = doc.createElement(tag); if (text != null) el.textContent = text; return el; };
  const idOf = key => `${context.id}-${key}`;
  const button = (parent, key, fn, disabled = false) => { const el = node('button', word(key)); el.type = 'button'; el.dataset.aiAction = key; el.disabled = disabled;
    el.onclick = () => { error = '';try { fn(); } catch (caught) { error = String(caught.message); } paint(); }; parent.append(el); return el; };
  const field = (parent, key, value, change, choices = null, type = 'text') => {
    const el = node(choices ? 'select' : 'input'); el.id = idOf(key); el.dataset.aiField = key;
    if (choices) { const options = [...new Set([...choices, ...(value != null && !choices.includes(value) ? [value] : [])])];
      for (const choice of options) { const option = node('option', choice || word('none')); option.value = choice; el.append(option); }
      el.value = value ?? '';
    } else { el.type = type; if (type === 'checkbox') el.checked = value === true; else el.value = value ?? ''; }
    const label = node('label', word(key.split('.').at(-1))); label.htmlFor = el.id; parent.append(label, el);
    el.onchange = () => { change(type === 'checkbox' ? el.checked : el.value); if (choices || type === 'checkbox') paint(); }; return el;
  };
  const channels = () => context.channels(owner, schema);
  const numericChannels = () => Object.keys(schema?.numeric_channels || {});
  const unique = (rows, prefix) => { let n = 1; while (rows.some(row => row.id === `${prefix}-${n}`)) n += 1; return `${prefix}-${n}`; };
  const resolved = tree => { try { return resolveStateTree(draft, tree.id); } catch { return tree; } };
  const sourceDefault = source => ({ source, ...(source === 'literal' ? { value: false } : source === 'pending' ? { channel: channels()[0] || '' }
    : source === 'call_result' ? { task: '', result: 'success' }
      : source === 'number_compare' ? { name: schema?.numeric_facts?.[0] || '', comparison: 'less_or_equal', value: 0 } : { name: '' }) });
  const sourceEditor = (parent, prefix, value, assign, tree) => {
    const box = node('fieldset'); parent.append(box);
    field(box, `${prefix}.source`, value?.source || 'literal', kind => assign(sourceDefault(kind)), schema?.sources || []);
    const current = value || sourceDefault('literal'), context = resolved(tree);
    if (current.source === 'literal') field(box, `${prefix}.value`, current.value, next => { current.value = next; assign(current); }, null, 'checkbox');
    else if (current.source === 'pending') field(box, `${prefix}.channel`, current.channel, next => { current.channel = next; assign(current); }, channels());
    else if (current.source === 'call_result') {
      const calls = (context.state || []).flatMap(state => (state.task || []).filter(task => task.action?.kind === 'call').map(task => `${state.id}/${task.id}`));
      field(box, `${prefix}.task`, current.task, next => { current.task = next; assign(current); }, calls);
      field(box, `${prefix}.result`, current.result, next => { current.result = next; assign(current); }, schema?.results || []);
    } else if (current.source === 'number_compare') {
      field(box, `${prefix}.name`, current.name, next => { current.name = next; assign(current); }, schema?.numeric_facts || []);
      field(box, `${prefix}.comparison`, current.comparison, next => { current.comparison = next; assign(current); }, ['less', 'less_or_equal', 'equal', 'greater_or_equal', 'greater']);
      const threshold = field(box, `${prefix}.number`, current.value, next => { current.value = Number(next); assign(current); }, null, 'number');
      threshold.step = 'any';
    } else {
      const choices = current.source === 'fact' ? schema?.facts || [] : Object.keys(context[current.source === 'parameter' ? 'parameters' : 'memory'] || {});
      field(box, `${prefix}.name`, current.name, next => { current.name = next; assign(current); }, choices);
    }
  };
  const ordered = (parent, key, rows, make, render) => {
    const box = node('fieldset'); box.dataset.aiList = key; box.append(node('legend', word(key))); parent.append(box);
    const select = node('select'); select.id = idOf(key); select.size = 4;
    rows.forEach((row, index) => { const option = node('option', `${index + 1}. ${row.id}`); option.value = String(index); select.append(option); });
    const at = Math.min(selections.get(key) || 0, rows.length - 1); select.value = String(at);
    select.onchange = () => { selections.set(key, Number(select.value)); paint(); }; box.append(select);
    button(box, 'add', () => { rows.push(make()); selections.set(key, rows.length - 1); });
    button(box, 'remove', () => { rows.splice(at, 1); selections.set(key, Math.max(0, at - 1)); }, at < 0);
    for (const [label, step] of [['up', -1], ['down', 1]]) button(box, label, () => {
      [rows[at], rows[at + step]] = [rows[at + step], rows[at]]; selections.set(key, at + step);
    }, at < 0 || at + step < 0 || at + step >= rows.length);
    if (at >= 0) render(box, rows[at], at);
  };
  const boolMap = (parent, key, values) => {
    const box = node('fieldset'); box.dataset.aiMap = key; box.append(node('legend', word(key))); parent.append(box);
    for (const [name, value] of Object.entries(values)) {
      const line = node('div'); box.append(line); let currentName = name;
      field(line, `${key}.${name}.name`, name, next => { if (next && !Object.hasOwn(values, next)) { const currentValue = values[currentName]; delete values[currentName]; values[next] = currentValue; currentName = next; } });
      field(line, `${key}.${name}.value`, value, next => { values[currentName] = next; }, null, 'checkbox');
      button(line, 'remove', () => delete values[currentName]);
    }
    button(box, 'add', () => { let n = 1; while (Object.hasOwn(values, `value-${n}`)) n += 1; values[`value-${n}`] = false; });
  };
  const conditions = (parent, key, rows, tree) => {
    const box = node('fieldset'); box.dataset.aiConditions = key; box.append(node('legend', word('conditions'))); parent.append(box);
    rows.forEach((condition, index) => {
      const line = node('div'); line.dataset.aiCondition = String(index); box.append(line);
      sourceEditor(line, `${key}.${index}`, condition.value, value => { condition.value = value; }, tree);
      field(line, `${key}.${index}.equals`, condition.equals, value => { condition.equals = value; }, null, 'checkbox');
      button(line, 'remove', () => rows.splice(index, 1));
      button(line, 'up', () => { [rows[index - 1], rows[index]] = [rows[index], rows[index - 1]]; }, index === 0);
      button(line, 'down', () => { [rows[index + 1], rows[index]] = [rows[index], rows[index + 1]]; }, index === rows.length - 1);
    });
    button(box, 'add', () => rows.push({ value: sourceDefault('literal'), equals: true }));
  };
  const actionDefault = kind => ({ kind, ...(kind === 'write' ? { channel: channels()[0] || '', value: sourceDefault('literal') }
    : kind === 'write_number' ? { channel: numericChannels()[0] || '', value: schema?.numeric_channels?.[numericChannels()[0]]?.min ?? 0 }
      : kind === 'remember' ? { key: '', value: sourceDefault('literal') } : kind === 'call' ? { tree: '', parameters: {} } : { result: 'success' }) });
  const taskEditor = (parent, task, tree) => {
    field(parent, 'task.id', task.id, value => { task.id = value; });
    conditions(parent, 'task.when', task.when ||= [], tree);
    field(parent, 'action.kind', task.action.kind, kind => { task.action = actionDefault(kind); }, schema?.actions || []);
    const action = task.action;
    if (action.kind === 'write' || action.kind === 'remember') {
      const target = action.kind === 'write' ? 'channel' : 'key';
      field(parent, `action.${target}`, action[target], value => { action[target] = value; }, target === 'channel' ? channels() : Object.keys(resolved(tree).memory || {}));
      sourceEditor(parent, 'action.value', action.value, value => { action.value = value; }, tree);
    } else if (action.kind === 'write_number') {
      field(parent, 'action.channel', action.channel, value => { action.channel = value; }, numericChannels());
      const control = field(parent, 'action.number', action.value, value => { action.value = Number(value); }, null, 'number');
      const bounds = schema?.numeric_channels?.[action.channel]; control.step = 'any';
      if (bounds) { control.min = String(bounds.min); control.max = String(bounds.max); }
    } else if (action.kind === 'complete') field(parent, 'action.result', action.result, value => { action.result = value; }, schema?.results || []);
    else if (action.kind === 'call') {
      field(parent, 'action.tree', action.tree, value => { action.tree = value; }, draft.tree.map(row => row.id));
      const bindings = action.parameters ||= {};
      const callTree = draft.tree.find(row => row.id === action.tree);
      const available = callTree ? Object.keys(resolved(callTree).parameters || {}) : [];
      for (const [name, value] of Object.entries(bindings)) {
        const box = node('fieldset'); parent.append(box);
        field(box, `binding.${name}.name`, name, next => { if (!Object.hasOwn(bindings, next)) { delete bindings[name]; bindings[next] = value; } }, available);
        sourceEditor(box, `binding.${name}.value`, value, next => { bindings[name] = next; }, tree);
        button(box, 'remove', () => delete bindings[name]);
      }
      button(parent, 'bind', () => { const next = available.find(name => !Object.hasOwn(bindings, name)); if (next) bindings[next] = sourceDefault('literal'); }, !available.some(name => !Object.hasOwn(bindings, name)));
    }
  };
  const transitionEditor = (parent, transition, tree) => {
    field(parent, 'transition.id', transition.id, value => { transition.id = value; });
    conditions(parent, 'transition.when', transition.when ||= [], tree);
    field(parent, 'target.kind', transition.target.kind, kind => { transition.target = kind === 'state' ? { kind, state: tree.initial || '' } : { kind, result: 'success' }; }, ['state', 'complete']);
    const target = transition.target;
    if (target.kind === 'state') field(parent, 'target.state', target.state, value => { target.state = value; }, (resolved(tree).state || []).map(row => row.id));
    else field(parent, 'target.result', target.result, value => { target.result = value; }, schema?.results || []);
  };
  const treeEditor = (parent, tree) => {
    field(parent, 'tree.id', tree.id, value => { tree.id = value; });
    field(parent, 'tree.extends', tree.extends || '', value => { if (value) tree.extends = value; else delete tree.extends; }, ['', ...draft.tree.filter(row => row !== tree).map(row => row.id)]);
    const effective = resolved(tree), states = effective.state || [];
    field(parent, 'tree.initial', tree.initial || '', value => { if (value) tree.initial = value; else delete tree.initial; }, ['', ...states.map(row => row.id)]);
    boolMap(parent, 'parameters', tree.parameters ||= {}); boolMap(parent, 'memory', tree.memory ||= {});
    const overview = node('ol'); overview.id = idOf('resolved');
    try {
      for (const state of resolveStateTree(draft, tree.id).state || []) {
        const item = node('li', `${state.id}${state.parent ? ` → ${state.parent}` : ''}`);
        const tasks = node('ol'); for (const task of state.task || []) tasks.append(node('li', `${task.id}: ${task.action.kind}${task.action.tree ? ` → ${task.action.tree}` : ''}`));
        const transitions = node('ol'); for (const transition of state.transition || []) transitions.append(node('li', `${transition.id} → ${transition.target.state || transition.target.result}`));
        item.append(tasks, transitions); overview.append(item);
      }
    } catch (error) { overview.append(node('li', String(error.message))); }
    parent.append(node('h4', word('resolved')), overview);
    button(parent, 'flatten', () => { const index = draft.tree.indexOf(tree); draft.tree[index] = resolveStateTree(draft, tree.id); }, !tree.extends);
    ordered(parent, 'states', tree.state ||= [], () => { const id = unique(tree.state, 'state'); if (!tree.initial) tree.initial = id; return { id, task: [], transition: [] }; }, (box, state) => {
      field(box, 'state.id', state.id, value => { state.id = value; });
      field(box, 'state.parent', state.parent || '', value => { if (value) state.parent = value; else delete state.parent; }, ['', ...states.filter(row => row.id !== state.id).map(row => row.id)]);
      field(box, 'state.initial_child', state.initial_child || '', value => { if (value) state.initial_child = value; else delete state.initial_child; }, ['', ...states.filter(row => row.parent === state.id).map(row => row.id)]);
      ordered(box, 'tasks', state.task ||= [], () => ({ id: unique(state.task, 'task'), when: [], action: actionDefault('write') }), (item, task) => taskEditor(item, task, tree));
      ordered(box, 'transitions', state.transition ||= [], () => ({ id: unique(state.transition, 'transition'), when: [], target: { kind: 'state', state: tree.initial || state.id } }), (item, transition) => transitionEditor(item, transition, tree));
    });
  };
  const paint = () => {
    section.replaceChildren(node('legend', word('title')), node('p', word('draft')), node('p', word('rebinding')));
    if (error) { const message = node('p', error); message.setAttribute('role', 'alert'); section.append(message); }
    if (!schema) return;
    const origin = node('p', `${word('origin')}: ${owner?.ai_owner || owner?.owner || ''}${inherited ? ` — ${word('inherited')}` : ''}`); section.append(origin);
    field(section, 'materialise', materialise, value => { materialise = value; }, null, 'checkbox');
    field(section, 'root', draft.root, value => { draft.root = value; }, draft.tree.map(row => row.id));
    ordered(section, 'trees', draft.tree, () => { const id = unique(draft.tree, 'tree'); if (!draft.root) draft.root = id;
      return { id, initial: 'idle', parameters: {}, memory: {}, state: [{ id: 'idle', task: [], transition: [] }] }; }, treeEditor);
    button(section, 'reset', () => { draft = structuredClone(context.value(owner) || context.defaultValue(schema, owner)); materialise = false; selections.clear(); });
    button(section, 'apply', () => apply(() => context.operation(owner, structuredClone(draft), materialise)), !context.available(owner) || (inherited && !materialise));
  };
  return { value: () => structuredClone(draft), refresh(system, metadata, disabled) {
    owner = system; schema = context.vocabulary ? context.vocabulary(system, metadata?.[context.schemaKey]) : metadata?.[context.schemaKey];
    inherited = context.inherited(system);
    const next = JSON.stringify([system?.id, context.value(system), inherited, system?.ai_owner, schema]);
    if (next !== stamp) { stamp = next; draft = structuredClone(context.value(system) || context.defaultValue(schema, system)); selections.clear(); materialise = false; }
    // The containing form resets descendant disabled flags; restore every draft
    // control boundary without discarding the staged edits.
    paint();
    section.disabled = disabled || !schema || !context.supported(system, schema);
  } };
}
