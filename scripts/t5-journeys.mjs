#!/usr/bin/env node
// Versioned, bounded evidence runner (#1553). Choose a lane explicitly and
// coordinate the native lane's Cargo slot; this is not the full integration gate.
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile, mkdir, stat, utimes } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const FORMAT = 'phoenix-t5-journeys-v1';
export const JOURNEYS = Object.freeze([
  { id: 'J1', lane: 'rust', target: 't5_journeys', filter: 'competing_slot_picks_freeze_into_actual_launch_and_convoy_report', expected: 1 },
  { id: 'GM', lane: 'rust', target: 'gm_objective', expected: 6 },
  { id: 'J3-runtime', lane: 'rust', target: 'lockstep_recovery', filter: 'a_diverged_host_is_healed_and_the_whole_fleet_reconverges', expected: 1 },
  { id: 'J4-runtime', lane: 'rust', target: 'cruiser_elimination', filter: 'competitive_world_results_and_destroyed_crew_keep_their_identity', expected: 1 },
  { id: 'J4-authority', lane: 'rust', package: 'phoenix-simulation', library: true, filter: 'command_admission::tests::crew_spectator_dead_hull_refuses_controls_while_live_crew_still_controls_own_ship', expected: 1 },
  { id: 'J2-J5-client', lane: 'js', expected: 4 },
]);
export function selectJourneys(lane) {
  if (!['js', 'rust'].includes(lane)) throw new Error('Choose --lane js or --lane rust; native work requires the coordinated Cargo slot');
  return JOURNEYS.filter(row => row.lane === lane);
}
export function verifyCount(row, output, json) {
  const count = row.lane === 'js' ? json?.numPassedTests
    : Number(output.match(/test result: ok\. (\d+) passed; 0 failed;/)?.[1]);
  if (count !== row.expected || (row.lane === 'js' && (json.numFailedTests || json.success !== true))) {
    throw new Error(`${row.id}: expected ${row.expected} passing tests, observed ${count}`);
  }
  return count;
}
export async function freshOutput(directory) {
  await mkdir(path.dirname(directory), { recursive: true });
  await mkdir(directory); // Never let stale logs/results establish a new pass.
}
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
async function main() {
  const options = new Map();
  for (let i = 2; i < process.argv.length; i += 2) options.set(process.argv[i], process.argv[i + 1]);
  const lane = options.get('--lane'), selected = selectJourneys(lane);
  if (!options.get('--out')) throw new Error('Choose --out <fresh evidence directory>');
  const output = path.resolve(options.get('--out'));
  await freshOutput(output);
  const git = args => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
  const paths = ['Cargo.toml', 'Cargo.lock', 'scripts/t5-journeys.mjs', 'tests/t5_journeys.rs',
    'tests/client/t5-journeys.test.js', 'tests/gm_objective.rs', 'tests/lockstep_recovery.rs',
    'tests/cruiser_elimination.rs', 'crates/phoenix-simulation/src/command_admission/mod.rs', 'assets/worlds/alliance_convoy_escort.toml',
    'assets/worlds/cruiser_elimination.toml', 'assets/entities/alliance_cruiser.toml',
    'assets/entities/alliance_destroyer.toml', 'assets/entities/dynasty_player_cruiser.toml',
    'assets/strings/strings.csv'];
  const hashes = {};
  for (const file of paths) hashes[file] = sha(await readFile(path.join(root, file)));
  const evidence = { format: FORMAT, startedUtc: new Date().toISOString(), lane,
    revision: git(['rev-parse', 'HEAD']), dirty: git(['status', '--porcelain']) !== '',
    trackedPatch: git(['diff', 'HEAD', '--', 'src', 'assets', 'tests', 'scripts']),
    hashes, node: process.version, cargoTargetDir: process.env.CARGO_TARGET_DIR || 'target',
    limitations: 'In-process protocol and jsdom presentation; headless authority uses synthetic inputs. No actual device, network handover, pixel layout, audio or human acceptance claim.',
    results: [], status: 'running' };
  const save = () => writeFile(path.join(output, 'evidence.json'), JSON.stringify(evidence, null, 2));
  await save();
  // AGENTS.md's shared-target rule: a new test executable is not sufficient.
  if (lane === 'rust') {
    const library = path.join(root, 'crates/phoenix-simulation/src/lib.rs'), metadata = await stat(library);
    await utimes(library, metadata.atime, new Date());
  }
  for (const row of selected) {
    const jsonPath = path.join(output, `${row.id}.json`);
    const executable = row.lane === 'rust' ? 'cargo' : process.execPath;
    const args = row.lane === 'rust'
      ? ['test', ...(row.package ? ['-p', row.package] : []), '--features', 'headless', ...(row.library ? ['--lib'] : ['--test', row.target]),
        ...(row.filter ? [row.filter, '--', '--exact', '--nocapture'] : ['--', '--nocapture'])]
      : [path.join(root, 'node_modules/vitest/vitest.mjs'), 'run', 'tests/client/t5-journeys.test.js',
        '--reporter=json', `--outputFile=${jsonPath}`];
    const result = { id: row.id, executable, args, startedUtc: new Date().toISOString(), status: 'running' };
    let log = '';
    try {
      const exit = await new Promise((resolve, reject) => {
        const child = spawn(executable, args, { cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
        child.stdout.on('data', bytes => { log += bytes; process.stderr.write(bytes); });
        child.stderr.on('data', bytes => { log += bytes; process.stderr.write(bytes); });
        child.on('error', reject); child.on('close', resolve);
      });
      if (exit !== 0) throw new Error(`Process exited ${exit}`);
      result.passed = verifyCount(row, log, row.lane === 'js' ? JSON.parse(await readFile(jsonPath, 'utf8')) : null);
      result.status = 'passed';
    } catch (error) { result.status = 'failed'; result.error = String(error); }
    result.finishedUtc = new Date().toISOString();
    await writeFile(path.join(output, `${row.id}.log`), log);
    evidence.results.push(result);
    if (result.status === 'failed') { evidence.status = 'failed'; await save(); process.exitCode = 1; return; }
    await save();
  }
  evidence.status = 'passed'; evidence.finishedUtc = new Date().toISOString(); await save();
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().catch(error => { process.stderr.write(`${error.stack}\n`); process.exitCode = 1; });
}
