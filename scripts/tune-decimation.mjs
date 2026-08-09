// tune-decimation.mjs — the INVERSE of tune-lods: keep a model's LOD switch
// ranges fixed and tune each generated level's mesh SIMPLIFICATION instead.
//
//   node scripts/tune-decimation.mjs <stem…> [--variant v] [--dry-run]
//        [--out dir] [--bin path] [--yaws N] [--resolution WxH]
//        [--steps N] [--ratio-max r] [--ratio-min r] [--generate]
//
// ── Why this exists ─────────────────────────────────────────────────────────
//
// The range tuner (scripts/tune-lods.mjs) moved the switch DISTANCES for a fixed
// set of meshes, and it did not pan out: adjacent GLB LODs are too visually
// similar at their switch distance to yield a monotonic diff-vs-distance ladder,
// so the knee wandered. Inverting the problem is well-posed. Fix the ranges (the
// ladder's existing `max_distance` values, which a designer set) and search each
// generated level's simplification for the most-aggressively-decimated mesh that
// is still perceptually indistinguishable from the BASE at the closest distance
// that level is ever shown — its band's near edge (the previous level's
// `max_distance`). That produces smaller, cheaper LODs, quality-bounded, and
// slots straight into the existing `[lod.generate]` → generate-lods → manifest
// pipeline: this driver only rewrites the `ratio`/`texture_size` a level already
// declares.
//
// ── The method, per generated level ─────────────────────────────────────────
//
//   1. near-edge d = the PREVIOUS level's `max_distance`, read from the sidecar.
//   2. Decimate a grid of candidate meshes from the level's own source (the
//      voxel-remesh intermediate when it declares one), light→heavy, with the
//      same simplify+resize steps generate-lods runs (`planSteps`).
//   3. Call the `tune-lods` bin ONCE in --decimate mode: it renders the base and
//      every candidate at d, multi-yaw, worst-case, and returns the per-candidate
//      alpha-aware diff plus the knee of that (convex, increasing) curve.
//   4. Choose the knee — the most aggressive candidate still close to the base —
//      subject to a byte CEILING (lod1 ≤ 2 MB, lod2 ≤ 0.5 MB). The ceiling is a
//      cap, not a target: among acceptable-quality candidates prefer the smaller,
//      but never exceed the cap and, where possible, never decimate past the knee
//      just to shrink.
//   5. Write the chosen ratio/texture back through `replaceLadder` (the same pure
//      helper the viewer's LOD panel uses) — every variant of the stem together —
//      and, with --generate, run generate-lods to rebuild the .glb + manifest.
//
// ── Dry run ─────────────────────────────────────────────────────────────────
//
// `--dry-run` prints the current vs. proposed ratio/texture/size per level and
// writes the review artifacts (diff-vs-decimation curve, ref-vs-knee montage)
// the bin produces, but touches no sidecar. It is the mode the tracer uses.
//
// ── Candidates live under assets/ on purpose ────────────────────────────────
//
// Bevy's asset server refuses to load a GLB outside its asset root, so the
// candidate meshes are written to `assets/models/.lodtune/<label>/` and removed
// when the run ends (the leading dot keeps them out of the shipped-sidecar
// assertions, which only look at `.toml`). Nothing here is ever committed.

import { readFile, writeFile, readdir, mkdir, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';
import os from 'node:os';
import { parse as parseToml } from 'smol-toml';

import {
  modelStem,
  sidecarsForStem,
  variantOfSidecar,
  ladderFromDoc,
  replaceLadder,
  validateLadder,
  validateProposal,
} from './viewer-lods.mjs';
import { planSteps, remeshPath } from './generate-lods.mjs';

const execFileAsync = promisify(execFile);
const ROOT = process.cwd();
const MODELS_DIR = path.join(ROOT, 'assets', 'models');
const CANDIDATE_DIR = 'assets/models/.lodtune';
const GLTF_CLI = ['node_modules', '@gltf-transform', 'cli', 'bin', 'cli.js'].join('/');

/** Byte ceilings per generated level. A cap, never a target. */
const CAP_LOD1 = 2 * 1024 * 1024;
const CAP_LOD2 = 0.5 * 1024 * 1024;

function parseArgs(argv) {
  const flags = new Map();
  const positional = [];
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--dry-run') flags.set('dry-run', true);
    else if (a === '--generate') flags.set('generate', true);
    else if (a.startsWith('--')) {
      flags.set(a.slice(2), argv[i + 1]);
      i += 1;
    } else positional.push(a);
  }
  return { flags, positional };
}

/** Round to 3 significant-ish decimals — as precise as a ratio ever needs. */
function round3(x) {
  return Math.round(x * 1000) / 1000;
}

