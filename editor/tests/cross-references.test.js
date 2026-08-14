import { describe, it, expect } from 'vitest';
import { CrossReferenceIndex } from '../cross-references.js';

describe('CrossReferenceIndex', () => {
  it('returns empty results for empty layers', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([]);
    expect(idx.getAllEntityNames()).toEqual([]);
    expect(idx.getAllAnchorNames()).toEqual([]);
    expect(idx.hasEntity('nonexistent')).toBe(false);
  });

  it('finds a named entity from a single layer', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      {
        path: 'worlds/default.toml',
        worldState: {
          entity: [{ name: 'raider_alpha', template_path: 'assets/entities/ship_harrow_patrol.toml' }],
        },
      },
    ]);
    const names = idx.getAllEntityNames();
    expect(names).toHaveLength(1);
    expect(names[0]).toEqual({ name: 'raider_alpha', layerPath: 'worlds/default.toml' });
  });

  it('disambiguates same entity name across multiple layers (first layer wins)', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      { path: 'worlds/default.toml', worldState: { entity: [{ name: 'raider_alpha' }] } },
      { path: 'worlds/patrol.toml', worldState: { entity: [{ name: 'raider_alpha' }] } },
    ]);
    const names = idx.getAllEntityNames();
    expect(names).toHaveLength(1);
    expect(names[0].layerPath).toBe('worlds/default.toml');
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

  it('hasEntity returns true for declared entities and false for others', () => {
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      { path: 'w.toml', worldState: { entity: [{ name: 'alpha' }, { name: 'beta' }] } },
    ]);
    expect(idx.hasEntity('alpha')).toBe(true);
    expect(idx.hasEntity('beta')).toBe(true);
    expect(idx.hasEntity('phantom')).toBe(false);
  });

  it('ignores a `[[trigger]]` / `[[comms]]` array a stale world still carries (#985)', () => {
    // `parse_world` refuses such a world outright now; the index simply does
    // not look at either array, so nothing it reports comes from them.
    const idx = new CrossReferenceIndex();
    idx.indexLayers([
      {
        path: 'w.toml',
        worldState: {
          entity: [{ name: 'alpha' }],
          trigger: [{ condition: 'on_destroyed', entity: 'phantom' }],
          comms: [{ from: 'ghost', entity: 'ghost' }],
        },
      },
    ]);
    expect(idx.getAllEntityNames().map((e) => e.name)).toEqual(['alpha']);
    expect(idx.hasEntity('phantom')).toBe(false);
    expect(idx.hasEntity('ghost')).toBe(false);
  });
});
