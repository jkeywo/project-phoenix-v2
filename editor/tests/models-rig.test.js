import { describe, it, expect } from 'vitest';
import { parse } from 'smol-toml';
import {
  DEFAULT_VARIANT,
  FORWARD,
  toVec3,
  vec3Length,
  normalizeDirection,
  defaultRig,
  computeExtents,
  addMarker,
  updateMarker,
  removeMarker,
  renameMarker,
  parseRigToml,
  buildRigToml,
  buildSidecarName,
  parseSidecarName,
  glbStem,
  groupModelFiles,
  validateVariantName,
} from '../models-rig.js';

describe('models-rig vec3 helpers', () => {
  it('toVec3 coerces arrays and falls back component-wise', () => {
    expect(toVec3([1, 2, 3])).toEqual([1, 2, 3]);
    expect(toVec3(['4', '5', '6'])).toEqual([4, 5, 6]);
    expect(toVec3([1, NaN, 3], [7, 8, 9])).toEqual([1, 8, 3]);
    expect(toVec3(undefined, [1, 1, 1])).toEqual([1, 1, 1]);
    expect(toVec3('nope', [0, 0, 0])).toEqual([0, 0, 0]);
  });

  it('vec3Length computes Euclidean length', () => {
    expect(vec3Length([3, 4, 0])).toBe(5);
    expect(vec3Length([0, 0, 0])).toBe(0);
  });

  it('normalizeDirection returns a unit vector', () => {
    const n = normalizeDirection([0, 0, -2]);
    expect(n).toEqual([0, 0, -1]);
    const d = normalizeDirection([3, 0, 4]);
    expect(vec3Length(d)).toBeCloseTo(1, 10);
    expect(d[0]).toBeCloseTo(0.6, 10);
    expect(d[2]).toBeCloseTo(0.8, 10);
  });

  it('normalizeDirection falls back to forward for zero/invalid', () => {
    expect(normalizeDirection([0, 0, 0])).toEqual([...FORWARD]);
    expect(normalizeDirection(undefined)).toEqual([...FORWARD]);
    expect(normalizeDirection([NaN, NaN, NaN])).toEqual([...FORWARD]);
  });
});

describe('defaultRig', () => {
  it('produces an identity rig with no markers', () => {
    const rig = defaultRig();
    expect(rig.base.offset).toEqual([0, 0, 0]);
    expect(rig.base.rotation).toEqual([0, 0, 0]);
    expect(rig.base.scale).toEqual([1, 1, 1]);
    expect(rig.extents.size).toEqual([0, 0, 0]);
    expect(rig.markers).toEqual({});
  });

  it('returns independent copies (no shared references)', () => {
    const a = defaultRig();
    const b = defaultRig();
    a.base.offset[0] = 99;
    a.markers.x = { position: [0, 0, 0], direction: [...FORWARD] };
    expect(b.base.offset[0]).toBe(0);
    expect(b.markers).toEqual({});
  });
});

describe('computeExtents', () => {
  it('computes size as max - min', () => {
    const e = computeExtents({ min: [-4, -1.2, -6], max: [4, 1.2, 6] });
    expect(e.min).toEqual([-4, -1.2, -6]);
    expect(e.max).toEqual([4, 1.2, 6]);
    expect(e.size).toEqual([8, 2.4, 12]);
  });

  it('handles missing input as zeroes', () => {
    expect(computeExtents()).toEqual({ min: [0, 0, 0], max: [0, 0, 0], size: [0, 0, 0] });
  });
});

