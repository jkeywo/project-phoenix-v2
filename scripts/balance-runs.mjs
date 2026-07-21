// balance-runs.mjs — TOML-driven matchup×seed batch runner for phoenix-headless
// (issue #845).
//
// Reads a TOML config describing a matrix of side compositions (or an explicit
// list of matchups, or a scenario sweep), fans out `phoenix-headless` processes
// in parallel across every (matchup × seed) pair, and folds the per-run report
// JSON into win/loss/draw rates, time-to-kill distributions, and damage margins
// per matchup — emitted as one merged JSON object plus a readable markdown table.
//
//   node scripts/balance-runs.mjs <config.toml> [--out <dir>]
//
// Prerequisites (same assumptions as the other scripts here):
//   - `npm install` has run (this script imports smol-toml).
//   - The release binary is built:
//       cargo build --release --features headless --bin phoenix-headless
//     This script does NOT build it — a release build is far too slow to do per
//     invocation; it errors with a build hint if the binary is missing.
//
// ── Design notes (the merge is a PURE fold; keep it that way) ────────────────
//
// The report contract this consumes (src/headless/report.rs, src/core/balance.rs):
//   report.outcome                         "victory" | "defeat" | "draw" | "timeout"
//   report.sides.{player,enemy}.damage_dealt
//   report.damage_by_ship[uuid].death      [tick, sim_t] | null
//   report.seed, report.final_phase, report.sim_seconds
//
// Merge conventions (documented so the numbers are unambiguous):
//   - win = victory, loss = defeat. `draw` and `timeout` are tallied in their
//     own columns and NOT merged together. Win-rate denominator is every run
//     that produced a report (wins+losses+draws+timeouts); failed/crashed runs
//     are excluded from the rate and reported separately.
//   - Time-to-kill per run = the LATEST ship death time (death[1], sim_t) in
//     damage_by_ship — the last death observed. In a 1v1 that is the decisive
//     kill (player death → defeat, last enemy death → victory), so it marks
//     when the fight concluded. In a multi-ship matchup that hits the
//     sim-seconds cutoff (outcome=timeout) the latest death may be a
//     non-decisive escort loss, not a fight-ending kill. A run with no deaths
//     (e.g. a bloodless draw/timeout) contributes no TTK sample. min/median/max
//     are taken over the collected samples per matchup.
//   - Damage margin per run = player.damage_dealt - enemy.damage_dealt; the
//     merge reports the mean over every run that produced a report. Positive =
//     the player side out-damaged the enemy.
//
// The pure exports (mergeReports / formatMarkdown / formatMatrix / expandMatchups
// / resolveSeeds) are unit-tested in tests/client/balance-runs.test.js with
// fabricated report objects — no simulation required. Everything that spawns a
// process lives in main() and its helpers.

import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { spawn } from 'node:child_process';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parse as parseToml } from 'smol-toml';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, '..');

// Default world for class-matrix / explicit matchups. Matches the headless
// `--side-a/--side-b` duel transform, which is authored against duel.toml.
const DEFAULT_MATCHUP_WORLD = 'assets/worlds/duel.toml';

// Wall-clock kill switch for one phoenix-headless process. A duel at 300 sim-
// seconds finishes in a few wall-seconds, so 300s wall is a very generous
// ceiling that only fires on a genuine hang (a run that never closes stdout /
// never exits). Overridable per-config via `run_timeout_secs`.
const DEFAULT_RUN_TIMEOUT_SECS = 300;

// ── Pure config expansion ───────────────────────────────────────────────────

/** A single-hull-or-list side rendered as a stable label fragment. */
function sideLabel(side) {
  return Array.isArray(side) ? side.join('+') : String(side);
}

/**
 * Canonical matchup label. `side_b` empty/absent (a scenario sweep) → just the
 * player composition; otherwise `<a>_vs_<b>`. Used as the summary key and the
 * grid lookup, so it must be deterministic.
 */
export function matchupLabel(sideA, sideB) {
  const a = sideLabel(sideA);
  return sideB && sideB.length ? `${a}_vs_${sideLabel(sideB)}` : a;
}

