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
//   report.station_activity.buckets[].stations[]  {station, human, ai, offline}
//   report.scenario.objectives[]           {id, mandatory, status, ...}
//   report.scenario.flags[]                {name, value}
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
//   - Mandatory-set completion is a whole-run measure: a measurable run has at
//     least one mandatory scenario objective, and succeeds only when ALL of
//     them finish `Completed`. `Active` and `Failed` both fail the set. Reports
//     with no usable objective projection are excluded from the denominator
//     and counted by telemetry-gap reason; process failures remain failures and
//     are excluded too.
//   - The Falling Skyway clean-ledger classification applies only when a report
//     carries `campaign.skyway.*` flags. A sampled run is `clean` when the
//     locked six-objective spine is complete and all four campaign facts hold
//     (no traffic lost, commitments clean, skyhook held, evidence filed),
//     `completedButMid` when that spine completed but a campaign fact did not,
//     and `incomplete` when the spine did not complete. Malformed objective or
//     flag telemetry is unmeasurable, never silently classified.
//
// The pure exports (mergeReports / formatMarkdown / formatMatrix / expandMatchups
// / buildRunTasks / buildRunArgs
// / resolveSeeds / evaluateThresholds / formatThresholds / failedThresholds / formatPhases /
// formatStationActivity / runFileName) are unit-tested in
// tests/client/balance-runs.test.js with
// fabricated report objects — no simulation required. Everything that spawns a
// process lives in main() and its helpers.
//
// ── Regression thresholds (issue #915) ──────────────────────────────────────
//
// A config may declare regression thresholds, globally and/or per-[[matchup]]:
//
//   [thresholds]                 # applies to every matchup
//   max_failures = 0
//
//   [[matchup]]
//   name = "destroyer_vs_harrow_patrol"
//   side_a = ["destroyer"]
//   side_b = ["ship_harrow_patrol"]
//     [matchup.thresholds]       # overrides the global table per key
//     min_win_rate = 0.5
//
// Known metrics: min_win_rate / max_win_rate (over completed runs),
// min_ttk_median / max_ttk_median (seconds), min_damage_margin /
// max_damage_margin (mean player-minus-enemy damage), max_failures.
// Threshold results are RECORDED — written into merged.json and summary.md —
// and never change the exit code unless the ratified config explicitly sets
// `enforce_thresholds = true`.

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
 * Resolve a scenario-sweep player-hull authoring value to `--ship`'s literal
 * template path. Scenario hulls are Alliance player ships, so a short class
 * name follows the same documented convention as the duel resolver
 * (`destroyer` -> `assets/entities/alliance_destroyer.toml`). A value already
 * written as a path/TOML filename is authoritative and passes through. PURE —
 * no hard-coded catalogue of whichever hulls happen to ship today.
 */
