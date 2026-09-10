// Stage only the actual prototype dependencies, not the entire game asset tree.
import { mkdirSync, copyFileSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const lab = dirname(fileURLToPath(import.meta.url));
const root = resolve(lab, '../..');
const model = readFileSync(resolve(lab, 'scene.toml'), 'utf8').match(/^model\s*=\s*"([^"#]+)/m)[1];
const files = [model, 'pfx/space_mote_streak_head.png', 'pfx/space_mote_streak_soft.png', 'pfx/space_mote_compact_core.png', 'shaders/dust_mote.wgsl'];
for (const file of files) {
  const dest = resolve(lab, 'staged-assets', file);
  mkdirSync(dirname(dest), { recursive: true });
  copyFileSync(resolve(root, 'assets', file), dest);
}
copyFileSync(resolve(lab, 'flare.wgsl'), resolve(lab, 'staged-assets/shaders/web_flare.wgsl'));