function formatBytes(bytes) {
  if (bytes === null || bytes === undefined) return '—';
  return bytes >= 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(2)} MB`
    : `${(bytes / 1024).toFixed(1)} KB`;
}

/**
 * A 1-D ladder of decimation candidates, light→heavy. Both knobs step down
 * together so the ladder is monotone in aggressiveness (and therefore monotone
 * in output size and in render diff), which is what makes the diff-vs-candidate
 * curve convex with a single clean knee. `ratio` sweeps geometrically; the
 * `texture_size` steps down across the three standard sizes so the far, heavier
 * candidates also shed texture bytes — the only way lod2's 0.5 MB cap is
 * reachable, since texture, not vertex count, dominates a hull GLB's size.
 */
export function buildGrid({
  ratioMax = 0.9,
  ratioMin = 0.04,
  steps = 7,
  textures = [512, 256, 128],
  error = 0.01,
} = {}) {
  const grid = [];
  for (let i = 0; i < steps; i += 1) {
    const t = steps > 1 ? i / (steps - 1) : 0;
    const ratio = round3(ratioMax * (ratioMin / ratioMax) ** t);
    const ti = Math.min(textures.length - 1, Math.floor(t * textures.length));
    grid.push({ ratio, texture_size: textures[ti], error });
  }
  return grid;
}

/** The byte cap for a generated level, by output filename then by order. */
export function capFor(output, genOrder) {
  if (/_lod1\.glb$/i.test(output)) return CAP_LOD1;
  if (/_lod2\.glb$/i.test(output)) return CAP_LOD2;
  return genOrder === 0 ? CAP_LOD1 : CAP_LOD2;
}

/**
 * Choose a candidate given the bin's knee and the measured candidate sizes.
 *
 * Candidates are light→heavy, so `bytes` descends. The knee is the most
 * aggressive candidate still perceptually close to the base — the pick, when it
 * fits the cap. When the knee is over the cap the ceiling wins: advance to
 * heavier (smaller) candidates until one fits, flagged so a reviewer sees the
 * budget forced the choice past the knee. With no knee at all, keep the authored
 * parameters rather than guess.
 */
export function chooseCandidate({ kneeIndex, kneeFound, bytes, cap }) {
  if (!kneeFound || kneeIndex < 0) return { index: null, reason: 'no-knee' };
  if (bytes[kneeIndex] <= cap) return { index: kneeIndex, reason: 'knee' };
  for (let j = kneeIndex + 1; j < bytes.length; j += 1) {
    if (bytes[j] <= cap) return { index: j, reason: 'budget-past-knee' };
  }
  return { index: bytes.length - 1, reason: 'over-budget' };
}

/** Run the bin once for one level and return its parsed decimate JSON. */
function runBin(modelPath, variant, candidates, distance, label, flags) {
  const bin = flags.get('bin');
  const decimateArgs = [
    modelPath,
    '--variant', variant,
    '--decimate',
    '--candidates', candidates.join(','),
    '--distance', String(distance),
    '--label', label,
  ];
  const args = bin
    ? decimateArgs
    : ['run', '-q', '--features', 'capture', '--bin', 'tune-lods', '--', ...decimateArgs];
  for (const key of ['resolution', 'yaws', 'pitch', 'out']) {
    if (flags.get(key)) args.push(`--${key}`, flags.get(key));
  }
  const cmd = bin || 'cargo';
  const res = spawnSync(cmd, args, { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  if (res.status !== 0) {
    throw new Error(`tune-lods --decimate failed for ${label}:\n${res.stderr || res.stdout}`);
  }
  const line = res.stdout
    .split('\n')
    .map((l) => l.trim())
    .reverse()
    .find((l) => l.startsWith('{') && l.includes('"candidates"'));
  if (!line) throw new Error(`no decimate JSON from tune-lods for ${label}`);
  return JSON.parse(line);
}

/**
 * Generate the candidate meshes for one level under assets/, returning
 * `[{ path, params, bytes }]` in light→heavy order. Reuses generate-lods'
 * `planSteps`, so a candidate is decimated exactly as the shipped pipeline would
 * decimate it from the same effective source.
 */
async function generateCandidates(effectiveSource, grid, label) {
  const outDir = path.join(ROOT, CANDIDATE_DIR, label);
  await mkdir(outDir, { recursive: true });
  const cli = [process.execPath, path.join(ROOT, GLTF_CLI)];
  const out = [];
  for (let i = 0; i < grid.length; i += 1) {
    const params = grid[i];
    const rel = `${CANDIDATE_DIR}/${label}/c${i}.glb`;
    const target = {
      effectiveSource,
      output: rel,
      params: { ratio: params.ratio, error: params.error, textureSize: params.texture_size },
    };
    const tmpDir = path.join(os.tmpdir(), `phoenix-lodtune-${label}-${i}`);
    await mkdir(tmpDir, { recursive: true });
    try {
      for (const step of planSteps(target, { cli, tmpDir: tmpDir.split(path.sep).join('/') })) {
        await execFileAsync(step.argv[0], step.argv.slice(1));
      }
    } finally {
      await rm(tmpDir, { recursive: true, force: true });
    }
    const bytes = (await stat(path.join(ROOT, rel))).size;
    out.push({ path: rel, params, bytes });
  }
  return out;
}

async function fileBytes(rel) {
  try {
    return (await stat(path.join(ROOT, rel))).size;
  } catch {
    return null;
  }
}

async function main() {
  const { flags, positional } = parseArgs(process.argv.slice(2));
  const dryRun = flags.get('dry-run') === true;
  const doGenerate = flags.get('generate') === true;
  if (!positional.length) {
    console.error(
      'usage: node scripts/tune-decimation.mjs <stem…> [--variant v] [--dry-run] ' +
        '[--out dir] [--bin path] [--yaws N] [--resolution WxH] [--steps N] [--generate]',
    );
    process.exit(2);
  }

  const gridOpts = {};
  if (flags.get('steps')) gridOpts.steps = Number(flags.get('steps'));
  if (flags.get('ratio-max')) gridOpts.ratioMax = Number(flags.get('ratio-max'));
  if (flags.get('ratio-min')) gridOpts.ratioMin = Number(flags.get('ratio-min'));

  const files = await readdir(MODELS_DIR);
  for (const arg of positional) {
    const stem = modelStem(arg);
    const modelPath = `assets/models/${stem}.glb`;
    let sidecars = sidecarsForStem(files, stem);
    if (flags.get('variant')) {
      const want = flags.get('variant') === 'model' ? '' : flags.get('variant');
      sidecars = sidecars.filter((f) => variantOfSidecar(f, stem) === want);
    }
    if (!sidecars.length) {
      console.error(`[tune-decimation] no rig sidecars for ${stem}`);
      continue;
    }

    // Parse every variant's ladder up front. A generated `_lodN.glb` is SHARED
    // by all of a stem's variants, but each variant shows it at its own near
    // edge (the huge asteroid's bands are the large's ×3). generate-lods rejects
    // a tree where two sidecars declare one output with different params, so a
    // shared output must be tuned ONCE — at the smallest (most demanding, closest
    // on screen) near edge across its variants — and identical params written to
    // every variant. Tuning per-variant would either be refused or corrupt the
    // tree; hence the two passes below.
    const parsed = [];
    for (const file of sidecars) {
      const current = await readFile(path.join(MODELS_DIR, file), 'utf8');
      parsed.push({
        file,
        variant: variantOfSidecar(file, stem) || 'model',
        current,
        levels: ladderFromDoc(parseToml(current)),
      });
    }

    // Pass 1: collect each shared generated output and the variant that shows it
    // closest.
    const outputs = new Map();
    for (const p of parsed) {
      let g = 0;
      p.levels.forEach((level, i) => {
        if (!(level.generate && level.model)) return;
        const nearEdge = p.levels[i - 1]?.max_distance;
        const source = level.generate.source ?? p.levels.find((l) => l.model)?.model;
        const effectiveSource =
          level.generate.remesh_voxel_size != null ? remeshPath(source) : source;
        let e = outputs.get(level.model);
        if (!e) {
          e = {
            output: level.model,
            effectiveSource,
            error: level.generate.error,
            ratio: level.generate.ratio ?? null,
            texture_size: level.generate.texture_size ?? null,
            cap: capFor(level.model, g),
            minNearEdge: Infinity,
            minVariant: p.variant,
          };
          outputs.set(level.model, e);
        }
        if (nearEdge > 0 && nearEdge < e.minNearEdge) {
          e.minNearEdge = nearEdge;
          e.minVariant = p.variant;
        }
        g += 1;
      });
    }

    // Pass 2: tune each shared output once, at its minimum near edge.
    const chosen = new Map();
    for (const out of outputs.values()) {
      if (!(out.minNearEdge > 0 && out.minNearEdge < Infinity)) {
        console.log(`\n=== ${out.output}: no near-edge distance in any variant; skipped ===`);
        continue;
      }
      const label = `${stem}_${path.basename(out.output, '.glb')}`;
      // Candidates keep the level's authored `error` — it is not a tuned knob,
      // so regenerating later with generate-lods reproduces the tuned mesh.
      const grid = buildGrid({ ...gridOpts, error: out.error ?? undefined });
      const candidates = await generateCandidates(out.effectiveSource, grid, label);
      const proposal = runBin(
        modelPath,
        out.minVariant,
        candidates.map((c) => c.path),
        out.minNearEdge,
        label,
        flags,
      );
      const bytes = candidates.map((c) => c.bytes);
      const choice = chooseCandidate({
        kneeIndex: proposal.knee_index,
        kneeFound: proposal.knee_found,
        bytes,
        cap: out.cap,
      });

      const curBytes = await fileBytes(out.output);
      console.log(
        `\n=== ${path.basename(out.output)}  near-edge d=${out.minNearEdge} (${out.minVariant})  cap=${formatBytes(out.cap)} ===`,
      );
      console.log(`    current : ratio=${out.ratio} texture=${out.texture_size}  size=${formatBytes(curBytes)}`);
      const cand = candidates.map((c, k) => {
        const diff = proposal.candidates[k]?.diff ?? 0;
        const mark = k === proposal.knee_index ? ' ←knee' : '';
        return `      c${k}: ratio=${c.params.ratio} tex=${c.params.texture_size} ` +
          `size=${formatBytes(c.bytes)} diff=${diff.toFixed(5)}${mark}`;
      });
      console.log(`    grid (light→heavy):\n${cand.join('\n')}`);

      if (choice.index === null) {
        console.log(`    proposed: (no knee found — keeping authored ratio/texture)`);
        chosen.set(out.output, null);
      } else {
        const c = candidates[choice.index];
        const diff = proposal.candidates[choice.index]?.diff ?? 0;
        const saving = curBytes != null ? curBytes - c.bytes : null;
        console.log(
          `    proposed: ratio=${c.params.ratio} texture=${c.params.texture_size}  ` +
            `size=${formatBytes(c.bytes)}  diff@choice=${diff.toFixed(5)}  ` +
            `[${choice.reason}]  saving=${saving != null ? formatBytes(saving) : '—'}`,
        );
        chosen.set(out.output, { ratio: c.params.ratio, texture_size: c.params.texture_size });
      }
      await rm(path.join(ROOT, CANDIDATE_DIR, label), { recursive: true, force: true });
    }

    // Pass 3: fold the chosen params into every variant's matching level. Only
    // `ratio`/`texture_size` move; `error`, `source`, `remesh_voxel_size` and all
    // other fields are preserved, and every variant gets the SAME params.
    const proposed = [];
    for (const p of parsed) {
      const newLevels = p.levels.map((l) => ({
        ...l,
        generate: l.generate ? { ...l.generate } : l.generate,
      }));
      let touched = false;
      newLevels.forEach((level) => {
        if (!(level.generate && level.model)) return;
        const c = chosen.get(level.model);
        if (c) {
          level.generate.ratio = c.ratio;
          level.generate.texture_size = c.texture_size;
          touched = true;
        }
      });
      if (!touched && !outputs.size) continue;
      const problems = validateLadder(newLevels);
      if (problems.length) console.log(`  ⚠ ${p.file} ladder invalid: ${problems.join('; ')}`);
      proposed.push({
        path: `assets/models/${p.file}`,
        text: replaceLadder(p.current, newLevels),
        current: p.current,
        problems,
      });
    }

    if (!proposed.length) {
      console.error(`[tune-decimation] ${stem}: no sidecar carried a generated level`);
      continue;
    }

    const proposalErrors = validateProposal(proposed.map(({ path: p, text }) => ({ path: p, text })));
    if (proposalErrors.length) {
      console.log(`\n[tune-decimation] ${stem}: proposal rejected:\n  ${proposalErrors.join('\n  ')}`);
    }

    // Tidy the candidate scratch dir for this stem.
    await rm(path.join(ROOT, CANDIDATE_DIR), { recursive: true, force: true });

    if (dryRun) {
      console.log(`\n[tune-decimation] ${stem}: DRY RUN — nothing written.`);
      continue;
    }
    if (proposed.some((p) => p.problems.length) || proposalErrors.length) {
      console.error(`\n[tune-decimation] ${stem}: refusing to write an invalid ladder.`);
      process.exitCode = 1;
      continue;
    }
    const written = [];
    for (const item of proposed) {
      if (item.text === item.current) continue;
      await writeFile(path.join(ROOT, item.path), item.text);
      written.push(item.path);
    }
    console.log(`\n[tune-decimation] ${stem}: wrote ${written.length} sidecar(s): ${written.join(', ') || '(none changed)'}`);

    if (doGenerate && written.length) {
      console.log(`[tune-decimation] ${stem}: rebuilding .glb + manifest…`);
      const res = spawnSync(process.execPath, ['scripts/generate-lods.mjs', stem], {
        cwd: ROOT,
        encoding: 'utf8',
        stdio: 'inherit',
      });
      if (res.status !== 0) process.exitCode = 1;
    }
  }
}

main().catch(async (err) => {
  await rm(path.join(ROOT, CANDIDATE_DIR), { recursive: true, force: true }).catch(() => {});
  console.error(err.stack || String(err));
  process.exit(1);
});
