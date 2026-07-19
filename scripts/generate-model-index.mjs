// Writes assets/models/index.json — the list of GLB models plus the rig
// variants each one has sidecars for.
//
// The viewer's model dropdown reads this at runtime. Generating it beats
// hardcoding a list in viewer.html, which would silently go stale every time a
// model is added or renamed. Run as a Trunk pre_build hook (viewer-trunk.toml).

import { readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

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
    };
  });

await writeFile(OUT, JSON.stringify({ models }, null, 2) + '\n');
console.log(`[generate-model-index] ${models.length} models → ${path.relative(process.cwd(), OUT)}`);
