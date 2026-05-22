import { describe, it, expect } from 'vitest';
import {
  setSpawnPosition,
  getSpawnPosition,
  getRelativeInfo,
  setSpawnRotation,
  getSpawnRotation,
  setSpawnScale,
  getSpawnScale,
} from '../toml-utils.js';

// Positioning helpers must write/read the unified nested `transform` shape
// for spawns (`template_path`). Flat-field positioning is gone.

describe('toml-utils positioning is uniformly transform-only', () => {
  describe('spawn (template_path)', () => {
    it('setSpawnPosition absolute writes nested transform.position', () => {
      const spawn = { template_path: 'assets/entities/star_sun.toml' };
      setSpawnPosition(spawn, 10, 20, 'absolute');
      expect(spawn.transform).toEqual({ position: [10, 0, 20] });
      expect(spawn.position).toBeUndefined();
      expect(spawn.anchor).toBeUndefined();
      expect(spawn.relative_to).toBeUndefined();
      expect(spawn.offset).toBeUndefined();
    });

    it('setSpawnPosition anchor writes nested transform.anchor', () => {
      const spawn = { template_path: 'assets/entities/raider.toml' };
      setSpawnPosition(spawn, 0, 0, 'anchor', 'patrol_alpha');
      expect(spawn.transform).toEqual({ anchor: 'patrol_alpha' });
      expect(spawn.anchor).toBeUndefined();
    });

    it('setSpawnPosition relative writes nested transform.relative_to + offset', () => {
      const spawn = { template_path: 'assets/entities/raider.toml' };
      setSpawnPosition(spawn, 0, 0, 'relative', 'flagship', { x: 5, z: -7 });
      expect(spawn.transform).toEqual({
        relative_to: 'flagship',
        offset: [5, 0, -7],
      });
      expect(spawn.relative_to).toBeUndefined();
      expect(spawn.offset).toBeUndefined();
    });

    it('switching modes wipes previous keys from transform', () => {
      const spawn = { template_path: 'foo.toml' };
      setSpawnPosition(spawn, 10, 20, 'absolute');
      setSpawnPosition(spawn, 0, 0, 'anchor', 'alpha');
      expect(spawn.transform).toEqual({ anchor: 'alpha' });
    });

    it('getSpawnPosition reads nested transform.position', () => {
      const spawn = {
        template_path: 'foo.toml',
        transform: { position: [100, 0, -50] },
      };
      expect(getSpawnPosition(spawn, [])).toEqual({ x: 100, z: -50 });
    });

    it('getSpawnPosition does not read flat-field position', () => {
      const spawn = { template_path: 'foo.toml', position: [100, 0, -50] };
      expect(getSpawnPosition(spawn, [])).toEqual({ x: 0, z: 0 });
    });

    it('getSpawnPosition reads nested transform.anchor via anchors list', () => {
      const spawn = { template_path: 'foo.toml', transform: { anchor: 'alpha' } };
      const anchors = [{ name: 'alpha', position: [50, 0, 75] }];
      expect(getSpawnPosition(spawn, anchors)).toEqual({ x: 50, z: 75 });
    });

    it('getRelativeInfo reads nested transform.relative_to + offset', () => {
      const spawn = {
        template_path: 'foo.toml',
        transform: { relative_to: 'flagship', offset: [5, 0, -7] },
      };
      expect(getRelativeInfo(spawn)).toEqual({
        parent: 'flagship',
        offset: { x: 5, z: -7 },
      });
    });

    it('getRelativeInfo does not read flat-field relative_to', () => {
      const spawn = {
        template_path: 'foo.toml',
        relative_to: 'flagship',
        offset: [5, 0, -7],
      };
      expect(getRelativeInfo(spawn)).toBeNull();
    });
  });
});

