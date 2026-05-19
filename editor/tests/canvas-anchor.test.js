import { describe, it, expect } from 'vitest';
import { getAnchorMarkers, moveAnchor, resolveEntityPosition } from '../canvas-anchor.js';

// Tests for the pure anchor-logic module used by Scenario Mode canvas.

describe('getAnchorMarkers', () => {
  it('returns an empty array for null/undefined input', () => {
    expect(getAnchorMarkers(null)).toEqual([]);
    expect(getAnchorMarkers(undefined)).toEqual([]);
  });

  it('returns an empty array for an empty anchors object', () => {
    expect(getAnchorMarkers({})).toEqual([]);
  });

  it('converts a single anchor to a marker with correct x and z', () => {
    const anchors = { starbase_alpha: [500.0, 0.0, 0.0] };
    const result = getAnchorMarkers(anchors);
    expect(result).toHaveLength(1);
    expect(result[0]).toEqual({ name: 'starbase_alpha', x: 500.0, z: 0.0 });
  });

  it('converts multiple anchors from default.toml to markers', () => {
    const anchors = {
      starbase_alpha: [500.0, 0.0, 0.0],
      patrol_alpha:   [300.0, 0.0, -300.0],
      patrol_beta:    [-300.0, 0.0, -200.0],
      patrol_gamma:   [0.0, 0.0, -500.0],
    };
    const result = getAnchorMarkers(anchors);
    expect(result).toHaveLength(4);

    const alpha = result.find(m => m.name === 'starbase_alpha');
    expect(alpha).toEqual({ name: 'starbase_alpha', x: 500.0, z: 0.0 });

    const patrolAlpha = result.find(m => m.name === 'patrol_alpha');
    expect(patrolAlpha).toEqual({ name: 'patrol_alpha', x: 300.0, z: -300.0 });

    const patrolBeta = result.find(m => m.name === 'patrol_beta');
    expect(patrolBeta).toEqual({ name: 'patrol_beta', x: -300.0, z: -200.0 });

    const patrolGamma = result.find(m => m.name === 'patrol_gamma');
    expect(patrolGamma).toEqual({ name: 'patrol_gamma', x: 0.0, z: -500.0 });
  });

  it('extracts x from index 0 and z from index 2 of the position array', () => {
    const anchors = { origin: [10.0, 99.0, 20.0] };
    const result = getAnchorMarkers(anchors);
    expect(result[0].x).toBe(10.0);
    expect(result[0].z).toBe(20.0);
  });

  it('skips entries that are not arrays', () => {
    const anchors = {
      valid: [1.0, 0.0, 2.0],
      invalid: 'not-an-array',
    };
    const result = getAnchorMarkers(anchors);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe('valid');
  });

  it('skips entries that are arrays with fewer than 3 elements', () => {
    const anchors = {
      short: [1.0, 2.0],
      good: [1.0, 0.0, 3.0],
    };
    const result = getAnchorMarkers(anchors);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe('good');
  });
});

describe('moveAnchor', () => {
  const worldState = {
    global: { seed: 42 },
    anchors: {
      starbase_alpha: [500.0, 0.0, 0.0],
      patrol_alpha:   [300.0, 0.0, -300.0],
    },
    entity: [],
  };

  it('returns a new object (immutable update)', () => {
    const updated = moveAnchor(worldState, 'patrol_alpha', 400.0, -400.0);
    expect(updated).not.toBe(worldState);
  });

  it('returns a new anchors object (does not mutate)', () => {
    const updated = moveAnchor(worldState, 'patrol_alpha', 400.0, -400.0);
    expect(updated.anchors).not.toBe(worldState.anchors);
  });

  it('updates the correct anchor x and z', () => {
    const updated = moveAnchor(worldState, 'patrol_alpha', 400.0, -400.0);
    expect(updated.anchors.patrol_alpha[0]).toBe(400.0);
    expect(updated.anchors.patrol_alpha[2]).toBe(-400.0);
  });

  it('preserves the Y component of the moved anchor', () => {
    const wsWithY = {
      ...worldState,
      anchors: { ...worldState.anchors, patrol_alpha: [300.0, 50.0, -300.0] },
    };
    const updated = moveAnchor(wsWithY, 'patrol_alpha', 400.0, -400.0);
    expect(updated.anchors.patrol_alpha[1]).toBe(50.0);
  });

  it('defaults Y to 0.0 if the anchor had no prior value', () => {
    const wsNoAnchor = {
      ...worldState,
      anchors: { ...worldState.anchors, new_anchor: [1.0, 0.0, 2.0] },
    };
    // Moving an anchor that was [x, 0, z] should keep Y = 0
    const updated = moveAnchor(wsNoAnchor, 'new_anchor', 5.0, 6.0);
    expect(updated.anchors.new_anchor[1]).toBe(0.0);
  });

  it('leaves other anchors unchanged', () => {
    const updated = moveAnchor(worldState, 'patrol_alpha', 400.0, -400.0);
    expect(updated.anchors.starbase_alpha).toEqual([500.0, 0.0, 0.0]);
  });

  it('preserves other top-level fields (global, entity, etc.)', () => {
    const updated = moveAnchor(worldState, 'patrol_alpha', 400.0, -400.0);
    expect(updated.global).toBe(worldState.global);
    expect(updated.entity).toBe(worldState.entity);
  });

  it('returns worldState unchanged if worldState has no anchors section', () => {
    const ws = { global: { seed: 1 } };
    const result = moveAnchor(ws, 'foo', 1.0, 2.0);
    expect(result).toBe(ws);
  });

  it('returns worldState unchanged for null input', () => {
    expect(moveAnchor(null, 'foo', 1.0, 2.0)).toBeNull();
  });

  it('can add a new anchor name that did not previously exist', () => {
    const updated = moveAnchor(worldState, 'brand_new', 100.0, 200.0);
    expect(updated.anchors.brand_new).toEqual([100.0, 0.0, 200.0]);
    // original unchanged
    expect(worldState.anchors.brand_new).toBeUndefined();
  });
});

