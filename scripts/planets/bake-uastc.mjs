// Regenerate the approved one-map browser variant after bake-natural.mjs.
// --check verifies source/artifact/runtime drift without running the encoder.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
const require = createRequire(import.meta.url);
const root = 'assets/texture-codecs';
const source = 'assets/planets/gas_giant/surface_colour.ktx2';
const output = 'assets/planets/gas_giant/surface_colour.uastc.ktx2';
const tracked = [source, output, `${root}/gas-base-templates.json`,
  `${root}/basis/basis_transcoder.js`, `${root}/basis/basis_transcoder.wasm`];
const hashes = async () => Object.fromEntries(await Promise.all(tracked.map(async file =>
  [file, createHash('sha256').update(await fs.readFile(file)).digest('hex')])));
if (process.argv.includes('--check')) {
  const manifest = JSON.parse(await fs.readFile(`${root}/manifest.json`, 'utf8'));
  const actual = await hashes();
  for (const file of tracked) {
    if (manifest.hashes[file] !== actual[file]) throw new Error(`UASTC artifact drift: ${file}`);
  }
  console.log('UASTC source, texture, templates and pinned runtime match.');
} else {
  const temp = path.resolve('target/uastc-bake');
  await fs.mkdir(temp, { recursive: true });
  const run = (tool, args) => execFileSync(process.execPath, [require.resolve(`ktx2tools/${tool}.js`), ...args], { stdio: 'inherit' });
  run('ktx', ['extract', source, `${temp}/source.png`]);
  run('toktx', ['--t2', '--genmipmap', '--assign_oetf', 'srgb', '--encode', 'uastc',
    '--uastc_quality', '3', '--uastc_rdo_l', '0.5', '--uastc_rdo_m', '--threads', '4',
    '--zcmp', '9', output, `${temp}/source.png`]);
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
  await fs.writeFile(`${root}/gas-base-templates.json`, JSON.stringify(templates));
  await fs.writeFile(`${root}/manifest.json`, JSON.stringify({
    transcoderPackage: 'three@0.180.0', hashes: await hashes(),
  }, null, 2) + '\n');
}
