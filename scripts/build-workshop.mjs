// Build the standalone offline Authoring surface from the locked local deps.
// A Trunk post_build hook; offline validation uses its shared WASM artifact.
import { cp, copyFile, mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { soundCuesJson, SOUND_CUES_JSON, soundCueInventoryJs, SOUND_CUE_INVENTORY } from './sound-cues.mjs';
import { crc32 } from '../editor/crc32.js';
import { assetDependencies } from '../editor/asset-dependencies.js';
import { gmConsoleMarkup } from './gm-console-markup.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const out = path.resolve(root, process.argv[2] || 'dist');
const editorModules = [
  'asset-dependencies', 'crc32', 'workshop-assets', 'workshop-handoff', 'workshop-source-provider',
  'workshop-models', 'workshop-model-preview', 'workshop-entity-composition', 'workshop-ship-authoring', 'workshop-preview', 'workshop-preview-runtime',
  'workshop-document', 'workshop-runtime', 'workshop-recovery', 'workshop-provider', 'workshop-test', 'workshop-sound-cues', 'mod-actions', 'mod-pack-workspace', 'mod-pack-export',
  'workshop-migration', 'workshop-diff', 'workshop-definitions', 'workshop-composition', 'workshop-entity',
  'workshop-presets', 'workshop-scripts', 'script-editor', 'script-editor-view',
  'undo-stack', 'validation', 'entity-includes', 'component-schema', 'component-templates', 'world-toml', 'entity-toml',
  'stations-validate', 'marker-validate', 'blaster-validate', 'torpedo-validate',
  'workshop-test-frame', 'workshop-test-child', 'workshop-test-runtime', 'workshop-test-snapshot',
];
await mkdir(path.join(out, 'editor'), { recursive: true });
await mkdir(path.join(out, 'assets', 'strings'), { recursive: true });
await writeFile(path.join(root,SOUND_CUES_JSON),await soundCuesJson(root),'utf8');
await writeFile(path.join(root,SOUND_CUE_INVENTORY),await soundCueInventoryJs(root),'utf8');
await cp(path.join(root,'assets/audio'),path.join(out,'assets/audio'),{recursive:true});
await cp(path.join(root,'assets/sounds'),path.join(out,'assets/sounds'),{recursive:true});
await copyFile(path.join(root, 'workshop.html'), path.join(out, 'workshop.html'));
// The disposable Test carries the REAL GM console markup, injected here rather
// than written a second time. Its omniscient view mounts the ordinary
// gm-workspace over the ordinary markup (issue #1472); a Workshop-only
// substitute would be a second surface to keep correct and would stop being
// evidence about the real one the moment it drifted.
const testPage = await readFile(path.join(root, 'workshop-test.html'), 'utf8');
const GM_SLOT = '<!--gm-console-->';
if (!testPage.includes(GM_SLOT)) throw new Error('workshop-test.html has no GM console slot');
await writeFile(path.join(out, 'workshop-test.html'),
  testPage.replace(GM_SLOT, gmConsoleMarkup(await readFile(path.join(root, 'server.html'), 'utf8'))), 'utf8');
await cp(path.join(root, 'gui'), path.join(out, 'gui'), { recursive: true });
await copyFile(path.join(root, 'assets/strings/strings.csv'), path.join(out, 'assets/strings/strings.csv'));
for (const name of editorModules) {
  await copyFile(path.join(root, `editor/${name}.js`), path.join(out, `editor/${name}.js`));
}
const tomlDist = path.dirname(require.resolve('smol-toml'));
await cp(tomlDist, path.join(out, 'vendor/smol-toml'), { recursive: true });
await copyFile(path.resolve(tomlDist, '../LICENSE'), path.join(out, 'vendor/smol-toml/LICENSE'));
// The browser gets immutable read-only dependencies, never filesystem access.
// Snapshot the shipped textual content under its ordinary asset paths so the
// same runtime parser/include/compiler can resolve an unsaved pack offline.
const baseFiles = {};
const baseAssetManifest = {};
async function collect(directory) {
  for (const entry of await readdir(path.join(root, directory), { withFileTypes: true })) {
    const relative = `${directory}/${entry.name}`;
    if (entry.isDirectory()) await collect(relative);
    else if (/\.(toml|rhai)$/.test(entry.name)) baseFiles[relative] = await readFile(path.join(root, relative), 'utf8');
    else if (/\.(glb|bin|png|jpg|jpeg|ktx2|ptex|wav|ogg|mp3|wgsl)$/.test(entry.name)) {
      const bytes = await readFile(path.join(root, relative));
      baseAssetManifest[relative] = { length: bytes.length, crc32: crc32(bytes), requires: assetDependencies(relative, bytes) };
    }
  }
}
await collect('assets');
await writeFile(path.join(out, 'workshop-base.json'), JSON.stringify({ base_files: baseFiles, base_asset_manifest: baseAssetManifest, packs: [] }));
console.log(`Workshop Authoring built → ${path.join(out, 'workshop.html')}`);
