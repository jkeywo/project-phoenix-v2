// Regenerate approved browser variants; --check verifies drift, --force rebakes.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import assert from 'node:assert/strict';
import sharp from 'sharp';
import { maps } from './uastc-maps.mjs';
const require = createRequire(import.meta.url);
const root = 'assets/texture-codecs';
const tracked = [...maps.flatMap(({ stem }) => [`assets/planets/${stem}.ktx2`,
  `assets/planets/${stem}.uastc.ktx2`, `assets/planets/${stem}.ptex`]),
  `${root}/opaque-4k-templates.json`, `${root}/basis/basis_transcoder.js`, `${root}/basis/basis_transcoder.wasm`];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const hashes = async () => Object.fromEntries(await Promise.all(tracked.map(async file =>
  [file, digest(await fs.readFile(file))])));
if (process.argv.includes('--check')) {
  const manifest = JSON.parse(await fs.readFile(`${root}/manifest.json`, 'utf8'));
  const actual = await hashes();
  for (const file of tracked) {
    if (manifest.hashes[file] !== actual[file]) throw new Error(`UASTC artifact drift: ${file}`);
  }
  console.log('All UASTC sources, variants, descriptors, templates and runtime match.');
} else {
  const temp = path.resolve('target/uastc-bake');
  await fs.mkdir(temp, { recursive: true });
  const run = (tool, args) => execFileSync(process.execPath, [require.resolve(`ktx2tools/${tool}.js`), ...args], { stdio: 'inherit' });
  const prior = await fs.readFile(`${root}/manifest.json`, 'utf8').then(JSON.parse).catch(() => ({ hashes: {} }));
  for (const { stem } of maps) {
    const source = `assets/planets/${stem}.ktx2`;
    const output = `assets/planets/${stem}.uastc.ktx2`;
    const input = await fs.readFile(source);
    assert.equal(input.readUInt32LE(12), 43, `${source}: sRGB RGBA8`);
    assert.equal(input.readUInt32LE(20), 4096, `${source}: width`);
    assert.equal(input.readUInt32LE(24), 2048, `${source}: height`);
    assert.equal(input.readUInt32LE(40), 13, `${source}: mips`);
    const current = await fs.readFile(output).catch(() => null);
    if (!process.argv.includes('--force') && current && prior.hashes[source] === digest(input) && prior.hashes[output] === digest(current)) {
      console.log(`${stem}: unchanged`);
      continue;
    }
    run('ktx', ['extract', source, `${temp}/source.png`]);
    const stats = await sharp(`${temp}/source.png`).stats();
    assert.ok(stats.channels.length === 3 || stats.channels[3].min === 255, `${source}: alpha must be opaque`);
    run('toktx', ['--t2', '--genmipmap', '--assign_oetf', 'srgb', '--encode', 'uastc',
      '--uastc_quality', '3', '--uastc_rdo_l', '0.5', '--uastc_rdo_m', '--threads', '4',
      '--zcmp', '9', output, `${temp}/source.png`]);
    console.log(`${stem}: ${(input.length / 1048576).toFixed(2)} -> ${((await fs.stat(output)).size / 1048576).toFixed(2)} MiB`);
  }
  // Headers/DFDs describe the shared shape and format, not the pixel content.
  const output = `assets/planets/${maps[0].stem}.uastc.ktx2`;
  const templates = {};
  for (const [name, format] of [['astc', 'astc'], ['bc7', 'bc7'], ['etc2', 'etc-rgba'], ['rgba', 'rgba8']]) {
    const file = `${temp}/${name}.ktx2`;
    run('ktx', ['transcode', '--target', format, output, file]);
    const bytes = await fs.readFile(file);
    const levels = Array.from({ length: bytes.readUInt32LE(40) }, (_, i) => ({
      offset: Number(bytes.readBigUInt64LE(80 + i * 24)), length: Number(bytes.readBigUInt64LE(88 + i * 24)),
    }));
    templates[name] = { header: bytes.subarray(0, Math.min(...levels.map(l => l.offset))).toString('base64'),
      length: bytes.length, levels, vkFormat: bytes.readUInt32LE(12) };
  }
  await fs.writeFile(`${root}/opaque-4k-templates.json`, JSON.stringify(templates));
  await fs.writeFile(`${root}/manifest.json`, JSON.stringify({
    transcoderPackage: 'three@0.180.0', hashes: await hashes(),
  }, null, 2) + '\n');
}
