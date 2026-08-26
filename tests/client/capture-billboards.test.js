// tests/client/capture-billboards.test.js - issue #1245 capture currency.

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { parse as parseToml } from 'smol-toml';
import {
  CAPTURE_MANIFEST_PATH,
  CAPTURE_RECIPE_VERSION,
  canonicalBase,
  canonicalCaptureParams,
  captureCommand,
  captureManifestEntry,
  collectCaptureTargets,
  compareCaptureManifest,
  formatCaptureManifest,
  observeCaptureTargets,
  parseCaptureManifest,
  pngDimensions,
  readCaptureSidecars,
  sha256,
  updateCaptureManifestEntries,
} from '../../scripts/capture-billboards.mjs';

const ROOT = process.cwd();
const HASH_A = sha256('a');
const HASH_B = sha256('b');
const HASH_C = sha256('c');

const sidecar = (path, text) => ({ path, doc: parseToml(text) });

function shipSidecar({
  path = 'assets/models/ship.model.toml',
  source = 'assets/models/ship.glb',
  output = 'assets/models/ship_lod3.png',
  views = 8,
  resolution = 256,
  pitch = 20,
  scale = '[2.0, 1.0, 1.0]',
  baseScale = '[1.0, 1.0, 1.0]',
} = {}) {
  return sidecar(
    path,
    `[base]\noffset = [0.0, -0.2, 0.1]\nrotation = [0.0, 3.14, 0.0]\nscale = ${baseScale}\n\n` +
      `[[lod]]\nbillboard = "${output}"\nscale = ${scale}\n\n` +
      `[lod.capture]\nsource = "${source}"\nyaw_views = ${views}\nresolution = ${resolution}\npitch = ${pitch}.0\n`,
  );
}

function fixture() {
  const { targets, errors } = collectCaptureTargets([shipSidecar()]);
  expect(errors).toEqual([]);
  const target = targets[0];
  const observed = {
    target,
    sourceSha256: HASH_A,
    outputSha256: HASH_B,
    outputBytes: 42,
    outputDimensions: [2048, 256],
  };
  return { target, observed, entry: captureManifestEntry(target, observed) };
}

describe('collectCaptureTargets', () => {
  it('collects the billboard path, capture recipe, source base, and declarer', () => {
    const { targets, errors } = collectCaptureTargets([shipSidecar()]);
    expect(errors).toEqual([]);
    expect(targets).toEqual([
      expect.objectContaining({
        output: 'assets/models/ship_lod3.png',
        source: 'assets/models/ship.glb',
        params: { yawViews: 8, resolution: 256, pitch: 20 },
        baseSource: 'assets/models/ship.model.toml',
        recipeVersion: CAPTURE_RECIPE_VERSION,
        declaredBy: ['assets/models/ship.model.toml'],
      }),
    ]);
    expect(targets[0].base).toEqual({
      offset: [0, -0.2, 0.1],
      rotation: [0, 3.14, 0],
      scale: [1, 1, 1],
    });
    expect(targets[0].baseSha256).toBe(sha256(canonicalBase(targets[0].base)));
  });

  it('deduplicates shared atlases while allowing variant base and billboard scales', () => {
    const shared = (variant, baseScale, quadScale) =>
      shipSidecar({
        path: `assets/models/rock.${variant}.toml`,
        source: 'assets/models/rock.glb',
        output: 'assets/models/rock_lod3.png',
        baseScale,
        scale: quadScale,
      });
    const { targets, errors } = collectCaptureTargets([
      shared('small', '[2.0, 2.0, 2.0]', '[4.0, 3.0, 1.0]'),
      shared('large', '[8.0, 8.0, 8.0]', '[16.0, 12.0, 1.0]'),
    ]);
    expect(errors).toEqual([]);
    expect(targets).toHaveLength(1);
    expect(targets[0]).toMatchObject({
      baseSource: 'identity',
      base: { offset: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1] },
      declaredBy: ['assets/models/rock.large.toml', 'assets/models/rock.small.toml'],
    });
  });

  it('rejects conflicting recipes for one shared output', () => {
    const { errors } = collectCaptureTargets([
      shipSidecar({ path: 'assets/models/a.model.toml', output: 'assets/models/shared.png' }),
      shipSidecar({
        path: 'assets/models/b.model.toml',
        output: 'assets/models/shared.png',
        views: 12,
      }),
    ]);
    expect(errors.join('\n')).toContain('declared differently');
  });

  it.each([
    ['yaw_views = 8', 'yaw_views = 0', 'yaw_views must be a positive whole number'],
    ['resolution = 256', 'resolution = 0', 'resolution must be a positive whole number'],
    ['pitch = 20.0', 'pitch = "up"', 'pitch must be a finite number'],
  ])('rejects malformed capture fields: %s', (from, replacement, expected) => {
    const base = shipSidecar();
    const text = readFixtureText(base).replace(from, replacement);
    const { errors } = collectCaptureTargets([sidecar(base.path, text)]);
    expect(errors.join('\n')).toContain(expected);
  });

  it.each([
    ['C:\\outside\\atlas.png', 'absolute paths and .. are forbidden'],
    ['../outside.png', 'absolute paths and .. are forbidden'],
  ])('rejects capture output path %s', (output, expected) => {
    const value = shipSidecar();
    value.doc.lod[0].billboard = output;
    const { errors } = collectCaptureTargets([value]);
    expect(errors.join('\n')).toContain(expected);
  });

  it('canonicalises backslashes in otherwise-safe paths', () => {
    const value = shipSidecar();
    value.doc.lod[0].capture.source = 'assets\\models\\ship.glb';
    value.doc.lod[0].billboard = 'assets\\models\\ship_lod3.png';
    const { targets, errors } = collectCaptureTargets([value]);
    expect(errors).toEqual([]);
    expect(targets[0].source).toBe('assets/models/ship.glb');
    expect(targets[0].output).toBe('assets/models/ship_lod3.png');
  });
});

