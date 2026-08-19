import { describe, it, expect } from 'vitest';
import { readdir, readFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { parse as parseToml } from 'smol-toml';
import { NodeIO, getBounds } from '@gltf-transform/core';

// John's second invariant, held against the meshes themselves: a model's
// VISIBLE SIZE must not change by LOD level.
//
// The Rust side already holds the GLB tiers of that claim
// (`every_shipped_ladder_holds_one_world_size_across_its_tiers`) and, since the
// starbase blink, the rule that a billboard tier's size is convention-independent
// (`every_shipped_billboard_tier_is_convention_independent`). Neither can hold
// the MAGNITUDE, because neither can open a `.glb` — so both were green through
// a ladder whose far imposter was six times the width of its own mesh.
//
// This is where that number is checkable. `[[lod]] billboard.scale` is the quad's
// world size, so it must equal the near GLB's own bounding box scaled by the
// sidecar's `[base].scale` — the size every GLB tier of the same ladder reaches.
// A stale atlas (the model rescaled without a re-capture, which is how
// alliance_starbase came to carry a size measured back when it was [5,6,6]) fails
// here, and so does a capture script that starts writing raw units instead of
// world units.
//
// Width rides the larger horizontal axis and height rides y, matching
// `capture_billboard`'s `[size.x.max(size.z), size.y]` and
// `billboard_quad_size`'s "width rides z, height rides y".

const MODELS_DIR = 'assets/models';
// Slack for the 4-decimal rounding `capture-billboards.mjs` writes with, plus a
// little for f32/f64 drift through the capture tool. Anything real here is a
// whole-number factor (a base scale folded in twice), not a rounding tail.
const TOLERANCE = 0.005;
// This suite opens every shipped near-model — tens of megabytes of .glb, one of
// them 13 MB on its own — so it is IO-bound in a way no other test here is. The
// default 5 s runs out the moment the machine is also compiling or running the
// headless suite, and a timeout would read as "the billboards are wrong".
const IO_TIMEOUT_MS = 120_000;

const io = new NodeIO();
const boundsCache = new Map();

async function glbSize(file) {
  if (!boundsCache.has(file)) {
    const doc = await io.read(file);
    const box = getBounds(doc.getRoot().listScenes()[0]);
    boundsCache.set(file, [0, 1, 2].map((i) => box.max[i] - box.min[i]));
  }
  return boundsCache.get(file);
}

/** Every ladder-bearing sidecar, with the pieces this check needs. */
async function shippedLadders() {
  const files = (await readdir(MODELS_DIR)).filter((f) => f.endsWith('.toml')).sort();
  const out = [];
  for (const file of files) {
    let doc;
    try {
      doc = parseToml(await readFile(`${MODELS_DIR}/${file}`, 'utf8'));
    } catch {
      continue;
    }
    if (!Array.isArray(doc.lod) || !doc.lod.length) continue;
    const billboard = doc.lod.find((l) => l.billboard);
    if (!billboard) continue;
    const near = doc.lod.find((l) => l.model)?.model;
    if (!near || !existsSync(near)) continue;
    out.push({
      file,
      near,
      baseScale: doc?.base?.scale ?? [1, 1, 1],
      scale: billboard.scale,
    });
  }
  return out;
}

describe('shipped billboard imposters', () => {
  it('are the same world size as the mesh they stand in for', async () => {
    const ladders = await shippedLadders();
    expect(ladders.length).toBeGreaterThan(0);

    const problems = [];
    for (const { file, near, baseScale, scale } of ladders) {
      if (!Array.isArray(scale)) {
        problems.push(`${file}: billboard level authors no scale`);
        continue;
      }
      const raw = await glbSize(near);
      const world = [0, 1, 2].map((i) => raw[i] * Number(baseScale[i]));
      const want = [Math.max(world[0], world[2]), world[1]];
      const ratio = [scale[0] / want[0], scale[1] / want[1]];
      if (Math.abs(ratio[0] - 1) > TOLERANCE || Math.abs(ratio[1] - 1) > TOLERANCE) {
        problems.push(
          `${file}: imposter ${scale[0]}×${scale[1]} vs mesh world size ` +
            `${want[0].toFixed(4)}×${want[1].toFixed(4)} ` +
            `(off by ${ratio[0].toFixed(3)}×, ${ratio[1].toFixed(3)}×) — ` +
            `re-capture with scripts/capture-billboards.mjs`,
        );
      }
    }

    expect(problems, problems.join('\n')).toEqual([]);
  }, IO_TIMEOUT_MS);
});
