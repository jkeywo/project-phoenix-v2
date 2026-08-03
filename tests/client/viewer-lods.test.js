import { describe, it, expect } from 'vitest';
import { parse as parseToml } from 'smol-toml';
import {
  modelStem,
  sidecarsForStem,
  variantOfSidecar,
  ladderFromDoc,
  extentFromDoc,
  modelUnitExtent,
  renderLadder,
  splitLadder,
  replaceLadder,
  validateLadder,
  validateProposal,
  templateLadder,
  generatedOutputs,
} from '../../scripts/viewer-lods.mjs';

/** A sidecar shaped like the shipped asteroids: rig, then ladder, with prose. */
const ASTEROID = `[base]
offset = [ 0, -2.4, 0 ]
scale = [ 4.2, 4.2, 4.2 ]

[extents]
min = [ -4, -2.4, -3.5 ]
max = [ 4, 2.4, 3.5 ]
size = [ 8, 4.8, 7 ]

[markers]

# Distance-based LOD: full GLB up close, one decimated step, then a sphere.
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
shape = "sphere"
`;

describe('modelStem', () => {
  it('takes the file stem of a model path', () => {
    expect(modelStem('assets/models/alliance_cruiser.glb')).toBe('alliance_cruiser');
  });

  it('leaves a bare name alone', () => {
    expect(modelStem('alliance_cruiser')).toBe('alliance_cruiser');
  });
});

describe('sidecarsForStem', () => {
  const files = [
    'rock.glb',
    'rock.large.toml',
    'rock.small.toml',
    'rock.cosmetic.toml',
    'rock_lod1.glb',
    'rock_lod1.large.toml',
    'ship.model.toml',
  ];

  it('finds every variant sidecar of one model', () => {
    expect(sidecarsForStem(files, 'rock')).toEqual([
      'rock.cosmetic.toml',
      'rock.large.toml',
      'rock.small.toml',
    ]);
  });

  /** The generated level's own rig is a different model, not a variant. */
  it('does not claim a generated level’s sidecar', () => {
    expect(sidecarsForStem(files, 'rock')).not.toContain('rock_lod1.large.toml');
  });

  it('treats the base .model.toml as a sidecar', () => {
    expect(sidecarsForStem(files, 'ship')).toEqual(['ship.model.toml']);
  });
});

describe('ladderFromDoc', () => {
  it('reads the levels a sidecar declares', () => {
    const levels = ladderFromDoc(parseToml(ASTEROID));
    expect(levels).toHaveLength(3);
    expect(levels[0]).toEqual({ max_distance: 50, model: 'assets/models/rock.glb' });
    expect(levels[1].generate).toEqual({
      source: 'assets/models/rock.glb',
      ratio: 0.25,
      error: 0.01,
      texture_size: 512,
    });
    expect(levels[2]).toEqual({ shape: 'sphere' });
  });

  it('is empty for a sidecar with no ladder', () => {
    expect(ladderFromDoc(parseToml('[base]\nscale = [1, 1, 1]\n'))).toEqual([]);
  });

  /** An omitted field means "inherit", which is not the same as a null. */
  it('does not invent keys the level omitted', () => {
    const [level] = ladderFromDoc(parseToml('[[lod]]\nshape = "sphere"\n'));
    expect('colour' in level).toBe(false);
    expect('radius' in level).toBe(false);
  });
});

describe('extentFromDoc', () => {
  it('is the largest declared dimension', () => {
    expect(extentFromDoc(parseToml(ASTEROID))).toBe(8);
  });

  it('is null when the sidecar declares no extents', () => {
    expect(extentFromDoc(parseToml('[base]\n'))).toBe(null);
  });
});

describe('modelUnitExtent', () => {
  const sidecar = (scale, size) =>
    parseToml(`${scale ? `[base]
scale = ${scale}

` : ''}[extents]
size = ${size}
`);

  /** The units `remesh_voxel_size` is measured in — and the only thing in the
      sidecar that is not written in post-scale world units. */
  it('divides the world extent back out by the base scale', () => {
    expect(modelUnitExtent(sidecar('[4, 4, 4]', '[8, 4, 7]'))).toBe(2);
  });

  it('handles a non-uniform scale per axis', () => {
    expect(modelUnitExtent(sidecar('[1, 4, 1]', '[3, 8, 1]'))).toBe(3);
  });

  it('treats a missing base scale as 1', () => {
    expect(modelUnitExtent(sidecar(null, '[2, 5, 1]'))).toBe(5);
  });

  it('is null with no extents to divide', () => {
    expect(modelUnitExtent(parseToml('[base]'))).toBe(null);
  });

  /** The worked example: a voxel of 1.0 spans over half of this. */
  it('reports the shipped asteroids at under two units', () => {
    expect(modelUnitExtent(parseToml(ASTEROID))).toBeLessThan(2);
  });
});

