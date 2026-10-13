/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { mountStateTrees } from '../../gui/workshop-state-trees.js';
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
});
