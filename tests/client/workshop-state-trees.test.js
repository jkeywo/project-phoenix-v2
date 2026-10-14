/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { mountStateTrees, systemTreeVocabulary } from '../../gui/workshop-state-trees.js';
import { parse, stringify } from 'smol-toml';
import { idleStateTree } from '../../editor/workshop-state-trees.js';
import { stripRemovals } from '../../editor/entity-includes.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { inspectShipAuthoring, applyShipOperation } from '../../editor/workshop-ship-authoring.js';
const metadata = { ai_state_tree: { system_kinds: ['dock'], sources: ['literal', 'fact', 'parameter', 'memory', 'pending', 'call_result'], actions: ['write', 'remember', 'call', 'complete'], facts: ['order_present', 'ready', 'active', 'engaged'], results: ['success', 'failure'] } };
const owner = () => ({ id: 'dock', kind: 'dock', local: true, ai_inherited: true, ai_owner: 'base.toml', component: [{ kind: 'dock_mating', enabled_channel: 'hold' }], config: { ai: {
  root: 'variant', tree: [
    { id: 'variant', extends: 'base' },
    { id: 'base', initial: 'idle', parameters: { allowed: true }, memory: { owned: false }, state: [{ id: 'idle', task: [{ id: 'write', action: { kind: 'write', channel: 'hold', value: { source: 'literal', value: true } } }], transition: [] }] },
    { id: 'procedure', initial: 'done', parameters: { requested: false }, state: [{ id: 'done', task: [{ id: 'complete', action: { kind: 'complete', result: 'success' } }] }] },
  ],
} } });
const el = key => document.getElementById(`workshop-ai-${key}`);
const change = (key, value) => { const field = el(key); if (field.type === 'checkbox') field.checked = value; else field.value = value; field.dispatchEvent(new Event('change', { bubbles: true })); };
const action = (key, list = null) => (list ? document.querySelector(`[data-ai-list="${list}"] > [data-ai-action="${key}"]`) : document.querySelector(`#workshop-ai > [data-ai-action="${key}"]`)).click();
const setup = (system = owner()) => { document.body.innerHTML = '<main></main>'; const apply = vi.fn(); const editor = mountStateTrees({ root: document.querySelector('main'), apply }); editor.refresh(system, metadata, false); return { editor, apply }; };
describe('visual typed System AI authoring', () => {
  it('inspects inherited order and explicitly materialises before a single atomic Apply', () => {
    const { editor, apply } = setup();
    expect(el('resolved').textContent).toContain('write: write');
    expect(document.querySelector('#workshop-ai > [data-ai-action="apply"]').disabled).toBe(true);
    change('materialise', true);
    document.querySelector('[data-ai-action="flatten"]').click();
    expect(editor.value().tree[0].extends).toBeUndefined();
    action('add', 'tasks'); change('task.id', 'invoke'); change('action.kind', 'call'); change('action.tree', 'procedure');
    document.querySelector('[data-ai-action="bind"]').click(); change('binding.requested.value.source', 'parameter'); change('binding.requested.value.name', 'allowed');
    expect(apply).not.toHaveBeenCalled(); action('apply');
    const result = apply.mock.calls[0][0]();
    expect(result).toMatchObject({ type: 'system-ai-set', id: 'dock', materialise: true });
    expect(result.ai.tree[0].state[0].task[1].action).toEqual({ kind: 'call', tree: 'procedure', parameters: { requested: { source: 'parameter', name: 'allowed' } } });
  });
  it('offers only schema facts and actual bound output channels, with typed sources', () => {
    const { editor } = setup(); change('trees', '1');
    expect([...el('action.channel').options].map(row => row.value)).toEqual(['hold']);
    change('action.value.source', 'fact'); expect([...el('action.value.name').options].map(row => row.value)).toContain('ready'); change('action.value.name', 'ready');
    expect(editor.value().tree[1].state[0].task[0].action.value).toEqual({ source: 'fact', name: 'ready' });
    change('action.value.source', 'pending'); expect(el('action.value.channel').value).toBe('hold');
    expect(document.querySelector('textarea')).toBeNull();
  });
  it('edits hierarchy, transition priority and completion without raw code', () => {
    const { editor } = setup(); change('trees', '1'); action('add', 'states'); change('state.id', 'child'); change('state.parent', 'idle');
    action('add', 'transitions'); change('transition.id', 'done'); change('target.kind', 'complete'); change('target.result', 'failure');
    const state = editor.value().tree[1].state[1]; expect(state.parent).toBe('idle'); expect(state.transition[0].target).toEqual({ kind: 'complete', result: 'failure' });
    action('add', 'transitions'); action('up', 'transitions'); expect(editor.value().tree[1].state[1].transition[1].id).toBe('done');
  });
  it('preserves draft and disabled order boundaries when the parent refreshes controls', () => {
    const system = owner(), { editor } = setup(system); change('trees', '1');
    change('parameters.allowed.name', 'renamed'); change('parameters.allowed.value', false);
    expect(editor.value().tree[1].parameters).toEqual({ renamed: false });
    document.querySelectorAll('#workshop-ai button').forEach(button => { button.disabled = false; });
    editor.refresh(system, metadata, false);
    expect(editor.value().tree[1].parameters).toEqual({ renamed: false });
    expect(document.querySelector('[data-ai-list="tasks"] > [data-ai-action="up"]').disabled).toBe(true);
    expect(document.querySelector('#workshop-ai > [data-ai-action="apply"]').disabled).toBe(true);
  });
  it('supplies an explicit idle tree for new Dock authoring and never applies to no System', () => {
    const { editor } = setup({ id: null, kind: 'dock', local: true });
    expect(editor.value().root).toBe('idle'); expect(document.querySelector('#workshop-ai').disabled).toBe(false);
    expect(document.querySelector('#workshop-ai > [data-ai-action="apply"]').disabled).toBe(true);
  });
  it('selects per-kind vocabulary and derives Power bounds from actual authored hardware', () => {
    const profile = { reactor: { max_output_per_sec: 240, max_cooling_per_sec: 7 }, group: [{ id: 'engine', max_flow_per_sec: 35, max_cooling_per_sec: 3 }] };
    const schema = { ...metadata.ai_state_tree, sources: [...metadata.ai_state_tree.sources, 'number_compare'], actions: [...metadata.ai_state_tree.actions, 'write_number'],
      hosts: { power_reactor: { facts: ['alert'] }, dock: { facts: ['ready'] } } };
    const power = systemTreeVocabulary({ kind: 'power_reactor', energy_profile: profile }, schema);
    expect(power.facts).toEqual(['alert']); expect(power.numeric_channels).toEqual({ 'reactor-output': { min: 0, max: 240 }, 'reactor-cooling': { min: 0, max: 7 },
      'flow-engine': { min: 0, max: 35 }, 'cooling-engine': { min: 0, max: 3 } });
    expect(power.numeric_facts).toContain('reserve-engine');
    expect(systemTreeVocabulary({ kind: 'dock' }, schema).sources).not.toContain('number_compare');
    expect(systemTreeVocabulary({ kind: 'dock' }, schema).actions).not.toContain('write_number');
    expect(systemTreeVocabulary({ kind: 'power_reactor' }, schema).actions).not.toContain('write_number');
    profile.reactor.max_output_per_sec = 18.24; profile.group[0].max_flow_per_sec = 35.04;
    const fractional = systemTreeVocabulary({ kind: 'power_reactor', energy_profile: profile }, schema);
    expect(fractional.numeric_channels['reactor-output'].max).toBe(Math.fround(18.24));
    expect(fractional.numeric_channels['flow-engine'].max).toBe(Math.fround(35.04));
  });
  it('authors typed numeric conditions and outputs through controlled UI, accepted source and history', async () => {
    const path = 'assets/entities/power-ship.toml';
    const energy = { reactor: { max_output_per_sec: 240, max_cooling_per_sec: 7 }, group: [{ id: 'engine', max_flow_per_sec: 35, max_cooling_per_sec: 3 }] };
    const system = { id: 'power-reactor', kind: 'power_reactor', console: 'engineering', config: { ai: idleStateTree() } };
    const source = '# retain\n' + stringify({ tags: ['ship'], station: [{ id: 'engineering' }], console: [{ id: 'engineering', home_station: 'engineering', screen: 'engineering.html' }], system: [system], energy });
    const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }]));
    const schema = { ...metadata.ai_state_tree, system_kinds: ['power_reactor'], sources: [...metadata.ai_state_tree.sources, 'number_compare'], actions: [...metadata.ai_state_tree.actions, 'write_number'], hosts: { power_reactor: { facts: ['alert'] } } };
    const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), shipSchema: async () => ({ system_kinds: ['power_reactor'] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    document.body.innerHTML = '<main></main>'; let pending;
    const editor = mountStateTrees({ root: document.querySelector('main'), apply: factory => { pending = applyShipOperation({ draft, runtime, operation: { path, ...factory() } }); } });
    const selected = inspectShipAuthoring(draft, { base_files: {}, packs: [] }, path).systems[0];
    expect(selected.energy_profile).toEqual(energy); editor.refresh(selected, { ai_state_tree: schema }, false);
    action('add', 'tasks'); change('action.kind', 'write_number'); change('action.channel', 'flow-engine');
    expect(el('action.number').max).toBe('35'); change('action.number', '12.5');
    change('task.id', 'route'); document.querySelector('[data-ai-conditions="task.when"] > [data-ai-action="add"]').click();
    change('task.when.0.source', 'number_compare'); change('task.when.0.name', 'reserve-engine');
    change('task.when.0.comparison', 'less'); change('task.when.0.number', '0.4');
    expect(document.querySelector('textarea')).toBeNull(); action('apply'); await pending;
    const after = draft.read(path), ai = stripRemovals(parse(after)).system[0].config.ai;
    expect(ai.tree[0].state[0].task[0]).toMatchObject({ action: { kind: 'write_number', channel: 'flow-engine', value: 12.5 },
      when: [{ value: { source: 'number_compare', name: 'reserve-engine', comparison: 'less', value: 0.4 }, equals: true }] });
    expect(parse(after).energy).toEqual(energy); expect(after.startsWith('# retain')).toBe(true);
    expect(draft.snapshot().history.undo).toHaveLength(1); draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(draft.read(path)).toBe(after);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'numeric output exceeds hardware bound' }] });
    change('action.number', '1000'); action('apply'); await expect(pending).rejects.toThrow(); expect(draft.read(path)).toBe(after);
  });
});
