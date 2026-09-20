import { describe, expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { applyModelStructureOperation, inspectModelStructure, prepareModelStructureOperation,
  validateModelStructure } from '../workshop-model-structure.js';

const path = 'assets/models/ship.model.toml';
const model = 'assets/models/ship.glb';
const source = `# exact source\r\n[base] # rig\r\noffset = [ 1,  2, 3 ] # keep\r\nrotation = [0.0, 0.0, 0.0]\r\n\r\n[markers.muzzle] # marker comment\r\nposition = [1, 2, 3] # position comment\r\ndirection = [0, 0, 1]\r\n\r\n[extents] # editor-untouched runtime field\r\nmin = [-1, -1, -1]\r\nmax = [1, 1, 1]\r\nsize = [2, 2, 2]\r\n\r\n[[target_points]] # fore\r\nposition = [4, 5, 6]\r\n\r\n[[target_points]] # aft\r\nposition = [-4, 5, 6]\r\n\r\n[[lod]] # near\r\nmodel = "assets/models/ship.glb"\r\nmax_distance = 20 # metres\r\nradius = 1.500 # editor-untouched runtime field\r\n\r\n[[lod]] # far\r\nshape = "sphere"\r\n`;

function makeDraft() {
  return new WorkshopDocument(createStoreZip([
    { path, text: source }, { path: model, bytes: new Uint8Array([1, 2, 3]) },
  ]), { kind: 'project' });
}
const runtime = () => ({
  dependencies: vi.fn(async () => ({ base_files: {}, base_asset_manifest: {}, packs: [] })),
  validate: vi.fn(async () => ({ accepted: true, findings: [] })),
});

describe('Workshop structural model authoring', () => {
  it('accepts a captured PNG with complete authored production capture metadata', () => {
    const draft = makeDraft();
    draft.put('assets/models/ship.png', Uint8Array.of(0x89, 0x50, 0x4e, 0x47));
    const exact = draft.read(path).replace('shape = "sphere"', `billboard = 'assets/models/ship.png'\n[lod.capture]\nsource = 'assets/models/ship.glb'\nyaw_views = 8\nresolution = 256\npitch = 20.0`);
    draft.edit(path, exact);
    expect(validateModelStructure(path, exact, draft).lod.at(-1).capture).toEqual({
      source: 'assets/models/ship.glb', yaw_views: 8, resolution: 256, pitch: 20,
    });
  });

  it('can inspect a complete capture recipe before its generated PNG exists without accepting it for save', () => {
    const draft = makeDraft();
    const exact = draft.read(path).replace('shape = "sphere"', `billboard = 'assets/models/future.png'\n[lod.capture]\nsource = 'assets/models/ship.glb'\nyaw_views = 8\nresolution = 256\npitch = 20.0`);
    draft.edit(path, exact);
    expect(inspectModelStructure(draft, model, path).lod.at(-1).capture.yaw_views).toBe(8);
    expect(() => validateModelStructure(path, exact, draft)).toThrow('outside the captured source');
  });
  it('creates the first identity variant for a captured model with no sidecar', async () => {
    const draft = new WorkshopDocument(createStoreZip([
      { path: model, bytes: new Uint8Array([1, 2, 3]) },
    ]), { kind: 'project' });
    const service = runtime();
    const added = await applyModelStructureOperation({ draft, runtime: service, operation: {
      type: 'variant-add', path: null, model, name: 'model', clone: false } });
    expect(added.selected).toBe(path); expect(draft.read(path)).toBe('');
    expect(service.validate).toHaveBeenCalledOnce();
    draft.undo(); expect(draft.read(path)).toBeUndefined();
  });

  it('inspects ordered local structures with source ownership and line numbers', () => {
    const result = inspectModelStructure(makeDraft(), model, path);
    expect(result.markers).toMatchObject([{ name: 'muzzle', owner: path, line: 6 }]);
    expect(result.target_points.map(row => [row.position, row.owner, row.line])).toEqual([
      [[4, 5, 6], path, 15], [[-4, 5, 6], path, 18],
    ]);
    expect(result.lod.map(row => [row.model || row.shape, row.line])).toEqual([[model, 21], ['sphere', 26]]);
  });

  it('edits and reorders exact spans without normalising comments, unknown fields, or line endings', () => {
    const draft = makeDraft();
    const edited = prepareModelStructureOperation(draft, {}, { type: 'marker-set', path, name: 'muzzle',
      newName: 'weapon', position: [9, 8, 7], direction: [1, 0, 0] }).changes[0].after;
    expect(edited).toContain('[base] # rig\r\noffset = [ 1,  2, 3 ] # keep');
    expect(edited).toContain('[markers.weapon] # marker comment\r\nposition = [9, 8, 7] # position comment');
    expect(edited).toContain('[extents] # editor-untouched runtime field');
    expect(edited).not.toContain('\n[markers.muzzle]');

    const moved = prepareModelStructureOperation(draft, {}, { type: 'target-move', path, index: 1, direction: -1 }).changes[0].after;
    expect(moved.indexOf('# aft')).toBeLessThan(moved.indexOf('# fore'));
    expect(moved).toContain('radius = 1.500 # editor-untouched runtime field');
    expect(moved).toContain('max_distance = 20 # metres');
  });

  it('patches only changed known fields and preserves their noncanonical source spellings', () => {
    const exact = source
      .replace('position = [1, 2, 3]', 'position = [ 1,  2, 3 ]')
      .replace('direction = [0, 0, 1]', 'direction = [ 0, 0, 1 ]')
      .replace('model = "assets/models/ship.glb"', "model = 'assets/models/ship.glb'")
      .replace('max_distance = 20', 'max_distance = 2_0');
    const draft = new WorkshopDocument(createStoreZip([
      { path, text: exact }, { path: model, bytes: new Uint8Array([1, 2, 3]) },
    ]), { kind: 'project' });
    const renamed = prepareModelStructureOperation(draft, {}, { type: 'marker-set', path, name: 'muzzle',
      newName: 'weapon', position: [1, 2, 3], direction: [0, 0, 1] }).changes[0].after;
    expect(renamed).toContain('position = [ 1,  2, 3 ] # position comment');
    expect(renamed).toContain('direction = [ 0, 0, 1 ]');
    const lod = prepareModelStructureOperation(draft, {}, { type: 'lod-set', path, index: 0,
      level: { model, max_distance: 30 } }).changes[0].after;
    expect(lod).toContain("model = 'assets/models/ship.glb'");
    expect(lod).toContain('max_distance = 30 # metres');
  });

  it('keeps marker-owned nested tables attached through reorder and remove', () => {
    const nested = `[markers.alpha]\nposition = [0, 0, 0]\ndirection = [0, 0, 1]\n`
      + `[markers.alpha.metadata]\nunknown = "kept"\n\n`
      + `[markers.beta]\nposition = [1, 0, 0]\ndirection = [0, 1, 0]\n`;
    const draft = new WorkshopDocument(createStoreZip([
      { path, text: nested }, { path: model, bytes: new Uint8Array([1, 2, 3]) },
    ]), { kind: 'project' });
    expect(inspectModelStructure(draft, model, path).markers.map(marker => marker.name)).toEqual(['alpha', 'beta']);
    const moved = prepareModelStructureOperation(draft, {}, {
      type: 'marker-move', path, name: 'alpha', direction: 1,
    }).changes[0].after;
    expect(moved.indexOf('[markers.beta]')).toBeLessThan(moved.indexOf('[markers.alpha]'));
    expect(moved.indexOf('[markers.alpha]')).toBeLessThan(moved.indexOf('[markers.alpha.metadata]'));
    expect(moved).toContain('unknown = "kept"');
    const renamed = prepareModelStructureOperation(draft, {}, { type: 'marker-set', path, name: 'alpha',
      newName: 'renamed', position: [0, 0, 0], direction: [0, 0, 1] }).changes[0].after;
    expect(renamed).toContain('[markers.renamed.metadata]');
    expect(renamed).not.toContain('[markers.alpha');
    const removed = prepareModelStructureOperation(draft, {}, {
      type: 'marker-remove', path, name: 'alpha',
    }).changes[0].after;
    expect(removed).not.toContain('[markers.alpha');
    expect(removed).toContain('[markers.beta]');
  });

  it('finds the complete value span when multiline vector comments contain TOML punctuation', () => {
    const multiline = `[markers.muzzle]\nposition = [\n  1, # ] " } comment\n  2, 3,\n]\ndirection = [0, 0, 1]\n`;
    const draft = new WorkshopDocument(createStoreZip([
      { path, text: multiline }, { path: model, bytes: new Uint8Array([1, 2, 3]) },
    ]), { kind: 'project' });
    const edited = prepareModelStructureOperation(draft, {}, { type: 'marker-set', path, name: 'muzzle',
      newName: 'muzzle', position: [4, 5, 6], direction: [0, 0, 1] }).changes[0].after;
    expect(edited).toBe('[markers.muzzle]\nposition = [4, 5, 6]\ndirection = [0, 0, 1]\n');
    expect(() => validateModelStructure(path, edited, draft)).not.toThrow();
  });

  it('supports marker, target point, LOD, and variant add/remove as ordinary reversible history', async () => {
    const draft = makeDraft(), service = runtime();
    draft.put('assets/models/ship.png', new Uint8Array([4, 5, 6]));
    await applyModelStructureOperation({ draft, runtime: service, operation: { type: 'marker-add', path,
      name: 'dock', position: [0, 0, 0], direction: [0, 1, 0] } });
    await applyModelStructureOperation({ draft, runtime: service, operation: { type: 'target-add', path,
      position: [7, 8, 9] } });
    await applyModelStructureOperation({ draft, runtime: service, operation: { type: 'lod-add', path,
      index: 1, level: { billboard: 'assets/models/ship.png', max_distance: 40 } } });
    expect(inspectModelStructure(draft, model, path).markers.at(-1).name).toBe('dock');
    expect(inspectModelStructure(draft, model, path).target_points.at(-1).position).toEqual([7, 8, 9]);
    expect(inspectModelStructure(draft, model, path).lod.find(level => level.billboard)?.billboard).toBe('assets/models/ship.png');
    draft.undo(); draft.undo(); draft.undo();
    expect(draft.read(path)).toBe(source);

    const added = await applyModelStructureOperation({ draft, runtime: service, operation: {
      type: 'variant-add', path, model, name: 'damaged', clone: true } });
    expect(added.selected).toBe('assets/models/ship.damaged.toml');
    expect(draft.read(added.selected)).toBe(source);
    const renamed = await applyModelStructureOperation({ draft, runtime: service, operation: {
      type: 'variant-rename', path: added.selected, model, name: 'battle' } });
    expect(draft.read(added.selected)).toBeUndefined();
    expect(draft.read(renamed.selected)).toBe(source);
    draft.undo();
    expect(draft.read(added.selected)).toBe(source);
    expect(draft.read(renamed.selected)).toBeUndefined();
    await applyModelStructureOperation({ draft, runtime: service, operation: {
      type: 'variant-remove', path: added.selected, model } });
    expect(draft.read(added.selected)).toBeUndefined();
    draft.undo(); expect(draft.read(added.selected)).toBe(source);
    const restored = WorkshopDocument.restore(draft.snapshot());
    expect(restored.read(added.selected)).toBe(source);
    expect(new WorkshopDocument(draft.archive(), { kind: 'project' }).read(added.selected)).toBe(source);
  });

  it('refuses invalid runtime structure with an exact source location and never mutates on runtime or stale refusal', async () => {
    const draft = makeDraft();
    expect(() => validateModelStructure(path, source.replace('direction = [0, 0, 1]', 'direction = [2, 0, 0]'), draft))
      .toThrowError(expect.objectContaining({ report: expect.objectContaining({ findings: [expect.objectContaining({ file: path, line: 6 })] }) }));
    expect(() => prepareModelStructureOperation(draft, {}, { type: 'lod-set', path, index: 1,
      level: { model, max_distance: 10 } })).toThrow('increase');
    const service = runtime(); service.validate.mockResolvedValue({ accepted: false, findings: [{ message: 'renderer refusal' }] });
    await expect(applyModelStructureOperation({ draft, runtime: service, operation: { type: 'target-remove', path, index: 0 } }))
      .rejects.toThrow('runtime-validation-refused');
    expect(draft.read(path)).toBe(source); expect(draft.canUndo()).toBe(false);
    service.validate.mockResolvedValue({ accepted: true, findings: [] });
    await expect(applyModelStructureOperation({ draft, runtime: service, current: () => false,
      operation: { type: 'target-remove', path, index: 0 } })).rejects.toThrow('model-structure-stale');
    expect(draft.read(path)).toBe(source);
  });
});
