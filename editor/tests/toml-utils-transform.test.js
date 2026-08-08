import { describe, it, expect } from 'vitest';
import {
  setSpawnPosition,
  getSpawnPosition,
  getRelativeInfo,
  setSpawnRotation,
  getSpawnRotation,
  setSpawnScale,
  getSpawnScale,
  getSpawnName,
  getSpawnReference,
  matchesSpawnReference,
  getRelativeToCandidates,
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
// Issue #969: an unresolvable `relative_to` now blocks the whole world instead
// of costing one misplaced entity, so the editor must not be able to author
// one. Everything the Relative To picker offers has to be something
// `build_named_entity_positions` will actually put in the runtime table.
describe('toml-utils relative_to parent candidates', () => {
  const layerOf = (...entity) => ({ isMap: true, toml: { entity } });

  it('getSpawnReference returns the authored identifier, never the display fallback', () => {
    expect(getSpawnReference({ name: 'beacon', id: 'b1' })).toBe('beacon');
    expect(getSpawnReference({ id: 'nebula-1' })).toBe('nebula-1');
    expect(getSpawnReference({ template_path: 'foo.toml' })).toBeNull();
    // The display helper still has its fallback; the two must not be confused.
    expect(getSpawnName({ template_path: 'foo.toml' })).toBe('unnamed');
  });

  it('drops spawns with neither name nor id — the shipped worlds have several', () => {
    const anonymous = { template_path: 'assets/entities/star_sun.toml' };
    const named = { template_path: 'assets/entities/planet_earth.toml', id: 'earth' };
    const layer = layerOf(anonymous, named);
    expect(getRelativeToCandidates(layer)).toEqual([named]);
  });

  it('drops spawns that are themselves relative_to-positioned — chains are unsupported', () => {
    const planet = { template_path: 'planet.toml', id: 'planet', transform: { position: [1, 0, 0] } };
    const moon = { template_path: 'moon.toml', id: 'moon', transform: { relative_to: 'planet', offset: [1, 0, 0] } };
    const layer = layerOf(planet, moon);
    expect(getRelativeToCandidates(layer)).toEqual([planet]);
  });

  it('drops the subject itself — nothing can be positioned relative to itself', () => {
    const planet = { template_path: 'planet.toml', id: 'planet', transform: { position: [1, 0, 0] } };
    const station = { template_path: 'station.toml', id: 'station', transform: { position: [2, 0, 0] } };
    const layer = layerOf(planet, station);
    expect(getRelativeToCandidates(layer, station)).toEqual([planet]);
  });

  it('never reaches beyond the given layer — the runtime table is per world file', () => {
    const here = { template_path: 'here.toml', id: 'here', transform: { position: [0, 0, 0] } };
    const elsewhere = { template_path: 'there.toml', id: 'there', transform: { position: [9, 0, 9] } };
    // Two separate layers; only the one passed in contributes.
    expect(getRelativeToCandidates(layerOf(here))).toEqual([here]);
    expect(getRelativeToCandidates(layerOf(elsewhere))).toEqual([elsewhere]);
  });

  it('offers a landmark by whichever identifier the runtime resolves', () => {
    // combat_test.toml's shape: short `id` for authors, strings.csv key as `name`.
    const gasGiant = {
      template_path: 'planet.toml',
      id: 'gas-giant',
      name: 'world.entity.gas_giant.name',
      transform: { position: [-1200, 0, 300] },
    };
    const [candidate] = getRelativeToCandidates(layerOf(gasGiant));
    expect(getSpawnReference(candidate)).toBe('world.entity.gas_giant.name');
  });

  // Writing a reference picks ONE identifier; recognising one already authored
  // must accept EITHER, because the runtime table is keyed by both.
  it('matchesSpawnReference accepts id or name, not just the one getSpawnReference picks', () => {
    const gasGiant = { id: 'gas-giant', name: 'world.entity.gas_giant.name' };
    expect(matchesSpawnReference(gasGiant, 'gas-giant')).toBe(true);
    expect(matchesSpawnReference(gasGiant, 'world.entity.gas_giant.name')).toBe(true);
    // The identifier getSpawnReference would NOT have chosen still matches.
    expect(getSpawnReference(gasGiant)).not.toBe('gas-giant');
    expect(matchesSpawnReference(gasGiant, 'earth')).toBe(false);
  });

  it('matchesSpawnReference never matches an absent reference', () => {
    // An anonymous spawn must not compare equal to "no reference" by way of
    // `undefined === undefined`.
    const anonymous = { template_path: 'star.toml' };
    expect(matchesSpawnReference(anonymous, undefined)).toBe(false);
    expect(matchesSpawnReference(anonymous, null)).toBe(false);
    expect(matchesSpawnReference({ id: 'x' }, null)).toBe(false);
  });
});

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
