import { describe, it, expect, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { resolveStateTree, writeStateTrees, rebindStateTreeChannel, idleStateTree } from '../workshop-state-trees.js';
import { mergeComposeFragments, stripRemovals } from '../entity-includes.js';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { applyShipOperation } from '../workshop-ship-authoring.js';
const task = (id, value) => ({ id, when: [], action: { kind: 'write', channel: 'enabled', value: { source: 'literal', value } } });
const library = () => ({ root: 'variant', tree: [
  { id: 'base', initial: 'parent', parameters: { allowed: true }, memory: { owned: false }, state: [
    { id: 'parent', initial_child: 'leaf', task: [task('first', true), task('last', false)], transition: [] },
    { id: 'leaf', parent: 'parent', task: [], transition: [{ id: 'done', when: [], target: { kind: 'complete', result: 'success' } }] },
  ] },
  { id: 'variant', extends: 'base', parameters: { allowed: false }, state: [{ id: 'parent', task: [task('first', false), task('extra', true)] }] },
] });
const system = () => ({ id: 'dock', kind: 'dock', console: 'helm', config: { note: 'keep', ai: library() } });

describe('state-tree authoring source and inheritance', () => {
  it('shows stable inherited order with replacement and appended nodes without mutating authorship', () => {
    const ai = library(), copy = structuredClone(ai), tree = resolveStateTree(ai, 'variant');
    expect(tree.state.map(row => row.id)).toEqual(['parent', 'leaf']);
    expect(tree.state[0].task.map(row => row.id)).toEqual(['first', 'last', 'extra']);
    expect(tree.state[0].task[0].action.value.value).toBe(false);
    expect(tree.state[0].initial_child).toBe('leaf'); expect(tree.state[1].parent).toBe('parent');
    expect(tree.parameters.allowed).toBe(false); expect(tree.memory.owned).toBe(false);
    expect(tree.extends).toBeUndefined(); expect(ai).toEqual(copy);
  });
  it('reports incomplete inheritance instead of hanging while editing a cycle', () => {
    expect(() => resolveStateTree({ tree: [{ id: 'a', extends: 'b' }, { id: 'b', extends: 'a' }] }, 'a')).toThrow(/cycle/);
    expect(() => resolveStateTree({ tree: [{ id: 'a', extends: 'missing' }] }, 'a')).toThrow(/Unknown/);
  });
  it('edits only AI while preserving other config and neighboring source bytes', () => {
    const original = system(); const source = '# keep header\n' + stringify({ system: [original] }) + '\n[[system]] # keep neighbor\nid="other"\nkind="sensors"\n';
    const ai = library(); ai.tree[1].parameters.allowed = true;
    const after = writeStateTrees(source, original, ai);
    expect(after.startsWith('# keep header')).toBe(true);
    expect(after.slice(after.indexOf('[[system]] # keep neighbor'))).toBe(source.slice(source.indexOf('[[system]] # keep neighbor')));
    expect(parse(after).system[0].config.note).toBe('keep');
    expect(stripRemovals(parse(after)).system[0].config.ai).toEqual(ai);
  });
  it('materialises inherited trees explicitly, clears extends and removes inherited nodes without modifying base', () => {
    const original = system(), before = structuredClone(original), local = 'includes=["base.toml"]\n[[system]]\nid="dock"\n';
    const ai = library(); ai.tree[1] = resolveStateTree(ai, 'variant'); ai.tree[1].state[0].task.pop(); ai.tree[0].state[0].task[0].action = { kind: 'complete', result: 'failure' };
    expect(() => writeStateTrees(local, original, ai)).toThrow(/materialise/);
    const after = writeStateTrees(local, original, ai, { materialise: true });
    const merged = mergeComposeFragments({ system: [original] }, parse(after));
    expect(merged.system[0].config.ai).toEqual(ai);
    expect(original).toEqual(before);
  });
  it('keeps materialisation resets across subsequent local edits and reloads', () => {
    const original = system();
    const ai = library(); ai.tree = [resolveStateTree(ai, 'variant')];
    const first = writeStateTrees('includes=["base.toml"]\n[[system]]\nid="dock"\n', original, ai, { materialise: true });
    const effective = mergeComposeFragments({ system: [original] }, parse(first)).system[0];
    const next = structuredClone(ai); next.tree[0].state[0].task[0].action = { kind: 'complete', result: 'failure' };
    const second = writeStateTrees(first, effective, next);
    expect(mergeComposeFragments({ system: [original] }, parse(second)).system[0].config.ai).toEqual(next);
  });
  it('edits the recreated owner after a System tombstone without changing that tombstone', () => {
    const original = system(), tombstone = '[[system]]\nid="dock"\n_remove=true\n\n';
    const source = tombstone + stringify({ system: [original] });
    const ai = idleStateTree();
    const after = writeStateTrees(source, original, ai, { materialise: true });
    expect(after.startsWith(tombstone)).toBe(true);
    expect(parse(after).system[0]).toEqual({ id: 'dock', _remove: true });
    expect(stripRemovals(parse(after)).system[0].config.ai).toEqual(ai);
  });
  it('can explicitly materialise AI for a wholly inherited System', () => {
    const original = system(), after = writeStateTrees('includes=["base.toml"]\n', original, library(), { materialise: true });
    expect(mergeComposeFragments({ system: [original] }, parse(after)).system[0]).toEqual(original);
  });
  it('rewrites only write and pending channel references in its own copied program', () => {
    const ai = library(); ai.tree[0].state[0].task[0].when = [{ value: { source: 'pending', channel: 'enabled' }, equals: true }];
    ai.tree[0].parameters.enabled = false;
    const next = rebindStateTreeChannel(ai, 'enabled', 'hold');
    expect(next.tree[0].state[0].task[0].action.channel).toBe('hold');
    expect(next.tree[0].state[0].task[0].when[0].value.channel).toBe('hold');
    expect(next.tree[0].parameters).toHaveProperty('enabled');
    expect(ai.tree[0].state[0].task[0].action.channel).toBe('enabled');
  });
  it('rebinds numeric outputs without renaming numeric facts or memory', () => {
    const ai = library(); ai.tree[0].state[0].task[0] = { id: 'power', when: [{ equals: true,
      value: { source: 'number_compare', name: 'flow-main', comparison: 'less', value: 0.5 } }],
    action: { kind: 'write_number', channel: 'flow-main', value: 10 } };
    const next = rebindStateTreeChannel(ai, 'flow-main', 'flow-engine');
    expect(next.tree[0].state[0].task[0].action.channel).toBe('flow-engine');
    expect(next.tree[0].state[0].task[0].when[0].value.name).toBe('flow-main');
    expect(ai.tree[0].state[0].task[0].action.channel).toBe('flow-main');
  });
  it('renames a component output and inherited AI references in one accepted history edit', async () => {
    const path = 'assets/entities/ship.toml', basePath = 'assets/entities/ai.toml';
    const owned = system(); delete owned.config.ai;
    owned.component = [{ id: 'mate', kind: 'dock_mating', enabled_channel: 'enabled', target_channel: 'target', power_input: 'power', config: {} }, { id: 'power', kind: 'power_input' }];
    owned.channel = [{ name: 'enabled', kind: 'boolean' }, { name: 'target', kind: 'entity_target' }];
    const source = stringify({ includes: ['ai.toml'], tags: ['ship'], station: [{ id: 'helm' }], console: [{ id: 'helm', home_station: 'helm', screen: 'helm.html' }], system: [owned] });
    const base = stringify({ system: [{ id: 'dock', config: { ai: library() } }] });
    const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }]));
    const runtime = { dependencies: async () => ({ base_files: { [basePath]: base }, packs: [] }), shipSchema: async () => ({ system_kinds: ['dock'] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const components = structuredClone(owned.component); components[0].enabled_channel = 'hold';
    await applyShipOperation({ draft, runtime, operation: { path, type: 'system-composition-set', id: 'dock', components, channels: [{ name: 'hold', kind: 'boolean' }, owned.channel[1]] } });
    const edited = stripRemovals(parse(draft.read(path))).system[0];
    expect(edited.component[0].enabled_channel).toBe('hold');
    expect(edited.config.ai.tree[0].state[0].task[0].action.channel).toBe('hold');
    expect(edited.config.note).toBe('keep'); expect(draft.snapshot().history.undo).toHaveLength(1);
    draft.undo(); expect(draft.read(path)).toBe(source); expect(runtime.validate).toHaveBeenCalledTimes(1);
  });
  it('accepts one history edit, preserves source on runtime refusal and restores exact bytes', async () => {
    const path = 'assets/entities/ship.toml'; const source = stringify({ tags: ['ship'], station: [{ id: 'helm' }], console: [{ id: 'helm', home_station: 'helm', screen: 'helm.html' }], system: [system()] });
    const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }]));
    const runtime = { dependencies: async () => ({ base_files: {}, packs: [] }), shipSchema: async () => ({ system_kinds: ['dock'] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const ai = idleStateTree();
    await applyShipOperation({ draft, runtime, operation: { path, type: 'system-ai-set', id: 'dock', ai } });
    const after = draft.read(path); expect(stripRemovals(parse(after)).system[0].config.ai.root).toBe('idle');
    expect(draft.snapshot().history.undo).toHaveLength(1); draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(draft.read(path)).toBe(after);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ severity: 'error', message: 'unknown fact' }] });
    await expect(applyShipOperation({ draft, runtime, operation: { path, type: 'system-ai-set', id: 'dock', ai: library(), materialise: true } })).rejects.toThrow();
    expect(draft.read(path)).toBe(after);
  });
});
