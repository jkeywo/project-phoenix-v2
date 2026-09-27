#!/usr/bin/env node
// Source-matched bundle provenance for the bounded real-browser fleet runners.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, realpath, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const hashPattern = /^[0-9a-f]{64}$/;
const revisionPattern = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/;

export function sourceState(source) {
  const git = args => execFileSync('git', args, { cwd: source, encoding: 'utf8' }).trim();
  return { sourceRevision: git(['rev-parse', 'HEAD']), sourcePatch: git(['status', '--porcelain', '--untracked-files=all']) };
}

export async function hashBundle(directory) {
  const files = {};
  async function walk(relative, ancestors = new Set()) {
    const canonical = await realpath(path.join(directory, relative));
    if (ancestors.has(canonical)) throw new Error('Cyclic bundle directory link');
    const visited = new Set(ancestors).add(canonical);
    for (const entry of await readdir(path.join(directory, relative), { withFileTypes: true })) {
      const name = path.join(relative, entry.name);
      if (entry.isDirectory() || (entry.isSymbolicLink() && (await stat(path.join(directory, name))).isDirectory())) await walk(name, visited);
      else files[name.replaceAll('\\', '/')] = sha(await readFile(path.join(directory, name)));
    }
  }
  await walk('');
  return files;
}

function wasmFiles(bundleHashes) {
  return Object.fromEntries(Object.entries(bundleHashes).filter(([name]) => name.endsWith('.wasm')));
}

export function createWasmBuildReceipt(source, bundleHashes, builtUtc = new Date().toISOString()) {
  if (source.sourcePatch !== '') throw new Error('Browser WASM build requires a clean source checkout');
  if (!revisionPattern.test(source.sourceRevision)) throw new Error('Invalid source revision');
  const wasmSha256 = wasmFiles(bundleHashes);
  if (!Object.keys(wasmSha256).length) throw new Error('Built bundle has no WASM artifact');
  if (!bundleHashes['client/index.html'] || !bundleHashes['server.html']) throw new Error('Built bundle lacks host or client page');
  return { format: 'phoenix-browser-wasm-build-v1', sourceRevision: source.sourceRevision, sourcePatch: '',
    builtUtc, buildCommands: ['trunk build --release', 'node scripts/build-client.mjs'], wasmSha256, bundleHashes };
}

export function verifyWasmBuildReceipt(receipt, source, bundleHashes) {
  if (!receipt || receipt.format !== 'phoenix-browser-wasm-build-v1') throw new Error('Invalid browser WASM build receipt format');
  if (source.sourcePatch !== '' || receipt.sourcePatch !== '') throw new Error('Browser WASM receipt requires clean source');
  if (!revisionPattern.test(source.sourceRevision) || receipt.sourceRevision !== source.sourceRevision) throw new Error('Browser WASM receipt source revision mismatch');
  if (JSON.stringify(receipt.buildCommands) !== JSON.stringify(['trunk build --release', 'node scripts/build-client.mjs'])) throw new Error('Browser WASM receipt build commands mismatch');
  if (!receipt.bundleHashes || typeof receipt.bundleHashes !== 'object' || Array.isArray(receipt.bundleHashes)) throw new Error('Browser WASM receipt bundle hashes missing');
  const current = Object.entries(bundleHashes).sort(([a], [b]) => a.localeCompare(b));
  const recorded = Object.entries(receipt.bundleHashes).sort(([a], [b]) => a.localeCompare(b));
  if (current.length !== recorded.length || current.some(([name, hash], i) => name !== recorded[i][0] || hash !== recorded[i][1] || !hashPattern.test(hash))) throw new Error('Browser WASM receipt bundle mismatch');
  const wasm = wasmFiles(bundleHashes);
  if (!Object.keys(wasm).length || !receipt.wasmSha256 || typeof receipt.wasmSha256 !== 'object'
    || Object.keys(wasm).length !== Object.keys(receipt.wasmSha256).length
    || Object.entries(wasm).some(([name, hash]) => receipt.wasmSha256[name] !== hash)) throw new Error('Browser WASM receipt artifact mismatch');
  if (!bundleHashes['client/index.html'] || !bundleHashes['server.html']) throw new Error('Browser WASM receipt lacks host or client page');
  return true;
}

export async function readVerifiedWasmReceipt(file, source, bundleHashes) {
  const raw = await readFile(file);
  const receipt = JSON.parse(raw);
  verifyWasmBuildReceipt(receipt, sourceState(source), bundleHashes);
  return { path: file, sha256: sha(raw), receipt, sourceAndBundleVerified: true };
}

async function main(args) {
  if (args.length !== 2 || args[0] !== 'build') throw new Error('Usage: node scripts/fleet-wasm-build-receipt.mjs build <receipt.json>');
  const output = path.resolve(args[1]);
  const before = sourceState(root);
  if (before.sourcePatch !== '') throw new Error('Browser WASM build requires a clean source checkout');
  execFileSync('trunk', ['build', '--release'], { cwd: root, stdio: 'inherit' });
  execFileSync(process.execPath, ['scripts/build-client.mjs'], { cwd: root, stdio: 'inherit' });
  const after = sourceState(root);
  if (after.sourceRevision !== before.sourceRevision || after.sourcePatch !== '') throw new Error('Source changed during browser WASM build');
  const hashes = await hashBundle(path.join(root, 'dist'));
  const receipt = createWasmBuildReceipt(after, hashes);
  await mkdir(path.dirname(output), { recursive: true });
  await writeFile(output, JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx' });
  process.stdout.write(`${output}\n`);
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) main(process.argv.slice(2)).catch(error => { console.error(error); process.exitCode = 1; });
