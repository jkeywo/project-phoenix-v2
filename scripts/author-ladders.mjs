// author-ladders.mjs — (re)author a model's `[[lod]]` ladder in its rig
// sidecar(s), sized and budget-tuned, then leave the .glb generation to
// scripts/generate-lods.mjs.
//
//   node scripts/author-ladders.mjs <stem> [--lod1-mb 2] [--lod2-mb 0.5]
//     [--near 15] [--mid 100] [--far 400] [--remesh-voxel <size>]
//
// The four-level ladder every model gets:
//   level 0  base .glb          max_distance = near (15)
//   level 1  <stem>_lod1.glb    max_distance = mid  (100)   ≤ lod1 budget
//   level 2  <stem>_lod2.glb    max_distance = far  (400)   ≤ lod2 budget
//   level 3  sphere             (unbounded, 400+)           hull-coloured
//
// ── What this decides vs. what generate-lods decides ────────────────────────
//
// generate-lods.mjs is the authority on turning a `[lod.generate]` block into a
// .glb and recording its bytes; it does NOT choose the parameters. This script
// chooses them: it searches (ratio, texture_size) for the decimated levels until
// the output fits its budget, writes the winning numbers into every sidecar of
// the stem, and stops. Running `node scripts/generate-lods.mjs <stem>` then
// rebuilds the files and the manifest from what was authored — the same path a
// hand-edit through the viewer's LOD panel would take.
//
// ── The geometry is shared; the sphere and the bands are per-variant ─────────
//
// One model can carry several rig sidecars (asteroids: small/large/huge/
// cosmetic), and generate-lods requires them to AGREE on any generated file. So
// the near-model reference and both `[lod.generate]` blocks are written
// identically to every sidecar of the stem. Two things are allowed to — and
// must — differ between variants: the sphere, whose radius and scale come from
// each sidecar's own `[extents]` (a "large" rock gets a large stand-in and a
// "small" one a small); and the switch bands, which a bigger size class pushes
// proportionally further out (`bandMultiplier`, issue #947) because a LOD swap
// is an angular threshold and a huge rock reaches that on-screen size that much
// further away. The shared geometry never moves with either.

import { readdir, readFile, writeFile } from 'node:fs/promises';
import { statSync, existsSync } from 'node:fs';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { parse as parseToml } from 'smol-toml';
import { NodeIO } from '@gltf-transform/core';
import { getBounds } from '@gltf-transform/core';
import { replaceLadder, sidecarsForStem, variantOfSidecar } from './viewer-lods.mjs';
import { averageBaseColour, sharpMean } from './average-texture-colour.mjs';
import {
  remeshPath,
  blenderCandidates,
  capRemeshTextures,
  BLENDER_SCRIPT,
} from './generate-lods.mjs';

const execFileAsync = promisify(execFile);
const MB = 1024 * 1024;
const MODELS_DIR = 'assets/models';

/**
 * A voxel size in the model's OWN units for a decent silhouette: the largest
 * bounding-box dimension over ~90, so a remesh spans roughly 90 cells across
 * its longest axis regardless of how the art was exported. See the units caveat
 * in scripts/blender-voxel-remesh.py.
 */
function autoVoxel(document) {
  const box = getBounds(document.getRoot().listScenes()[0]);
  const dims = [box.max[0] - box.min[0], box.max[1] - box.min[1], box.max[2] - box.min[2]];
  const maxDim = Math.max(...dims);
  return Math.round((maxDim / 90) * 1e4) / 1e4;
}

/** Resolve a usable Blender executable, or throw with the same guidance generate-lods gives. */
async function resolveBlender() {
  const { readdir: rd } = await import('node:fs/promises');
  let installedDirs = [];
  const foundation = path.win32.join(process.env.ProgramFiles || 'C:\\Program Files', 'Blender Foundation');
  if (process.platform === 'win32' && existsSync(foundation)) installedDirs = await rd(foundation);
  for (const candidate of blenderCandidates({ env: process.env, installedDirs })) {
    try {
      await execFileAsync(candidate, ['--version']);
      return candidate;
    } catch {
      /* next */
    }
  }
  throw new Error('no Blender found — set PHOENIX_BLENDER or install it; needed for --remesh');
}

// ── Pure: the sphere stand-in from a model's extents ────────────────────────