function readFixtureText(value) {
  // smol-toml has no serializer; this fixture shape is intentionally tiny.
  const level = value.doc.lod[0];
  const base = value.doc.base;
  return `[base]\noffset = [${base.offset.join(', ')}]\nrotation = [${base.rotation.join(', ')}]\nscale = [${base.scale.join(', ')}]\n\n` +
    `[[lod]]\nbillboard = "${level.billboard}"\nscale = [${level.scale.join(', ')}]\n\n` +
    `[lod.capture]\nsource = "${level.capture.source}"\nyaw_views = ${level.capture.yaw_views}\n` +
    `resolution = ${level.capture.resolution}\npitch = ${level.capture.pitch}.0\n`;
}

describe('captureCommand', () => {
  it('forwards every authored capture parameter to the native renderer', () => {
    const { target } = fixture();
    expect(captureCommand(target, 'capture-billboard')).toEqual({
      file: 'capture-billboard',
      args: [
        'assets/models/ship.glb',
        'assets/models/ship_lod3.png',
        '--views',
        '8',
        '--resolution',
        '256',
        '--pitch',
        '20',
      ],
    });
  });
});

describe('capture manifest rendering and validation', () => {
  it('round-trips to one stable document with explicit base and recipe version', () => {
    const { entry } = fixture();
    const text = formatCaptureManifest([entry]);
    expect(text).toContain('recipe_version = 1');
    expect(text).toContain(
      'base = { offset = [0.0, -0.2, 0.1], rotation = [0.0, 3.14, 0.0], scale = [1.0, 1.0, 1.0] }',
    );
    expect(text).toContain(`base_sha256 = "${entry.base_sha256}"`);
    expect(formatCaptureManifest(parseCaptureManifest(text))).toBe(text);
  });

  it.each([
    ['version = 99', 'unsupported capture manifest version'],
    ['yaw_views = 8', 'yaw_views = 0', 'positive whole number'],
    ['output_bytes = 42', 'output_bytes = -1', 'non-negative whole number'],
    ['path = "assets/models/ship_lod3.png"', 'path = "../escape.png"', 'repo-relative'],
  ])('rejects malformed manifest data', (...parts) => {
    const { entry } = fixture();
    let text = formatCaptureManifest([entry]);
    let expected;
    if (parts.length === 2) {
      [text, expected] = parts;
    } else {
      const [from, to, message] = parts;
      text = text.replace(from, to);
      expected = message;
    }
    expect(() => parseCaptureManifest(text)).toThrow(expected);
  });
});

describe('compareCaptureManifest', () => {
  it('accepts matching provenance and catches every independently drifting input', () => {
    const { target, observed, entry } = fixture();
    expect(compareCaptureManifest([entry], [observed])).toEqual([]);

    const cases = [
      [{ ...observed, sourceSha256: HASH_C }, 'source-changed'],
      [{ ...observed, outputSha256: HASH_C }, 'output-changed'],
      [{ ...observed, outputBytes: 99 }, 'output-changed'],
      [{ ...observed, outputDimensions: [1024, 256] }, 'dimensions-changed'],
      [{ ...observed, target: { ...target, params: { ...target.params, yawViews: 12 } } }, 'params-changed'],
      [{ ...observed, target: { ...target, recipeVersion: 2 } }, 'recipe-changed'],
      [{ ...observed, target: { ...target, baseSha256: HASH_C } }, 'base-changed'],
      [{ ...observed, target: { ...target, declaredBy: [...target.declaredBy, 'assets/models/other.toml'] } }, 'declarations-changed'],
    ];
    for (const [changed, kind] of cases) {
      expect(compareCaptureManifest([entry], [changed]).map((finding) => finding.kind)).toContain(kind);
    }
  });

  it('reports new, missing, repointed, and orphaned files', () => {
    const { target, observed, entry } = fixture();
    expect(compareCaptureManifest([], [observed]).map((finding) => finding.kind)).toEqual(['unrecorded']);
    expect(compareCaptureManifest([entry], []).map((finding) => finding.kind)).toEqual(['orphaned']);
    expect(compareCaptureManifest([entry], [{ ...observed, outputSha256: null }])[0].kind).toBe('missing-output');
    expect(compareCaptureManifest([entry], [{ ...observed, sourceSha256: null }])[0].kind).toBe('missing-source');
    expect(
      compareCaptureManifest([entry], [{ ...observed, target: { ...target, source: 'assets/models/new.glb' } }])
        .map((finding) => finding.kind),
    ).toContain('source-repointed');
  });
});

