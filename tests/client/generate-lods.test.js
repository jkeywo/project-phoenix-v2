import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { parse as parseToml } from 'smol-toml';
import {
  collectTargets,
  canonicalParams,
  matchesFilter,
  planSteps,
  remeshStep,
  remeshPath,
  remeshTextureCap,
  blenderCandidates,
  sha256,
  manifestEntry,
  formatManifest,
  parseManifest,
  compareManifest,
  sizeReport,
  blockedGrowth,
  describeBlockedGrowth,
} from '../../scripts/generate-lods.mjs';

// Everything here is a fabricated sidecar string and a fabricated byte string.
// No .glb is read, no gltf-transform runs, no Blender is looked for — the
// generator's decisions are pure functions over parsed TOML, and those are what
// this file pins down. (Same contract as balance-runs.test.js.)

function sidecar(path, text) {
  return { path, doc: parseToml(text) };
}

// These are the two shipped cruiser hull models: Alliance Cruiser is the
// player cruiser and Dynasty Cruiser is used by the Harrow cruiser/patrol
// templates. Keep this at the generated-GLB contract level; billboards have
// their separate capture pipeline (tracked by #1245), and this test never
// opens a .glb or asks the generator to rewrite one.
const SHIPPED_CRUISER_LODS = [
  {
    sidecar: 'assets/models/alliance_cruiser.model.toml',
    levels: [
      {
        output: 'assets/models/alliance_cruiser_lod1.glb',
        source: 'assets/models/alliance_cruiser.glb',
        params: { ratio: 0.19, error: 0.01, textureSize: 256, remeshVoxelSize: 0.0211 },
      },
      {
        output: 'assets/models/alliance_cruiser_lod2.glb',
        source: 'assets/models/alliance_cruiser.glb',
        params: { ratio: 0.113, error: 0.01, textureSize: 128, remeshVoxelSize: 0.0211 },
      },
    ],
  },
  {
    sidecar: 'assets/models/dynasty_cruiser.model.toml',
    levels: [
      {
        output: 'assets/models/dynasty_cruiser_lod1.glb',
        source: 'assets/models/dynasty_cruiser.glb',
        params: { ratio: 0.319, error: 0.01, textureSize: 256, remeshVoxelSize: 0.0211 },
      },
      {
        output: 'assets/models/dynasty_cruiser_lod2.glb',
        source: 'assets/models/dynasty_cruiser.glb',
        params: { ratio: 0.113, error: 0.01, textureSize: 128, remeshVoxelSize: 0.0211 },
      },
    ],
  },
];

function shippedCruiserSidecars() {
  return SHIPPED_CRUISER_LODS.map(({ sidecar: path }) => ({
    path,
    doc: parseToml(readFileSync(path, 'utf8')),
  }));
}

/** The shipped shape: a full model, two decimated steps, a procedural tail. */
const ROCK_LADDER = `
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 100.0
model = "assets/models/rock_lod1.glb"
[lod.generate]
source = "assets/models/rock.glb"
ratio = 0.25
error = 0.01
texture_size = 512

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod2.glb"
[lod.generate]
source = "assets/models/rock.glb"
ratio = 0.05
error = 0.1
texture_size = 256

[[lod]]
shape = "sphere"
`;

