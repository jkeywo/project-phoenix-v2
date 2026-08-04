// Writes assets/models/index.json — the list of GLB models plus the rig
// variants each one has sidecars for.
//
// The viewer's model dropdown reads this at runtime. Generating it beats
// hardcoding a list in viewer.html, which would silently go stale every time a
// model is added or renamed. Run as a Trunk pre_build hook (viewer-trunk.toml).

import { readdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { REMESH_SUFFIX } from './generate-lods.mjs';

// A `.glb` this repo PRODUCES rather than one an artist delivered: a decimated
// level, named `<stem>_lod<n>.glb` by the ladder that generates it, or the
// `.remesh.glb` intermediate the Blender pre-pass writes. They are listed —
// this file is the inventory of what is on disk — but flagged, because they
// have no business in a "pick a model" dropdown: they are outputs of a ladder,
// and the ladder is how you look at them.
const GENERATED_LEVEL = /_lod\d+$/;

const MODELS_DIR = path.join(process.cwd(), 'assets', 'models');
const OUT = path.join(MODELS_DIR, 'index.json');

const files = await readdir(MODELS_DIR);

// `<stem>.model.toml` is the base rig; `<stem>.<variant>.toml` is a variant
// (large, small, cosmetic, …). Anything else in here is not a sidecar.
const variantsByStem = new Map();
for (const file of files) {
  if (!file.endsWith('.toml')) continue;
  const withoutExt = file.slice(0, -'.toml'.length);
  const dot = withoutExt.lastIndexOf('.');
  if (dot === -1) continue;
  const stem = withoutExt.slice(0, dot);
  const variant = withoutExt.slice(dot + 1);
  if (variant === 'model') continue; // the base rig, not a variant
  if (!variantsByStem.has(stem)) variantsByStem.set(stem, []);
  variantsByStem.get(stem).push(variant);
}

const models = files
  .filter((f) => f.endsWith('.glb'))
  .sort()
  .map((file) => {
    const stem = file.slice(0, -'.glb'.length);
    return {
      path: `assets/models/${file}`,
      name: stem,
      variants: (variantsByStem.get(stem) ?? []).sort(),
      generated: GENERATED_LEVEL.test(stem) || file.endsWith(REMESH_SUFFIX),
    };
  });

// Write only when the content actually differs.
//
// This runs as a Trunk pre_build hook, and it writes INTO a watched directory
// (assets/models/), so an unconditional write is a build that triggers the next
// build forever — the viewer's page reloading itself every minute or two while
// nobody touches anything. viewer-trunk.toml lists this file under `[watch]
// ignore` for exactly that reason, and the loop happens anyway, so the write
// itself is what has to stop: a rewrite with identical bytes still moves the
// mtime, and the mtime is what a file watcher reads.
const next = JSON.stringify({ models }, null, 2) + '\n';
const current = await readFile(OUT, 'utf8').catch(() => null);
const relative = path.relative(process.cwd(), OUT);
if (current === next) {
  console.log(`[generate-model-index] ${models.length} models — ${relative} already current`);
} else {
  await writeFile(OUT, next);
  console.log(`[generate-model-index] ${models.length} models → ${relative}`);
}