describe('updateCaptureManifestEntries', () => {
  it('updates a filtered output, preserves unrelated declared records, and prunes only true orphans', () => {
    const first = fixture();
    const secondTarget = { ...first.target, output: 'assets/models/other.png' };
    const second = captureManifestEntry(secondTarget, {
      sourceSha256: HASH_A,
      outputSha256: HASH_B,
      outputBytes: 9,
    });
    const orphan = { ...second, path: 'assets/models/orphan.png' };
    const refreshed = { ...first.observed, outputSha256: HASH_C, outputBytes: 43 };
    const updated = updateCaptureManifestEntries(
      [first.target, secondTarget],
      [first.entry, second, orphan],
      [refreshed],
    );
    expect(updated.map((entry) => entry.path)).toEqual([
      'assets/models/other.png',
      'assets/models/ship_lod3.png',
    ]);
    expect(updated.find((entry) => entry.path === second.path)).toEqual(second);
    expect(updated.find((entry) => entry.path === first.entry.path).output_sha256).toBe(HASH_C);
  });
});

describe('PNG dimensions', () => {
  it('reads the IHDR dimensions without decoding image pixels', () => {
    const bytes = Buffer.alloc(24);
    Buffer.from('89504e470d0a1a0a', 'hex').copy(bytes, 0);
    bytes.write('IHDR', 12, 'ascii');
    bytes.writeUInt32BE(2048, 16);
    bytes.writeUInt32BE(256, 20);
    expect(pngDimensions(bytes)).toEqual([2048, 256]);
    expect(pngDimensions(Buffer.from('not a png'))).toBeNull();
  });
});

describe('shipped Cruiser capture records', () => {
  async function shipped() {
    const { targets, errors } = collectCaptureTargets(await readCaptureSidecars(ROOT));
    expect(errors).toEqual([]);
    const cruisers = targets.filter((target) => /(?:alliance|dynasty)_cruiser_lod3\.png$/.test(target.output));
    expect(cruisers.map((target) => target.output)).toEqual([
      'assets/models/alliance_cruiser_lod3.png',
      'assets/models/dynasty_cruiser_lod3.png',
    ]);
    const paths = new Set(cruisers.map((target) => target.output));
    const entries = parseCaptureManifest(readFileSync(CAPTURE_MANIFEST_PATH, 'utf8')).filter((entry) => paths.has(entry.path));
    return { entries, cruisers };
  }

  it('keeps both committed Cruiser atlases current', async () => {
    const { entries, cruisers } = await shipped();
    expect(compareCaptureManifest(entries, observeCaptureTargets(ROOT, cruisers))).toEqual([]);
  });

  it('makes a doctored Alliance Cruiser yaw count fail the same check CI runs', async () => {
    const sidecars = await readCaptureSidecars(ROOT);
    const cruiser = sidecars.find((item) => item.path === 'assets/models/alliance_cruiser.model.toml');
    cruiser.doc.lod.find((level) => level.capture).capture.yaw_views = 12;
    const { targets, errors } = collectCaptureTargets(sidecars);
    expect(errors).toEqual([]);
    const target = targets.find((item) => item.output === 'assets/models/alliance_cruiser_lod3.png');
    const entries = parseCaptureManifest(readFileSync(CAPTURE_MANIFEST_PATH, 'utf8')).filter(
      (entry) => entry.path === target.output,
    );
    expect(compareCaptureManifest(entries, observeCaptureTargets(ROOT, [target])).map((finding) => finding.kind))
      .toContain('params-changed');
  });

  it('--check succeeds even when the configured capture binary is missing', () => {
    const result = spawnSync(
      process.execPath,
      ['scripts/capture-billboards.mjs', '--check', 'alliance_cruiser'],
      {
        cwd: ROOT,
        encoding: 'utf8',
        env: { ...process.env, PHOENIX_CAPTURE_BIN: 'target/definitely-missing/capture-billboard' },
      },
    );
    expect(result.status, result.stderr).toBe(0);
    expect(result.stderr).toContain('1 capture output(s) up to date');
  });
});

describe('canonicalCaptureParams', () => {
  it('names every field, including a removed one', () => {
    expect(canonicalCaptureParams({ yawViews: 8, resolution: 256, pitch: 20 }))
      .toBe('yaw_views=8 resolution=256 pitch=20');
    expect(canonicalCaptureParams({ yawViews: 8, resolution: null, pitch: 20 }))
      .not.toBe('yaw_views=8 resolution=256 pitch=20');
  });
});