describe('collectTargets', () => {
  it('collects one target per generated level, in output order', () => {
    const { targets, errors } = collectTargets([sidecar('assets/models/rock.large.toml', ROCK_LADDER)]);
    expect(errors).toEqual([]);
    expect(targets.map((t) => t.output)).toEqual([
      'assets/models/rock_lod1.glb',
      'assets/models/rock_lod2.glb',
    ]);
    expect(targets[0].source).toBe('assets/models/rock.glb');
    expect(targets[0].params).toEqual({
      ratio: 0.25,
      error: 0.01,
      textureSize: 512,
      remeshVoxelSize: null,
    });
    // The full-detail level declares no generation and is never a target.
    expect(targets.some((t) => t.output === 'assets/models/rock.glb')).toBe(false);
  });

  it('falls back to the ladder\u2019s own near level when no source is named', () => {
    const { targets, errors } = collectTargets([
      sidecar(
        'assets/models/rock.model.toml',
        `
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
model = "assets/models/rock_lod1.glb"
[lod.generate]
ratio = 0.25
texture_size = 512
`,
      ),
    ]);
    expect(errors).toEqual([]);
    expect(targets[0].source).toBe('assets/models/rock.glb');
  });

  it('de-duplicates one generated file declared by every variant of a model', () => {
    const { targets, errors } = collectTargets([
      sidecar('assets/models/rock.small.toml', ROCK_LADDER),
      sidecar('assets/models/rock.cosmetic.toml', ROCK_LADDER),
      sidecar('assets/models/rock.large.toml', ROCK_LADDER),
    ]);
    expect(errors).toEqual([]);
    expect(targets).toHaveLength(2);
    // Recorded in sorted order whatever order the sidecars arrived in, so the
    // manifest does not churn on a directory listing.
    expect(targets[0].declaredBy).toEqual([
      'assets/models/rock.cosmetic.toml',
      'assets/models/rock.large.toml',
      'assets/models/rock.small.toml',
    ]);
  });

  it('refuses two sidecars that disagree about the same generated file', () => {
    const other = ROCK_LADDER.replace('ratio = 0.25', 'ratio = 0.3');
    const { errors } = collectTargets([
      sidecar('assets/models/rock.large.toml', ROCK_LADDER),
      sidecar('assets/models/rock.small.toml', other),
    ]);
    expect(errors).toHaveLength(1);
    expect(errors[0]).toContain('assets/models/rock_lod1.glb');
    expect(errors[0]).toContain('must agree');
  });

  it('rejects parameters that cannot describe a decimation', () => {
    const cases = [
      ['[[lod]]\nmodel = "a.glb"\n[lod.generate]\nsource = "a.glb"\nratio = 0.5\n', 'source and output'],
      ['[[lod]]\n[lod.generate]\nratio = 0.5\n', 'names no model'],
      ['[[lod]]\nmodel = "b.glb"\n[lod.generate]\nsource = "a.glb"\nratio = 1.5\n', 'between 0 and 1'],
      ['[[lod]]\nmodel = "b.glb"\n[lod.generate]\nsource = "a.glb"\n', 'neither ratio nor texture_size'],
      [
        '[[lod]]\nmodel = "b.glb"\n[lod.generate]\nsource = "a.glb"\nratio = 0.5\ntexture_size = 0\n',
        'positive whole number',
      ],
    ];
    for (const [text, expected] of cases) {
      const { errors } = collectTargets([sidecar('assets/models/x.model.toml', text)]);
      expect(errors.join('\n'), text).toContain(expected);
    }
  });

  it('ignores a sidecar with no ladder at all', () => {
    const { targets, errors } = collectTargets([
      sidecar('assets/models/ship.model.toml', '[base]\nscale = [1.0, 1.0, 1.0]\n'),
    ]);
    expect(targets).toEqual([]);
    expect(errors).toEqual([]);
  });

  it('routes decimation through the remesh intermediate when one is declared', () => {
    const { targets } = collectTargets([
      sidecar(
        'assets/models/rock.model.toml',
        `
[[lod]]
model = "assets/models/rock_lod1.glb"
[lod.generate]
source = "assets/models/rock.glb"
ratio = 0.25
remesh_voxel_size = 0.02
`,
      ),
    ]);
    expect(targets[0].source).toBe('assets/models/rock.glb');
    expect(targets[0].effectiveSource).toBe('assets/models/rock.remesh.glb');
    expect(remeshPath('assets/models/rock.glb')).toBe('assets/models/rock.remesh.glb');
  });
});