describe('marker CRUD', () => {
  it('addMarker normalizes direction and coerces position', () => {
    const rig = defaultRig();
    addMarker(rig, 'fore_emitter', { position: [0, 0, -6], direction: [0, 0, -3] });
    expect(rig.markers.fore_emitter.position).toEqual([0, 0, -6]);
    expect(rig.markers.fore_emitter.direction).toEqual([0, 0, -1]);
  });

  it('addMarker defaults direction to forward when omitted', () => {
    const rig = defaultRig();
    addMarker(rig, 'm', { position: [1, 2, 3] });
    expect(rig.markers.m.direction).toEqual([...FORWARD]);
  });

  it('addMarker trims names and rejects empty', () => {
    const rig = defaultRig();
    addMarker(rig, '  aft  ', {});
    expect(rig.markers.aft).toBeTruthy();
    expect(() => addMarker(rig, '   ', {})).toThrow();
    expect(() => addMarker(rig, '', {})).toThrow();
  });

  it('updateMarker changes position/direction in place', () => {
    const rig = defaultRig();
    addMarker(rig, 'm', { position: [0, 0, 0] });
    updateMarker(rig, 'm', { position: [5, 5, 5] });
    expect(rig.markers.m.position).toEqual([5, 5, 5]);
    updateMarker(rig, 'm', { direction: [10, 0, 0] });
    expect(rig.markers.m.direction).toEqual([1, 0, 0]);
    // position untouched when only direction passed
    expect(rig.markers.m.position).toEqual([5, 5, 5]);
  });

  it('updateMarker is a no-op for unknown markers', () => {
    const rig = defaultRig();
    updateMarker(rig, 'ghost', { position: [1, 1, 1] });
    expect(rig.markers.ghost).toBeUndefined();
  });

  it('removeMarker deletes by name', () => {
    const rig = defaultRig();
    addMarker(rig, 'm', {});
    removeMarker(rig, 'm');
    expect(rig.markers.m).toBeUndefined();
  });

  it('renameMarker preserves data and ordering', () => {
    const rig = defaultRig();
    addMarker(rig, 'a', { position: [1, 0, 0] });
    addMarker(rig, 'b', { position: [2, 0, 0] });
    addMarker(rig, 'c', { position: [3, 0, 0] });
    renameMarker(rig, 'b', 'bee');
    expect(Object.keys(rig.markers)).toEqual(['a', 'bee', 'c']);
    expect(rig.markers.bee.position).toEqual([2, 0, 0]);
  });

  it('renameMarker throws on collision', () => {
    const rig = defaultRig();
    addMarker(rig, 'a', {});
    addMarker(rig, 'b', {});
    expect(() => renameMarker(rig, 'a', 'b')).toThrow();
  });

  it('renameMarker is a no-op when from === to', () => {
    const rig = defaultRig();
    addMarker(rig, 'a', { position: [1, 1, 1] });
    renameMarker(rig, 'a', 'a');
    expect(rig.markers.a.position).toEqual([1, 1, 1]);
  });
});

