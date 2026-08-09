// optimise-base.mjs — turn a raw PBR export into a shippable base .glb under a
// byte budget, preserving as much of the hero silhouette as the budget allows.
//
//   node scripts/optimise-base.mjs <input.glb> <output.glb> [--budget-mb 8]
//   node scripts/optimise-base.mjs <in> <out> --budget-mb 8 --plan   # search only, no write
//
// This is the base-level companion to scripts/generate-lods.mjs: that script
// builds the FAR levels of a model's ladder (decimated _lod1/_lod2), this one
// builds level 0 — the full-detail model the ladder is derived from — from the
// raw art export. It is a local, out-of-tree step (the raw art is not committed;
// only its optimised output under assets/models is), so it takes explicit
// input/output paths rather than knowing where the raw tree lives.
//
// ── The budget, and why textures go before geometry ─────────────────────────
//
// The base level is what the camera sees from 0 to its first `max_distance`
// (15 units for the ships here) — close range, where the SILHOUETTE is what a
// player reads. So the search keeps the mesh at full resolution and shrinks
// TEXTURES first: base-colour down a ladder, and the normal / metallic-roughness
// maps a notch smaller again, since those carry lower-frequency detail. Only
// when the smallest acceptable texture set still overflows does it start
// decimating geometry, and only then does it escalate the budget (8 → 12 → 18
// MB, the +50%-then-+50% the art direction allows) rather than melt the hull to
// fit a number.
//
// ── Why textures are resized in-process, not via `gltf-transform resize` ─────
//
// The CLI's `resize --pattern` matches a texture by NAME, and a name is whatever
// the exporter wrote ("texture_diffuse" here, but not guaranteed across every
// art source). The base-colour vs. everything-else split has to be exact — a
// normal map resized as if it were base colour, or vice versa, is a silent
// quality bug — so the split is made on the glTF SLOT (which material channel
// points at the texture) using @gltf-transform/core, and sharp does the actual
// resample. Geometry still goes through the CLI (meshoptimizer's weld/simplify),
// the same tool the LOD generator uses, so the two stages agree on the mesh.
//
// Textures stay PNG throughout (never WebP/KTX2): Bevy's glTF loader rejects
// EXT_texture_webp, the same reason scripts/…/optimise.ps1 converts to PNG.

