// Writes assets/entities/index.json for the standalone viewer's entity picker.
//
// The game owns entity TOML; this is deliberately only an inventory of the
// top-level templates that the viewer can render. Generated test fixtures and
// include fragments are not authored subjects a designer should pick here.

import { readdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const ENTITIES_DIR = path.join(process.cwd(), 'assets', 'entities');
const OUT = path.join(ENTITIES_DIR, 'index.json');
const visualKind = (source) => {
  // Planet templates also carry a procedural `[mesh]` fallback. The game and
  // viewer both prefer the custom visual, so the picker must describe that
  // actual render path rather than whichever section happens to appear first.
  for (const kind of ['star', 'planet', 'mesh']) {
    if (new RegExp(`^\\[${kind}\\]\\s*$`, 'm').test(source)) return kind;
  }
  return null;
};

const files = (await readdir(ENTITIES_DIR))
  .filter((file) => file.endsWith('.toml'))
  .sort();

const entities = [];
for (const file of files) {
  const source = await readFile(path.join(ENTITIES_DIR, file), 'utf8');
  const kind = visualKind(source);
  if (!kind) continue;
  entities.push({
    path: `assets/entities/${file}`,
    name: file.slice(0, -'.toml'.length),
    kind,
  });
}

const next = JSON.stringify({ entities }, null, 2) + '\n';
const current = await readFile(OUT, 'utf8').catch(() => null);
const relative = path.relative(process.cwd(), OUT);
if (current === next) {
  console.log(`[generate-entity-index] ${entities.length} entities — ${relative} already current`);
} else {
  await writeFile(OUT, next);
  console.log(`[generate-entity-index] ${entities.length} entities → ${relative}`);
}