export function scenarioShipPath(hull) {
  const value = String(hull);
  if (value.includes('/') || value.includes('\\') || value.endsWith('.toml')) return value;
  return `assets/entities/alliance_${value}.toml`;
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
        // Per-matchup threshold overrides, carried through so
        // evaluateThresholds can merge them over the global [thresholds].
        thresholds: m.thresholds,
      });
    }
  }

  if (Array.isArray(config.scenario_hulls)) {
    if (!config.world) {
      throw new Error('`scenario_hulls` needs a scenario `world` (the world defines the enemies)');
    }
    for (const hull of config.scenario_hulls) {
      matchups.push({
        label: matchupLabel([hull], []),
        sideA: [hull],
        sideB: [],
        world: config.world,
        playerShip: scenarioShipPath(hull),
      });
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
        playerShip: m.playerShip,
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

/**
 * Filename for one run's persisted AAR report: `<matchup>-seed<seed>.json`
 * with anything filesystem-hostile in the label replaced. PURE.
 */
export function runFileName(matchup, seed) {
  const safe = String(matchup).replace(/[^A-Za-z0-9._-]/g, '_');
  return `${safe}-seed${seed}.json`;
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

/** Validate the canonical objective rows once for every scenario metric. */
function objectiveRowsObservation(report) {
  const scenario = report?.scenario;
  if (scenario === null || typeof scenario !== 'object' || Array.isArray(scenario)) {
    return { sampled: false, reason: 'missingScenario' };
  }
  const objectives = scenario.objectives;
  if (!Array.isArray(objectives)) {
    return { sampled: false, reason: 'malformedObjectives' };
  }
  const ids = new Set();
  for (const objective of objectives) {
    if (
      objective === null
      || typeof objective !== 'object'
      || typeof objective.id !== 'string'
      || objective.id.trim().length === 0
      || typeof objective.mandatory !== 'boolean'
      || !['Active', 'Completed', 'Failed'].includes(objective.status)
      || ids.has(objective.id)
    ) {
      return { sampled: false, reason: 'malformedObjectives' };
    }
    ids.add(objective.id);
  }
  return { sampled: true, objectives };
}

/**
 * Classify one report's generic mandatory-objective set without allowing absent
 * telemetry to masquerade as either a loss or vacuous success.
 */
function mandatorySetObservation(report) {
  const observation = objectiveRowsObservation(report);
  if (!observation.sampled) return observation;
  const { objectives } = observation;
  const mandatory = objectives.filter((objective) => objective.mandatory);
  if (mandatory.length === 0) {
    return { sampled: false, reason: 'noMandatoryObjectives' };
  }
  return {
    sampled: true,
    completed: mandatory.every((objective) => objective.status === 'Completed'),
  };
}

const FALLING_SKYWAY_CAMPAIGN_PREFIX = 'campaign.skyway.';
const FALLING_SKYWAY_CLEAN_FLAGS = [
  'campaign.skyway.traffic.none',
  'campaign.skyway.commitments.clean',
  'campaign.skyway.skyhook.held',
  'campaign.skyway.evidence.filed',
];
const FALLING_SKYWAY_MANDATORY_SPINE = [
  'obj-a1-survey',
  'obj-a1-triage',
  'obj-a2-line',
  'obj-a2-rescue',
  'obj-a2-storm',
  'obj-a3-head',
];

/**
 * Classify the Falling Skyway clean-ledger benchmark from the canonical
 * end-of-run scenario projection. Reports from other scenarios are explicitly
 * not applicable; telemetry gaps in an identifiable Falling Skyway report are
 * unmeasurable rather than a benchmark failure.
 */
function fallingSkywayCleanLedgerObservation(report) {
  const scenario = report?.scenario;
  if (scenario === null || typeof scenario !== 'object' || Array.isArray(scenario)) {
    return { applicable: false };
  }

  const flags = scenario.flags;
  const applicable = Array.isArray(flags) && flags.some((flag) => (
    flag !== null
    && typeof flag === 'object'
    && typeof flag.name === 'string'
    && flag.name.startsWith(FALLING_SKYWAY_CAMPAIGN_PREFIX)
  ));
  if (!applicable) return { applicable: false };

  const values = new Map();
  for (const flag of flags) {
    if (
      flag === null
      || typeof flag !== 'object'
      || Array.isArray(flag)
      || typeof flag.name !== 'string'
      || flag.name.trim().length === 0
      || !Number.isSafeInteger(flag.value)
      || flag.value === 0
      || values.has(flag.name)
    ) {
      return { applicable: true, sampled: false, reason: 'malformedFlags' };
    }
    values.set(flag.name, flag.value);
  }

  const objectiveRows = objectiveRowsObservation(report);
  if (!objectiveRows.sampled) {
    return { applicable: true, sampled: false, reason: objectiveRows.reason };
  }
  const objectivesById = new Map(objectiveRows.objectives.map((objective) => [objective.id, objective]));
  const mandatorySpineComplete = FALLING_SKYWAY_MANDATORY_SPINE.every((id) => {
    const objective = objectivesById.get(id);
    return objective?.mandatory === true && objective.status === 'Completed';
  });
  if (!mandatorySpineComplete) {
    return { applicable: true, sampled: true, classification: 'incomplete' };
  }

  const clean = FALLING_SKYWAY_CLEAN_FLAGS.every((name) => (values.get(name) ?? 0) > 0);
  return {
    applicable: true,
    sampled: true,
    classification: clean ? 'clean' : 'completedButMid',
  };
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
        phaseSeconds: {},
        stationActivity: {},
        mandatorySetSampled: 0,
        mandatorySetCompleted: 0,
        mandatorySetUnmeasurable: {
          missingScenario: 0,
          malformedObjectives: 0,
          noMandatoryObjectives: 0,
        },
        fallingSkywayCleanLedgerApplicable: false,
        fallingSkywayCleanLedgerSampled: 0,
        fallingSkywayCleanLedgerClean: 0,
        fallingSkywayCleanLedgerCompletedButMid: 0,
        fallingSkywayCleanLedgerIncomplete: 0,
        fallingSkywayCleanLedgerUnmeasurable: {
          missingScenario: 0,
          malformedObjectives: 0,
          noMandatoryObjectives: 0,
          malformedFlags: 0,
        },
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

    // Doctrine phase occupancy (issue #915): sum every ship's per-phase
    // sim-seconds over the matchup's runs. Both sides are folded together —
    // the per-run reports persisted under runs/ keep the per-ship split.
    for (const ledger of Object.values(run.report.damage_by_ship ?? {})) {
      for (const [phase, secs] of Object.entries(ledger?.phase_seconds ?? {})) {
        if (typeof secs === 'number') {
          m.phaseSeconds[phase] = (m.phaseSeconds[phase] ?? 0) + secs;
        }
      }
    }

    // Station activity (issue #1147): sum the always-on per-station,
    // per-control-source admitted-command counts across every bucket of every
    // seed's run. The report carries the full per-bucket series; the merge folds
    // it flat to a per-station busyness total, split by source, so a sweep shows
    // how busied each station was and by whom next to the win rates.
    for (const bucket of run.report.station_activity?.buckets ?? []) {
      for (const st of bucket?.stations ?? []) {
        if (typeof st?.station !== 'string') continue;
        const acc = (m.stationActivity[st.station] ??= { human: 0, ai: 0, offline: 0 });
        acc.human += st.human ?? 0;
        acc.ai += st.ai ?? 0;
        acc.offline += st.offline ?? 0;
      }
    }

    const mandatorySet = mandatorySetObservation(run.report);
    if (mandatorySet.sampled) {
      m.mandatorySetSampled += 1;
      if (mandatorySet.completed) m.mandatorySetCompleted += 1;
    } else {
      m.mandatorySetUnmeasurable[mandatorySet.reason] += 1;
    }

    const cleanLedger = fallingSkywayCleanLedgerObservation(run.report);
    if (cleanLedger.applicable) {
      m.fallingSkywayCleanLedgerApplicable = true;
      if (cleanLedger.sampled) {
        m.fallingSkywayCleanLedgerSampled += 1;
        if (cleanLedger.classification === 'clean') {
          m.fallingSkywayCleanLedgerClean += 1;
        } else if (cleanLedger.classification === 'completedButMid') {
          m.fallingSkywayCleanLedgerCompletedButMid += 1;
        } else {
          m.fallingSkywayCleanLedgerIncomplete += 1;
        }
      } else {
        m.fallingSkywayCleanLedgerUnmeasurable[cleanLedger.reason] += 1;
      }
    }
  }

  const matchups = {};
  for (const m of byMatchup.values()) {
    const completed = m.wins + m.losses + m.draws + m.timeouts;
    const mandatorySetUnmeasurable = {
      total: Object.values(m.mandatorySetUnmeasurable).reduce((a, n) => a + n, 0),
      ...m.mandatorySetUnmeasurable,
    };
    const fallingSkywayCleanLedgerUnmeasurable = {
      total: Object.values(m.fallingSkywayCleanLedgerUnmeasurable).reduce((a, n) => a + n, 0),
      ...m.fallingSkywayCleanLedgerUnmeasurable,
    };
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
      mandatorySetCompletion: {
        completed: m.mandatorySetCompleted,
        sampled: m.mandatorySetSampled,
        rate: m.mandatorySetSampled > 0
          ? m.mandatorySetCompleted / m.mandatorySetSampled
          : null,
        unmeasurable: mandatorySetUnmeasurable,
      },
      fallingSkywayCleanLedger: m.fallingSkywayCleanLedgerApplicable ? {
        clean: m.fallingSkywayCleanLedgerClean,
        completedButMid: m.fallingSkywayCleanLedgerCompletedButMid,
        incomplete: m.fallingSkywayCleanLedgerIncomplete,
        sampled: m.fallingSkywayCleanLedgerSampled,
        unmeasurable: fallingSkywayCleanLedgerUnmeasurable,
      } : null,
      // Phase → summed sim-seconds, keys sorted and values rounded to ms so
      // merged.json is stable and diffably free of float-sum noise.
      phases: Object.fromEntries(
        Object.entries(m.phaseSeconds)
          .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
          .map(([phase, secs]) => [phase, Math.round(secs * 1000) / 1000]),
      ),
      // Station → summed {human, ai, offline, total} admitted commands, keys
      // sorted so merged.json is stable (issue #1147). Integer counts, so no
      // rounding needed.
      stationActivity: Object.fromEntries(
        Object.entries(m.stationActivity)
          .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
          .map(([station, c]) => [
            station,
            { human: c.human, ai: c.ai, offline: c.offline, total: c.human + c.ai + c.offline },
          ]),
      ),
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

function formatMandatorySet(metric) {
  const unmeasurable = metric.unmeasurable;
  const reasons = [
    [unmeasurable.missingScenario, 'missing scenario'],
    [unmeasurable.malformedObjectives, 'malformed objectives'],
    [unmeasurable.noMandatoryObjectives, 'no mandatory objectives'],
  ]
    .filter(([count]) => count > 0)
    .map(([count, reason]) => `${count} ${reason}`)
    .join(', ');
  if (metric.sampled === 0) {
    return `0/0 (no data${reasons ? `; ${reasons}` : ''})`;
  }
  const gap = reasons ? `; ${reasons}` : '';
  return `${metric.completed}/${metric.sampled} (${pct(metric.rate)}${gap})`;
}

function formatFallingSkywayCleanLedger(metric) {
  if (metric === null) return '—';
  const unmeasurable = metric.unmeasurable;
  const reasons = [
    [unmeasurable.missingScenario, 'missing scenario'],
    [unmeasurable.malformedObjectives, 'malformed objectives'],
    [unmeasurable.noMandatoryObjectives, 'no mandatory objectives'],
    [unmeasurable.malformedFlags, 'malformed flags'],
  ]
    .filter(([count]) => count > 0)
    .map(([count, reason]) => `${count} ${reason}`)
    .join(', ');
  const gap = reasons ? `; ${reasons}` : '';
  return `${metric.clean} clean / ${metric.completedButMid} completed but mid / ${metric.incomplete} incomplete (${metric.sampled} sampled${gap})`;
}

/**
 * Render a per-matchup summary table. PURE — string in, string out. Works for
 * every config shape (one row per matchup).
 */
export function formatMarkdown(summary) {
  const rows = Object.values(summary.matchups);
  const showFallingSkywayCleanLedger = rows.some((s) => s.fallingSkywayCleanLedger !== null);
  const lines = [];
  const headings = ['Matchup', 'Runs', 'Win%', 'W/L/D/T', 'Fail', 'Mandatory set complete'];
  const separators = ['---', '---:', '---:', ':---:', '---:', ':---:'];
  if (showFallingSkywayCleanLedger) {
    headings.push('Falling Skyway clean ledger');
    separators.push(':---:');
  }
  headings.push('TTK min/med/max (s)', 'Dmg margin');
  separators.push(':---:', '---:');
  lines.push(`| ${headings.join(' | ')} |`);
  lines.push(`|${separators.join('|')}|`);
  for (const s of rows) {
    const wldt = `${s.wins}/${s.losses}/${s.draws}/${s.timeouts}`;
    const ttk = s.ttk.count
      ? `${num(s.ttk.min)} / ${num(s.ttk.median)} / ${num(s.ttk.max)}`
      : '—';
    const cells = [
      s.label,
      s.total,
      pct(s.winRate),
      wldt,
      s.failures,
      formatMandatorySet(s.mandatorySetCompletion),
    ];
    if (showFallingSkywayCleanLedger) {
      cells.push(formatFallingSkywayCleanLedger(s.fallingSkywayCleanLedger));
    }
    cells.push(ttk, num(s.damageMargin.mean));
    lines.push(`| ${cells.join(' | ')} |`);
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

/**
 * Render each matchup's doctrine-phase occupancy as a markdown list, phases
 * ordered by share descending. Matchups with no phase data are skipped; returns
 * '' when nothing has any. PURE.
 */
export function formatPhases(summary) {
  const lines = [];
  for (const s of Object.values(summary.matchups)) {
    const entries = Object.entries(s.phases ?? {});
    const total = entries.reduce((a, [, secs]) => a + secs, 0);
    if (total <= 0) continue;
    const parts = entries
      .sort(([, a], [, b]) => b - a)
      .map(([phase, secs]) => `${phase} ${((secs / total) * 100).toFixed(0)}%`);
    lines.push(`- **${s.label}**: ${parts.join(', ')} (${total.toFixed(0)} phase-seconds)`);
  }
  if (lines.length === 0) return '';
  return ['### Doctrine phase occupancy', '', ...lines].join('\n');
}

/**
 * Render each matchup's per-station busyness as a markdown table, split by
 * control source (issue #1147). One row per (matchup × station), so a sweep
 * shows how busied each station was and whether a human worked it or Backfill
 * carried it — the evidence next to the win rates. Matchups with no station
 * activity contribute no rows; returns '' when nothing has any. PURE.
 */
export function formatStationActivity(summary) {
  const rows = [];
  for (const s of Object.values(summary.matchups)) {
    for (const [station, c] of Object.entries(s.stationActivity ?? {})) {
      rows.push(`| ${s.label} | ${station} | ${c.human} | ${c.ai} | ${c.offline} | ${c.total} |`);
    }
  }
  if (rows.length === 0) return '';
  return [
    '### Station activity (admitted commands by control source)',
    '',
    '| Matchup | Station | Human | AI | Offline | Total |',
    '|---|---|---:|---:|---:|---:|',
    ...rows,
  ].join('\n');
}

// The recordable threshold metrics: how to read the actual off a matchup
// summary, and which way the limit points.
const THRESHOLD_METRICS = {
  min_win_rate: { actual: (s) => s.winRate, ok: (a, limit) => a >= limit },
  max_win_rate: { actual: (s) => s.winRate, ok: (a, limit) => a <= limit },
  min_ttk_median: { actual: (s) => s.ttk.median, ok: (a, limit) => a >= limit },
  max_ttk_median: { actual: (s) => s.ttk.median, ok: (a, limit) => a <= limit },
  min_damage_margin: { actual: (s) => s.damageMargin.mean, ok: (a, limit) => a >= limit },
  max_damage_margin: { actual: (s) => s.damageMargin.mean, ok: (a, limit) => a <= limit },
  max_failures: { actual: (s) => s.failures, ok: (a, limit) => a <= limit },
};

/**
 * Evaluate the config's recorded thresholds against a merged summary. PURE.
 *
 * For each matchup the effective spec is the global `[thresholds]` table with
 * that matchup's own `thresholds` (carried on the descriptor by
 * expandMatchups) merged over it per key. Returns one record per (matchup ×
 * metric): `{matchup, metric, limit, actual, pass}` where `pass` is null when
 * the metric has no data (e.g. no TTK sample). Unknown metric names throw — a
 * typo must fail loudly, not silently record nothing. NON-GATING by design:
 * callers record the result; nothing here exits or throws on a failed check.
 */
export function evaluateThresholds(summary, matchups, globalThresholds = {}) {
  const checks = [];
  for (const s of Object.values(summary.matchups)) {
    const own = matchups.find((m) => m.label === s.label)?.thresholds ?? {};
    const spec = { ...globalThresholds, ...own };
    for (const [metric, limit] of Object.entries(spec)) {
      const def = THRESHOLD_METRICS[metric];
      if (!def) {
        throw new Error(
          `unknown threshold metric ${JSON.stringify(metric)}; known: ${Object.keys(THRESHOLD_METRICS).join(', ')}`,
        );
      }
      if (typeof limit !== 'number') {
        throw new Error(`threshold ${metric} must be a number, got ${JSON.stringify(limit)}`);
      }
      const actual = def.actual(s);
      checks.push({
        matchup: s.label,
        metric,
        limit,
        actual: actual ?? null,
        pass: actual == null ? null : def.ok(actual, limit),
      });
    }
  }
  return checks;
}

/**
 * Render threshold records as a markdown table. PURE. Returns '' for none.
 */
export function formatThresholds(checks, { enforced = false } = {}) {
  if (!checks.length) return '';
  const lines = [];
  lines.push(`### Thresholds (${enforced ? 'gating' : 'recorded, non-gating'})`);
  lines.push('');
  lines.push('| Matchup | Metric | Limit | Actual | Status |');
  lines.push('|---|---|---:|---:|:---:|');
  for (const c of checks) {
    const status = c.pass === null ? 'no data' : c.pass ? 'PASS' : 'FAIL';
    const actual = c.actual === null ? '—' : Number(c.actual).toFixed(2);
    lines.push(`| ${c.matchup} | ${c.metric} | ${c.limit} | ${actual} | ${status} |`);
  }
  return lines.join('\n');
}

/** A ratified gate rejects both failed and unmeasurable required metrics. */
export function failedThresholds(checks) {
  return checks.filter((check) => check.pass !== true);
}

// ── Side-effecting run machinery (kept out of the pure exports) ──────────────

/** Absolute path to the release binary, platform-suffixed. */
function binaryPath() {
  const name = process.platform === 'win32' ? 'phoenix-headless.exe' : 'phoenix-headless';
  return path.join(root, 'target', 'release', name);
}

/** Exact phoenix-headless CLI arguments for one task. PURE and unit-tested. */
export function buildRunArgs(task) {
  const args = ['--world', task.world];
  if (task.playerShip) {
    // Scenario sweeps preserve the scenario's authored opposition and replace
    // only the player hull. Side flags are the duel transform and are invalid
    // on worlds without its `// duel:slots` marker.
    args.push('--ship', task.playerShip);
  } else {
    args.push('--side-a', task.sideA.join(','));
    if (task.sideB && task.sideB.length) args.push('--side-b', task.sideB.join(','));
  }
  args.push('--seed', String(task.seed));
  args.push('--sim-seconds', String(task.simSeconds));
  if (task.hz != null) args.push('--hz', String(task.hz));
  args.push('--report-format', 'json');
  return args;
}

/** Spawn one headless run and resolve to a run result (never rejects). */
function runOne(bin, task) {
  const args = buildRunArgs(task);

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
      // `stdoutRaw` keeps the binary's own bytes so a persisted per-run report
      // is byte-identical with what the run printed — same-seed reruns diff
      // empty (the binary zeroes wall timings under --seed). Not merged into
      // merged.json.
      finish({ ...taskId(task), report, stdoutRaw: stdout });
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
merged.json + summary.md + the per-run AAR reports (runs/<matchup>-seed<N>.json)
are also written there (keep that dir out of git). A config [thresholds] table
(and per-[[matchup]] overrides) is evaluated and recorded in both outputs.
Set enforce_thresholds = true in a ratified config to fail the batch when a
threshold is exceeded or cannot be measured.

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
  const phases = formatPhases(summary);
  if (phases) out = `${out}\n\n${phases}`;
  // Per-station busyness next to the win rates (issue #1147).
  const stationActivity = formatStationActivity(summary);
  if (stationActivity) out = `${out}\n\n${stationActivity}`;
  // Every config records its threshold observations; only an explicitly
  // ratified config promotes them to a gate.
  const thresholds = evaluateThresholds(summary, matchups, config.thresholds ?? {});
  const enforceThresholds = config.enforce_thresholds === true;
  if (thresholds.length) out = `${out}\n\n${formatThresholds(thresholds, { enforced: enforceThresholds })}`;
  console.log(out);

  if (cli.out) {
    await mkdir(cli.out, { recursive: true });
    const merged = thresholds.length ? { ...summary, thresholds } : summary;
    await writeFile(path.join(cli.out, 'merged.json'), `${JSON.stringify(merged, null, 2)}\n`);
    await writeFile(path.join(cli.out, 'summary.md'), `${out}\n`);
    // Persist each run's own AAR report verbatim — the seeded evidence the
    // merged numbers are derived from, and the artifact a tuning pass diffs.
    const runsDir = path.join(cli.out, 'runs');
    await mkdir(runsDir, { recursive: true });
    let persisted = 0;
    for (const run of runs) {
      if (typeof run.stdoutRaw === 'string') {
        await writeFile(path.join(runsDir, runFileName(run.matchup, run.seed)), run.stdoutRaw);
        persisted += 1;
      }
    }
    console.error(
      `[balance-runs] wrote ${path.join(cli.out, 'merged.json')}, summary.md, and ${persisted} per-run reports under runs/`,
    );
  }

  if (enforceThresholds) {
    const failed = failedThresholds(thresholds);
    if (failed.length) {
      console.error(
        `[balance-runs] ${failed.length} gating threshold${failed.length === 1 ? '' : 's'} failed: ` +
        failed.map((check) => `${check.matchup}/${check.metric}`).join(', '),
      );
      process.exitCode = 1;
    }
  }
}

// Guard the CLI entry so importing this module (for tests, or from `node -e`
// where argv[1] is undefined) never spawns.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