describe('shipped cruiser generated LODs', () => {
  const expected = SHIPPED_CRUISER_LODS.flatMap(({ sidecar, levels }) =>
    levels.map((level) => ({ ...level, declaredBy: [sidecar] })),
  );
  const outputs = new Set(expected.map(({ output }) => output));

  function manifestEntriesForCruisers() {
    return parseManifest(readFileSync('scripts/lod-manifest.toml', 'utf8')).filter(({ path }) =>
      outputs.has(path),
    );
  }

  function observed(targets) {
    const targetByOutput = new Map(targets.map((target) => [target.output, target]));
    return manifestEntriesForCruisers().map((entry) => ({
      target: targetByOutput.get(entry.path),
      sourceSha256: entry.source_sha256,
      outputSha256: entry.output_sha256,
      outputBytes: entry.output_bytes,
    }));
  }

  it('keeps every shipped cruiser generated level declared and recorded', () => {
    const { targets, errors } = collectTargets(shippedCruiserSidecars());
    expect(errors).toEqual([]);
    expect(targets).toEqual(
      expected.map(({ output, source, params, declaredBy }) => ({
        output,
        source,
        effectiveSource: source.replace('.glb', '.remesh.glb'),
        params,
        declaredBy,
      })),
    );

    // This checks the authored generation parameters against the committed
    // manifest without reading or regenerating the checked-in binaries.
    expect(compareManifest(manifestEntriesForCruisers(), observed(targets))).toEqual([]);
  });

  it('makes a cruiser ratio edit fail the same manifest currency check CI uses', () => {
    const sidecars = shippedCruiserSidecars();
    sidecars[0].doc.lod[1].generate.ratio = 0.2;
    const { targets, errors } = collectTargets(sidecars);
    expect(errors).toEqual([]);

    const findings = compareManifest(manifestEntriesForCruisers(), observed(targets));
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      output: 'assets/models/alliance_cruiser_lod1.glb',
      kind: 'params-changed',
    });
    expect(findings[0].detail).toContain('ratio=0.2');
  });
});

describe('canonicalParams', () => {
  it('names every parameter, so removing one still reads as a change', () => {
    const full = canonicalParams({ ratio: 0.25, error: 0.01, textureSize: 512, remeshVoxelSize: null });
    const dropped = canonicalParams({ ratio: 0.25, error: null, textureSize: 512, remeshVoxelSize: null });
    expect(full).toBe('ratio=0.25 error=0.01 texture_size=512 remesh_voxel_size=none');
    expect(dropped).not.toBe(full);
  });
});

describe('matchesFilter', () => {
  const target = {
    output: 'assets/models/rock_lod1.glb',
    source: 'assets/models/rock.glb',
    declaredBy: ['assets/models/rock.large.toml'],
  };
  it('matches on model name, output path or sidecar, and passes everything when empty', () => {
    expect(matchesFilter(target, [])).toBe(true);
    expect(matchesFilter(target, ['rock'])).toBe(true);
    expect(matchesFilter(target, ['rock_lod1'])).toBe(true);
    expect(matchesFilter(target, ['rock.large.toml'])).toBe(true);
    expect(matchesFilter(target, ['asteroid'])).toBe(false);
  });
});

describe('planSteps — the fixture pipeline', () => {
  const cli = ['gltf-transform'];
  const tmpDir = '/tmp/lod';

  it('turns a known level into the exact simplify \u2192 resize pair, every time', () => {
    const { targets } = collectTargets([sidecar('assets/models/rock.large.toml', ROCK_LADDER)]);
    const steps = planSteps(targets[1], { cli, tmpDir });
    expect(steps.map((s) => s.argv)).toEqual([
      [
        'gltf-transform',
        'simplify',
        'assets/models/rock.glb',
        '/tmp/lod/simplified.glb',
        '--ratio',
        '0.05',
        '--error',
        '0.1',
      ],
      [
        'gltf-transform',
        'resize',
        '/tmp/lod/simplified.glb',
        'assets/models/rock_lod2.glb',
        '--width',
        '256',
        '--height',
        '256',
      ],
    ]);
    // Deterministic: the same declaration plans the same work.
    expect(planSteps(targets[1], { cli, tmpDir })).toEqual(steps);
  });

  it('writes straight to the output when no texture resize is asked for', () => {
    const { targets } = collectTargets([
      sidecar(
        'assets/models/rock.model.toml',
        '[[lod]]\nmodel = "b.glb"\n[lod.generate]\nsource = "a.glb"\nratio = 0.5\n',
      ),
    ]);
    const steps = planSteps(targets[0], { cli, tmpDir });
    expect(steps).toHaveLength(1);
    expect(steps[0].argv).toEqual(['gltf-transform', 'simplify', 'a.glb', 'b.glb', '--ratio', '0.5']);
  });

  it('resizes straight from the source when no decimation is asked for', () => {
    const { targets } = collectTargets([
      sidecar(
        'assets/models/rock.model.toml',
        '[[lod]]\nmodel = "b.glb"\n[lod.generate]\nsource = "a.glb"\ntexture_size = 128\n',
      ),
    ]);
    const steps = planSteps(targets[0], { cli, tmpDir });
    expect(steps).toHaveLength(1);
    expect(steps[0].argv).toEqual([
      'gltf-transform',
      'resize',
      'a.glb',
      'b.glb',
      '--width',
      '128',
      '--height',
      '128',
    ]);
  });
});

