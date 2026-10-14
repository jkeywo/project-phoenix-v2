/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { mountStateTrees, repairJobTreeContext } from '../../gui/workshop-state-trees.js';
const workflow = { root: 'job', tree: [{ id: 'job', initial: 'work', parameters: {}, memory: {}, state: [{ id: 'work', task: [{ id: 'advance', when: [], action: { kind: 'write', channel: 'apply_repair', value: { source: 'fact', name: 'repairing' } } }], transition: [] }] }] };
const schema = { job_workflow: { default_tree: workflow, sources: ['literal', 'fact', 'parameter', 'memory', 'pending', 'call_result'], actions: ['write', 'remember', 'call', 'complete'], facts: ['internal', 'travelling', 'repairing', 'returning', 'external', 'target_present'], channels: ['advance_travel', 'apply_repair', 'advance_return', 'apply_external'], results: ['success', 'failure'] } };
const field = key => document.getElementById(`workshop-repair-workflow-${key}`);
const change = (key, value) => { const item = field(key); if (item.type === 'checkbox') item.checked = value; else item.value = value; item.dispatchEvent(new Event('change', { bubbles: true })); };
const setup = repair => { document.body.innerHTML = '<main></main>'; const apply = vi.fn(); const editor = mountStateTrees({ root: document.querySelector('main'), apply, context: repairJobTreeContext }); editor.refresh(repair, schema, false); return { editor, apply }; };
describe('visual Repair Job context reuses the state-tree editor', () => {
  it('uses runtime default workflow and only Job facts/effects while retaining atomic Apply', () => {
    const { editor, apply } = setup({ repair_team_count: 2 });
    expect(editor.value()).toEqual(workflow);
    expect([...field('action.channel').options].map(row => row.value)).toEqual(schema.job_workflow.channels);
    expect([...field('action.value.name').options].map(row => row.value)).toEqual(schema.job_workflow.facts);
    change('action.channel', 'apply_external'); change('action.value.name', 'external');
    expect(apply).not.toHaveBeenCalled();
    document.querySelector('#workshop-repair-workflow > [data-ai-action="apply"]').click();
    expect(apply.mock.calls[0][0]()).toMatchObject({ type: 'repair-workflow-set', materialise: false, workflow: { root: 'job' } });
    expect(editor.value().tree[0].state[0].task[0].action).toEqual({ kind: 'write', channel: 'apply_external', value: { source: 'fact', name: 'external' } });
    expect(document.querySelector('textarea')).toBeNull();
  });
  it('requires explicit materialisation and keeps staged changes across refresh', () => {
    const owner = { workflow, ai_inherited: true, ai_owner: 'repair.toml' }, { editor } = setup(owner);
    const apply = () => document.querySelector('#workshop-repair-workflow > [data-ai-action="apply"]');
    expect(apply().disabled).toBe(true); change('materialise', true); change('action.channel', 'advance_return');
    editor.refresh(owner, schema, false); expect(apply().disabled).toBe(false);
    expect(editor.value().tree[0].state[0].task[0].action.channel).toBe('advance_return');
    document.querySelector('#workshop-repair-workflow > [data-ai-action="reset"]').click();
    expect(editor.value()).toEqual(workflow); expect(apply().disabled).toBe(true);
  });
  it('keeps Dock and Job drafts, element identities and outputs independent', () => {
    setup({}); const dockApply = vi.fn();
    const dock = mountStateTrees({ root: document.querySelector('main'), apply: dockApply });
    dock.refresh({ id: 'dock', kind: 'dock', component: [{ kind: 'dock_mating', enabled_channel: 'hold' }], config: { ai: { ...workflow, tree: [] } } }, { ai_state_tree: { ...schema.job_workflow, system_kinds: ['dock'], facts: ['ready'] } }, false);
    expect(document.querySelectorAll('#workshop-ai')).toHaveLength(1);
    expect(document.querySelectorAll('#workshop-repair-workflow')).toHaveLength(1);
    change('action.channel', 'apply_external'); expect(dock.value().tree).toEqual([]);
    expect(dockApply).not.toHaveBeenCalled();
  });
  it('disables workflow authoring when the entity has no Repair capability', () => {
    setup(null); expect(document.getElementById('workshop-repair-workflow').disabled).toBe(true);
  });
});
