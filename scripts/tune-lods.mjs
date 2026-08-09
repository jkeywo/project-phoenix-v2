// tune-lods.mjs — drive the `tune-lods` bin and write the proposed switch
// distances back into a model's rig sidecars.
//
// The bin (src/bin/tune_lods.rs) renders each adjacent LOD pair at swept
// distances and prints the knee of the difference-vs-distance curve as the
// proposed `max_distance` for the fine level. This driver is the disk half: it
// runs the bin per variant, reads the proposals, folds them into each sidecar's
// ladder with the same pure helpers the viewer's LOD panel uses
// (`ladderFromDoc` + `replaceLadder`, scripts/viewer-lods.mjs), and enforces the
// same structural rules (`validateLadder` / `validateProposal`) before writing.
//
//   node scripts/tune-lods.mjs <stem…> [--variant v] [--dry-run]
//                              [--resolution WxH] [--distances N] [--yaws N]
//                              [--out <dir>] [--bin <path>]
//
// ── Per-variant, not scale-by-ratio ─────────────────────────────────────────
//
// A switch distance is an angular threshold wearing a distance's clothes: a rock
// at three times the radius is "that small on screen" three times further away.
// Rather than tune one variant and multiply, this driver runs the bin ONCE PER
// VARIANT — each variant's sidecar carries its own `[base] scale` and
// `[extents]`, so the bin frames and sweeps in that variant's own world units
// and the knee comes back already scaled. Tuning `large` and `huge` separately
// therefore reproduces `huge ≈ large × 3` by construction (the invariant
// `the_huge_asteroid_variant_scales_its_bands_and_not_its_ratios` asserts),
// without this file owning the ×3.
//
// ── Dry run ─────────────────────────────────────────────────────────────────
//
// `--dry-run` prints the proposed ladder and the validation result for each
// sidecar and writes NOTHING. It is the mode the tracer uses: the render + knee
// pipeline is exercised end-to-end, the sidecar-write code path is exercised up
// to the final `writeFile`, but no shipped sidecar is mutated.

import { readFile, writeFile, readdir } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
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

const ROOT = process.cwd();
const MODELS_DIR = path.join(ROOT, 'assets', 'models');

function parseArgs(argv) {
  const flags = new Map();
  const positional = [];
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--dry-run') flags.set('dry-run', true);
    else if (a.startsWith('--')) {
      flags.set(a.slice(2), argv[i + 1]);
      i += 1;
    } else positional.push(a);
  }
  return { flags, positional };
}

/** Run the bin for one model + variant and return its parsed JSON proposal. */
function runBin(modelPath, variant, flags) {
  const bin = flags.get('bin');
  const args = bin
    ? [modelPath, '--variant', variant]
    : ['run', '-q', '--features', 'capture', '--bin', 'tune-lods', '--', modelPath, '--variant', variant];
  for (const key of ['resolution', 'distances', 'yaws', 'pitch', 'out']) {
    if (flags.get(key)) args.push(`--${key}`, flags.get(key));
  }
  const cmd = bin || 'cargo';
  const res = spawnSync(cmd, args, { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  if (res.status !== 0) {
    throw new Error(`tune-lods bin failed for ${modelPath} (${variant}):\n${res.stderr || res.stdout}`);
  }
  // The proposal is the last JSON object line on stdout.
  const line = res.stdout
    .split('\n')
    .map((l) => l.trim())
    .reverse()
    .find((l) => l.startsWith('{') && l.includes('"pairs"'));
  if (!line) throw new Error(`no proposal JSON from tune-lods for ${modelPath} (${variant})`);
  return JSON.parse(line);
}

/**
 * Fold a proposal's per-pair `proposed_max_distance` into a sidecar's ladder.
 *
 * Pair `i` sets the fine level `i`'s bound; the final level stays unbounded. The
 * generation/capture provenance and every other level field are carried through
 * untouched — only `max_distance` moves.
 */
function applyProposal(levels, proposal) {
  const out = levels.map((l) => ({ ...l }));
  for (const pair of proposal.pairs) {
    const i = pair.fine;
    // Only move a bound when the sweep actually found a knee. A pair with no
    // detectable knee (two near-identical adjacent LODs, or a blank sweep) keeps
    // its authored `max_distance` — never overwritten with a guessed distance.
    if (i < out.length - 1 && pair.knee_found) {
      out[i].max_distance = pair.proposed_max_distance;
    }
  }
  return out;
}

async function main() {
  const { flags, positional } = parseArgs(process.argv.slice(2));
  const dryRun = flags.get('dry-run') === true;
  if (!positional.length) {
    console.error('usage: node scripts/tune-lods.mjs <stem…> [--variant v] [--dry-run] [--bin <path>] [--out <dir>] …');
    process.exit(2);
  }

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
      console.error(`[tune-lods] no rig sidecars for ${stem}${flags.get('variant') ? ` (variant ${flags.get('variant')})` : ''}`);
      continue;
    }

    // Tune every variant that carries a ladder, each in its own world units.
    const proposed = [];
    for (const file of sidecars) {
      const current = await readFile(path.join(MODELS_DIR, file), 'utf8');
      const doc = parseToml(current);
      const levels = ladderFromDoc(doc);
      if (levels.length < 2) continue; // no ladder to tune here

      const variant = variantOfSidecar(file, stem) || 'model';
      const proposal = runBin(modelPath, variant, flags);
      const newLevels = applyProposal(levels, proposal);
      const problems = validateLadder(newLevels);

      console.log(`\n=== ${file} (${variant}) ===`);
      for (const pair of proposal.pairs) {
        const arrow = pair.current_max_distance === pair.proposed_max_distance ? '=' : '→';
        console.log(
          `  L${pair.fine}: max_distance ${pair.current_max_distance} ${arrow} ${pair.proposed_max_distance}` +
            `  (knee_found=${pair.knee_found}, peak_diff=${pair.peak_diff})`,
        );
      }
      if (problems.length) {
        console.log(`  ⚠ ladder invalid: ${problems.join('; ')}`);
      }

      proposed.push({
        path: `assets/models/${file}`,
        text: replaceLadder(current, newLevels),
        current,
        problems,
      });
    }

    if (!proposed.length) {
      console.error(`[tune-lods] ${stem}: no sidecar carried a ladder`);
      continue;
    }

    // Structural check across ALL of the stem's proposed sidecars at once.
    const proposalErrors = validateProposal(proposed.map(({ path: p, text }) => ({ path: p, text })));
    if (proposalErrors.length) {
      console.log(`\n[tune-lods] ${stem}: proposal rejected:\n  ${proposalErrors.join('\n  ')}`);
    }

    const anyProblems = proposed.some((p) => p.problems.length) || proposalErrors.length;
    if (dryRun) {
      console.log(`\n[tune-lods] ${stem}: DRY RUN — nothing written.`);
      continue;
    }
    if (anyProblems) {
      console.error(`\n[tune-lods] ${stem}: refusing to write an invalid ladder. Re-run with --dry-run to inspect.`);
      process.exitCode = 1;
      continue;
    }
    const written = [];
    for (const item of proposed) {
      if (item.text === item.current) continue; // unchanged: leave the mtime alone
      await writeFile(path.join(ROOT, item.path), item.text);
      written.push(item.path);
    }
    console.log(`\n[tune-lods] ${stem}: wrote ${written.length} sidecar(s): ${written.join(', ') || '(none changed)'}`);
  }
}

main().catch((err) => {
  console.error(err.stack || String(err));
  process.exit(1);
});