describe('the Blender pre-pass', () => {
  it('is planned only for a level that asks for it', () => {
    const { targets } = collectTargets([sidecar('assets/models/rock.large.toml', ROCK_LADDER)]);
    expect(remeshStep(targets[0], { blender: 'blender' })).toBeNull();
  });

  it('passes the source, the intermediate and the voxel size after the -- separator', () => {
    const { targets } = collectTargets([
      sidecar(
        'assets/models/rock.model.toml',
        '[[lod]]\nmodel = "b.glb"\n[lod.generate]\nsource = "assets/models/rock.glb"\nratio = 0.5\nremesh_voxel_size = 0.02\n',
      ),
    ]);
    const step = remeshStep(targets[0], { blender: 'blender', script: 'scripts/blender-voxel-remesh.py' });
    expect(step.argv).toEqual([
      'blender',
      '--background',
      '--factory-startup',
      '--python',
      'scripts/blender-voxel-remesh.py',
      '--',
      'assets/models/rock.glb',
      'assets/models/rock.remesh.glb',
      '0.02',
    ]);
  });

  it('looks for Blender by env var, then PATH, then the newest Windows install', () => {
    expect(
      blenderCandidates({
        env: { PHOENIX_BLENDER: 'D:\\blender\\blender.exe', ProgramFiles: 'C:\\Program Files' },
        platform: 'win32',
        installedDirs: ['Blender 4.2', 'Blender 5.0', 'unrelated'],
      }),
    ).toEqual([
      'D:\\blender\\blender.exe',
      'blender',
      'C:\\Program Files\\Blender Foundation\\Blender 5.0\\blender.exe',
      'C:\\Program Files\\Blender Foundation\\Blender 4.2\\blender.exe',
    ]);
    // Elsewhere there is nothing to guess at: PATH or nothing.
    expect(blenderCandidates({ env: {}, platform: 'linux' })).toEqual(['blender']);
  });
});