describe('TOML round-trip', () => {
  it('builds a sidecar matching the agreed schema', () => {
    const rig = defaultRig();
    rig.base.offset = [0, 0, 0];
    rig.extents = computeExtents({ min: [-4, -1.2, -6], max: [4, 1.2, 6] });
    addMarker(rig, 'fore_emitter', { position: [0, 0, -6], direction: [0, 0, -1] });

    const toml = buildRigToml(rig);
    const parsed = parse(toml);

    expect(parsed.base.offset).toEqual([0, 0, 0]);
    expect(parsed.base.rotation).toEqual([0, 0, 0]);
    expect(parsed.base.scale).toEqual([1, 1, 1]);
    expect(parsed.extents.min).toEqual([-4, -1.2, -6]);
    expect(parsed.extents.max).toEqual([4, 1.2, 6]);
    expect(parsed.extents.size).toEqual([8, 2.4, 12]);
    expect(parsed.markers.fore_emitter.position).toEqual([0, 0, -6]);
    expect(parsed.markers.fore_emitter.direction).toEqual([0, 0, -1]);
  });

  it('round-trips through build -> parse with markers (position + direction)', () => {
    const rig = defaultRig();
    rig.base.offset = [1, 2, 3];
    rig.base.rotation = [0.1, 0.2, 0.3];
    rig.base.scale = [2, 2, 2];
    rig.extents = computeExtents({ min: [-1, -2, -3], max: [4, 5, 6] });
    addMarker(rig, 'fore', { position: [0, 0, -6], direction: [0, 0, -1] });
    addMarker(rig, 'aft', { position: [0, 0, 6], direction: [0, 0, 1] });
    addMarker(rig, 'starboard', { position: [4, 0, 0], direction: [1, 0, 0] });

    const reparsed = parseRigToml(buildRigToml(rig));

    expect(reparsed.base).toEqual(rig.base);
    expect(reparsed.extents).toEqual(rig.extents);
    expect(Object.keys(reparsed.markers)).toEqual(['fore', 'aft', 'starboard']);
    expect(reparsed.markers.fore.position).toEqual([0, 0, -6]);
    expect(reparsed.markers.fore.direction).toEqual([0, 0, -1]);
    expect(reparsed.markers.starboard.direction).toEqual([1, 0, 0]);
  });

  it('parseRigToml fills defaults for missing sections', () => {
    const rig = parseRigToml('[base]\noffset = [1, 0, 0]\n');
    expect(rig.base.offset).toEqual([1, 0, 0]);
    expect(rig.base.scale).toEqual([1, 1, 1]);
    expect(rig.extents.size).toEqual([0, 0, 0]);
    expect(rig.markers).toEqual({});
  });

  it('parseRigToml re-normalizes hand-edited marker directions', () => {
    const rig = parseRigToml(
      '[markers]\nm = { position = [0,0,0], direction = [0,0,-5] }\n',
    );
    expect(rig.markers.m.direction).toEqual([0, 0, -1]);
  });

  it('parseRigToml recomputes size from stored min/max', () => {
    const rig = parseRigToml(
      '[extents]\nmin = [-1,-1,-1]\nmax = [1,1,1]\nsize = [999,999,999]\n',
    );
    expect(rig.extents.size).toEqual([2, 2, 2]);
  });

  it('emits a flat markers map keyed by name', () => {
    const rig = defaultRig();
    addMarker(rig, 'm', { position: [1, 2, 3], direction: [0, 0, -1] });
    const toml = buildRigToml(rig);
    // smol-toml expands an object value to a [markers.<name>] sub-table,
    // which is semantically the same flat map (no arrays). Assert on the
    // parsed structure rather than the exact surface syntax.
    const parsed = parse(toml);
    expect(parsed.markers.m.position).toEqual([1, 2, 3]);
    expect(parsed.markers.m.direction).toEqual([0, 0, -1]);
  });

  // The editor round-trip was dropping [[lod]] and [[target_points]]
  // entirely, because parseRigToml never read them and buildRigToml never
  // wrote them back out. Editing a marker (the normal Models Mode workflow)
  // and saving must not destroy either section.
  it('carries [[lod]] and [[target_points]] through a load -> edit -> save round-trip', () => {
    const sidecar = `[base]
offset = [ 0, -2.44, 0 ]
rotation = [ 0, 0, 0 ]
scale = [ 4.2, 4.2, 4.2 ]

[extents]
min = [ -4, -2.44, -3.5 ]
max = [ 4, 2.44, 3.5 ]
size = [ 8, 4.88, 7 ]

[[target_points]]
position = [ 0.5, -0.1, 0 ]

[[target_points]]
position = [ -0.25, -0.1, 0.25 ]

[markers.fore_emitter]
position = [ 0, 0, -6 ]
direction = [ 0, 0, -1 ]

[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 100.0
model = "assets/models/rock_lod1.glb"

[[lod]]
shape = "sphere"
`;

    const rig = parseRigToml(sidecar);
    expect(rig.target_points).toEqual([
      { position: [0.5, -0.1, 0] },
      { position: [-0.25, -0.1, 0.25] },
    ]);
    expect(rig.lod).toEqual([
      { max_distance: 50.0, model: 'assets/models/rock.glb' },
      { max_distance: 100.0, model: 'assets/models/rock_lod1.glb' },
      { shape: 'sphere' },
    ]);

    // The normal Models Mode edit: move an existing marker.
    updateMarker(rig, 'fore_emitter', { position: [0, 0, -7] });

    const rebuilt = buildRigToml(rig);
    const reparsed = parseRigToml(rebuilt);

    expect(reparsed.markers.fore_emitter.position).toEqual([0, 0, -7]);
    expect(reparsed.target_points).toEqual(rig.target_points);
    expect(reparsed.lod).toEqual(rig.lod);

    // And the raw TOML text actually contains both sections — not just the
    // parsed shape (a build that silently produced markers-only text but
    // happened to leave `rig.lod` populated in memory would still fail the
    // NEXT save; assert on the bytes that hit disk).
    const parsedRaw = parse(rebuilt);
    expect(parsedRaw.lod).toHaveLength(3);
    expect(parsedRaw.target_points).toHaveLength(2);
  });

  it('a rig with no [[lod]] / [[target_points]] serializes without those sections', () => {
    const rig = defaultRig();
    addMarker(rig, 'm', { position: [1, 2, 3] });
    const toml = buildRigToml(rig);
    expect(toml).not.toMatch(/\[\[lod\]\]/);
    expect(toml).not.toMatch(/\[\[target_points\]\]/);
    const parsed = parse(toml);
    expect(parsed.lod).toBeUndefined();
    expect(parsed.target_points).toBeUndefined();
  });
});

