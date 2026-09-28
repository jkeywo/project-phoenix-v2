import { describe, expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { applyWorkshopScript, createWorkshopScript, workshopScriptUnits } from '../workshop-scripts.js';
import { applySpatialOperation } from '../workshop-spatial.js';
import { applyModelStructureOperation } from '../workshop-model-structure.js';

const world = 'assets/worlds/root.toml', scripted = 'assets/worlds/scripted.toml';
const model = 'assets/models/ship.glb', sidecar = 'assets/models/ship.model.toml';
const rootSource = '# exact\r\nextra_worlds=[]\n[global]\r\ntitle="Root"\n[anchors]\nstart=[0,0,0]\n';
const modelSource = '# exact rig\r\n[base]\nscale = 1.00 # scale\r\n';
const manifest = '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n';
const modes = ['browser', 'native-mod', 'native-project'];
function setup(mode) {
  const native = mode !== 'browser';
  const files = { 'scenarios.toml': manifest, [world]: rootSource,
    [scripted]: '# retain\r\n[script]\nsetup="fn old(ctx) {}" # source\r\n',
    [sidecar]: modelSource, [model]: native ? { asset: '0123456789abcdef-3', length: 3 } : new Uint8Array([1, 2, 3]) };
  const draft = native ? WorkshopDocument.fromNativeFiles(files, { kind: mode === 'native-project' ? 'project' : 'mod' })
    : new WorkshopDocument(createStoreZip(Object.entries(files).map(([path, value]) =>
      typeof value === 'string' ? { path, text: value } : { path, bytes: value })));
  const provider = native ? { save() {}, restoreDocument: snapshot => WorkshopDocument.restore(snapshot, { native: true }) } : undefined;
  return { draft, provider, runtime: { validate: vi.fn(async () => ({ accepted: true, findings: [] })) } };
}
const workflows = [
  { name: 'script creation', stale: 'workshop.scripts.stale', refusal: 'workshop.scripts.validation_refused',
    apply: context => createWorkshopScript({ ...context, worldPath: world }),
    assert: candidate => expect(candidate.read(world)).toContain("[script]\nsetup = '''") },
  { name: 'inline script', stale: 'workshop.scripts.stale', refusal: 'workshop.scripts.validation_refused',
    apply: context => applyWorkshopScript({ ...context, unit: workshopScriptUnits(context.draft, scripted)[0], source: 'fn next(ctx) {}' }),
    assert: candidate => expect(candidate.read(scripted)).toBe('# retain\r\n[script]\nsetup="fn next(ctx) {}" # source\r\n') },
  { name: 'spatial layer', stale: 'stale-spatial-operation', refusal: 'runtime-validation-refused',
    apply: context => applySpatialOperation({ ...context, operation: { type: 'layer-add', path: world, layer: 'assets/worlds/child.toml' } }),
    assert: candidate => { expect(candidate.read(world)).toContain('assets/worlds/child.toml'); expect(candidate.read('assets/worlds/child.toml')).toContain('[anchors]'); } },
  { name: 'model variant', stale: 'model-structure-stale', refusal: 'runtime-validation-refused',
    apply: context => applyModelStructureOperation({ ...context, dependencies: {},
      operation: { type: 'variant-rename', path: sidecar, model, name: 'damaged' } }),
    assert: candidate => { expect(candidate.read(sidecar)).toBeUndefined(); expect(candidate.read('assets/models/ship.damaged.toml')).toBe(modelSource); } },
];

for (const mode of modes) for (const workflow of workflows) describe(`${workflow.name} shared acceptance: ${mode}`, () => {
  it('validates the exact candidate and adopts all changed members in one undo entry', async () => {
    const context = setup(mode), before = context.draft.members();
    const result = await workflow.apply(context);
    const [archive, candidate] = context.runtime.validate.mock.calls[0];
    expect(archive === null).toBe(mode !== 'browser');
    workflow.assert(candidate); workflow.assert(context.draft);
    expect(candidate.members().get(model)).toEqual(before.get(model));
    expect(context.draft.snapshot().history.undo).toHaveLength(1);
    if (workflow.name === 'inline script') expect(result).toBe(true);
    if (workflow.name === 'script creation') expect(result.accepted).toBe(true);
    if (workflow.name === 'model variant') expect(result.selected).toBe('assets/models/ship.damaged.toml');
    context.draft.undo(); expect(context.draft.members()).toEqual(before);
    context.draft.redo(); workflow.assert(context.draft);
  });

  it('retains findings and leaves all source and history unchanged on refusal', async () => {
    const context = setup(mode), before = context.draft.snapshot();
    const report = { accepted: false, findings: [{ file: world, line: 2, message: 'runtime refusal' }] };
    context.runtime.validate.mockResolvedValue(report);
    await expect(workflow.apply(context)).rejects.toMatchObject({ message: workflow.refusal, report });
    expect(context.draft.snapshot()).toEqual(before);
  });

  it('internally refuses a different member edited during validation with current always true', async () => {
    const context = setup(mode); let intervened;
    context.runtime.validate.mockImplementation(async () => {
      context.draft.put('assets/entities/concurrent.toml', '# later\n');
      intervened = context.draft.snapshot();
      return { accepted: true, findings: [] };
    });
    await expect(workflow.apply({ ...context, current: () => true })).rejects.toThrow(workflow.stale);
    expect(context.draft.snapshot()).toEqual(intervened);
  });
});

it.each(modes)('an unchanged script returns false without validation or history through %s', async mode => {
  const context = setup(mode), before = context.draft.snapshot();
  const unit = workshopScriptUnits(context.draft, scripted)[0];
  expect(await applyWorkshopScript({ ...context, unit, source: unit.source })).toBe(false);
  expect(context.runtime.validate).not.toHaveBeenCalled();
  expect(context.draft.snapshot()).toEqual(before);
});

it('refuses a stale model dependency capture before validation', async () => {
  const context = setup('native-project'); let resolve;
  context.runtime.dependencies = () => new Promise(done => { resolve = done; });
  const pending = applyModelStructureOperation({ ...context,
    operation: { type: 'variant-rename', path: sidecar, model, name: 'damaged' } });
  context.draft.put('assets/entities/concurrent.toml', '# changed\n');
  const before = context.draft.snapshot(); resolve({});
  await expect(pending).rejects.toThrow('model-structure-stale');
  expect(context.runtime.validate).not.toHaveBeenCalled();
  expect(context.draft.snapshot()).toEqual(before);
});