describe('resolveEntityPosition', () => {
  const anchors = {
    starbase_alpha: [500.0, 0.0, 0.0],
    patrol_alpha:   [300.0, 0.0, -300.0],
  };

  it('returns {x:0, z:0} for null entity', () => {
    expect(resolveEntityPosition(null, anchors)).toEqual({ x: 0, z: 0 });
  });

  it('uses inline position when present', () => {
    const entity = { position: [100.0, 0.0, -200.0] };
    expect(resolveEntityPosition(entity, anchors)).toEqual({ x: 100.0, z: -200.0 });
  });

  it('extracts x from position[0] and z from position[2]', () => {
    const entity = { position: [10.0, 999.0, 20.0] };
    const result = resolveEntityPosition(entity, anchors);
    expect(result.x).toBe(10.0);
    expect(result.z).toBe(20.0);
  });

  it('resolves anchor when entity has no inline position', () => {
    const entity = { anchor: 'starbase_alpha' };
    expect(resolveEntityPosition(entity, anchors)).toEqual({ x: 500.0, z: 0.0 });
  });

  it('resolves patrol_alpha anchor correctly', () => {
    const entity = { anchor: 'patrol_alpha' };
    expect(resolveEntityPosition(entity, anchors)).toEqual({ x: 300.0, z: -300.0 });
  });

  it('prefers inline position over anchor when both present', () => {
    const entity = { position: [1.0, 0.0, 2.0], anchor: 'starbase_alpha' };
    expect(resolveEntityPosition(entity, anchors)).toEqual({ x: 1.0, z: 2.0 });
  });

  it('returns {x:0, z:0} when anchor name is not found', () => {
    const entity = { anchor: 'nonexistent_anchor' };
    expect(resolveEntityPosition(entity, anchors)).toEqual({ x: 0, z: 0 });
  });

  it('returns {x:0, z:0} when entity has neither position nor anchor', () => {
    const entity = { tags: ['ship'] };
    expect(resolveEntityPosition(entity, anchors)).toEqual({ x: 0, z: 0 });
  });

  it('returns {x:0, z:0} when anchors map is null', () => {
    const entity = { anchor: 'starbase_alpha' };
    expect(resolveEntityPosition(entity, null)).toEqual({ x: 0, z: 0 });
  });

  it('returns {x:0, z:0} when anchors map is empty', () => {
    const entity = { anchor: 'starbase_alpha' };
    expect(resolveEntityPosition(entity, {})).toEqual({ x: 0, z: 0 });
  });

  it('reflects a moved anchor — entity position updates after moveAnchor', () => {
    const worldState = {
      global: { seed: 42 },
      anchors: { patrol_alpha: [300.0, 0.0, -300.0] },
    };
    const entity = { anchor: 'patrol_alpha' };

    const before = resolveEntityPosition(entity, worldState.anchors);
    expect(before).toEqual({ x: 300.0, z: -300.0 });

    const updated = moveAnchor(worldState, 'patrol_alpha', 400.0, -400.0);
    const after = resolveEntityPosition(entity, updated.anchors);
    expect(after).toEqual({ x: 400.0, z: -400.0 });
  });
});