describe('variant filename helpers', () => {
  it('buildSidecarName builds default and named variants', () => {
    expect(buildSidecarName('asteroid_large')).toBe('asteroid_large.model.toml');
    expect(buildSidecarName('asteroid_large', 'model')).toBe('asteroid_large.model.toml');
    expect(buildSidecarName('asteroid_large', 'weathered')).toBe('asteroid_large.weathered.toml');
    expect(buildSidecarName('x', '  ')).toBe('x.model.toml');
  });

  it('parseSidecarName splits stem and variant', () => {
    expect(parseSidecarName('asteroid_large.model.toml')).toEqual({
      stem: 'asteroid_large',
      variant: 'model',
    });
    expect(parseSidecarName('asteroid_large.weathered.toml')).toEqual({
      stem: 'asteroid_large',
      variant: 'weathered',
    });
  });

  it('parseSidecarName rejects non-sidecars', () => {
    expect(parseSidecarName('asteroid_large.glb')).toBeNull();
    expect(parseSidecarName('nodots.toml')).toBeNull();
    expect(parseSidecarName('foo.bar')).toBeNull();
    expect(parseSidecarName(null)).toBeNull();
  });

  it('round-trips build <-> parse for variant names', () => {
    const name = buildSidecarName('dynasty_cruiser', 'damaged');
    expect(parseSidecarName(name)).toEqual({ stem: 'dynasty_cruiser', variant: 'damaged' });
  });

  it('glbStem extracts the stem from a .glb name', () => {
    expect(glbStem('alliance_cruiser.glb')).toBe('alliance_cruiser');
    expect(glbStem('Mixed.GLB')).toBe('Mixed');
    expect(glbStem('foo.toml')).toBeNull();
  });
});

