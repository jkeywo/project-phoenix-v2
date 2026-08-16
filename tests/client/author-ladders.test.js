import { describe, it, expect } from 'vitest';
import { bandMultiplier, BAND_MULTIPLIERS, buildLadder } from '../../scripts/author-ladders.mjs';

// Pure-function coverage for the per-variant switch bands (issue #947). The
// disk-touching authorLadders() orchestration is out of scope here — Blender,
// the gltf CLI and real .glb bytes belong to generate-lods — but the decision
// that a huge size class switches 3× further out while every other variant
// keeps the base ladder is a pure function of the variant name, and that is the
// regression b78bf9ff introduced by writing one ladder to every sidecar.

describe('bandMultiplier', () => {
  it('scales the huge size class by 3 and nothing else', () => {
    expect(bandMultiplier('huge')).toBe(3);
  });

  it('keeps small, large, cosmetic and the base rig on the base ladder', () => {
    expect(bandMultiplier('small')).toBe(1);
    expect(bandMultiplier('large')).toBe(1);
    expect(bandMultiplier('cosmetic')).toBe(1);
    expect(bandMultiplier('')).toBe(1); // the base `<stem>.model.toml` rig
  });

  it('leaves any unknown variant on the base ladder rather than guessing', () => {
    expect(bandMultiplier('enormous')).toBe(1);
    expect(BAND_MULTIPLIERS).toEqual({ huge: 3 });
  });
});

describe('the bands buildLadder lays out for each variant', () => {
  const base = { near: 15, mid: 100, far: 400 };
  const ladderFor = (variant) => {
    const m = bandMultiplier(variant);
    return buildLadder({
      stem: 'asteroid_common_1',
      near: base.near * m,
      mid: base.mid * m,
      far: base.far * m,
      colour: [0, 0, 0],
      sphere: { radius: 1, scale: [1, 1, 1] },
      lod1Gen: {},
      lod2Gen: {},
    }).map((level) => level.max_distance);
  };

  it('gives large the base 15/100/400 and an unbounded far sphere', () => {
    expect(ladderFor('large')).toEqual([15, 100, 400, undefined]);
  });

  it('gives huge the base ladder ×3 — 45/300/1200 — derived, not hardcoded', () => {
    expect(ladderFor('huge')).toEqual([45, 300, 1200, undefined]);

    // The invariant the test exists to defend: huge is large's bands, ×3.
    const large = ladderFor('large').slice(0, 3);
    const huge = ladderFor('huge').slice(0, 3);
    expect(huge).toEqual(large.map((d) => d * 3));
  });
});

describe('the tier_rig convention buildLadder records', () => {
  const ladder = () =>
    buildLadder({
      stem: 'alliance_destroyer',
      near: 15,
      mid: 100,
      far: 400,
      colour: [0, 0, 0],
      sphere: { radius: 1, scale: [1, 1, 1] },
      lod1Gen: {},
      lod2Gen: {},
    });

  // This script writes only a stem's PRIMARY sidecars and never one beside a
  // generated .glb, so its generated tiers resolve an identity rig. Recording
  // that here is what stops the renderer fetching the absent file to find out —
  // a 404 per hull model per browser session.
  it('marks both generated tiers identity', () => {
    expect(ladder().map((l) => l.tier_rig)).toEqual([
      undefined,
      'identity',
      'identity',
      undefined,
    ]);
  });

  it('leaves the near tier and the far sphere unmarked', () => {
    const [near, , , sphere] = ladder();
    // The near level's model IS the primary GLB, whose sidecar is the file
    // being read; the sphere has no GLB to have a rig at all.
    expect(near.tier_rig).toBeUndefined();
    expect(sphere.tier_rig).toBeUndefined();
    expect(sphere.model).toBeUndefined();
  });
});