/**
 * Resolve the `seeds` knob to an explicit list. A number N → seeds 1..N; an
 * array is taken verbatim. PURE.
 */
export function resolveSeeds(seeds) {
  if (Array.isArray(seeds)) return seeds.slice();
  const n = Number(seeds);
  if (!Number.isInteger(n) || n < 1) {
    throw new Error(`\`seeds\` must be a positive integer count or an array, got ${JSON.stringify(seeds)}`);
  }
  return Array.from({ length: n }, (_, i) => i + 1);
}

/**
 * Expand a parsed config into matchup descriptors `{label, sideA, sideB, world}`.
 * PURE — no filesystem, no spawning.
 *
 * Three shapes, checked in order (a config may combine class_matrix + explicit
 * [[matchup]] entries; scenario_hulls is standalone):
 *   1. class_matrix = ["courier", ...]  → every ORDERED pair incl. mirrors,
 *      side_a = [a] vs side_b = [b], on the matchup world (default duel.toml).
 *   2. [[matchup]] { side_a, side_b, name? } → arbitrary X-vs-Y.
 *   3. scenario_hulls = ["courier", ...] + a scenario `world` → each hull is a
 *      side_a with NO side_b (the world defines the enemies). Same world, N
 *      seeds per hull.
 */
export function expandMatchups(config) {
  const matchups = [];
  const matchupWorld = config.world ?? DEFAULT_MATCHUP_WORLD;

  if (Array.isArray(config.class_matrix)) {
    for (const a of config.class_matrix) {
      for (const b of config.class_matrix) {
        matchups.push({ label: matchupLabel([a], [b]), sideA: [a], sideB: [b], world: matchupWorld });
      }
    }
  }

  if (Array.isArray(config.matchup)) {
    for (const m of config.matchup) {
      const sideA = Array.isArray(m.side_a) ? m.side_a : [m.side_a];
      const sideB = m.side_b == null ? [] : Array.isArray(m.side_b) ? m.side_b : [m.side_b];
      matchups.push({
        label: m.name ?? matchupLabel(sideA, sideB),
        sideA,
        sideB,
        world: m.world ?? matchupWorld,
      });
    }
  }

  if (Array.isArray(config.scenario_hulls)) {
    if (!config.world) {
      throw new Error('`scenario_hulls` needs a scenario `world` (the world defines the enemies)');
    }
    for (const hull of config.scenario_hulls) {
      matchups.push({ label: matchupLabel([hull], []), sideA: [hull], sideB: [], world: config.world });
    }
  }

  if (matchups.length === 0) {
    throw new Error('config declares no matchups: give class_matrix, [[matchup]], or scenario_hulls');
  }
  return matchups;
}

/**
 * Cross matchups with seeds into a flat list of run tasks. PURE.
 */
export function buildRunTasks(config, matchups) {
  const seeds = resolveSeeds(config.seeds ?? 1);
  const simSeconds = config.sim_seconds ?? 60;
  const hz = config.hz; // optional; undefined means the binary's own default
  const timeoutSecs = config.run_timeout_secs ?? DEFAULT_RUN_TIMEOUT_SECS;
  const tasks = [];
  for (const m of matchups) {
    for (const seed of seeds) {
      tasks.push({
        matchup: m.label,
        sideA: m.sideA,
        sideB: m.sideB,
        world: m.world,
        seed,
        simSeconds,
        hz,
        timeoutSecs,
      });
    }
  }
  return tasks;
}

// ── Pure merge + formatting ─────────────────────────────────────────────────

