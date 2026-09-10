// Quantify base-colour loss independently of animated clouds/lighting.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import sharp from 'sharp';
import { maps } from './uastc-maps.mjs';
const require = createRequire(import.meta.url);
const directory = path.resolve('target/uastc-quality');
await fs.mkdir(directory, { recursive: true });
const ktx = args => execFileSync(process.execPath, [require.resolve('ktx2tools/ktx.js'), ...args], { stdio: 'pipe' });
const results = [];
for (const { entity, stem } of maps) {
  const original = `assets/planets/${stem}.ktx2`;
  const compressed = `assets/planets/${stem}.uastc.ktx2`;
  const reference = `${directory}/${entity}-original.png`;
  const decoded = `${directory}/${entity}-decoded.png`;
  ktx(['extract', original, reference]);
  ktx(['transcode', '--target', 'rgba8', compressed, `${directory}/decoded.ktx2`]);
  ktx(['extract', `${directory}/decoded.ktx2`, decoded]);
  const a = await sharp(reference).removeAlpha().raw().toBuffer();
  const b = await sharp(decoded).removeAlpha().raw().toBuffer();
  if (a.length !== b.length) throw new Error('Decoded dimensions changed');
  let absolute = 0, squared = 0, maximum = 0;
  for (let i = 0; i < a.length; i++) {
    const error = Math.abs(a[i] - b[i]);
    absolute += error; squared += error * error; maximum = Math.max(maximum, error);
  }
  const result = { entity, originalBytes: (await fs.stat(original)).size,
    uastcBytes: (await fs.stat(compressed)).size, mae: absolute / a.length,
    psnr: 10 * Math.log10(255 ** 2 / (squared / a.length)), maximum };
  results.push(result); console.log(JSON.stringify(result));
}
await fs.writeFile(`${directory}/measurements.json`, JSON.stringify(results, null, 2) + '\n');
