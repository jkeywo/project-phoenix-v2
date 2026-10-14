import { describe, it, expect, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { writeRepairWorkflow, idleStateTree } from '../workshop-state-trees.js';
import { mergeComposeFragments, stripRemovals } from '../entity-includes.js';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { applyShipOperation, inspectShipAuthoring } from '../workshop-ship-authoring.js';
const workflow = () => ({ root: 'repair', tree: [{ id: 'repair', initial: 'work', state: [{ id: 'work',
  task: [{ id: 'advance', action: { kind: 'write', channel: 'apply_repair', value: { source: 'fact', name: 'repairing' } } }], transition: [] }] }] });
describe('Repair Job workflow source and history', () => {
  it.each(['\n', '\r\n'])('preserves repair tunables and neighboring System AI with %j newlines', nl => {
    const repair = { repair_team_count: 2, workflow: workflow() };
    const prefix = '# untouched\n[repair]\nrepair_team_count=2\n';
    const source = (prefix + stringify({ repair: { workflow: repair.workflow } }).replace('[repair]\n', '') + '\n[[system]]\nid="dock"\n[system.config.ai]\nroot="keep"\n').replace(/\n/g, nl);
    const next = workflow(); next.tree[0].memory = { reviewed: true };
    const after = writeRepairWorkflow(source, repair, next);
    expect(after.startsWith(prefix.replace(/\n/g, nl))).toBe(true);
    expect(after.slice(after.indexOf('[[system]]'))).toBe(source.slice(source.indexOf('[[system]]')));
    expect(stripRemovals(parse(after)).repair).toEqual({ ...repair, workflow: next });
  });
  it('materialises inherited definitions and retains removal resets after reload', () => {
    const base = { repair: { repair_team_count: 2, workflow: workflow() } }, source = 'includes=["base.toml"]\n';
    const next = idleStateTree();
    expect(() => writeRepairWorkflow(source, base.repair, next)).toThrow(/materialise/);
    const first = writeRepairWorkflow(source, base.repair, next, { materialise: true });
    const effective = mergeComposeFragments(base, parse(first));
    expect(effective.repair.workflow).toEqual(next);
    next.tree[0].memory = { retained: true };
    const second = writeRepairWorkflow(first, effective.repair, next);
    expect(mergeComposeFragments(base, parse(second)).repair).toEqual({ repair_team_count: 2, workflow: next });
    expect(base.repair.workflow.root).toBe('repair');
  });
  it('refuses inline source instead of creating a duplicate TOML table', () => {
    const repair = { workflow: workflow() };
    expect(() => writeRepairWorkflow('repair = { workflow = { root="repair", tree=[] } }\n', repair, workflow(), { materialise: true })).toThrow(/Expand inline/);
  });
  it('validates one accepted mutation, restores exact history and refuses invalid runtime candidates', async () => {
    const path = 'assets/entities/ship.toml', basePath = 'assets/entities/base.toml';
    const source = 'includes=["base.toml"]\ntags=["ship"]\n[repair]\nrepair_team_count=2\n';
    const dependencies = { base_files: { [basePath]: stringify({ repair: { workflow: workflow() } }) }, packs: [] };
    const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }]));
    const runtime = { dependencies: async () => dependencies, shipSchema: async () => ({ system_kinds: [] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    expect(inspectShipAuthoring(draft, dependencies, path).repair.ai_inherited).toBe(true);
    const next = workflow(); next.tree[0].memory = { checked: true };
    await applyShipOperation({ draft, runtime, operation: { path, type: 'repair-workflow-set', workflow: next, materialise: true } });
    const after = draft.read(path); expect(runtime.validate).toHaveBeenCalledTimes(1);
    expect(draft.snapshot().history.undo).toHaveLength(1);
    draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(draft.read(path)).toBe(after);
    const reloaded = new WorkshopDocument(draft.archive());
    expect(inspectShipAuthoring(reloaded, dependencies, path).repair.workflow).toEqual(next);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ severity: 'error', message: 'unsupported Job channel' }] });
    await expect(applyShipOperation({ draft, runtime, operation: { path, type: 'repair-workflow-set', workflow: idleStateTree() } })).rejects.toThrow();
    expect(draft.read(path)).toBe(after); expect(draft.snapshot().history.undo).toHaveLength(1);
  });
});
