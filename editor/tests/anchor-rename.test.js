import { describe, it, expect } from 'vitest';
import { analyzeAnchorRename } from '../anchor-rename.js';

function layer(path, worldState) {
  return { path, worldState };
}

describe('analyzeAnchorRename', () => {

  it('allows rename with no references', () => {
    const layers = [
      layer('worlds/default.toml', {
        global: { seed: 42 },
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'starbase_beta', layers);
    expect(result.allowed).toBe(true);
    expect(result.error).toBeNull();
    expect(result.inLayerReferences).toEqual([]);
    expect(result.crossLayerReferences).toEqual([]);
    expect(result.rewritePairs).toEqual([
      { layerPath: 'worlds/default.toml', newAnchorValue: 'starbase_beta' },
    ]);
  });

  it('blocks rename when newName already exists', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: {
          starbase_alpha: [500.0, 0.0, 0.0],
          patrol_alpha: [300.0, 0.0, -300.0],
        },
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'patrol_alpha', layers);
    expect(result.allowed).toBe(false);
    expect(result.error).toContain('patrol_alpha');
    expect(result.error).toContain('already exists');
  });

  it('blocks rename when newName exists in a different layer', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
      }),
      layer('worlds/patrol.toml', {
        anchors: { patrol_alpha: [300.0, 0.0, -300.0] },
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'patrol_alpha', layers);
    expect(result.allowed).toBe(false);
    expect(result.error).toContain('patrol_alpha');
    expect(result.error).toContain('already exists');
  });

  it('finds in-layer references from entity anchor fields', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
        entity: [
          { name: 'Starbase Alpha', anchor: 'starbase_alpha' },
        ],
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'starbase_beta', layers);
    expect(result.allowed).toBe(true);
    expect(result.inLayerReferences).toHaveLength(1);
    expect(result.inLayerReferences[0]).toEqual({
      layerPath: 'worlds/default.toml',
      entityName: 'Starbase Alpha',
      field: 'anchor',
    });
    expect(result.crossLayerReferences).toEqual([]);
  });

  it('finds cross-layer references', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
      }),
      layer('worlds/patrol.toml', {
        anchors: { patrol_alpha: [300.0, 0.0, -300.0] },
        entity: [
          { name: 'raider_alpha', anchor: 'starbase_alpha' },
        ],
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'starbase_beta', layers);
    expect(result.allowed).toBe(true);
    expect(result.inLayerReferences).toEqual([]);
    expect(result.crossLayerReferences).toHaveLength(1);
    expect(result.crossLayerReferences[0]).toEqual({
      layerPath: 'worlds/patrol.toml',
      entityName: 'raider_alpha',
      field: 'anchor',
    });
  });

  it('finds both in-layer and cross-layer references simultaneously', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
        entity: [
          { name: 'Starbase Alpha', anchor: 'starbase_alpha' },
        ],
      }),
      layer('worlds/patrol.toml', {
        entity: [
          { name: 'raider_alpha', anchor: 'starbase_alpha' },
        ],
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'starbase_beta', layers);
    expect(result.allowed).toBe(true);
    expect(result.inLayerReferences).toHaveLength(1);
    expect(result.inLayerReferences[0].entityName).toBe('Starbase Alpha');
    expect(result.crossLayerReferences).toHaveLength(1);
    expect(result.crossLayerReferences[0].entityName).toBe('raider_alpha');
  });

  it('detects anchor references in trigger action parameters', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
        trigger: [
          {
            entity: 'raider_alpha',
            condition: 'on_destroyed',
            action: [
              { type: 'spawn', anchor: 'starbase_alpha' },
            ],
          },
        ],
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'starbase_beta', layers);
    expect(result.allowed).toBe(true);
    expect(result.inLayerReferences).toHaveLength(1);
    expect(result.inLayerReferences[0]).toEqual({
      layerPath: 'worlds/default.toml',
      entityName: 'raider_alpha',
      field: 'action.anchor',
    });
  });

  it('returns safe result for empty layers array', () => {
    const result = analyzeAnchorRename('old_anchor', 'new_anchor', []);
    expect(result.allowed).toBe(true);
    expect(result.error).toBeNull();
    expect(result.inLayerReferences).toEqual([]);
    expect(result.crossLayerReferences).toEqual([]);
    expect(result.rewritePairs).toEqual([]);
  });

  it('returns safe result for null/undefined layers', () => {
    const result1 = analyzeAnchorRename('old_anchor', 'new_anchor', null);
    expect(result1.allowed).toBe(true);
    const result2 = analyzeAnchorRename('old_anchor', 'new_anchor', undefined);
    expect(result2.allowed).toBe(true);
  });

  it('returns allowed when newName equals oldName', () => {
    const layers = [
      layer('worlds/patrol.toml', {
        anchors: { patrol_alpha: [300.0, 0.0, -300.0] },
        entity: [
          { name: 'raider_alpha', anchor: 'patrol_alpha' },
        ],
      }),
    ];
    const result = analyzeAnchorRename('patrol_alpha', 'patrol_alpha', layers);
    expect(result.allowed).toBe(true);
    expect(result.error).toBeNull();
    expect(result.inLayerReferences).toEqual([]);
    expect(result.crossLayerReferences).toEqual([]);
    expect(result.rewritePairs).toEqual([]);
  });

  it('generates rewritePairs for each layer that owns the anchor', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
      }),
      layer('worlds/alternate.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'starbase_beta', layers);
    expect(result.allowed).toBe(true);
    expect(result.rewritePairs).toHaveLength(2);
    expect(result.rewritePairs).toContainEqual({
      layerPath: 'worlds/default.toml',
      newAnchorValue: 'starbase_beta',
    });
    expect(result.rewritePairs).toContainEqual({
      layerPath: 'worlds/alternate.toml',
      newAnchorValue: 'starbase_beta',
    });
  });

  it('handles unnamed entities referencing an anchor', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
        entity: [
          { anchor: 'starbase_alpha' },
        ],
      }),
    ];
    const result = analyzeAnchorRename('starbase_alpha', 'starbase_beta', layers);
    expect(result.allowed).toBe(true);
    expect(result.inLayerReferences).toHaveLength(1);
    expect(result.inLayerReferences[0].entityName).toBe('(unnamed)');
  });

});
