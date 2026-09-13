// The parser-free browser's generated projection of authored room comfort.
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { parse } from 'smol-toml';

export async function roomDuckingJson(root) {
  return `${JSON.stringify(parse(await readFile(path.join(root, 'assets/audio/room-ducking.toml'), 'utf8')), null, 2)}\n`;
}