/**
 * Radius + non-uniform scale that make a unit sphere into an ellipsoid matching
 * a model's `[extents].size`. The sphere is sized by the entity's `[mesh].radius`
 * times the level `scale` (the rig's base scale never touches a procedural
 * level), so this bakes the half-width into `radius` and the axis proportions
 * into `scale`: radius = sx/2, scale = [1, sy/sx, sz/sx].
 *
 * Falls back to a unit sphere when a sidecar has no usable extents.
 */
export function sphereFromExtents(size) {
  if (!Array.isArray(size) || size.length < 3) return { radius: 1, scale: [1, 1, 1] };
  const [sx, sy, sz] = size.map(Number);
  const width = sx > 0 ? sx : Math.max(sy, sz, 1);
  const round = (x) => Math.round(x * 1e4) / 1e4;
  return {
    radius: round(width / 2),
    scale: [1, round((sy || width) / width), round((sz || width) / width)],
  };
}

/**
 * How far a size variant's switch bands are pushed out, relative to the base
 * ladder. A LOD switch is really an angular threshold — "swap detail when this
 * thing gets small on screen" — so a variant authored at N× the scale reaches
 * that same on-screen size N× further away and must switch N× further out, or a
 * huge rock drops to its procedural far sphere while still filling a fifth of
 * the viewscreen (issue #947). Only the `huge` size class scales its bands; the
 * shared `.glb` geometry and its decimation ratios never move with it. Any
 * variant not listed here — small, large, cosmetic, the base rig — keeps the
 * base ladder.
 */
export const BAND_MULTIPLIERS = { huge: 3 };

/** The band multiplier for a variant name (`""` is the base rig), 1 if unlisted. */
export function bandMultiplier(variant) {
  return BAND_MULTIPLIERS[variant] ?? 1;
}

/**
 * Build the four-level ladder for one sidecar. `lod1Gen`/`lod2Gen` are the
 * complete `[lod.generate]` blocks (already mode-specific), so this function
 * only lays out the levels and never decides how they are made.
 *
 * The two generated levels are marked `tier_rig = "identity"` because this
 * script writes ONLY the primary sidecars of a stem (`sidecarsForStem`) and
 * never a sidecar beside a generated `.glb` — so by construction there is none
 * there for the renderer to read. Recorded here, at the one place that decides
 * it, rather than discovered later by fetching the file and getting a 404.
 */
export function buildLadder({ stem, near, mid, far, colour, sphere, lod1Gen, lod2Gen }) {
  return [
    { max_distance: near, model: `${MODELS_DIR}/${stem}.glb` },
    {
      max_distance: mid,
      model: `${MODELS_DIR}/${stem}_lod1.glb`,
      tier_rig: 'identity',
      generate: lod1Gen,
    },
    {
      max_distance: far,
      model: `${MODELS_DIR}/${stem}_lod2.glb`,
      tier_rig: 'identity',
      generate: lod2Gen,
    },
    { shape: 'sphere', colour, radius: sphere.radius, scale: sphere.scale },
  ];
}

// ── Impure: budget search for a decimated level ─────────────────────────────

function gltfCli() {
  const entry = 'node_modules/@gltf-transform/cli/bin/cli.js';
  if (!existsSync(entry)) throw new Error(`${entry} not found — run \`npm install\``);
  return [process.execPath, entry];
}

/**
 * Candidate (ratio, texture) pairs for a decimated level, best quality first.
 * `textures` and `ratios` are laddered so the search keeps the crispest texture
 * and the most geometry that still fits under budget.
 */
export function decimationCandidates(textures, ratios) {
  const out = [];
  for (const textureSize of textures) {
    for (const ratio of ratios) out.push({ ratio, error: 0.01, textureSize });
  }
  return out;
}

/**
 * Texture ladders the two decimated levels search, crispest first.
 *
 * Named rather than written inline at the two call sites because the voxel
 * remesh is cut BEFORE the search that picks the real sizes, and the cap
 * applied to that intermediate has to know the largest size the search could
 * still land on.
 */
export const LOD1_TEXTURES = [512, 384, 256];
export const LOD2_TEXTURES = [256, 128];