describe('remeshTextureCap — how big a remesh intermediate’s textures need to be', () => {
  /** A hull whose two far levels are both cut from one voxel remesh. */
  function remeshLadder(lod1Texture, lod2Texture) {
    const level = (name, ratio, texture) => `
[[lod]]
model = "assets/models/hull_${name}.glb"
[lod.generate]
source = "assets/models/hull.glb"
ratio = ${ratio}
error = 0.01
${texture === null ? '' : `texture_size = ${texture}`}
remesh_voxel_size = 0.0211
`;
    return sidecar(
      'assets/models/hull.model.toml',
      `
[[lod]]
max_distance = 15.0
model = "assets/models/hull.glb"
${level('lod1', 0.319, lod1Texture)}${level('lod2', 0.113, lod2Texture)}`,
    );
  }

  const REMESH = 'assets/models/hull.remesh.glb';

  it('takes the largest size any level sharing the intermediate cuts', () => {
    const { targets } = collectTargets([remeshLadder(256, 128)]);
    expect(remeshTextureCap(targets, REMESH)).toBe(256);
  });

  // The shipped `dynasty_battleship` shape, and the reason the cap is derived
  // rather than written as 256: exactly one level in the tree asks for more,
  // and a fixed number would have silently halved it.
  it('does not clamp a ladder whose near level asks for more than the usual 256', () => {
    const { targets } = collectTargets([remeshLadder(512, 128)]);
    expect(remeshTextureCap(targets, REMESH)).toBe(512);
  });

  // Order must not matter: the cap is applied once to a file both levels read,
  // so whichever level the generator happens to process first cannot be allowed
  // to starve the other.
  it('is the same whichever level is declared first', () => {
    const { targets } = collectTargets([remeshLadder(128, 512)]);
    expect(remeshTextureCap(targets, REMESH)).toBe(512);
  });

  it('leaves the file alone when a consumer declares no texture_size at all', () => {
    const { targets } = collectTargets([remeshLadder(256, null)]);
    expect(remeshTextureCap(targets, REMESH)).toBeNull();
  });

  it('leaves a file alone that nothing is cut from', () => {
    const { targets } = collectTargets([remeshLadder(256, 128)]);
    expect(remeshTextureCap(targets, 'assets/models/other.remesh.glb')).toBeNull();
  });

  // A level with no voxel pre-pass decimates the base .glb directly, and that
  // file is the one that SHIPS — its textures are not an intermediate and are
  // never capped by this.
  it('does not offer a cap for a source that is not a remesh intermediate', () => {
    const { targets } = collectTargets([sidecar('assets/models/rock.large.toml', ROCK_LADDER)]);
    expect(remeshTextureCap(targets, 'assets/models/rock.glb')).toBe(512);
    expect(targets.every((t) => t.effectiveSource === t.source)).toBe(true);
  });
});

describe('the manifest', () => {
  const SOURCE_BYTES = 'tiny-source-glb';
  const OUTPUT_BYTES = 'tiny-lod1-glb';
  const SOURCE_SHA = '1e7542c714b3d122fb2c30183d8bda59e67d4503f5d3d2c664d66f65ca004a81';
  const OUTPUT_SHA = 'c2b520c0c436315cdf4a79e2da4ed105be1529edef408457eb7f786df01f95cd';

  function fixture() {
    const { targets } = collectTargets([
      sidecar('assets/models/rock.large.toml', ROCK_LADDER),
      sidecar('assets/models/rock.small.toml', ROCK_LADDER),
    ]);
    const observed = {
      sourceSha256: sha256(SOURCE_BYTES),
      outputSha256: sha256(OUTPUT_BYTES),
      outputBytes: 4096,
    };
    return { target: targets[0], observed };
  }

  it('hashes bytes reproducibly', () => {
    expect(sha256(SOURCE_BYTES)).toBe(SOURCE_SHA);
    expect(sha256(OUTPUT_BYTES)).toBe(OUTPUT_SHA);
  });

  it('renders a known input to exactly one expected document', () => {
    const { target, observed } = fixture();
    const text = formatManifest([manifestEntry(target, observed)]);
    expect(text).toContain('[[output]]\npath = "assets/models/rock_lod1.glb"');
    expect(text).toContain(`source_sha256 = "${SOURCE_SHA}"`);
    expect(text).toContain('params = { ratio = 0.25, error = 0.01, texture_size = 512 }');
    expect(text).toContain(`output_sha256 = "${OUTPUT_SHA}"`);
    expect(text).toContain('output_bytes = 4096');
    expect(text).toContain(
      'declared_by = ["assets/models/rock.large.toml", "assets/models/rock.small.toml"]',
    );
    // Byte-identical on a second render: the manifest is a value, not a log.
    expect(formatManifest([manifestEntry(target, observed)])).toBe(text);
  });

  it('sorts records by output path however they were collected', () => {
    const { targets } = collectTargets([sidecar('assets/models/rock.large.toml', ROCK_LADDER)]);
    const entries = targets
      .map((t) => manifestEntry(t, { sourceSha256: 'a', outputSha256: 'b', outputBytes: 1 }))
      .reverse();
    const paths = [...formatManifest(entries).matchAll(/^path = "(.+)"$/gm)].map((m) => m[1]);
    expect(paths).toEqual(['assets/models/rock_lod1.glb', 'assets/models/rock_lod2.glb']);
  });

  it('round-trips through TOML', () => {
    const { target, observed } = fixture();
    const parsed = parseManifest(formatManifest([manifestEntry(target, observed)]));
    expect(parsed).toHaveLength(1);
    expect(parsed[0].path).toBe('assets/models/rock_lod1.glb');
    expect(parsed[0].output_bytes).toBe(4096);
    expect(parsed[0].params).toBe(canonicalParams(target.params));
    // A parsed record re-renders identically, so an untouched output keeps its
    // record verbatim when a single model is regenerated.
    expect(formatManifest(parsed)).toBe(formatManifest([manifestEntry(target, observed)]));
  });

  it('records the voxel pre-pass origin alongside the file it decimated', () => {
    const { targets } = collectTargets([
      sidecar(
        'assets/models/rock.model.toml',
        '[[lod]]\nmodel = "b.glb"\n[lod.generate]\nsource = "assets/models/rock.glb"\nratio = 0.5\nremesh_voxel_size = 0.02\n',
      ),
    ]);
    const text = formatManifest([
      manifestEntry(targets[0], {
        sourceSha256: 'aa',
        originSha256: 'bb',
        outputSha256: 'cc',
        outputBytes: 8,
      }),
    ]);
    expect(text).toContain('source = "assets/models/rock.remesh.glb"');
    expect(text).toContain('origin = "assets/models/rock.glb"');
    expect(text).toContain('origin_sha256 = "bb"');
    expect(text).toContain('remesh_voxel_size = 0.02');
  });
});

