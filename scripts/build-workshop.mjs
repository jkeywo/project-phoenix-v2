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
  'asset-dependencies', 'crc32', 'indexed-db', 'workshop-assets', 'workshop-handoff', 'workshop-source-provider',
  'workshop-models', 'workshop-model-structure', 'workshop-model-preview', 'workshop-entity-composition', 'workshop-ship-authoring', 'workshop-system-composition', 'workshop-state-trees', 'workshop-spatial', 'workshop-preview', 'workshop-preview-runtime',
  'workshop-damage', 'workshop-damage-model',
  'workshop-document', 'workshop-asset-job', 'workshop-acceptance', 'workshop-runtime', 'workshop-recovery', 'workshop-provider', 'workshop-launch', 'workshop-billboard-capture', 'workshop-lod-generation', 'workshop-test', 'workshop-sound-cues', 'mod-actions', 'mod-pack-workspace', 'mod-pack-export',
  'ordered-form', 'workshop-migration', 'workshop-diff', 'workshop-definitions', 'workshop-composition', 'workshop-entity', 'workshop-resources', 'workshop-logistics', 'workshop-production', 'workshop-fields', 'workshop-fitting', 'workshop-sensing', 'workshop-helm-projection', 'workshop-resource-recovery', 'workshop-ports', 'workshop-stock-bindings', 'workshop-weapons', 'workshop-weapon-model', 'workshop-weapon-groups', 'workshop-energy', 'workshop-energy-ai', 'workshop-energy-model', 'workshop-generated-systems', 'workshop-shields', 'workshop-shield-ai', 'workshop-interiors',
