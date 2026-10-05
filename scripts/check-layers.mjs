#!/usr/bin/env node
/** Compiler packages establish visibility; this gate preserves the allowed dependency directions. */
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, resolve, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { parse } from 'smol-toml';
import { init, parse as parseModules } from 'es-module-lexer';
await init;
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const allowed = {
  'phoenix-math': [], 'phoenix-runtime': ['phoenix-math'],
  'phoenix-transport': ['phoenix-runtime'], 'phoenix-platform': [],
  'phoenix-model': ['phoenix-math', 'phoenix-transport'],
  'phoenix-content': ['phoenix-model', 'phoenix-math', 'phoenix-platform'],
  'phoenix-simulation': ['phoenix-runtime', 'phoenix-transport', 'phoenix-platform', 'phoenix-model', 'phoenix-content', 'phoenix-math'],
  'phoenix-presentation': ['phoenix-simulation', 'phoenix-model', 'phoenix-content', 'phoenix-math'],
  'phoenix-grid': ['phoenix-runtime', 'phoenix-transport', 'phoenix-platform', 'phoenix-math'],
};
const failures = [];
for (const [name, permitted] of Object.entries(allowed)) {
  const folder = name === 'phoenix-grid' ? 'examples/grid' : `crates/${name}`;
  const manifest = parse(readFileSync(resolve(root, folder, 'Cargo.toml'), 'utf8'));
  const sections = [manifest, ...Object.values(manifest.target || {})];
  for (const section of sections) for (const kind of ['dependencies', 'dev-dependencies', 'build-dependencies']) {
    for (const [alias, definition] of Object.entries(section[kind] || {})) {
      const dependency = definition.package || alias;
      if ((dependency.startsWith('phoenix-') || dependency === 'project-phoenix') && !permitted.includes(dependency)) {
        failures.push(`${name} may not depend on ${dependency} (${kind})`);
      }
    }
  }
  if (name === 'phoenix-simulation' && manifest.dependencies.bevy['default-features'] !== false) failures.push('simulation must opt out of Bevy default features');
}
// Cargo unifies features across the host and its simulation dependency. A root
// host opting into Rapier defaults would inject mesh/scene-dependent systems into
// the simulation's renderer-free probe, even though its own manifest is clean.
for (const folder of ['.', ...Object.keys(allowed).map(name => name === 'phoenix-grid' ? 'examples/grid' : `crates/${name}`)]) {
  const manifest = parse(readFileSync(resolve(root, folder, 'Cargo.toml'), 'utf8'));
  for (const section of [manifest, ...Object.values(manifest.target || {})]) {
    for (const kind of ['dependencies', 'dev-dependencies', 'build-dependencies']) {
      for (const [alias, definition] of Object.entries(section[kind] || {})) {
        if ((definition.package || alias) !== 'bevy_rapier3d') continue;
        const forbidden = ['default', 'async-collider'];
        const forwardsRendering = Object.values(manifest.features || {}).flat().some(feature =>
          forbidden.some(value => feature === `${alias}/${value}` || feature === `${alias}?/${value}`));
        if (definition['default-features'] !== false || definition.features?.some(feature => forbidden.includes(feature)) || forwardsRendering) {
          failures.push(`${folder}: Rapier must not install mesh/scene-dependent physics systems (${kind})`);
        }
      }
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
  const tree = execFileSync('cargo', ['tree', '-p', 'phoenix-simulation', '--no-default-features', '--edges', 'normal', '--prefix', 'none', '--format', '{p}'], { cwd: root, encoding: 'utf8' });
  const banned = /^(bevy_(render|pbr|window|winit|scene|ui|camera|light|gltf)|vellum-ultralight|ul-next|wgpu) v/m;
  if (banned.test(tree)) failures.push(`simulation pulls presentation machinery: ${tree.match(banned)[0]}`);
}
if (failures.length) { console.error(failures.join('\n')); process.exitCode = 1; }
else console.log('Layer dependency rules passed.');