import { NodeIO, getBounds } from '@gltf-transform/core';
import sharp from 'sharp';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { statSync, existsSync } from 'node:fs';
import { mkdtemp, rm, copyFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { blenderCandidates, BLENDER_SCRIPT } from './generate-lods.mjs';

const execFileAsync = promisify(execFile);
const MB = 1024 * 1024;

/**
 * A voxel size in the input's own units for a base-level remesh: the largest
 * bounding-box dimension over ~130, finer than a far LOD's remesh because the
 * base is seen up close. Organic bodies (asteroids, moons) whose raw export is
 * a half-million-triangle stall decimate cleanly off this; hard-surface hulls
 * do NOT want it (it melts panel edges) and pass through un-remeshed.
 */
function autoVoxel(document) {
  const box = getBounds(document.getRoot().listScenes()[0]);
  const dims = [box.max[0] - box.min[0], box.max[1] - box.min[1], box.max[2] - box.min[2]];
  return Math.round((Math.max(...dims) / 130) * 1e4) / 1e4;
}

async function resolveBlender() {
  let installedDirs = [];
  const foundation = path.win32.join(process.env.ProgramFiles || 'C:\\Program Files', 'Blender Foundation');
  if (process.platform === 'win32' && existsSync(foundation)) installedDirs = await readdir(foundation);
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

// ── Pure: the search ladder ─────────────────────────────────────────────────

/**
 * Candidate builds, best quality first.
 *
 * Outer loop is geometry (keep the whole mesh as long as possible); inner loop
 * walks textures down. `bc` caps the base-colour map, `aux` the normal /
 * metallic-roughness maps. A caller stops at the first candidate that fits the
 * budget, so ordering *is* the quality policy.
 */
export function candidates() {
  const textureLadder = [
    { bc: 2048, aux: 1024 },
    { bc: 1024, aux: 1024 },
    { bc: 1024, aux: 512 },
    { bc: 768, aux: 512 },
    { bc: 512, aux: 512 },
    { bc: 512, aux: 256 },
  ];
  const ratios = [1.0, 0.75, 0.5];
  const out = [];
  for (const ratio of ratios) {
    for (const tex of textureLadder) {
      out.push({ ratio, bc: tex.bc, aux: tex.aux });
    }
  }
  return out;
}

/** The budget escalation the art direction allows: 8 MB, then +50%, then +50%. */
export function budgetLadder(baseMb) {
  return [baseMb, baseMb * 1.5, baseMb * 1.5 * 1.5];
}

/** A stable key for a candidate, so identical geometry ratios reuse one build. */
export function candidateKey(c) {
  return `r${c.ratio}_bc${c.bc}_aux${c.aux}`;
}

// ── Impure: geometry (CLI) and textures (core + sharp) ──────────────────────

function gltfCli() {
  const entry = 'node_modules/@gltf-transform/cli/bin/cli.js';
  if (!existsSync(entry)) {
    throw new Error(`${entry} not found — run \`npm install\` first`);
  }
  return [process.execPath, entry];
}

/** weld → dedup → prune: candidate-independent cleanup, run once. */
async function cleanupGeometry(cli, input, output) {
  const tmp = `${output}.weld.glb`;
  const tmp2 = `${output}.dedup.glb`;
  await execFileAsync(cli[0], [...cli.slice(1), 'weld', input, tmp]);
  await execFileAsync(cli[0], [...cli.slice(1), 'dedup', tmp, tmp2]);
  await execFileAsync(cli[0], [...cli.slice(1), 'prune', tmp2, output]);
  await rm(tmp, { force: true });
  await rm(tmp2, { force: true });
}

/** meshoptimizer simplify to a vertex ratio (weld already done upstream). */
async function simplifyGeometry(cli, input, output, ratio) {
  await execFileAsync(cli[0], [
    ...cli.slice(1),
    'simplify',
    input,
    output,
    '--ratio',
    String(ratio),
    '--error',
    '0.01',
  ]);
}

/**
 * Resize a document's textures in place by SLOT: base-colour maps to `bc`,
 * every other map (normal, metallic-roughness, emissive, occlusion) to `aux`.
 * Never enlarges. Re-encodes PNG so Bevy's loader accepts the result.
 */
async function resizeTexturesBySlot(document, { bc, aux }) {
  const baseColour = new Set();
  for (const material of document.getRoot().listMaterials()) {
    const t = material.getBaseColorTexture();
    if (t) baseColour.add(t);
  }
  for (const texture of document.getRoot().listTextures()) {
    const image = texture.getImage();
    if (!image) continue;
    const cap = baseColour.has(texture) ? bc : aux;
    const resized = await sharp(Buffer.from(image))
      .resize(cap, cap, { fit: 'inside', withoutEnlargement: true })
      .png()
      .toBuffer();
    texture.setImage(new Uint8Array(resized));
    texture.setMimeType('image/png');
  }
}

/**
 * Build one candidate to `outPath` and return its size in bytes. `geomCache`
 * maps a ratio to an already-simplified geometry file so repeated texture
 * candidates over the same ratio do not re-run meshoptimizer.
 */
async function buildCandidate(cli, cleaned, candidate, outPath, workDir, geomCache) {
  let geom = geomCache.get(candidate.ratio);
  if (!geom) {
    if (candidate.ratio >= 1.0) {
      geom = cleaned;
    } else {
      geom = path.join(workDir, `geom_r${candidate.ratio}.glb`);
      await simplifyGeometry(cli, cleaned, geom, candidate.ratio);
    }
    geomCache.set(candidate.ratio, geom);
  }
  const io = new NodeIO();
  const document = await io.read(geom);
  await resizeTexturesBySlot(document, candidate);
  await io.write(outPath, document);
  return statSync(outPath).size;
}

// ── Orchestration ───────────────────────────────────────────────────────────

/**
 * Search candidates against the escalating budget and return the winner.
 * `{ candidate, bytes, budgetMb }`. Throws only if even the coarsest build at
 * the highest budget overflows — a genuinely oversized model that needs a look.
 */
export async function optimiseBase(input, output, { budgetMb = 8, plan = false, remesh = false, remeshVoxel = null } = {}) {
  const cli = gltfCli();
  const workDir = await mkdtemp(path.join(tmpdir(), 'phoenix-base-'));
  try {
    // Organic bodies stall meshoptimizer (a half-million split-vertex triangles
    // collapse to nothing), so voxel-remesh the raw into a clean watertight
    // surface FIRST; the search below then decimates it predictably. Hard-surface
    // hulls skip this and keep their real geometry.
    let source = input;
    let voxel = null;
    if (remesh) {
      const doc = await new NodeIO().read(input);
      voxel = remeshVoxel != null ? remeshVoxel : autoVoxel(doc);
      const blender = await resolveBlender();
      source = path.join(workDir, 'remeshed.glb');
      await execFileAsync(blender, [
        '--background', '--factory-startup', '--python', BLENDER_SCRIPT, '--',
        input, source, String(voxel),
      ]);
    }
    const cleaned = path.join(workDir, 'cleaned.glb');
    await cleanupGeometry(cli, source, cleaned);

    const geomCache = new Map();
    const cands = candidates();
    const budgets = budgetLadder(budgetMb);
    const measured = new Map(); // key → { candidate, bytes }
    const scratch = path.join(workDir, 'candidate.glb');

    for (const budget of budgets) {
      const budgetBytes = budget * MB;
      for (const candidate of cands) {
        const key = candidateKey(candidate);
        let entry = measured.get(key);
        if (!entry) {
          const bytes = await buildCandidate(cli, cleaned, candidate, scratch, workDir, geomCache);
          entry = { candidate, bytes, file: `${scratch}.${key}` };
          await copyFile(scratch, entry.file);
          measured.set(key, entry);
        }
        if (entry.bytes <= budgetBytes) {
          if (!plan) await copyFile(entry.file, output);
          return { candidate, bytes: entry.bytes, budgetMb: budget, voxel };
        }
      }
    }
    // Nothing fit even the top budget — hand back the smallest we made so the
    // caller can report it rather than silently ship nothing.
    const smallest = [...measured.values()].sort((a, b) => a.bytes - b.bytes)[0];
    if (!plan && smallest) await copyFile(smallest.file, output);
    throw Object.assign(
      new Error(
        `no candidate fits ${budgets[budgets.length - 1].toFixed(1)} MB; ` +
          `smallest is ${(smallest.bytes / MB).toFixed(2)} MB (${candidateKey(smallest.candidate)})`,
      ),
      { smallest },
    );
  } finally {
    await rm(workDir, { recursive: true, force: true });
  }
}

async function main() {
  const args = process.argv.slice(2);
  const positional = args.filter((a) => !a.startsWith('--'));
  const [input, output] = positional;
  if (!input || !output) {
    console.error('usage: node scripts/optimise-base.mjs <input.glb> <output.glb> [--budget-mb 8] [--plan] [--remesh] [--remesh-voxel <s>]');
    process.exit(2);
  }
  const budgetIdx = args.indexOf('--budget-mb');
  const budgetMb = budgetIdx !== -1 ? Number(args[budgetIdx + 1]) : 8;
  const plan = args.includes('--plan');
  const voxIdx = args.indexOf('--remesh-voxel');
  const remeshVoxel = voxIdx !== -1 ? Number(args[voxIdx + 1]) : null;
  const remesh = args.includes('--remesh') || remeshVoxel != null;

  const before = existsSync(input) ? statSync(input).size : null;
  const result = await optimiseBase(input, output, { budgetMb, plan, remesh, remeshVoxel });
  const c = result.candidate;
  console.error(
    `[optimise-base] ${input} → ${output}\n` +
      `  ${before !== null ? (before / MB).toFixed(2) + ' MB → ' : ''}${(result.bytes / MB).toFixed(2)} MB ` +
      `(budget ${result.budgetMb.toFixed(0)} MB)  base-colour ${c.bc}px, aux ${c.aux}px, geometry ratio ${c.ratio}` +
      `${result.voxel != null ? `, voxel remesh ${result.voxel}` : ''}` +
      (plan ? '  [plan — not written]' : ''),
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error(`[optimise-base] ${err.message}`);
    process.exit(1);
  });
}
