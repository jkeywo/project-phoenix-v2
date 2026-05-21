import { describe, it, expect } from 'vitest';
import { getWorldTomlPaths, scanWorldActions } from '../world-file-picker.js';

describe('getWorldTomlPaths', () => {
  it('filters to only assets/worlds/*.toml', () => {
    const files = ['assets/worlds/default.toml', 'assets/entities/ship.toml', 'assets/worlds/patrol.toml', 'data.json'];
    const result = getWorldTomlPaths(files);
    expect(result).toHaveLength(2);
    expect(result[0].path).toBe('assets/worlds/default.toml');
    expect(result[1].path).toBe('assets/worlds/patrol.toml');
  });

  it('excludes non-toml and non-worlds files', () => {
    const files = ['assets/worlds/notes.txt', 'assets/entities/ship.toml'];
    const result = getWorldTomlPaths(files);
    expect(result).toHaveLength(0);
  });

  it('returns empty for empty input', () => {
    expect(getWorldTomlPaths([])).toEqual([]);
    expect(getWorldTomlPaths(null)).toEqual([]);
    expect(getWorldTomlPaths(undefined)).toEqual([]);
  });
});

describe('scanWorldActions', () => {
  it('detects load_world actions and returns their paths', () => {
    const world = {
      trigger: [
        { condition: 'on_destroyed', entity: 'raider', action: [{ type: 'load_world', path: 'assets/worlds/patrol.toml' }] },
      ],
    };
    const result = scanWorldActions(world);
    expect(result.hasLoadWorld).toBe(true);
    expect(result.loadPaths).toEqual(['assets/worlds/patrol.toml']);
  });

  it('detects unload_world actions and returns their paths', () => {
    const world = {
      trigger: [
        { condition: 'on_timer', entity: 'raider', action: [{ type: 'unload_world', path: 'assets/worlds/patrol.toml' }] },
      ],
    };
    const result = scanWorldActions(world);
    expect(result.hasUnloadWorld).toBe(true);
    expect(result.unloadPaths).toEqual(['assets/worlds/patrol.toml']);
  });

  it('returns empty arrays when no load/unload actions', () => {
    const world = { trigger: [{ action: [{ type: 'add_objective', id: 'obj1' }] }] };
    const result = scanWorldActions(world);
    expect(result.hasLoadWorld).toBe(false);
    expect(result.hasUnloadWorld).toBe(false);
    expect(result.loadPaths).toEqual([]);
    expect(result.unloadPaths).toEqual([]);
  });

  it('handles partial trigger data', () => {
    expect(scanWorldActions(null)).toEqual({ hasLoadWorld: false, hasUnloadWorld: false, loadPaths: [], unloadPaths: [] });
    expect(scanWorldActions({})).toEqual({ hasLoadWorld: false, hasUnloadWorld: false, loadPaths: [], unloadPaths: [] });
    expect(scanWorldActions({ trigger: [{}] })).toEqual({ hasLoadWorld: false, hasUnloadWorld: false, loadPaths: [], unloadPaths: [] });
  });
});