describe('compareManifest — the drift check CI runs', () => {
  const { targets } = collectTargets([sidecar('assets/models/rock.large.toml', ROCK_LADDER)]);
  const current = targets.map((t) =>
    manifestEntry(t, { sourceSha256: 'src', outputSha256: `out-${t.output}`, outputBytes: 10 }),
  );
  const onDisk = targets.map((t) => ({
    target: t,
    sourceSha256: 'src',
    outputSha256: `out-${t.output}`,
    outputBytes: 10,
  }));

  it('says nothing about a tree that matches its record', () => {
    expect(compareManifest(current, onDisk)).toEqual([]);
  });

  it('catches a source replaced without regenerating', () => {
    const observed = [{ ...onDisk[0], sourceSha256: 'other' }, onDisk[1]];
    const findings = compareManifest(current, observed);
    expect(findings.map((f) => f.kind)).toEqual(['source-changed']);
    expect(findings[0].output).toBe('assets/models/rock_lod1.glb');
  });

  it('catches parameters retuned without regenerating', () => {
    const retuned = collectTargets([
      sidecar('assets/models/rock.large.toml', ROCK_LADDER.replace('ratio = 0.25', 'ratio = 0.4')),
    ]).targets;
    const observed = retuned.map((t, i) => ({ ...onDisk[i], target: t }));
    const findings = compareManifest(current, observed);
    expect(findings.map((f) => f.kind)).toEqual(['params-changed']);
    expect(findings[0].detail).toContain('ratio=0.4');
  });

  it('catches a generated file edited by hand', () => {
    const observed = [{ ...onDisk[0], outputSha256: 'tampered' }, onDisk[1]];
    expect(compareManifest(current, observed).map((f) => f.kind)).toEqual(['output-changed']);
  });

  it('catches a level pointed at a different source', () => {
    const repointed = collectTargets([
      sidecar(
        'assets/models/rock.large.toml',
        ROCK_LADDER.replace('source = "assets/models/rock.glb"\nratio = 0.25', 'source = "assets/models/boulder.glb"\nratio = 0.25'),
      ),
    ]).targets;
    const observed = repointed.map((t, i) => ({ ...onDisk[i], target: t }));
    expect(compareManifest(current, observed).map((f) => f.kind)).toEqual(['source-repointed']);
  });

  it('catches a missing output, a new declaration and an abandoned record', () => {
    const missing = [{ ...onDisk[0], outputSha256: null, outputBytes: null }, onDisk[1]];
    expect(compareManifest(current, missing).map((f) => f.kind)).toEqual(['missing-output']);

    expect(compareManifest([current[0]], onDisk).map((f) => f.kind)).toEqual(['unrecorded']);
    expect(compareManifest(current, [onDisk[0]]).map((f) => f.kind)).toEqual(['orphaned']);
  });
});