describe('renderLadder', () => {
  it('writes whole numbers as TOML floats', () => {
    const text = renderLadder([{ max_distance: 50, model: 'a.glb' }]);
    expect(text).toContain('max_distance = 50.0');
  });

  it('writes texture_size as a whole number of pixels', () => {
    const text = renderLadder([
      { max_distance: 100, model: 'a.glb', generate: { ratio: 0.25, texture_size: 512 } },
    ]);
    expect(text).toContain('texture_size = 512');
    expect(text).toContain('ratio = 0.25');
  });

  it('omits max_distance on the fallback level', () => {
    expect(renderLadder([{ shape: 'sphere' }])).toBe('[[lod]]\nshape = "sphere"');
  });
});

describe('splitLadder', () => {
  it('keeps the rig sections and the ladder’s own comment', () => {
    const { before, comment, hadLadder } = splitLadder(ASTEROID);
    expect(hadLadder).toBe(true);
    expect(before).toContain('[base]');
    expect(before).toContain('[markers]');
    expect(before).not.toContain('[[lod]]');
    expect(comment).toBe(
      '# Distance-based LOD: full GLB up close, one decimated step, then a sphere.',
    );
  });

  it('keeps sections that follow the ladder', () => {
    const text = '[[lod]]\nshape = "sphere"\n\n[markers.engine]\nposition = [0, 0, 0]\n';
    const { after } = splitLadder(text);
    expect(after).toContain('[markers.engine]');
    expect(after).toContain('position = [0, 0, 0]');
  });

  it('reports a sidecar with no ladder at all', () => {
    const { hadLadder, before } = splitLadder('[base]\nscale = [1, 1, 1]\n');
    expect(hadLadder).toBe(false);
    expect(before).toContain('scale');
  });
});

describe('replaceLadder', () => {
  /** The edit a person makes most: change one number, leave everything else. */
  it('round-trips an unedited ladder byte for byte', () => {
    const levels = ladderFromDoc(parseToml(ASTEROID));
    expect(replaceLadder(ASTEROID, levels)).toBe(ASTEROID);
  });

  it('writes an edited ratio back into the same file', () => {
    const levels = ladderFromDoc(parseToml(ASTEROID));
    levels[1].generate.ratio = 0.1;
    const updated = replaceLadder(ASTEROID, levels);
    expect(updated).toContain('ratio = 0.1');
    expect(updated).not.toContain('ratio = 0.25');
    expect(updated).toContain('[markers]');
  });

  it('adds a ladder to a sidecar that had none', () => {
    const updated = replaceLadder('[base]\nscale = [1, 1, 1]\n', [
      { max_distance: 40, model: 'a.glb' },
      { shape: 'sphere' },
    ]);
    expect(updated).toContain('[base]');
    expect(updated).toContain('max_distance = 40.0');
    expect(parseToml(updated).lod).toHaveLength(2);
  });

  it('removes the ladder and its comment when the last level goes', () => {
    const updated = replaceLadder(ASTEROID, []);
    expect(updated).not.toContain('[[lod]]');
    expect(updated).not.toContain('Distance-based LOD');
    expect(updated).toContain('[extents]');
    expect(parseToml(updated).lod).toBeUndefined();
  });
});

describe('validateLadder', () => {
  it('accepts the shipped shape', () => {
    expect(validateLadder(ladderFromDoc(parseToml(ASTEROID)))).toEqual([]);
  });

  it('rejects a level that renders nothing', () => {
    expect(validateLadder([{ max_distance: 50 }])).toContain(
      'level 0: needs either a model or a procedural shape',
    );
  });

  it('rejects distances that do not increase near→far', () => {
    const problems = validateLadder([
      { max_distance: 100, model: 'a.glb' },
      { max_distance: 50, model: 'b.glb' },
    ]);
    expect(problems.join('\n')).toContain('must increase near→far');
  });

  /** An unbounded level swallows everything after it. */
  it('rejects an unbounded level that is not last', () => {
    const problems = validateLadder([{ model: 'a.glb' }, { max_distance: 50, model: 'b.glb' }]);
    expect(problems).toContain('level 0: only the final level may omit max_distance');
  });

  it('rejects generation parameters on a level with no model', () => {
    const problems = validateLadder([{ shape: 'sphere', generate: { ratio: 0.25 } }]);
    expect(problems).toContain('level 0: declares [lod.generate] but is not a generated GLB level');
  });

  it('has nothing to say about an empty ladder', () => {
    expect(validateLadder([])).toEqual([]);
  });
});