/** Median of a numeric array (already unsorted is fine); null if empty. */
function median(nums) {
  if (nums.length === 0) return null;
  const s = [...nums].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

const mean = (nums) => (nums.length ? nums.reduce((a, b) => a + b, 0) / nums.length : null);

/** Latest ship-death sim_t in a report, or null if nobody died (draw/timeout). */
function runTtk(report) {
  const ships = report?.damage_by_ship ?? {};
  let latest = null;
  for (const ledger of Object.values(ships)) {
    const death = ledger?.death;
    if (Array.isArray(death) && death.length >= 2 && typeof death[1] === 'number') {
      if (latest === null || death[1] > latest) latest = death[1];
    }
  }
  return latest;
}

/**
 * Fold an array of run results into per-matchup summary metrics. PURE — a fold
 * over report objects, no I/O.
 *
 * Each run is `{ matchup, sideA?, sideB?, seed, report? , error?, exitCode? }`.
 * A run WITH a report is classified by `report.outcome`; a run WITHOUT one
 * (error set) is a failure — counted, never fatal.
 */
export function mergeReports(runs) {
  const byMatchup = new Map();
  const ensure = (run) => {
    let m = byMatchup.get(run.matchup);
    if (!m) {
      m = {
        label: run.matchup,
        sideA: run.sideA ?? null,
        sideB: run.sideB ?? null,
        total: 0,
        wins: 0,
        losses: 0,
        draws: 0,
        timeouts: 0,
        failures: 0,
        ttkSamples: [],
        marginSamples: [],
        failuresDetail: [],
      };
      byMatchup.set(run.matchup, m);
    }
    return m;
  };

  for (const run of runs) {
    const m = ensure(run);
    m.total += 1;

    if (!run.report) {
      m.failures += 1;
      m.failuresDetail.push({
        seed: run.seed ?? null,
        error: run.error ?? 'unknown error',
        exitCode: run.exitCode ?? null,
      });
      continue;
    }

    switch (run.report.outcome) {
      case 'victory': m.wins += 1; break;
      case 'defeat': m.losses += 1; break;
      case 'draw': m.draws += 1; break;
      case 'timeout': m.timeouts += 1; break;
      default: m.timeouts += 1; break; // unknown outcome: bucket with timeout, still "completed"
    }

    const ttk = runTtk(run.report);
    if (ttk !== null) m.ttkSamples.push(ttk);

    const player = run.report.sides?.player?.damage_dealt;
    const enemy = run.report.sides?.enemy?.damage_dealt;
    if (typeof player === 'number' && typeof enemy === 'number') {
      m.marginSamples.push(player - enemy);
    }
  }

  const matchups = {};
  for (const m of byMatchup.values()) {
    const completed = m.wins + m.losses + m.draws + m.timeouts;
    matchups[m.label] = {
      label: m.label,
      sideA: m.sideA,
      sideB: m.sideB,
      total: m.total,
      completed,
      wins: m.wins,
      losses: m.losses,
      draws: m.draws,
      timeouts: m.timeouts,
      failures: m.failures,
      winRate: completed > 0 ? m.wins / completed : null,
      ttk: {
        min: m.ttkSamples.length ? Math.min(...m.ttkSamples) : null,
        median: median(m.ttkSamples),
        max: m.ttkSamples.length ? Math.max(...m.ttkSamples) : null,
        count: m.ttkSamples.length,
      },
      damageMargin: { mean: mean(m.marginSamples), count: m.marginSamples.length },
      failuresDetail: m.failuresDetail,
    };
  }

  const totals = { runs: 0, wins: 0, losses: 0, draws: 0, timeouts: 0, failures: 0 };
  for (const s of Object.values(matchups)) {
    totals.runs += s.total;
    totals.wins += s.wins;
    totals.losses += s.losses;
    totals.draws += s.draws;
    totals.timeouts += s.timeouts;
    totals.failures += s.failures;
  }

  return { matchups, totals };
}

const pct = (r) => (r === null ? '—' : `${(r * 100).toFixed(0)}%`);
const num = (n, d = 1) => (n === null || n === undefined ? '—' : n.toFixed(d));

/**
 * Render a per-matchup summary table. PURE — string in, string out. Works for
 * every config shape (one row per matchup).
 */
export function formatMarkdown(summary) {
  const rows = Object.values(summary.matchups);
  const lines = [];
  lines.push('| Matchup | Runs | Win% | W/L/D/T | Fail | TTK min/med/max (s) | Dmg margin |');
  lines.push('|---|---:|---:|:---:|---:|:---:|---:|');
  for (const s of rows) {
    const wldt = `${s.wins}/${s.losses}/${s.draws}/${s.timeouts}`;
    const ttk = s.ttk.count
      ? `${num(s.ttk.min)} / ${num(s.ttk.median)} / ${num(s.ttk.max)}`
      : '—';
    lines.push(
      `| ${s.label} | ${s.total} | ${pct(s.winRate)} | ${wldt} | ${s.failures} | ${ttk} | ${num(s.damageMargin.mean)} |`,
    );
  }
  const t = summary.totals;
  lines.push('');
  lines.push(
    `**Totals:** ${t.runs} runs — ${t.wins}W / ${t.losses}L / ${t.draws}D / ${t.timeouts}T, ${t.failures} failed.`,
  );
  return lines.join('\n');
}

/**
 * Render a win-rate grid for a class matrix: rows = side_a class, cols = side_b
 * class, cell = win-rate%. PURE. Cells with no matching matchup show `—`.
 */
export function formatMatrix(summary, classes, metric = 'winRate') {
  const cell = (a, b) => {
    const s = summary.matchups[matchupLabel([a], [b])];
    if (!s) return '—';
    if (metric === 'ttk') return s.ttk.median === null ? '—' : num(s.ttk.median);
    return pct(s.winRate);
  };
  const header = `| side_a \\ side_b | ${classes.join(' | ')} |`;
  const sep = `|---|${classes.map(() => '---:').join('|')}|`;
  const lines = [header, sep];
  for (const a of classes) {
    lines.push(`| **${a}** | ${classes.map((b) => cell(a, b)).join(' | ')} |`);
  }
  return lines.join('\n');
}

// ── Side-effecting run machinery (kept out of the pure exports) ──────────────

/** Absolute path to the release binary, platform-suffixed. */
function binaryPath() {
  const name = process.platform === 'win32' ? 'phoenix-headless.exe' : 'phoenix-headless';
  return path.join(root, 'target', 'release', name);
}

/** Spawn one headless run and resolve to a run result (never rejects). */
function runOne(bin, task) {
  const args = [
    '--world', task.world,
    '--side-a', task.sideA.join(','),
  ];
  if (task.sideB && task.sideB.length) args.push('--side-b', task.sideB.join(','));
  args.push('--seed', String(task.seed));
  args.push('--sim-seconds', String(task.simSeconds));
  if (task.hz != null) args.push('--hz', String(task.hz));
  args.push('--report-format', 'json');

  const timeoutSecs = task.timeoutSecs ?? DEFAULT_RUN_TIMEOUT_SECS;

  return new Promise((resolve) => {
    const child = spawn(bin, args, { cwd: root, shell: false });
    let stdout = '';
    let stderr = '';
    let settled = false;
    let timer = null;

    // Resolve at most once and always tear down the kill timer, so a run that
    // closes normally can never later trip the timeout kill.
    const finish = (result) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      resolve(result);
    };

    // Per-process kill switch: a hung phoenix-headless (never closes stdout,
    // never exits) would otherwise wedge a concurrency-pool slot forever. Kill
    // it and record a FAILED run — same shape as every other failure — so the
    // unattended batch keeps going.
    timer = setTimeout(() => {
      const stderrTail = stderr.split('\n').filter(Boolean).slice(-5).join('\n');
      try {
        child.kill('SIGKILL'); // best-effort; on win32 kills the direct child
      } catch {
        // ignore — the process may already be gone
      }
      finish({ ...taskId(task), error: `timeout after ${timeoutSecs}s`, exitCode: null, stderrTail });
    }, timeoutSecs * 1000);
    if (typeof timer.unref === 'function') timer.unref();

    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    child.on('error', (err) => {
      finish({ ...taskId(task), error: `spawn failed: ${err.message}`, exitCode: null });
    });
    child.on('close', (code) => {
      // Note: we deliberately do NOT pass --fail-on-game-over, so a duel loser
      // reaching GameOver exits 0. A non-zero exit or unparseable stdout is a
      // genuine failure — recorded per-run, never sinking the batch.
      const stderrTail = stderr.split('\n').filter(Boolean).slice(-5).join('\n');
      if (code !== 0) {
        finish({ ...taskId(task), error: `exit ${code}`, exitCode: code, stderrTail });
        return;
      }
      let report;
      try {
        report = JSON.parse(stdout);
      } catch (e) {
        finish({ ...taskId(task), error: `unparseable report: ${e.message}`, exitCode: code, stderrTail });
        return;
      }
      finish({ ...taskId(task), report });
    });
  });
}

