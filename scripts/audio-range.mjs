import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { parse } from 'smol-toml';

export async function audioRangeModule(root) {
  const spec = parse(await readFile(path.join(root, 'assets/audio/reduced-range.toml'), 'utf8'));
  return `// Generated from assets/audio/reduced-range.toml by scripts/audio-range.mjs.\nexport default ${JSON.stringify(spec, null, 2)};\n`;
}