/**
 * The largest texture any level this script can author will ask for.
 *
 * Deliberately the loosest safe answer. `generate-lods.mjs` re-cuts the same
 * intermediate afterwards and caps it against the sizes actually authored
 * (`remeshTextureCap`), which is usually tighter — 256 for eight of the nine
 * remeshed models. This one runs before there is an authored size to read, so
 * it only has to avoid starving a level the search might still choose.
 */
export function candidateTextureCap() {
  return Math.max(...LOD1_TEXTURES, ...LOD2_TEXTURES);
}

/**
 * Search for the highest-quality (ratio, texture_size) whose simplify+resize of
 * `source` fits `budgetMb`. Runs the same two steps generate-lods would, into a
 * temp dir, and measures. Returns the winning params (adds nothing to disk).
 */
async function tuneLevel(cli, source, budgetMb, candidates, workDir, label) {
  const budgetBytes = budgetMb * MB;
  let smallest = null;
  for (const cand of candidates) {
    const simplified = path.join(workDir, `${label}_r${cand.ratio}.glb`);
    const out = path.join(workDir, `${label}_r${cand.ratio}_t${cand.textureSize}.glb`);
    if (!existsSync(simplified)) {
      await execFileAsync(cli[0], [
        ...cli.slice(1), 'simplify', source, simplified,
        '--ratio', String(cand.ratio), '--error', String(cand.error),
      ]);
    }
    await execFileAsync(cli[0], [
      ...cli.slice(1), 'resize', simplified, out,
      '--width', String(cand.textureSize), '--height', String(cand.textureSize),
    ]);
    const bytes = statSync(out).size;
    if (smallest === null || bytes < smallest.bytes) smallest = { cand, bytes };
    if (bytes <= budgetBytes) return { ...cand, bytes };
  }
  // Nothing fit: hand back the smallest so the caller can report/escalate.
  return { ...smallest.cand, bytes: smallest.bytes, overBudget: true };
}

// ── Orchestration ───────────────────────────────────────────────────────────

export async function authorLadders(stem, opts) {
  const {
    near = 15, mid = 100, far = 400,
    lod1Mb = 2, lod2Mb = 0.5, remesh = false, remeshVoxel = null,
  } = opts;
  const base = `${MODELS_DIR}/${stem}.glb`;
  if (!existsSync(base)) throw new Error(`base model not found: ${base} — run optimise-base first`);

  // Hull colour for the sphere, once per stem.
  const document = await new NodeIO().read(base);
  const colour = await averageBaseColour(document, sharpMean);

  // A stubborn hard-surface hull will not decimate (meshoptimizer stalls on the
  // split vertices at every hard edge), so the far levels are cut from a
  // Blender voxel remesh instead — a watertight surface that reduces cleanly.
  // The remesh is written to the source's `.remesh.glb` (the path generate-lods
  // reads for a level with `remesh_voxel_size`), so the tuning below and the
  // final `node scripts/generate-lods.mjs <stem>` decimate the same intermediate
  // and Blender runs exactly once.
  let voxel = null;
  let effectiveSource = base;
  if (remesh) {
    voxel = remeshVoxel != null ? remeshVoxel : autoVoxel(document);
    const blender = await resolveBlender();
    effectiveSource = remeshPath(base);
    await execFileAsync(blender, [
      '--background', '--factory-startup', '--python', BLENDER_SCRIPT, '--',
      base, effectiveSource, String(voxel),
    ]);
    // The remesh is a geometry pass, so Blender carries the base's full-size
    // materials onto it — resolution no level cut from it will ever read. Cap
    // it here as well as in generate-lods so the budget search below measures
    // the same intermediate the final `generate-lods.mjs <stem>` decimates.
    await capRemeshTextures(effectiveSource, candidateTextureCap());
  }

  // Tune the two decimated levels against their budgets, once per stem. A remesh
  // reduces predictably, so it can keep a higher ratio; a raw source needs the
  // aggressive ladder that at least tries before reporting it cannot fit.
  const cli = gltfCli();
  const workDir = await mkdtemp(path.join(tmpdir(), `phoenix-ladder-${stem}-`));
  let lod1, lod2;
  try {
    lod1 = await tuneLevel(
      cli, effectiveSource, lod1Mb,
      decimationCandidates(LOD1_TEXTURES, remesh ? [0.95, 0.8, 0.6, 0.45, 0.3] : [0.5, 0.35, 0.25, 0.18, 0.12]),
      workDir, 'lod1',
    );
    lod2 = await tuneLevel(
      cli, effectiveSource, lod2Mb,
      decimationCandidates(LOD2_TEXTURES, remesh ? [0.25, 0.15, 0.1, 0.06, 0.04] : [0.08, 0.05, 0.03, 0.02]),
      workDir, 'lod2',
    );
  } finally {
    await rm(workDir, { recursive: true, force: true });
  }

  // Write the ladder into every sidecar of the stem; the sphere is per-variant.
  // `sidecarsForStem` matches on bare filenames, so feed it those and prefix
  // the directory back on when touching disk.
  const files = await readdir(MODELS_DIR);
  const sidecars = sidecarsForStem(files, stem).map((f) => `${MODELS_DIR}/${f}`);
  if (!sidecars.length) throw new Error(`no rig sidecar found for stem "${stem}"`);

  // A decimated `[lod.generate]` block: simplify the base (or its voxel remesh,
  // when one was cut) to `ratio`, then resize textures. `remesh_voxel_size` is
  // carried only when the far levels were cut from a remesh, so generate-lods
  // reads the same intermediate this run produced.
  const genOf = (level) => {
    const g = {
      source: base,
      ratio: level.ratio,
      error: level.error,
      texture_size: level.textureSize,
    };
    if (voxel != null) g.remesh_voxel_size = voxel;
    return g;
  };

  for (const rel of sidecars) {
    const text = await readFile(rel, 'utf8');
    const doc = parseToml(text);
    const sphere = sphereFromExtents(doc?.extents?.size);
    // The bands are the one thing that DOES differ per variant besides the
    // sphere: a bigger size class switches proportionally further out. Derive
    // them as base × the variant's multiplier so a huge sidecar reads 45/300/
    // 1200 without any variant hardcoding a distance (issue #947).
    const mult = bandMultiplier(variantOfSidecar(path.basename(rel), stem));
    const ladder = buildLadder({
      stem, near: near * mult, mid: mid * mult, far: far * mult, colour, sphere,
      lod1Gen: genOf(lod1),
      lod2Gen: genOf(lod2),
    });
    await writeFile(rel, replaceLadder(text, ladder));
  }

  return { stem, colour, lod1, lod2, sidecars, voxel };
}

