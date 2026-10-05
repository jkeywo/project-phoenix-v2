#!/usr/bin/env node
// Refresh the standalone graph from declared Cargo edges and actual JS imports.
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { resolve, dirname, relative, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse as toml } from 'smol-toml';
import { dependencyEntries } from './layer-policy.mjs';
import { init, parse as modules } from 'es-module-lexer';
await init;
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = path => readFileSync(resolve(root, path), 'utf8');
const descriptions = {
  'project-phoenix': 'Host composition, native and browser adapters, headless runner and Workshop host.',
  'phoenix-presentation': 'Shared 3D view: renderer, effects, cameras, HUD and Workshop preview.',
  'phoenix-simulation': 'Composition: schedules, live cross-domain adapters, materialization, snapshots and recovery.',
  'phoenix-sim-gameplay': 'EntityConfig, ship mechanics, physics, weapon rules, policy machines and Helm AI operators.',
  'phoenix-sim-world': 'WorldConfig, scripts, objectives, commitments, deadlines and narrative state.',
  'phoenix-sim-session': 'Crew, Station tenure, lobby decisions, connection lifecycle and fleet protocol state.',
  'phoenix-sim-contracts': 'Shared identities, tick and RNG, commands, authority, schedules and authored vocabulary.',
  'phoenix-content': 'Asset preparation, archives, includes, manifests, strings and content ledger.',
  'phoenix-model': 'Shared Phoenix types, identities, messages and visual declarations.',
  'phoenix-runtime': 'Generic command ordering, continuation, digests and recovery.',
  'phoenix-transport': 'Generic connections, delivery classes, relay protocol and sockets.',
  'phoenix-platform': 'Files, monitors, pane surfaces, input routing and frame lifetimes.',
  'phoenix-math': 'Deterministic numerical foundation.',
  'phoenix-grid': 'Separate native and WASM game proving reuse without Phoenix game packages.',
};
const workspace = toml(read('Cargo.toml'));
const rustNodes = ['.', ...workspace.workspace.members].map(path => {
  const manifest = toml(read(`${path}/Cargo.toml`));
  return { id: manifest.package.name, label: manifest.package.name.replace('phoenix-', ''), path, source: `${path}/Cargo.toml`, description: descriptions[manifest.package.name], manifest };
});
const rustIds = new Set(rustNodes.map(n => n.id));
const rustEdges = new Map();
for (const node of rustNodes) {
  for (const { name: to, definition, kind, target, alias } of dependencyEntries(node.manifest, workspace.workspace)) {
    if (!rustIds.has(to)) continue;
    const key = `${node.id}/${to}`;
    if (!rustEdges.has(key)) rustEdges.set(key, { from: node.id, to, evidence: [], normal: false });
    const edge = rustEdges.get(key);
    edge.normal ||= kind === 'dependencies';
    edge.evidence.push(`${node.source}: ${kind}, ${target}${definition.optional ? ', optional' : ''}${alias !== to ? `, alias ${alias}` : ''}${definition.workspace ? ', inherited' : ''}`);
  }
  delete node.manifest;
  node.group = node.id === 'project-phoenix' || node.id === 'phoenix-grid' ? 'host' : ['phoenix-runtime','phoenix-transport','phoenix-platform','phoenix-math','phoenix-sim-contracts'].includes(node.id) ? 'shared' : 'game';
}
const jsNodes = [
  { id: 'browser-ui', label: 'Phoenix browser UI', path: 'gui', source: 'server.html', description: 'Phone consoles, shared screen controls and Workshop. Includes root HTML entry points.', group: 'host' },
  { id: 'worker', label: 'Rendezvous Worker', path: 'worker-rendezvous/src', source: 'worker-rendezvous/src/index.js', description: 'Cloud service composition around the reusable transport registry and relay.', group: 'host' },
  { id: 'grid-js', label: 'Grid console', path: 'examples/grid', source: 'examples/grid/app.js', description: 'Pure JavaScript console and WASM host adapter for the separate grid game.', group: 'host' },
  { id: 'session-js', label: 'Session · JS', path: 'packages/session', source: 'packages/session/package.json', description: 'Injected browser session identity and continuation mechanisms.', group: 'shared' },
  { id: 'transport-js', label: 'Transport · JS', path: 'packages/transport', source: 'packages/transport/package.json', description: 'Rendezvous, ICE, reconnect, join-code parsing, relay and service registry.', group: 'shared' },
];
function owner(path) {
  const p = relative(root, path).replaceAll('\\', '/');
  return jsNodes.find(n => p === n.path || p.startsWith(n.path + '/'))?.id || (dirname(path) === root && extname(path) === '.html' ? 'browser-ui' : undefined);
}
function files(path) {
  return readdirSync(resolve(root, path), { withFileTypes: true }).flatMap(e => {
    if (['node_modules','vendor','pkg'].includes(e.name)) return [];
    const p = `${path}/${e.name}`;
    return e.isDirectory() ? files(p) : /\.[cm]?js$/.test(e.name) ? [p] : [];
  });
}
const jsFiles = [...jsNodes.flatMap(n => files(n.path)), ...readdirSync(root).filter(p => p.endsWith('.html'))];
const jsEdges = new Map();
for (const file of new Set(jsFiles)) {
  const absolute = resolve(root, file), from = owner(absolute), source = read(file);
  const chunks = file.endsWith('.html') ? [...source.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)].filter(m => /type=["']module["']/.test(m[1])).map(m => m[2]) : [source];
  for (const chunk of chunks) for (const entry of modules(chunk)[0]) {
    if (!entry.n?.startsWith('.')) continue;
    const to = owner(resolve(dirname(absolute), entry.n));
    if (!from || !to || from === to) continue;
    const key = `${from}/${to}`;
    if (!jsEdges.has(key)) jsEdges.set(key, { from, to, normal: true, evidence: [] });
    jsEdges.get(key).evidence.push(`${file}: ${entry.n}`);
  }
}
const data = { rust: { nodes: rustNodes, edges: [...rustEdges.values()] }, javascript: { nodes: jsNodes, edges: [...jsEdges.values()] } };
const path = resolve(root, 'docs/architecture/layers.html');
const before = readFileSync(path, 'utf8');
if (!before.includes('<script id="graph-data" type="application/json">')) throw new Error('Missing graph data element');
const embedded = JSON.stringify(data, null, 2).replaceAll('<', '\\u003c');
const after = before.replace(/(<script id="graph-data" type="application\/json">)[\s\S]*?(<\/script>)/, (_, start, end) => `${start}\n${embedded}\n${end}`);
if (after === before) console.log('Layer graph is current.');
else if (process.argv.includes('--check')) { console.error('Layer graph is stale. Run npm run layers:graph.'); process.exitCode = 1; }
else { writeFileSync(path, after, 'utf8'); console.log(`Updated layer graph: ${rustEdges.size} Rust and ${jsEdges.size} JavaScript dependency edges.`); }
