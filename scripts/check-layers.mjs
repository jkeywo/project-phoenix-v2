#!/usr/bin/env node
/** Compiler packages establish visibility; this gate preserves the allowed dependency directions. */
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, resolve, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { parse } from 'smol-toml';
import { allowed, dependencyEntries, layerDependencyFailures } from './layer-policy.mjs';
import { init, parse as parseModules } from 'es-module-lexer';
await init;
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const failures = [];
const workspace = parse(readFileSync(resolve(root, 'Cargo.toml'), 'utf8')).workspace;
for (const name of Object.keys(allowed)) {
  const folder = name === 'phoenix-grid' ? 'examples/grid' : `crates/${name}`;
  const manifest = parse(readFileSync(resolve(root, folder, 'Cargo.toml'), 'utf8'));
  failures.push(...layerDependencyFailures(name, manifest, workspace));
  if (name.startsWith('phoenix-sim')) {
    for (const entry of dependencyEntries(manifest, workspace).filter(e => e.name === 'bevy')) {
      if (entry.definition['default-features'] !== false) failures.push(`${name} must opt out of Bevy default features (${entry.kind}, ${entry.target})`);
    }
  }
}
// Cargo unifies features across the host and its simulation dependency. A root
// host opting into Rapier defaults would inject mesh/scene-dependent systems into
// the simulation's renderer-free probe, even though its own manifest is clean.
for (const folder of ['.', ...Object.keys(allowed).map(name => name === 'phoenix-grid' ? 'examples/grid' : `crates/${name}`)]) {
  const manifest = parse(readFileSync(resolve(root, folder, 'Cargo.toml'), 'utf8'));
  for (const { alias, name, definition, kind } of dependencyEntries(manifest, workspace)) {
    if (name !== 'bevy_rapier3d') continue;
    const forbidden = ['default', 'async-collider'];
    const forwardsRendering = Object.values(manifest.features || {}).flat().some(feature =>
      forbidden.some(value => feature === `${alias}/${value}` || feature === `${alias}?/${value}`));
    if (definition['default-features'] !== false || definition.features?.some(feature => forbidden.includes(feature)) || forwardsRendering) {
      failures.push(`${folder}: Rapier must not install mesh/scene-dependent physics systems (${kind})`);
    }
  }
}

function files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = resolve(directory, entry.name);
    if (['node_modules', 'pkg'].includes(entry.name)) return [];
    return entry.isDirectory() ? files(path) : /\.[cm]?js$/.test(entry.name) ? [path] : [];
  });
}
function contained(path, directory) {
  const rel = relative(directory, path);
  return !isAbsolute(rel) && rel !== '..' && !rel.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`);
}
for (const folder of ['packages/transport', 'packages/session', 'examples/grid']) {
  const directory = resolve(root, folder);
  for (const path of files(directory)) {
    const source = readFileSync(path, 'utf8');
    for (const entry of parseModules(source)[0]) {
      if (entry.d === -2) continue; // import.meta has no dependency.
      const specifier = entry.n;
      if (!specifier) { failures.push(`${relative(root, path)} has an unbounded dynamic import`); continue; }
      if (specifier.startsWith('node:')) continue;
      if (!specifier.startsWith('.')) { failures.push(`${relative(root, path)}: undeclared import ${specifier}`); continue; }
      const target = resolve(dirname(path), specifier);
      const permitted = contained(target, directory) || (folder !== 'packages/transport' && contained(target, resolve(root, 'packages/transport'))) || (folder === 'examples/grid' && contained(target, resolve(root, 'packages/session')));
      if (!permitted) failures.push(`${relative(root, path)} reaches outside its layer: ${specifier}`);
    }
  }
}
if (process.argv.includes('--dependency-tree')) {
  // A separate root selection matters: workspace feature unification could conceal a renderer dependency.
  const banned = /^(bevy_(render|pbr|window|winit|scene|ui|camera|light|gltf)|vellum-ultralight|ul-next|wgpu) v/m;
  for (const name of Object.keys(allowed).filter(name => name.startsWith('phoenix-sim'))) {
    const tree = execFileSync('cargo', ['tree', '-p', name, '--no-default-features', '--edges', 'normal', '--prefix', 'none', '--format', '{p}'], { cwd: root, encoding: 'utf8' });
    if (banned.test(tree)) failures.push(`${name} pulls presentation machinery: ${tree.match(banned)[0]}`);
  }
}
if (failures.length) { console.error(failures.join('\n')); process.exitCode = 1; }
else console.log('Layer dependency rules passed.');