describe('validateProposal', () => {
  it('passes a ladder every variant agrees on', () => {
    const text = replaceLadder(ASTEROID, ladderFromDoc(parseToml(ASTEROID)));
    expect(
      validateProposal([
        { path: 'assets/models/rock.large.toml', text },
        { path: 'assets/models/rock.small.toml', text },
      ]),
    ).toEqual([]);
  });

  /** The state `npm run lods` refuses to run with, caught before the write. */
  it('catches two variants disagreeing about one generated file', () => {
    const levels = ladderFromDoc(parseToml(ASTEROID));
    const other = structuredClone(levels);
    other[1].generate.ratio = 0.5;
    const problems = validateProposal([
      { path: 'assets/models/rock.large.toml', text: replaceLadder(ASTEROID, levels) },
      { path: 'assets/models/rock.small.toml', text: replaceLadder(ASTEROID, other) },
    ]);
    expect(problems.join('\n')).toContain('must agree on how it is made');
  });

  it('catches a ratio the generator would refuse', () => {
    const levels = ladderFromDoc(parseToml(ASTEROID));
    levels[1].generate.ratio = 4;
    const problems = validateProposal([
      { path: 'assets/models/rock.large.toml', text: replaceLadder(ASTEROID, levels) },
    ]);
    expect(problems.join('\n')).toContain('must be between 0 and 1');
  });
});

describe('templateLadder', () => {
  const template = {
    levels: ladderFromDoc(parseToml(ASTEROID)),
    stem: 'rock',
    extent: 8,
  };

  it('renames every path to the target model', () => {
    const levels = templateLadder(template, { stem: 'alliance_destroyer', extent: 8 });
    expect(levels[1].model).toBe('assets/models/alliance_destroyer_lod1.glb');
    expect(levels[1].generate.source).toBe('assets/models/alliance_destroyer.glb');
  });

  it('scales the switch distances by the size difference', () => {
    const levels = templateLadder(template, { stem: 'big', extent: 16 });
    expect(levels[0].max_distance).toBe(100);
    expect(levels[1].max_distance).toBe(200);
  });

  it('carries the decimation parameters across untouched', () => {
    const levels = templateLadder(template, { stem: 'big', extent: 16 });
    expect(levels[1].generate).toEqual({
      source: 'assets/models/big.glb',
      ratio: 0.25,
      error: 0.01,
      texture_size: 512,
    });
  });

  it('leaves distances alone when either extent is unknown', () => {
    const levels = templateLadder({ ...template, extent: null }, { stem: 'big', extent: 16 });
    expect(levels[0].max_distance).toBe(50);
  });

  it('keeps the unbounded fallback unbounded', () => {
    const levels = templateLadder(template, { stem: 'big', extent: 16 });
    expect(levels[2]).toEqual({ shape: 'sphere' });
  });

  /** A proposal has to survive the checks a save runs. */
  it('produces a ladder that validates', () => {
    const levels = templateLadder(template, { stem: 'big', extent: 16 });
    expect(validateLadder(levels)).toEqual([]);
  });
});

describe('generatedOutputs', () => {
  it('names the files a run would rewrite', () => {
    expect(generatedOutputs(ladderFromDoc(parseToml(ASTEROID)))).toEqual([
      'assets/models/rock_lod1.glb',
    ]);
  });
});

describe('variantOfSidecar', () => {
  it('reads the variant out of a sidecar filename', () => {
    expect(variantOfSidecar('rock.large.toml', 'rock')).toBe('large');
  });

  /** The reserved default reports as the empty value the dropdown uses. */
  it('reports the base rig as no variant', () => {
    expect(variantOfSidecar('ship.model.toml', 'ship')).toBe('');
  });
});

describe('per-level scale', () => {
  it('round-trips an [x, y, z] scale through a sidecar', () => {
    const levels = ladderFromDoc(parseToml(ASTEROID));
    levels[2].scale = [3, 1, 0.5];
    const updated = replaceLadder(ASTEROID, levels);
    expect(updated).toContain('scale = [ 3.0, 1.0, 0.5 ]');
    expect(ladderFromDoc(parseToml(updated))[2].scale).toEqual([3, 1, 0.5]);
  });

  /** A shape level is the one that most needs it, and it validates like any
      other visual override. */
  it('accepts a scale on a procedural level', () => {
    expect(validateLadder([{ shape: 'sphere', scale: [2, 1, 1] }])).toEqual([]);
  });

  it('round-trips a rotation and a colour on a shape level', () => {
    const levels = ladderFromDoc(parseToml(ASTEROID));
    levels[2].rotation = [0, 1.5708, 0];
    levels[2].colour = [0.5, 0.25, 0.125];
    const updated = replaceLadder(ASTEROID, levels);
    const read = ladderFromDoc(parseToml(updated))[2];
    expect(read.rotation).toEqual([0, 1.5708, 0]);
    expect(read.colour).toEqual([0.5, 0.25, 0.125]);
  });

  /** Whole numbers still need a decimal point, including inside an array. */
  it('writes a zeroed rotation axis as a TOML float', () => {
    const text = renderLadder([{ shape: 'sphere', rotation: [0, 3, 0] }]);
    expect(text).toContain('rotation = [ 0.0, 3.0, 0.0 ]');
  });
});
