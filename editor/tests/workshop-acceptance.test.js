import { describe, expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { acceptWorkshopChanges } from '../workshop-acceptance.js';
import { applyEntityOperation } from '../workshop-entity-composition.js';
import { applyShipOperation } from '../workshop-ship-authoring.js';

const path = 'assets/entities/ship.toml';
const asset = 'assets/models/ship.glb';
const source = '# retained\r\ntags=["ship"]\n\n[[station]] # seat\r\nid="helm"\r\nname="Helm" # label\n';
const manifest = '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n';
const reference = { asset: '0123456789abcdef-3', length: 3 };
const modes = ['browser', 'native-mod', 'native-project'];
function setup(mode) {
  const native = mode.startsWith('native');
  const files = { 'scenarios.toml': manifest, [path]: source, [asset]: native ? reference : new Uint8Array([1, 2, 3]) };
  const draft = native ? WorkshopDocument.fromNativeFiles(files, { kind: mode === 'native-project' ? 'project' : 'mod' })
    : new WorkshopDocument(createStoreZip(Object.entries(files).map(([path, value]) =>
      typeof value === 'string' ? { path, text: value } : { path, bytes: value })));
  const provider = native ? { save() {}, restoreDocument: snapshot => WorkshopDocument.restore(snapshot, { native: true }) } : undefined;
  const dependencies = { base_files: { 'assets/entities/base.toml': '[hull]\nhull_integrity=20\n' }, packs: [] };
  const runtime = { dependencies: vi.fn(async () => dependencies), shipSchema: vi.fn(async () => ({ system_kinds: [], directive_kinds: [] })),
    validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
  return { draft, provider, runtime, dependencies };
}
const workflows = [
  { name: 'entity', apply: applyEntityOperation, operation: { path, type: 'include-add', include: 'assets/entities/base.toml' } },
  { name: 'ship', apply: applyShipOperation, operation: { path, type: 'station-update', id: 'helm', fields: { name: 'Flight' } } },
];
function deferred() { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; }

for (const mode of modes) for (const workflow of workflows) describe(`${workflow.name} acceptance through ${mode}`, () => {
  it('validates captured exact members and adopts one reversible group', async () => {
    const context = setup(mode), before = context.draft.members();
    await workflow.apply({ ...context, operation: workflow.operation });
    const [archive, candidate] = context.runtime.validate.mock.calls[0];
    expect(archive === null).toBe(mode !== 'browser');
    expect(candidate.read(path)).toBe(context.draft.read(path));
    expect(candidate.read(path)).toContain('# retained\r\ntags=["ship"]\n');
    expect(candidate.members().get(asset)).toEqual(before.get(asset));
    expect(context.draft.snapshot().history.undo).toHaveLength(1);
    context.draft.undo(); expect(context.draft.members()).toEqual(before);
    context.draft.redo(); expect(context.draft.read(path)).toBe(candidate.read(path));
  });

  it('returns runtime findings without changing source or history', async () => {
    const context = setup(mode), before = context.draft.snapshot();
    const report = { accepted: false, findings: [{ file: path, line: 3, message: 'refused' }] };
    context.runtime.validate.mockResolvedValue(report);
    await expect(workflow.apply({ ...context, operation: workflow.operation })).rejects.toMatchObject({ report });
    expect(context.draft.snapshot()).toEqual(before);
  });

  for (const change of ['edit', 'undo', 'redo', 'replacement', 'dependencies']) it(`refuses ${change} during validation`, async () => {
    const context = setup(mode), gate = deferred();
    if (change === 'undo' || change === 'redo') context.draft.edit(path, source + '# history\n');
    if (change === 'redo') context.draft.undo();
    let sameDocument = true;
    context.runtime.validate.mockImplementation(async () => {
      if (change === 'edit') context.draft.edit(path, source + '# later\n');
      if (change === 'undo') context.draft.undo();
      if (change === 'redo') context.draft.redo();
      if (change === 'replacement') sameDocument = false;
      if (change === 'dependencies') context.dependencies.base_files['assets/entities/base.toml'] += '# changed\n';
      gate.resolve(context.draft.snapshot());
      return { accepted: true, findings: [] };
    });
    const operation = workflow.apply({ ...context, operation: workflow.operation, current: () => sameDocument });
    const afterIntervention = await gate.promise;
    await expect(operation).rejects.toThrow(`stale-${workflow.name}-operation`);
    expect(context.draft.snapshot()).toEqual(afterIntervention);
  });

  it('refuses a changed draft while dependencies are being captured, before validation', async () => {
    const context = setup(mode), gate = deferred();
    context.runtime.dependencies.mockReturnValue(gate.promise);
    const operation = workflow.apply({ ...context, dependencies: undefined, operation: workflow.operation });
    context.draft.edit(path, source + '# concurrent\n');
    const changed = context.draft.snapshot();
    gate.resolve(context.dependencies);
    await expect(operation).rejects.toThrow(`stale-${workflow.name}-operation`);
    expect(context.runtime.validate).not.toHaveBeenCalled();
    expect(context.draft.snapshot()).toEqual(changed);
  });
});

it('captures ship source before waiting for schema preparation', async () => {
  const context = setup('native-mod'), gate = deferred();
  context.runtime.shipSchema.mockReturnValue(gate.promise);
  const operation = applyShipOperation({ ...context, operation: workflows[1].operation });
  context.draft.edit(path, source + '# concurrent\n');
  gate.resolve({ system_kinds: [], directive_kinds: [] });
  await expect(operation).rejects.toThrow('stale-ship-operation');
  expect(context.runtime.validate).not.toHaveBeenCalled();
});

it('refuses dependencies changed during schema preparation even when the draft is unchanged', async () => {
  const context = setup('native-mod'), gate = deferred();
  context.runtime.shipSchema.mockReturnValue(gate.promise);
  const operation = applyShipOperation({ ...context, operation: workflows[1].operation });
  context.dependencies.base_files['assets/entities/base.toml'] += '# replaced\n';
  gate.resolve({ system_kinds: [], directive_kinds: [] });
  await expect(operation).rejects.toThrow('stale-ship-operation');
  expect(context.runtime.validate).not.toHaveBeenCalled();
  expect(context.draft.canUndo()).toBe(false);
});

it.each(modes)('adopts multiple members in a single history entry through %s', async mode => {
  const context = setup(mode), before = context.draft.members();
  await acceptWorkshopChanges({ ...context, prepare: captured => [
    { path, before: captured.read(path), after: source + '# accepted\n' },
    { path: 'assets/entities/new.toml', before: null, after: '\uFEFF# exact\r\ntags=[]\n' },
  ] });
  expect(context.draft.snapshot().history.undo).toHaveLength(1);
  context.draft.undo(); expect(context.draft.members()).toEqual(before);
});
