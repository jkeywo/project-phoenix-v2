import { describe, expect, it, vi } from 'vitest';
import { createStoreZip } from '../mod-pack-export.js';
import { WorkshopDocument } from '../workshop-document.js';
import { applySpatialOperation, prepareSpatialOperation, spatialInventory } from '../workshop-spatial.js';

const manifest = '[pack]\nformat=1\nid="mine"\nversion="1"\nname="Mine"\n[pack.requires]\ncontent_id="phoenix-base"\ncontent_epoch=1\n';
const root = '# exact root\nextra_worlds = [] # retain\n[anchors]\nstart = [1.0, 2.0, 3.0] # keep\n\n[[entity]]\ntemplate_path = "assets/entities/region_nebula.toml"\nid = "fog"\ntransform = { position = [4.0, 0.0, 5.0], rotation = [0.0, 1.0, 0.0] } # placement\n[entity.overrides]\nunknown = 9\n\n[extension]\nkeep = true\n';
const pack = () => new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: manifest }, { path: 'assets/worlds/root.toml', text: root },
]));
const dependencies = { base_files: {
  'assets/entities/region_nebula.toml': 'tags=["region","nebula"]\n[shape]\ntype="sphere"\nradius=20\n',
}, packs: [] };

describe('Workshop spatial exact-source transactions', () => {
  it('inventories layers, anchors, Regions, entities and authored transforms', () => {
    expect(spatialInventory(pack(), dependencies)[0]).toMatchObject({ path: 'assets/worlds/root.toml',
      anchors: [{ name: 'start', position: [1, 2, 3] }],
      entities: [{ index: 0, id: 'fog', region: true,
        transform: { position: [4, 0, 5], rotation: [0, 1, 0] } }] });
  });

  it('classifies Regions from resolved runtime tags rather than filename', () => {
    const draft = pack();
    draft.edit('assets/worlds/root.toml', root.replace('assets/entities/region_nebula.toml', 'assets/entities/cloud.toml'));
    const effective = { base_files: { 'assets/entities/cloud.toml': 'includes=["region_base.toml"]\n',
      'assets/entities/region_base.toml': 'tags=["region"]\n[shape]\ntype="sphere"\nradius=2\n' }, packs: [] };
    expect(spatialInventory(draft, effective)[0].entities[0].region).toBe(true);
  });

  it('moves anchors and entities while preserving comments, unknown fields and headings', () => {
    const draft = pack();
    const anchor = prepareSpatialOperation(draft, { type: 'anchor-move', path: 'assets/worlds/root.toml',
      name: 'start', position: [8, 2, 9] });
    expect(anchor.changes[0].after).toContain('start = [8, 2, 9] # keep');
    const entity = prepareSpatialOperation(draft, { type: 'entity-move', path: 'assets/worlds/root.toml', index: 0,
      transform: { position: [10, 0, 11], rotation: [0, 2, 0] } });
    expect(entity.changes[0].after).toContain('rotation = [0, 2, 0]');
    expect(entity.changes[0].after).toContain('[entity.overrides]\nunknown = 9');
    expect(entity.changes[0].after).toContain('[extension]\nkeep = true');
    expect(draft.read('assets/worlds/root.toml')).toBe(root);
  });

  it('patches inline transform leaves without normalizing unknown fields or comments', () => {
    const draft = pack();
    draft.edit('assets/worlds/root.toml', root.replace(
      'transform = { position = [4.0, 0.0, 5.0], rotation = [0.0, 1.0, 0.0] } # placement',
      'transform = {  position=[4.0, 0.0, 5.0] , mystery={keep=7}, rotation=[0.0,1.0,0.0], scale=[1.00, 1.00, 1.00] } # placement'));
    const moved = prepareSpatialOperation(draft, { type: 'entity-move', path: 'assets/worlds/root.toml', index: 0,
      transform: { position: [6, 0, 7] } }).changes[0].after;
    expect(moved).toContain('transform = {  position=[6, 0, 7] , mystery={keep=7}, rotation=[0.0,1.0,0.0], scale=[1.00, 1.00, 1.00] } # placement');
  });

  it('edits subtable transforms and retains unknown transform fields and comments', () => {
    const draft = pack();
    draft.edit('assets/worlds/root.toml', '# root\n[[entity]]\ntemplate_path="assets/entities/star_sun.toml"\n[entity.transform]\nanchor="start" # remove\nrotation=[0.00,1.00,0.00] # heading\nscale=[1.00, 1.00, 1.00]\nextension={keep=7} # unknown\n');
    const moved = prepareSpatialOperation(draft, { type: 'entity-move', path: 'assets/worlds/root.toml', index: 0,
      transform: { position: [3, 0, 4] } }).changes[0].after;
    expect(moved).not.toContain('anchor=');
    expect(moved).toContain('rotation=[0.00,1.00,0.00] # heading');
    expect(moved).toContain('scale=[1.00, 1.00, 1.00]');
    expect(moved).toContain('extension={keep=7} # unknown');
    expect(moved).toContain('position = [3, 0, 4]');
  });

  it('adds and removes a layer as one grouped document change', () => {
    const draft = pack();
    const added = prepareSpatialOperation(draft, { type: 'layer-add', path: 'assets/worlds/root.toml',
      layer: 'assets/worlds/fog.toml' });
    expect(added.changes).toHaveLength(2);
    expect(added.changes[0].after).toContain('extra_worlds = ["assets/worlds/fog.toml"] # retain');
    draft.apply(added.changes);
    expect(draft.read('assets/worlds/fog.toml')).toContain('[anchors]');
    expect(draft.undo()).toBe('assets/worlds/root.toml');
    expect(draft.read('assets/worlds/fog.toml')).toBeUndefined();
    expect(draft.read('assets/worlds/root.toml')).toBe(root);
  });

  it('does not delete a layer still composed by another authored world', () => {
    const draft = pack();
    draft.edit('assets/worlds/root.toml', root.replace('extra_worlds = []', 'extra_worlds = ["assets/worlds/shared.toml"]'));
    draft.put('assets/worlds/shared.toml', '[anchors]\n');
    draft.put('assets/worlds/other.toml', 'extra_worlds=["assets/worlds/shared.toml"]\n');
    expect(() => prepareSpatialOperation(draft, { type: 'layer-remove', path: 'assets/worlds/root.toml',
      layer: 'assets/worlds/shared.toml' })).toThrow('layer-in-use');
  });

  it('adds and removes authored spawns without normalizing neighboring source', () => {
    const draft = pack();
    const added = prepareSpatialOperation(draft, { type: 'entity-add', path: 'assets/worlds/root.toml',
      template_path: 'assets/entities/station_axiom.toml', id: 'station',
      transform: { position: [20, 0, 30], rotation: [0, 1.57, 0] } });
    expect(added.changes[0].after).toContain('id = "station"\ntransform = { position = [20, 0, 30], rotation = [0, 1.57, 0] }');
    expect(added.changes[0].after).toContain('[extension]\nkeep = true');
    const removed = prepareSpatialOperation(draft, { type: 'entity-remove', path: 'assets/worlds/root.toml', index: 0 });
    expect(removed.changes[0].after).not.toContain('id = "fog"');
    expect(removed.changes[0].after).toContain('[extension]\nkeep = true');
  });

  it('refuses deleting an anchor used by a spawn in any draft layer', () => {
    const draft = pack();
    draft.edit('assets/worlds/root.toml', root.replace('extra_worlds = []', 'extra_worlds = ["assets/worlds/layer.toml"]'));
    draft.put('assets/worlds/layer.toml', '[[entity]]\ntemplate_path="assets/entities/star_sun.toml"\ntransform={anchor="start"}\n');
    expect(() => prepareSpatialOperation(draft, { type: 'anchor-remove', path: 'assets/worlds/root.toml', name: 'start' }))
      .toThrow('anchor-in-use');
  });

  it('does not confuse an unrelated world using the same anchor name', () => {
    const draft = pack();
    draft.put('assets/worlds/unrelated.toml', '[anchors]\nstart=[9,0,9]\n[[entity]]\ntemplate_path="assets/entities/star_sun.toml"\ntransform={anchor="start"}\n');
    expect(prepareSpatialOperation(draft, { type: 'anchor-remove', path: 'assets/worlds/root.toml', name: 'start' })
      .changes[0].after).not.toContain('start =');
  });

  it('validates the exact candidate and rejects stale async completion without mutating the draft', async () => {
    const draft = pack(), validate = vi.fn(async () => ({ accepted: true, findings: [] }));
    await applySpatialOperation({ draft, runtime: { validate }, operation: { type: 'anchor-move',
      path: 'assets/worlds/root.toml', name: 'start', position: [2, 2, 3] } });
    expect(draft.read('assets/worlds/root.toml')).toContain('start = [2, 2, 3]');
    expect(validate).toHaveBeenCalledOnce();
    const stale = pack();
    await expect(applySpatialOperation({ draft: stale, runtime: { validate }, current: () => false,
      operation: { type: 'anchor-move', path: 'assets/worlds/root.toml', name: 'start', position: [2, 2, 3] } }))
      .rejects.toThrow('stale-spatial-operation');
    expect(stale.read('assets/worlds/root.toml')).toBe(root);
  });

  it('can validate and land a new layer member', async () => {
    const draft = pack();
    await applySpatialOperation({ draft, runtime: { validate: async () => ({ accepted: true, findings: [] }) },
      operation: { type: 'layer-add', path: 'assets/worlds/root.toml', layer: 'assets/worlds/new.toml' } });
    expect(draft.read('assets/worlds/new.toml')).toContain('[anchors]');
  });
});