describe('sizeReport', () => {
  it('flags a level that came out bigger than the one it replaced', () => {
    const target = { output: 'assets/models/rock_lod2.glb' };
    const previous = [{ path: 'assets/models/rock_lod2.glb', output_bytes: 900_000 }];
    const [grew] = sizeReport(previous, [{ target, outputBytes: 7_800_000 }]);
    expect(grew.grew).toBe(true);
    expect(grew.line).toContain('LARGER');

    const [shrank] = sizeReport(previous, [{ target, outputBytes: 400_000 }]);
    expect(shrank.grew).toBe(false);
    expect(shrank.line).not.toContain('LARGER');

    // A brand-new output has nothing to compare against and is not a warning.
    const [fresh] = sizeReport([], [{ target, outputBytes: 400_000 }]);
    expect(fresh.grew).toBe(false);
    expect(fresh.previousBytes).toBeNull();
  });
});

describe('blockedGrowth — the growth gate', () => {
  // A level with no `remesh_voxel_size`: growing past its recorded baseline
  // has no assigned remedy, so the gate refuses it.
  const { targets: plain } = collectTargets([
    sidecar('assets/models/rock.large.toml', ROCK_LADDER),
  ]);
  const previous = [{ path: 'assets/models/rock_lod2.glb', output_bytes: 900_000 }];

  // A level that already declares the voxel pre-pass: the assigned remedy for
  // a stubborn mesh has already been taken, so growth here is exempt rather
  // than gated a second time with nothing further to suggest.
  const { targets: remeshed } = collectTargets([
    sidecar(
      'assets/models/rock.model.toml',
      '[[lod]]\nmodel = "assets/models/rock_lod2.glb"\n[lod.generate]\nsource = "assets/models/rock.glb"\nratio = 0.05\nremesh_voxel_size = 0.02\n',
    ),
  ]);

  it('fails a level with no declared remedy that grew past its baseline', () => {
    const sizes = sizeReport(previous, [{ target: plain[1], outputBytes: 7_800_000 }]);
    const blocked = blockedGrowth(sizes);
    expect(blocked).toHaveLength(1);
    expect(blocked[0].output).toBe('assets/models/rock_lod2.glb');
  });

  it('writes through when --force is passed', () => {
    const sizes = sizeReport(previous, [{ target: plain[1], outputBytes: 7_800_000 }]);
    expect(blockedGrowth(sizes, { force: true })).toEqual([]);
  });

  it('is fine with a level that shrank, force or not', () => {
    const sizes = sizeReport(previous, [{ target: plain[1], outputBytes: 400_000 }]);
    expect(blockedGrowth(sizes)).toEqual([]);
    expect(blockedGrowth(sizes, { force: true })).toEqual([]);
  });

  it('exempts a level that already declares the voxel pre-pass, grown or not', () => {
    const sizes = sizeReport(previous, [{ target: remeshed[0], outputBytes: 7_800_000 }]);
    expect(sizes[0].grew).toBe(true);
    expect(blockedGrowth(sizes)).toEqual([]);
  });

  it('names the model and both sizes for a blocked output', () => {
    const sizes = sizeReport(previous, [{ target: plain[1], outputBytes: 7_800_000 }]);
    const text = describeBlockedGrowth(blockedGrowth(sizes));
    expect(text).toContain('assets/models/rock_lod2.glb');
    expect(text).toContain('878.9 KB');
    expect(text).toContain('7.44 MB');
  });
});
