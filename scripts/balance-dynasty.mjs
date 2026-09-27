#!/usr/bin/env node
// Ratified #1547 evaluation. Generated worlds retain the ordinary fixed-slot
// crew simulation and authored factions; the legacy NPC duel transform is unused.
import { parse, stringify } from 'smol-toml';
import { createHash } from 'node:crypto';
import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import { spawn, execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const CONDITIONS = [
  { id: 'close-head-on', range: 40, allianceYaw: 0, dynastyYaw: Math.PI },
  { id: 'medium-head-on', range: 80, allianceYaw: 0, dynastyYaw: Math.PI },
  { id: 'long-head-on', range: 160, allianceYaw: 0, dynastyYaw: Math.PI },
  { id: 'alliance-facing-away', range: 80, allianceYaw: Math.PI, dynastyYaw: Math.PI },
  { id: 'dynasty-facing-away', range: 80, allianceYaw: 0, dynastyYaw: 0 },
];
export const SEEDS = Array.from({ length: 20 }, (_, index) => 1_547_001 + index);
export const SIM_SECONDS = 600;
export function tasks() {
  const duels = CONDITIONS.flatMap(condition => SEEDS.flatMap(seed => [false, true].map(mirrored => ({
    kind: 'duel', condition: condition.id, seed, mirrored,
  }))));
  const teams = SEEDS.slice(0, 10).flatMap(seed => [false, true].map(mirrored => ({
    kind: 'team', condition: 'reference-2v2', seed, mirrored,
  })));
  return [...duels, ...teams].map((task, index) => ({ ...task, id: String(index + 1).padStart(3, '0') }));
}

export function generatedWorld(source, task) {
  const world = parse(source);
  if (world.ship_slot?.length !== 4 || world.entity?.length !== 4) throw new Error('Expected four reference cruisers');
  if (task.kind === 'duel') {
    const condition = CONDITIONS.find(row => row.id === task.condition);
    if (!condition) throw new Error('Unknown engagement condition');
    world.ship_slot = [world.ship_slot[0], world.ship_slot[2]];
    world.entity = [world.entity[0], world.entity[2]];
    world.entity[0].transform = { position: [0, 0, condition.range / 2], rotation: [0, condition.allianceYaw, 0] };
    world.entity[1].transform = { position: [0, 0, -condition.range / 2], rotation: [0, condition.dynastyYaw, 0] };
    let script = world.script.setup;
    for (const team of ['alliance', 'dynasty']) {
      const death = `on_destroyed("world.cruiser_elimination.${team}_two", "${team}_loss");`;
      const recipients = `["${team}_one", "${team}_two"]`;
      const threshold = `ctx.flags.${team}_losses >= 2`;
      if (![death, recipients, threshold].every(fragment => script.includes(fragment))) {
        throw new Error(`Reference script contract changed for ${team}`);
      }
      script = script.replace(death, '').replace(recipients, `["${team}_one"]`)
        .replace(threshold, `ctx.flags.${team}_losses >= 1`);
    }
    world.script.setup = script;
  } else if (task.kind !== 'team') throw new Error('Unknown matchup kind');
  if (task.mirrored) for (const entity of world.entity) {
    entity.transform.position[0] *= -1;
    entity.transform.position[2] *= -1;
    entity.transform.rotation[1] += Math.PI;
  }
  return stringify(world);
}

export function classifyReport(report) {
  const flags = report.scenario?.flags;
  if (!Array.isArray(flags)) throw new Error('Missing scenario flag telemetry');
  const results = ['alliance_victory', 'dynasty_victory', 'match_draw']
    .filter(name => flags.some(flag => flag.name === name && flag.value === 1));
  if (report.outcome === 'timeout' && results.length === 0) return 'timeout';
  if (report.final_phase !== 'GameOver' || results.length !== 1) throw new Error('Missing or contradictory terminal result');
  return results[0] === 'match_draw' ? 'draw' : results[0].replace('_victory', '');
}

export function summarize(runs, kind) {
  const selected = runs.filter(run => run.kind === kind);
  const counts = { alliance: 0, dynasty: 0, draw: 0, timeout: 0, failed: 0 };
  for (const run of selected) counts[run.outcome ?? 'failed']++;
  const measured = selected.length - counts.failed;
  const half = (counts.draw + counts.timeout) / 2;
  const allianceRate = measured ? (counts.alliance + half) / measured : null;
  const dynastyRate = measured ? (counts.dynasty + half) / measured : null;
  const complete = selected.length === (kind === 'duel' ? 200 : 20) && counts.failed === 0;
  return { counts, measured, allianceRate, dynastyRate, complete,
    parity: kind === 'duel' && complete && allianceRate >= 0.4 && allianceRate <= 0.6 };
}

const sha = bytes => createHash('sha256').update(bytes).digest('hex');
export async function prepareOutput(output) {
  await mkdir(path.dirname(output), { recursive: true });
  // Refuse even an empty existing directory: no prior AAR may satisfy a run
  // whose process exits successfully without producing its requested report.
  await mkdir(output);
}
async function contentHashes(root, directory = 'assets') {
  const result = {};
  for (const entry of await readdir(path.join(root, directory), { withFileTypes: true })) {
    const relative = `${directory}/${entry.name}`;
    if (entry.isDirectory()) Object.assign(result, await contentHashes(root, relative));
    else if (/\.(toml|rhai|csv|json)$/.test(entry.name)) result[relative] = sha(await readFile(path.join(root, relative)));
  }
  return Object.fromEntries(Object.entries(result).sort(([left], [right]) => left.localeCompare(right)));
}
async function runOne(binary, root, output, task) {
  const reportPath = path.join(output, `${task.id}.json`);
  const args = ['--world', task.world, '--ship', 'assets/entities/alliance_cruiser.toml',
    '--seed', String(task.seed), '--sim-seconds', String(SIM_SECONDS), '--report', reportPath];
  let diagnostic = '';
  try {
    await new Promise((resolve, reject) => {
      const child = spawn(binary, args, { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
      const timer = setTimeout(() => { child.kill(); reject(new Error('Wall-clock execution limit exceeded')); }, 180_000);
      child.stdout.on('data', chunk => { diagnostic += String(chunk); });
      child.stderr.on('data', chunk => { diagnostic += String(chunk); });
      child.on('error', error => { clearTimeout(timer); reject(error); });
      child.on('exit', code => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error(`Exit ${code}`)); });
    });
    const bytes = await readFile(reportPath);
    const report = JSON.parse(bytes);
    if (report.seed !== task.seed) throw new Error('Report seed differs from requested seed');
    return { ...task, args, outcome: classifyReport(report), reportSha256: sha(bytes), simSeconds: report.sim_seconds };
  } catch (error) {
    return { ...task, args, outcome: 'failed', error: String(error) };
  } finally {
    await writeFile(path.join(output, `${task.id}.log`), diagnostic);
  }
}

async function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
  const options = new Map();
  for (let index = 2; index < process.argv.length; index += 2) options.set(process.argv[index], process.argv[index + 1]);
  if (!options.get('--binary') || !options.get('--out')) throw new Error('Usage: node scripts/balance-dynasty.mjs --binary <phoenix-headless> --out <directory> [--limit N] [--concurrency N]');
  const binary = path.resolve(options.get('--binary'));
  const output = path.resolve(options.get('--out'));
  const concurrency = Number(options.get('--concurrency') ?? 2);
  const limit = Number(options.get('--limit') ?? 220);
  if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > 8
      || !Number.isInteger(limit) || limit < 1 || limit > 220) throw new Error('Invalid concurrency or limit');
  await prepareOutput(output);
  const source = await readFile(path.join(root, 'assets/worlds/cruiser_elimination.toml'), 'utf8');
  const planned = tasks().slice(0, limit);
  for (const task of planned) {
    task.world = path.join(output, `${task.kind}-${task.condition}-${task.mirrored ? 'mirrored' : 'original'}.toml`);
    const world = generatedWorld(source, task);
    task.worldSha256 = sha(world);
    await writeFile(task.world, world);
  }
  const git = args => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
  const provenance = { revision: git(['rev-parse', 'HEAD']), sourcePatch: git(['diff', 'HEAD', '--', 'src', 'build.rs', 'Cargo.toml', 'Cargo.lock', '.cargo']),
    binary, binarySha256: sha(await readFile(binary)), runnerSha256: sha(await readFile(fileURLToPath(import.meta.url))),
    content: await contentHashes(root), conditions: CONDITIONS, seeds: SEEDS, simSeconds: SIM_SECONDS,
    denominator: '200 duels; each draw or simulation timeout contributes half a win to each cruiser; process failures invalidate the batch',
    concurrency, tasks: planned };
  await writeFile(path.join(output, 'manifest.json'), JSON.stringify(provenance, null, 2));
  const runs = [];
  let next = 0;
  await Promise.all(Array.from({ length: concurrency }, async () => {
    while (next < planned.length) {
      const task = planned[next++];
      const run = await runOne(binary, root, output, task);
      runs.push(run);
      process.stderr.write(`${runs.length}/${planned.length} ${task.id}: ${run.outcome}\n`);
      await writeFile(path.join(output, `${task.id}.result.json`), JSON.stringify(run, null, 2));
    }
  }));
  runs.sort((left, right) => left.id.localeCompare(right.id));
  const summary = { duel: summarize(runs, 'duel'), team: summarize(runs, 'team'),
    byCondition: Object.fromEntries(CONDITIONS.map(condition => [condition.id,
      summarize(runs.filter(run => run.condition === condition.id), 'duel')])), runs };
  await writeFile(path.join(output, 'summary.json'), JSON.stringify(summary, null, 2));
  process.stdout.write(JSON.stringify({ duel: summary.duel, team: summary.team }, null, 2) + '\n');
  if (!summary.duel.parity || !summary.team.complete) process.exitCode = 1;
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().catch(error => { process.stderr.write(`${error.stack}\n`); process.exitCode = 1; });
}