async function main() {
  const args = process.argv.slice(2);
  const stem = args.find((a) => !a.startsWith('--'));
  if (!stem) {
    console.error('usage: node scripts/author-ladders.mjs <stem> [--lod1-mb 2] [--lod2-mb 0.5] [--remesh] [--remesh-voxel <s>]');
    process.exit(2);
  }
  const num = (flag, dflt) => {
    const i = args.indexOf(flag);
    return i !== -1 ? Number(args[i + 1]) : dflt;
  };
  // `--remesh-voxel <s>` implies `--remesh` with an explicit voxel; `--remesh`
  // alone auto-sizes from the bounding box.
  const remeshVoxel = args.includes('--remesh-voxel') ? num('--remesh-voxel', null) : null;
  const result = await authorLadders(stem, {
    near: num('--near', 15), mid: num('--mid', 100), far: num('--far', 400),
    lod1Mb: num('--lod1-mb', 2), lod2Mb: num('--lod2-mb', 0.5),
    remesh: args.includes('--remesh') || remeshVoxel != null,
    remeshVoxel,
  });
  const mb = (b) => `${(b / MB).toFixed(2)} MB`;
  console.error(
    `[author-ladders] ${stem}: colour [${result.colour.join(', ')}]` +
      `${result.voxel != null ? `  (voxel remesh ${result.voxel})` : ''}\n` +
      `  lod1 ratio ${result.lod1.ratio} @ ${result.lod1.textureSize}px → ${mb(result.lod1.bytes)}` +
      `${result.lod1.overBudget ? ' (OVER budget — consider --remesh-voxel)' : ''}\n` +
      `  lod2 ratio ${result.lod2.ratio} @ ${result.lod2.textureSize}px → ${mb(result.lod2.bytes)}` +
      `${result.lod2.overBudget ? ' (OVER budget — consider --remesh-voxel)' : ''}\n` +
      `  wrote ${result.sidecars.length} sidecar(s): ${result.sidecars.join(', ')}\n` +
      `  now run: node scripts/generate-lods.mjs ${stem}`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error(`[author-ladders] ${err.message}`);
    process.exit(1);
  });
}
