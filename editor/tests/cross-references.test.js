import { describe, it, expect } from 'vitest';
import { CrossReferenceIndex } from '../cross-references.js';

describe('CrossReferenceIndex', () => {
  it('returns empty results for empty layers', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([]);
    expect(idx.getAllEntityNames()).toEqual([]);
    expect(idx.getAllAnchorNames()).toEqual([]);
    expect(idx.getAllObjectiveIds()).toEqual([]);
    expect(idx.findReferences('nonexistent')).toEqual([]);
  });

  it('finds a named entity from a single layer', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      {
        path: 'worlds/default.toml',
        worldState: {
          entity: [{ name: 'raider_alpha', template_path: 'assets/entities/pirate_raider.toml' }],
        },
      },
    ]);
    const names = idx.getAllEntityNames();
    expect(names).toHaveLength(1);
    expect(names[0]).toEqual({ name: 'raider_alpha', layerPath: 'worlds/default.toml' });
  });

  it('finds trigger entity name as a reference', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      {
        path: 'worlds/default.toml',
        worldState: {
          entity: [{ name: 'raider_alpha' }],
          trigger: [{ condition: 'on_destroyed', entity: 'raider_alpha' }],
        },
      },
    ]);
    const refs = idx.findReferences('raider_alpha');
    expect(refs).toHaveLength(1);
    expect(refs[0].type).toBe('trigger');
    expect(refs[0].layerPath).toBe('worlds/default.toml');
  });

  it('disambiguates same entity name across multiple layers', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      {
        path: 'worlds/default.toml',
        worldState: {
          entity: [{ name: 'raider_alpha' }],
        },
      },
      {
        path: 'worlds/patrol.toml',
        worldState: {
          entity: [{ name: 'raider_alpha' }],
          trigger: [{ condition: 'on_destroyed', entity: 'raider_alpha' }],
        },
      },
    ]);
    const names = idx.getAllEntityNames();
    expect(names).toHaveLength(1);
    expect(names[0].layerPath).toBe('worlds/default.toml');
    const refs = idx.findReferences('raider_alpha');
    const patrolRefs = refs.filter(r => r.layerPath === 'worlds/patrol.toml');
    expect(patrolRefs).toHaveLength(1);
    expect(patrolRefs[0].type).toBe('trigger');
  });

  it('returns anchor names from layers', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      {
        path: 'worlds/default.toml',
        worldState: {
          anchors: { starbase_alpha: [500, 0, 0], patrol_alpha: [300, 0, -300] },
        },
      },
    ]);
    const anchors = idx.getAllAnchorNames();
    expect(anchors).toHaveLength(2);
    expect(anchors).toContainEqual({ name: 'starbase_alpha', layerPath: 'worlds/default.toml' });
    expect(anchors).toContainEqual({ name: 'patrol_alpha', layerPath: 'worlds/default.toml' });
  });

  it('derives objective IDs from add_objective actions', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      {
        path: 'worlds/default.toml',
        worldState: {
          trigger: [{
            condition: 'on_destroyed',
            entity: 'raider_alpha',
            action: [{ type: 'add_objective', id: 'obj-raider-destroyed' }],
          }],
        },
      },
    ]);
    const objectives = idx.getAllObjectiveIds();
    expect(objectives).toHaveLength(1);
    expect(objectives[0]).toEqual({ id: 'obj-raider-destroyed', layerPath: 'worlds/default.toml' });
  });

  it('findReferences returns all reference types across layers', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      {
        path: 'worlds/default.toml',
        worldState: {
          entity: [{ name: 'Starbase Alpha' }],
          trigger: [{
            condition: 'on_attacked',
            entity: 'Starbase Alpha',
            action: [{ type: 'spawn', target_entity: 'Starbase Alpha' }],
          }],
          comms: [{
            from: 'Starbase Alpha',
            entity: 'Starbase Alpha',
            message: 'Hello',
          }],
        },
      },
    ]);
    const refs = idx.findReferences('Starbase Alpha');
    const types = refs.map(r => r.type);
    expect(types.filter(t => t === 'trigger')).toHaveLength(1);
    expect(types.filter(t => t === 'action')).toHaveLength(1);
    expect(types.filter(t => t === 'comms')).toHaveLength(2);
  });

  it('rebuilds index on each indexLayers call (no stale data)', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      { path: 'first.toml', worldState: { entity: [{ name: 'alpha' }] } },
    ]);
    expect(idx.getAllEntityNames()).toHaveLength(1);
    expect(idx.getAllEntityNames()[0].name).toBe('alpha');
    idx.indexLayers([
      { path: 'second.toml', worldState: { entity: [{ name: 'beta' }] } },
    ]);
    const names = idx.getAllEntityNames();
    expect(names).toHaveLength(1);
    expect(names[0].name).toBe('beta');
    expect(names[0].layerPath).toBe('second.toml');
  });
});
