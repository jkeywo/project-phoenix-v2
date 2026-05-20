import { describe, it, expect } from 'vitest';
import { canDeleteAnchor } from '../anchor-delete.js';

function layer(path, worldState) {
  return { path, worldState };
}

describe('canDeleteAnchor', () => {

  it('no references returns canDelete: true', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
        entity: [
          { name: 'Some Station', anchor: 'other_anchor' },
        ],
      }),
    ];
    const result = canDeleteAnchor('starbase_alpha', layers, 'worlds/default.toml');
    expect(result.canDelete).toBe(true);
    expect(result.blockers).toEqual([]);
  });

  it('entity in same layer references anchor returns canDelete: false with one blocker', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
        entity: [
          { name: 'Starbase Alpha', anchor: 'starbase_alpha' },
        ],
      }),
    ];
    const result = canDeleteAnchor('starbase_alpha', layers, 'worlds/default.toml');
    expect(result.canDelete).toBe(false);
    expect(result.blockers).toHaveLength(1);
    expect(result.blockers[0]).toEqual({
      layerPath: 'worlds/default.toml',
      entityName: 'Starbase Alpha',
      type: 'entity',
    });
  });

  it('entity in different layer references anchor returns canDelete: false', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
      }),
      layer('worlds/patrol.toml', {
        entity: [
          { name: 'raider_alpha', anchor: 'starbase_alpha' },
        ],
      }),
    ];
    const result = canDeleteAnchor('starbase_alpha', layers, 'worlds/default.toml');
    expect(result.canDelete).toBe(false);
    expect(result.blockers).toHaveLength(1);
    expect(result.blockers[0]).toEqual({
      layerPath: 'worlds/patrol.toml',
      entityName: 'raider_alpha',
      type: 'entity',
    });
  });

  it('multiple entities across layers references anchor returns canDelete: false with all blockers', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
        entity: [
          { name: 'Starbase Alpha', anchor: 'starbase_alpha' },
        ],
      }),
      layer('worlds/alternate.toml', {
        entity: [
          { name: 'Waypoint_1', anchor: 'starbase_alpha' },
          { name: 'Waypoint_2', anchor: 'starbase_alpha' },
        ],
      }),
    ];
    const result = canDeleteAnchor('starbase_alpha', layers, 'worlds/default.toml');
    expect(result.canDelete).toBe(false);
    expect(result.blockers).toHaveLength(3);
    expect(result.blockers[0]).toEqual({
      layerPath: 'worlds/default.toml',
      entityName: 'Starbase Alpha',
      type: 'entity',
    });
    expect(result.blockers[1]).toEqual({
      layerPath: 'worlds/alternate.toml',
      entityName: 'Waypoint_1',
      type: 'entity',
    });
    expect(result.blockers[2]).toEqual({
      layerPath: 'worlds/alternate.toml',
      entityName: 'Waypoint_2',
      type: 'entity',
    });
  });

  it('trigger action references anchor returns canDelete: false', () => {
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
    const result = canDeleteAnchor('starbase_alpha', layers, 'worlds/default.toml');
    expect(result.canDelete).toBe(false);
    expect(result.blockers).toHaveLength(1);
    expect(result.blockers[0]).toEqual({
      layerPath: 'worlds/default.toml',
      entityName: 'raider_alpha',
      type: 'trigger',
    });
  });

  it('empty layers returns canDelete: true', () => {
    const result = canDeleteAnchor('starbase_alpha', [], 'worlds/default.toml');
    expect(result.canDelete).toBe(true);
    expect(result.blockers).toEqual([]);
  });

  it('anchor does not exist in any layer returns canDelete: true', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { other_anchor: [100.0, 0.0, 0.0] },
        entity: [
          { name: 'Something', anchor: 'other_anchor' },
        ],
      }),
    ];
    const result = canDeleteAnchor('nonexistent_anchor', layers, 'worlds/default.toml');
    expect(result.canDelete).toBe(true);
    expect(result.blockers).toEqual([]);
  });

  it('entity with no name still shows as blocker with entityName: null', () => {
    const layers = [
      layer('worlds/default.toml', {
        anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
        entity: [
          { anchor: 'starbase_alpha' },
        ],
      }),
    ];
    const result = canDeleteAnchor('starbase_alpha', layers, 'worlds/default.toml');
    expect(result.canDelete).toBe(false);
    expect(result.blockers).toHaveLength(1);
    expect(result.blockers[0].entityName).toBeNull();
  });

});
