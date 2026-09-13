import { describe, it, expect, vi } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { modelDocuments, createModelVariant, patchModelFields } from '../workshop-models.js';

const model = 'assets/models/ship.glb', path = 'assets/models/ship.model.toml';
const source = '# 原型\r\n[base] # rig\r\noffset = [1, 2, 3] # retain\nscale = [1, 1, 1]\r\n';
const makeDraft = () => new WorkshopDocument(createStoreZip([
  { path, text: source }, { path: model, bytes: new Uint8Array([1, 2, 3]) },
]), { kind: 'project' });
const fields = [
  { path: ['base', 'offset', 0], source: '1' },
  { path: ['base', 'offset', 1], source: '2' },
];
const patcher = () => ({ patch: vi.fn(async (text, patch) => {
  expect(patch.expected_source).toBe(text);
  return patch.path[2] === 0 ? text.replace('[1, 2, 3]', `[${patch.value_source}, 2, 3]`)
    : text.replace(', 2, 3]', `, ${patch.value_source}, 3]`);
}) });

describe('Workshop model source transactions', () => {
  it('groups model assets and default/named source variants without treating textures as models', () => {
    expect(modelDocuments([path, model, 'assets/models/ship.damaged.toml', 'assets/models/ship.bin',
      'assets/models/ship.png', 'assets/entities/ship.model.toml'])).toEqual([
      { model, variants: [{ name: 'model', path }, { name: 'damaged', path: 'assets/models/ship.damaged.toml' }] },
    ]);
  });
  it('clones exact comments and mixed line endings into ordinary undo/redo and export', () => {
    const draft = makeDraft();
    const variant = createModelVariant(draft, model, 'damaged', path);
    expect(draft.read(variant)).toBe(source);
    expect(draft.undo()).toBe(variant);
    expect(draft.read(variant)).toBeUndefined();
    draft.redo();
    expect(new WorkshopDocument(draft.archive(), { kind: 'project' }).read(variant)).toBe(source);
    expect(draft.read(path)).toBe(source);
    expect(() => createModelVariant(draft, model, 'model', path)).toThrow('variant_exists');
    expect(() => createModelVariant(draft, model, '../escape', path)).toThrow('invalid_variant');
    expect(() => createModelVariant(draft, 'assets/models/unknown.glb', 'new')).toThrow('invalid_variant');
  });
  it('commits a vector change as one history entry after all runtime patches succeed', async () => {
    const draft = makeDraft(), runtime = patcher();
    expect(await patchModelFields({ draft, runtime, documentPath: path, source, fields, values: ['8', '9'] })).toBe(true);
    expect(draft.read(path)).toBe(source.replace('[1, 2, 3]', '[8, 9, 3]'));
    draft.undo();
    expect(draft.read(path)).toBe(source);
    expect(draft.canUndo()).toBe(false);
  });
  it('refuses the whole edit when a later scalar fails validation', async () => {
    const draft = makeDraft(), runtime = patcher();
    runtime.patch.mockImplementationOnce(async () => source.replace('[1, 2, 3]', '[8, 2, 3]'))
      .mockRejectedValueOnce(new Error('numeric value required'));
    await expect(patchModelFields({ draft, runtime, documentPath: path, source, fields, values: ['8', 'bad'] }))
      .rejects.toThrow('numeric value required');
    expect(draft.read(path)).toBe(source);
    expect(draft.canUndo()).toBe(false);
  });
  it('never overwrites a newer source or applies after leaving its context', async () => {
    for (const switchContext of [false, true]) {
      const draft = makeDraft(); let current = true;
      const runtime = { patch: async () => {
        if (switchContext) current = false;
        else draft.edit(path, `${source}# newer\n`);
        return source.replace('[1, 2, 3]', '[8, 2, 3]');
      } };
      await expect(patchModelFields({ draft, runtime, documentPath: path, source, fields,
        values: ['8', '2'], current: () => current })).rejects.toThrow('workshop.inspector_stale');
      expect(draft.read(path)).toBe(switchContext ? source : `${source}# newer\r\n`);
    }
  });
});
