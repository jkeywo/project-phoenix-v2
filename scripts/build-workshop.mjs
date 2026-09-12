// Build the standalone offline Authoring surface from the locked local deps.
// Also a Trunk post_build hook: no simulation/WASM is required by this slice.
import { cp, copyFile, mkdir } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const out = path.resolve(root, process.argv[2] || 'dist');
const editorModules = [
  'workshop-document', 'mod-actions', 'mod-pack-workspace', 'mod-pack-export',
  'undo-stack', 'validation', 'entity-includes', 'world-toml', 'entity-toml',
  'stations-validate', 'marker-validate', 'blaster-validate', 'torpedo-validate',
];
await mkdir(path.join(out, 'editor'), { recursive: true });
await mkdir(path.join(out, 'assets', 'strings'), { recursive: true });
await copyFile(path.join(root, 'workshop.html'), path.join(out, 'workshop.html'));
await cp(path.join(root, 'gui'), path.join(out, 'gui'), { recursive: true });
await copyFile(path.join(root, 'assets/strings/strings.csv'), path.join(out, 'assets/strings/strings.csv'));
for (const name of editorModules) {
  await copyFile(path.join(root, `editor/${name}.js`), path.join(out, `editor/${name}.js`));
}
const tomlDist = path.dirname(require.resolve('smol-toml'));
await cp(tomlDist, path.join(out, 'vendor/smol-toml'), { recursive: true });
await copyFile(path.resolve(tomlDist, '../LICENSE'), path.join(out, 'vendor/smol-toml/LICENSE'));
console.log(`Workshop Authoring built → ${path.join(out, 'workshop.html')}`);