// Rotation and scale mirror the position helpers but apply default-omission:
// rotation [0,0,0] and scale [1,1,1] are the Rust schema defaults and MUST
// NOT be written to the TOML — that keeps round-trips of unmodified spawns
// byte-clean.
describe('toml-utils rotation helpers', () => {
  it('setSpawnRotation writes nested transform.rotation', () => {
    const spawn = { template_path: 'foo.toml' };
    setSpawnRotation(spawn, [0.1, 0.2, 0.3]);
    expect(spawn.transform).toEqual({ rotation: [0.1, 0.2, 0.3] });
  });

  it('setSpawnRotation with default [0,0,0] deletes the field', () => {
    const spawn = { template_path: 'foo.toml', transform: { rotation: [1, 2, 3] } };
    setSpawnRotation(spawn, [0, 0, 0]);
    expect(spawn.transform).toBeUndefined();
  });

  it('setSpawnRotation default leaves other transform keys intact', () => {
    const spawn = { template_path: 'foo.toml', transform: { position: [10, 0, 20], rotation: [1, 0, 0] } };
    setSpawnRotation(spawn, [0, 0, 0]);
    expect(spawn.transform).toEqual({ position: [10, 0, 20] });
  });

  it('getSpawnRotation returns [0,0,0] when missing', () => {
    expect(getSpawnRotation({ template_path: 'foo.toml' })).toEqual([0, 0, 0]);
    expect(getSpawnRotation({ template_path: 'foo.toml', transform: {} })).toEqual([0, 0, 0]);
  });

  it('getSpawnRotation reads nested transform.rotation', () => {
    const spawn = { template_path: 'foo.toml', transform: { rotation: [0.5, 1.0, -0.25] } };
    expect(getSpawnRotation(spawn)).toEqual([0.5, 1.0, -0.25]);
  });

  it('setSpawnRotation co-exists with setSpawnPosition', () => {
    const spawn = { template_path: 'foo.toml' };
    setSpawnPosition(spawn, 10, 20, 'absolute');
    setSpawnRotation(spawn, [0, 1.57, 0]);
    expect(spawn.transform).toEqual({ position: [10, 0, 20], rotation: [0, 1.57, 0] });
  });
});

describe('toml-utils scale helpers', () => {
  it('setSpawnScale writes nested transform.scale', () => {
    const spawn = { template_path: 'foo.toml' };
    setSpawnScale(spawn, [2, 3, 4]);
    expect(spawn.transform).toEqual({ scale: [2, 3, 4] });
  });

  it('setSpawnScale with default [1,1,1] deletes the field', () => {
    const spawn = { template_path: 'foo.toml', transform: { scale: [2, 2, 2] } };
    setSpawnScale(spawn, [1, 1, 1]);
    expect(spawn.transform).toBeUndefined();
  });

  it('setSpawnScale default leaves other transform keys intact', () => {
    const spawn = { template_path: 'foo.toml', transform: { position: [1, 0, 2], scale: [5, 5, 5] } };
    setSpawnScale(spawn, [1, 1, 1]);
    expect(spawn.transform).toEqual({ position: [1, 0, 2] });
  });

  it('getSpawnScale returns [1,1,1] when missing', () => {
    expect(getSpawnScale({ template_path: 'foo.toml' })).toEqual([1, 1, 1]);
    expect(getSpawnScale({ template_path: 'foo.toml', transform: {} })).toEqual([1, 1, 1]);
  });

  it('getSpawnScale reads nested transform.scale', () => {
    const spawn = { template_path: 'foo.toml', transform: { scale: [2, 3, 4] } };
    expect(getSpawnScale(spawn)).toEqual([2, 3, 4]);
  });

  it('rotation and scale co-exist with position and each other', () => {
    const spawn = { template_path: 'foo.toml' };
    setSpawnPosition(spawn, 5, 7, 'absolute');
    setSpawnRotation(spawn, [0, 0.5, 0]);
    setSpawnScale(spawn, [2, 2, 2]);
    expect(spawn.transform).toEqual({
      position: [5, 0, 7],
      rotation: [0, 0.5, 0],
      scale: [2, 2, 2],
    });
  });
});
