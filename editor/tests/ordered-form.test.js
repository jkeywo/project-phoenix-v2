import { describe, expect, it } from 'vitest';
import { formNeighbours, liveFormPositions, survivingFormPosition, swapFormNeighbour } from '../ordered-form.js';

describe('ordered authored forms', () => {
  it('skips tombstones and requires both sides to carry their content', () => {
    const list = [{ id: 'a' }, { removed: true }, { id: 'b', locked: true }, { id: 'c' }];
    const movable = row => !row.locked;
    expect(liveFormPositions(list)).toEqual([0, 2, 3]);
    expect(formNeighbours(list, 0, movable)).toMatchObject({ up: false, down: false, selfBlocked: false, neighbourBlocked: true });
    expect(swapFormNeighbour(list, 0, 1, movable)).toBeNull();
    expect(swapFormNeighbour(list, 0, 1)).toBe(2);
    expect(list.map(row => row.id)).toEqual(['b', undefined, 'a', 'c']);
  });
  it('keeps widgets dense and preserves each removal focus policy', () => {
    const rows = [{ id: 'a' }, { id: 'b', removed: true }, { id: 'c' }];
    expect(swapFormNeighbour(rows, 0, 1, () => true, true)).toBe(1);
    expect(survivingFormPosition([{ id: 'a' }, { id: 'c' }], 0)).toBe(0);
    expect(survivingFormPosition([{ id: 'a' }, { id: 'c' }], 0, true)).toBe(1);
    expect(survivingFormPosition([{ id: 'a' }, { removed: true }], 1, true)).toBe(0);
    expect(survivingFormPosition([], 0)).toBeUndefined();
    expect(swapFormNeighbour(rows, -1, 1, () => true, true)).toBeNull();
    expect(swapFormNeighbour(rows, 2, 1, () => true, true)).toBeNull();
  });
});
