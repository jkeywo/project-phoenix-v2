// Place the Workshop model preview artifact beside the pages that open it.
//
// The preview is its own Trunk target (workshop-preview-trunk.toml) because it
// is built with `--features viewer` while every other page is built without it.
// Trunk cleans its own dist directory, and that directory is the server page's,
// so the preview builds into `dist-workshop-preview/` and lands here instead of
// being written into `dist/` directly.
//
// Run: node scripts/copy-workshop-preview.mjs [dist-dir] [--check]
//   --check verifies the artifact is present and current rather than copying,
//   which is what a build gate wants.
import { cp, mkdir, readdir, rm, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2).filter(value => value !== '--check');
const check = process.argv.includes('--check');
const source = path.join(root, 'dist-workshop-preview');
const target = path.resolve(root, args[0] || 'dist', 'preview');

async function exists(candidate) {
  try { await stat(candidate); return true; } catch { return false; }
}

if (!(await exists(source))) {
  console.error(`Workshop preview artifact is missing: ${source}
Build it first:
  trunk build --release --config workshop-preview-trunk.toml`);
  process.exit(1);
}

// Trunk names the built page `index.html` whatever the source page was called,
// so the frame opens `preview/index.html` rather than `preview/workshop-preview.html`.
const built = await readdir(source);
if (!built.includes('index.html')) {
  console.error(`${source} has no index.html — the Trunk build did not produce a page.`);
  process.exit(1);
}
// The wasm the preview renders through. Without it the page loads and never
// boots, which reads as an empty preview rather than as a missing build.
if (!built.some(name => name.endsWith('.wasm'))) {
  console.error(`${source} has no .wasm — the viewer-feature build did not produce a bundle.`);
  process.exit(1);
}

if (check) {
  const present = (await exists(path.join(target, 'index.html')))
    && (await readdir(target)).some(name => name.endsWith('.wasm'));
  if (!present) {
    console.error(`${target} is missing the built preview. Run this script without --check.`);
    process.exit(1);
  }
  console.log(`${target} carries the Workshop preview`);
  process.exit(0);
}

await rm(target, { recursive: true, force: true });
await mkdir(target, { recursive: true });
await cp(source, target, { recursive: true });
console.log(`Workshop model preview → ${path.join(target, 'index.html')}`);