describe('groupModelFiles', () => {
  it('pairs glbs with their sidecar variants, default first', () => {
    const entries = [
      { name: 'asteroid_large.glb', kind: 'file' },
      { name: 'asteroid_large.weathered.toml', kind: 'file' },
      { name: 'asteroid_large.model.toml', kind: 'file' },
      { name: 'dynasty_cruiser.glb', kind: 'file' },
      { name: 'README.md', kind: 'file' },
      { name: 'subdir', kind: 'directory' },
    ];
    const groups = groupModelFiles(entries);
    expect(groups).toEqual([
      {
        stem: 'asteroid_large',
        glb: 'asteroid_large.glb',
        variants: ['model', 'weathered'],
      },
      {
        stem: 'dynasty_cruiser',
        glb: 'dynasty_cruiser.glb',
        variants: [],
      },
    ]);
  });

  it('ignores sidecars with no matching glb is still grouped only by glb', () => {
    const entries = [
      { name: 'orphan.model.toml', kind: 'file' },
      { name: 'ship.glb', kind: 'file' },
    ];
    const groups = groupModelFiles(entries);
    expect(groups.map((g) => g.stem)).toEqual(['ship']);
  });

  it('handles empty / nullish input', () => {
    expect(groupModelFiles([])).toEqual([]);
    expect(groupModelFiles(null)).toEqual([]);
  });
});

describe('validateVariantName', () => {
  it('accepts a fresh, non-reserved name', () => {
    expect(validateVariantName('weathered', ['model'])).toEqual({
      ok: true,
      variant: 'weathered',
    });
  });

  it('trims surrounding whitespace', () => {
    expect(validateVariantName('  damaged  ', [])).toEqual({
      ok: true,
      variant: 'damaged',
    });
  });

  it('rejects empty / whitespace-only names', () => {
    expect(validateVariantName('', [])).toEqual({ ok: false, reason: 'empty' });
    expect(validateVariantName('   ', [])).toEqual({ ok: false, reason: 'empty' });
    expect(validateVariantName(undefined, [])).toEqual({ ok: false, reason: 'empty' });
  });

  it('rejects the reserved default name "model"', () => {
    expect(validateVariantName('model', [])).toEqual({ ok: false, reason: 'reserved' });
    expect(validateVariantName('  model  ', [])).toEqual({ ok: false, reason: 'reserved' });
  });

  it('flags an existing variant for overwrite confirmation', () => {
    expect(validateVariantName('weathered', ['model', 'weathered'])).toEqual({
      ok: true,
      variant: 'weathered',
      requiresConfirm: true,
    });
  });

  it('accepts a Set of existing variants', () => {
    const r = validateVariantName('weathered', new Set(['weathered']));
    expect(r).toEqual({ ok: true, variant: 'weathered', requiresConfirm: true });
  });
});

describe('non-identity base rig marker round-trip', () => {
  it('serializes marker position/direction verbatim in post-base-rig space', () => {
    // Markers are defined in POST-base-rig space, so the sidecar stores their
    // coordinates as-is regardless of the base transform. A non-identity base
    // must NOT mutate the serialized marker values.
    const rig = defaultRig();
    rig.base.offset = [10, -5, 3];
    rig.base.rotation = [Math.PI / 2, 0.3, -0.7]; // XYZ-order euler radians
    rig.base.scale = [2, 0.5, 3];
    rig.extents = computeExtents({ min: [-1, -2, -3], max: [4, 5, 6] });
    addMarker(rig, 'fore_emitter', { position: [0, 0, -6], direction: [0, 0, -1] });
    addMarker(rig, 'starboard', { position: [4, 0, 0], direction: [1, 0, 0] });

    const reparsed = parseRigToml(buildRigToml(rig));

    // Base survives the round-trip.
    expect(reparsed.base.offset).toEqual([10, -5, 3]);
    expect(reparsed.base.rotation).toEqual([Math.PI / 2, 0.3, -0.7]);
    expect(reparsed.base.scale).toEqual([2, 0.5, 3]);
    // Marker coords are unchanged by the non-identity base.
    expect(reparsed.markers.fore_emitter.position).toEqual([0, 0, -6]);
    expect(reparsed.markers.fore_emitter.direction).toEqual([0, 0, -1]);
    expect(reparsed.markers.starboard.position).toEqual([4, 0, 0]);
    expect(reparsed.markers.starboard.direction).toEqual([1, 0, 0]);
  });
});