/** The identity fields carried onto every run result. */
function taskId(task) {
  return { matchup: task.matchup, sideA: task.sideA, sideB: task.sideB, seed: task.seed };
}

/** Hand-rolled concurrency pool (no p-limit dependency). */
async function runPool(tasks, concurrency, worker) {
  const results = new Array(tasks.length);
  let next = 0;
  const drain = async () => {
    while (next < tasks.length) {
      const i = next++;
      results[i] = await worker(tasks[i], i);
    }
  };
  const workers = Array.from({ length: Math.max(1, Math.min(concurrency, tasks.length)) }, drain);
  await Promise.all(workers);
  return results;
}

function parseCliArgs(argv) {
  const out = { config: null, out: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--out') out.out = argv[++i];
    else if (a === '-h' || a === '--help') out.help = true;
    else if (!out.config) out.config = a;
    else throw new Error(`unexpected argument ${JSON.stringify(a)}`);
  }
  return out;
}

const USAGE = `balance-runs — TOML-driven matchup×seed batch runner for phoenix-headless

USAGE:
    node scripts/balance-runs.mjs <config.toml> [--out <dir>]

Reads <config.toml>, fans out phoenix-headless across every (matchup × seed),
and merges the per-run reports. Markdown summary → stdout; with --out <dir>,
merged.json + summary.md are also written there (keep that dir out of git).

Requires: npm install, and a release binary at target/release/phoenix-headless
(build with: cargo build --release --features headless --bin phoenix-headless).`;

async function main() {
  const cli = parseCliArgs(process.argv.slice(2));
  if (cli.help || !cli.config) {
    console.log(USAGE);
    process.exit(cli.help ? 0 : 2);
  }

  const bin = binaryPath();
  if (!existsSync(bin)) {
    console.error(`phoenix-headless release binary not found at:\n  ${bin}\n`);
    console.error('Build it first:');
    console.error('  cargo build --release --features headless --bin phoenix-headless');
    process.exit(1);
  }

  const config = parseToml(await readFile(cli.config, 'utf8'));
  const matchups = expandMatchups(config);
  const tasks = buildRunTasks(config, matchups);
  const concurrency = config.concurrency ?? os.cpus().length;

  console.error(
    `[balance-runs] ${matchups.length} matchups × ${resolveSeeds(config.seeds ?? 1).length} seeds ` +
      `= ${tasks.length} runs, concurrency ${concurrency}`,
  );

  let done = 0;
  const runs = await runPool(tasks, concurrency, async (task) => {
    const result = await runOne(bin, task);
    done += 1;
    const status = result.report ? result.report.outcome : `FAILED (${result.error})`;
    console.error(`[balance-runs] (${done}/${tasks.length}) ${task.matchup} seed ${task.seed}: ${status}`);
    return result;
  });

  const summary = mergeReports(runs);
  const markdown = formatMarkdown(summary);

  let out = markdown;
  if (Array.isArray(config.class_matrix)) {
    out = `${formatMatrix(summary, config.class_matrix)}\n\n${markdown}`;
  }
  console.log(out);

  if (cli.out) {
    await mkdir(cli.out, { recursive: true });
    await writeFile(path.join(cli.out, 'merged.json'), `${JSON.stringify(summary, null, 2)}\n`);
    await writeFile(path.join(cli.out, 'summary.md'), `${out}\n`);
    console.error(`[balance-runs] wrote ${path.join(cli.out, 'merged.json')} and summary.md`);
  }
}

// Guard the CLI entry so importing this module (for tests) never spawns.
if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
