import { describe, it, expect } from 'vitest';
import {
  mergeReports,
  formatMarkdown,
  formatMatrix,
  formatPhases,
  matchupLabel,
  resolveSeeds,
  expandMatchups,
  buildRunTasks,
  buildRunArgs,
  runFileName,
  evaluateThresholds,
  formatThresholds,
  failedThresholds,
  formatStationActivity,
} from '../../scripts/balance-runs.mjs';

// A fabricated report shaped like the real phoenix-headless JSON, minimal to
// what mergeReports consumes: outcome, sides.*.damage_dealt, damage_by_ship
// ledgers with a `death` of [tick, sim_t] | null. No simulation needed.
function report({
  outcome,
  playerDealt,
  enemyDealt,
  deaths = [],
  phases = [],
  stationBuckets = null,
  scenario = undefined,
}) {
  const damage_by_ship = {};
  let i = 0;
  for (const d of deaths) {
    damage_by_ship[`ship-${i++}`] = { death: d }; // d is [tick, sim_t] or null
  }
  // `phases` is a list of per-ship phase_seconds objects, one ledger each.
  for (const p of phases) {
    damage_by_ship[`ship-${i++}`] = { death: null, phase_seconds: p };
  }
  const out = {
    outcome,
    sides: {
      player: { damage_dealt: playerDealt },
      enemy: { damage_dealt: enemyDealt },
    },
    damage_by_ship,
  };
  // `stationBuckets` is a list of bucket station-lists (one array of
  // {station, human, ai, offline} entries per bucket), matching the report's
  // station_activity.buckets[].stations[] shape (issue #1147). Omitted for the
  // many tests that predate the surface — a report without the key.
  if (stationBuckets) {
    out.station_activity = {
      schema_version: 1,
      bucket_ticks: 900,
      bucket_secs: 15,
      buckets: stationBuckets.map((stations, b) => ({ start_tick: b * 900, stations })),
    };
  }
  if (scenario !== undefined) out.scenario = scenario;
  return out;
}

const objective = (status, mandatory = true, id = `objective-${status}`) => ({ id, status, mandatory });

describe('matchupLabel', () => {
  it('joins side compositions, dropping side_b for a scenario sweep', () => {
    expect(matchupLabel(['cruiser'], ['destroyer'])).toBe('cruiser_vs_destroyer');
    expect(matchupLabel(['cruiser', 'destroyer'], ['battleship'])).toBe('cruiser+destroyer_vs_battleship');
    expect(matchupLabel(['courier'], [])).toBe('courier');
    expect(matchupLabel(['courier'], null)).toBe('courier');
  });
});

describe('resolveSeeds', () => {
  it('expands a count to 1..N and passes an array through', () => {
    expect(resolveSeeds(3)).toEqual([1, 2, 3]);
    expect(resolveSeeds([1, 7, 42])).toEqual([1, 7, 42]);
  });
  it('rejects nonsense', () => {
    expect(() => resolveSeeds(0)).toThrow();
    expect(() => resolveSeeds('x')).toThrow();
  });
});

describe('expandMatchups', () => {
  it('expands a class_matrix to every ordered pair including mirrors', () => {
    const m = expandMatchups({ class_matrix: ['courier', 'destroyer'] });
    expect(m.map((x) => x.label)).toEqual([
      'courier_vs_courier',
      'courier_vs_destroyer',
      'destroyer_vs_courier',
      'destroyer_vs_destroyer',
    ]);
    expect(m[0].world).toBe('assets/worlds/duel.toml'); // default matchup world
  });

  it('expands explicit [[matchup]] entries with optional names and worlds', () => {
    const m = expandMatchups({
      matchup: [
        { side_a: ['cruiser'], side_b: ['destroyer'] },
        { name: 'pack', side_a: ['destroyer', 'destroyer'], side_b: 'battleship', world: 'assets/worlds/x.toml' },
      ],
    });
    expect(m[0].label).toBe('cruiser_vs_destroyer');
    expect(m[1].label).toBe('pack');
    expect(m[1].sideB).toEqual(['battleship']); // scalar coerced to a list
    expect(m[1].world).toBe('assets/worlds/x.toml');
  });

  it('expands a scenario sweep: one player hull each, no side_b, shared world', () => {
    const m = expandMatchups({ world: 'assets/worlds/combat_test.toml', scenario_hulls: ['courier', 'cruiser'] });
    expect(m.map((x) => x.label)).toEqual(['courier', 'cruiser']);
    expect(m.every((x) => x.sideB.length === 0)).toBe(true);
    expect(m.every((x) => x.world === 'assets/worlds/combat_test.toml')).toBe(true);
    expect(m.map((x) => x.playerShip)).toEqual([
      'assets/entities/alliance_courier.toml',
      'assets/entities/alliance_cruiser.toml',
    ]);
  });

  it('rejects a scenario sweep with no world, and an empty config', () => {
    expect(() => expandMatchups({ scenario_hulls: ['courier'] })).toThrow(/world/);
    expect(() => expandMatchups({})).toThrow(/no matchups/);
  });
});

describe('buildRunTasks', () => {
  it('crosses matchups with seeds and carries the global knobs', () => {
    const matchups = expandMatchups({ class_matrix: ['courier', 'destroyer'] });
    const tasks = buildRunTasks({ seeds: 2, sim_seconds: 30, hz: 60 }, matchups);
    expect(tasks.length).toBe(4 * 2);
    expect(tasks[0]).toMatchObject({ matchup: 'courier_vs_courier', seed: 1, simSeconds: 30, hz: 60 });
    expect(tasks[1].seed).toBe(2);
  });

  it('defaults run_timeout_secs to 300 and passes an explicit value through', () => {
    const matchups = expandMatchups({ class_matrix: ['courier'] });
    expect(buildRunTasks({ seeds: 1 }, matchups)[0].timeoutSecs).toBe(300);
    expect(buildRunTasks({ seeds: 1, run_timeout_secs: 45 }, matchups)[0].timeoutSecs).toBe(45);
  });
});

describe('buildRunArgs', () => {
  it('runs scenario_hulls with --ship and no duel-side transform', () => {
    const matchups = expandMatchups({
      world: 'assets/worlds/falling_skyway.toml',
      scenario_hulls: ['destroyer'],
    });
    const [task] = buildRunTasks({ seeds: [7], sim_seconds: 1850, hz: 30 }, matchups);
    expect(buildRunArgs(task)).toEqual([
      '--world', 'assets/worlds/falling_skyway.toml',
      '--ship', 'assets/entities/alliance_destroyer.toml',
      '--seed', '7',
      '--sim-seconds', '1850',
      '--hz', '30',
      '--report-format', 'json',
    ]);
    expect(buildRunArgs(task)).not.toContain('--side-a');
    expect(buildRunArgs(task)).not.toContain('--side-b');
  });

  it('takes an explicitly authored scenario hull path literally', () => {
    const [matchup] = expandMatchups({
      world: 'assets/worlds/falling_skyway.toml',
      scenario_hulls: ['mods/thin-margin/player_hull.toml'],
    });
    const [task] = buildRunTasks({ seeds: 1, sim_seconds: 60 }, [matchup]);
    expect(buildRunArgs(task).slice(0, 4)).toEqual([
      '--world', 'assets/worlds/falling_skyway.toml',
      '--ship', 'mods/thin-margin/player_hull.toml',
    ]);
  });

  it('keeps duel matchups on --side-a/--side-b', () => {
    const [matchup] = expandMatchups({
      matchup: [{ side_a: ['cruiser'], side_b: ['ship_harrow_patrol'] }],
    });
    const [task] = buildRunTasks({ seeds: 1, sim_seconds: 45 }, [matchup]);
    expect(buildRunArgs(task)).toEqual([
      '--world', 'assets/worlds/duel.toml',
      '--side-a', 'cruiser',
      '--side-b', 'ship_harrow_patrol',
      '--seed', '1',
      '--sim-seconds', '45',
      '--report-format', 'json',
    ]);
  });
});

describe('mergeReports — tallying', () => {
  it('counts win/loss/draw/timeout and computes win rate over completed runs', () => {
    const runs = [
      { matchup: 'a_vs_b', seed: 1, report: report({ outcome: 'victory', playerDealt: 100, enemyDealt: 40, deaths: [[10, 5.0]] }) },
      { matchup: 'a_vs_b', seed: 2, report: report({ outcome: 'defeat', playerDealt: 30, enemyDealt: 120, deaths: [[20, 8.0]] }) },
      { matchup: 'a_vs_b', seed: 3, report: report({ outcome: 'draw', playerDealt: 10, enemyDealt: 10, deaths: [null] }) },
      { matchup: 'a_vs_b', seed: 4, report: report({ outcome: 'timeout', playerDealt: 50, enemyDealt: 55, deaths: [null] }) },
    ];
    const s = mergeReports(runs).matchups.a_vs_b;
    expect(s).toMatchObject({ total: 4, completed: 4, wins: 1, losses: 1, draws: 1, timeouts: 1, failures: 0 });
    expect(s.winRate).toBeCloseTo(0.25); // 1 win / 4 completed
  });
});

describe('mergeReports — TTK distribution', () => {
  it('takes the latest death per run and reports min/median/max, excluding null-death runs', () => {
    const runs = [
      { matchup: 'm', seed: 1, report: report({ outcome: 'victory', playerDealt: 1, enemyDealt: 0, deaths: [[1, 4.0]] }) },
      { matchup: 'm', seed: 2, report: report({ outcome: 'victory', playerDealt: 1, enemyDealt: 0, deaths: [[1, 6.0], [2, 8.0]] }) }, // latest = 8.0
      { matchup: 'm', seed: 3, report: report({ outcome: 'defeat', playerDealt: 0, enemyDealt: 1, deaths: [[1, 12.0]] }) },
      { matchup: 'm', seed: 4, report: report({ outcome: 'draw', playerDealt: 0, enemyDealt: 0, deaths: [null, null] }) }, // excluded
    ];
    const s = mergeReports(runs).matchups.m;
    expect(s.ttk.count).toBe(3); // the draw contributes no sample
    expect(s.ttk.min).toBeCloseTo(4.0);
    expect(s.ttk.median).toBeCloseTo(8.0); // sorted [4, 8, 12]
    expect(s.ttk.max).toBeCloseTo(12.0);
  });

  it('averages the two middle samples for an even count', () => {
    const runs = [
      { matchup: 'm', seed: 1, report: report({ outcome: 'victory', playerDealt: 0, enemyDealt: 0, deaths: [[1, 2.0]] }) },
      { matchup: 'm', seed: 2, report: report({ outcome: 'victory', playerDealt: 0, enemyDealt: 0, deaths: [[1, 4.0]] }) },
      { matchup: 'm', seed: 3, report: report({ outcome: 'victory', playerDealt: 0, enemyDealt: 0, deaths: [[1, 6.0]] }) },
      { matchup: 'm', seed: 4, report: report({ outcome: 'victory', playerDealt: 0, enemyDealt: 0, deaths: [[1, 10.0]] }) },
    ];
    expect(mergeReports(runs).matchups.m.ttk.median).toBeCloseTo(5.0); // (4 + 6) / 2
  });
});

describe('mergeReports — damage margin', () => {
  it('means player.damage_dealt - enemy.damage_dealt over reports', () => {
    const runs = [
      { matchup: 'm', seed: 1, report: report({ outcome: 'victory', playerDealt: 100, enemyDealt: 40, deaths: [[1, 5.0]] }) }, // +60
      { matchup: 'm', seed: 2, report: report({ outcome: 'defeat', playerDealt: 20, enemyDealt: 120, deaths: [[1, 5.0]] }) }, // -100
    ];
    const s = mergeReports(runs).matchups.m;
    expect(s.damageMargin.mean).toBeCloseTo(-20.0); // (60 + -100) / 2
    expect(s.damageMargin.count).toBe(2);
  });
});

describe('mergeReports — failed runs', () => {
  it('counts a run with no report as a failure, not a crash of the batch', () => {
    const runs = [
      { matchup: 'm', seed: 1, report: report({ outcome: 'victory', playerDealt: 10, enemyDealt: 5, deaths: [[1, 3.0]] }) },
      { matchup: 'm', seed: 2, error: 'exit 101', exitCode: 101 },
      { matchup: 'm', seed: 3, error: 'unparseable report: Unexpected end of JSON input', exitCode: 0 },
    ];
    const summary = mergeReports(runs);
    const s = summary.matchups.m;
    expect(s.total).toBe(3);
    expect(s.completed).toBe(1);
    expect(s.failures).toBe(2);
    expect(s.winRate).toBeCloseTo(1.0); // 1 win / 1 completed — failures excluded from the rate
    expect(s.failuresDetail).toEqual([
      { seed: 2, error: 'exit 101', exitCode: 101 },
      { seed: 3, error: 'unparseable report: Unexpected end of JSON input', exitCode: 0 },
    ]);
    expect(summary.totals.failures).toBe(2);
  });

  it('tallies a timeout-shaped failure (killed hung run) as a failure, not a completed run', () => {
    const runs = [
      { matchup: 'm', seed: 1, report: report({ outcome: 'victory', playerDealt: 10, enemyDealt: 5, deaths: [[1, 3.0]] }) },
      { matchup: 'm', seed: 2, error: 'timeout after 300s', exitCode: null, stderrTail: '' },
    ];
    const s = mergeReports(runs).matchups.m;
    expect(s.total).toBe(2);
    expect(s.completed).toBe(1); // the hung run is excluded from completed/win-rate
    expect(s.failures).toBe(1);
    expect(s.timeouts).toBe(0); // report.outcome 'timeout' ≠ a killed hang; the latter has no report
    expect(s.failuresDetail).toEqual([{ seed: 2, error: 'timeout after 300s', exitCode: null }]);
  });
});

describe('mergeReports — mandatory objective sets', () => {
  it('completes a run only when every mandatory objective is Completed', () => {
    const runs = [
      {
        matchup: 'm', seed: 1,
        report: report({
          outcome: 'victory', playerDealt: 0, enemyDealt: 0,
          scenario: { objectives: [
            objective('Completed', true, 'rescue'),
            objective('Completed', true, 'escape'),
            objective('Active', false, 'optional-scan'),
          ] },
        }),
      },
      {
        matchup: 'm', seed: 2,
        report: report({
          outcome: 'defeat', playerDealt: 0, enemyDealt: 0,
          scenario: { objectives: [objective('Completed'), objective('Failed')] },
        }),
      },
      {
        matchup: 'm', seed: 3,
        report: report({
          outcome: 'timeout', playerDealt: 0, enemyDealt: 0,
          scenario: { objectives: [objective('Active')] },
        }),
      },
    ];

    expect(mergeReports(runs).matchups.m.mandatorySetCompletion).toEqual({
      completed: 1,
      sampled: 3,
      rate: 1 / 3,
      unmeasurable: {
        total: 0,
        missingScenario: 0,
        malformedObjectives: 0,
        noMandatoryObjectives: 0,
      },
    });
  });

  it('excludes telemetry gaps, optional-only reports, and process failures from the denominator', () => {
    const base = { outcome: 'timeout', playerDealt: 0, enemyDealt: 0 };
    const runs = [
      { matchup: 'm', seed: 1, report: report(base) },
      { matchup: 'm', seed: 2, report: report({ ...base, scenario: null }) },
      { matchup: 'm', seed: 3, report: report({ ...base, scenario: { objectives: {} } }) },
      {
        matchup: 'm', seed: 4,
        report: report({ ...base, scenario: { objectives: [objective('Completed', false)] } }),
      },
      { matchup: 'm', seed: 5, error: 'exit 101', exitCode: 101 },
    ];

    const s = mergeReports(runs).matchups.m;
    expect(s.mandatorySetCompletion).toEqual({
      completed: 0,
      sampled: 0,
      rate: null,
      unmeasurable: {
        total: 4,
        missingScenario: 2,
        malformedObjectives: 1,
        noMandatoryObjectives: 1,
      },
    });
    expect(s.failures).toBe(1);
  });

  it('rejects missing, empty, non-string, and duplicate objective identities', () => {
    const base = { outcome: 'victory', playerDealt: 0, enemyDealt: 0 };
    const runs = [
      {
        matchup: 'm', seed: 1,
        report: report({
          ...base,
          scenario: { objectives: [{ status: 'Completed', mandatory: true }] },
        }),
      },
      {
        matchup: 'm', seed: 2,
        report: report({
          ...base,
          scenario: { objectives: [objective('Completed', true, '')] },
        }),
      },
      {
        matchup: 'm', seed: 3,
        report: report({
          ...base,
          scenario: { objectives: [objective('Completed', true, 7)] },
        }),
      },
      {
        matchup: 'm', seed: 4,
        report: report({
          ...base,
          scenario: { objectives: [
            objective('Completed', true, 'same-id'),
            objective('Completed', true, 'same-id'),
          ] },
        }),
      },
    ];

    expect(mergeReports(runs).matchups.m.mandatorySetCompletion).toEqual({
      completed: 0,
      sampled: 0,
      rate: null,
      unmeasurable: {
        total: 4,
        missingScenario: 0,
        malformedObjectives: 4,
        noMandatoryObjectives: 0,
      },
    });
  });

  it('keeps independent deterministic aggregates for multiple matchups', () => {
    const make = (matchup, seed, statuses) => ({
      matchup,
      seed,
      report: report({
        outcome: 'timeout',
        playerDealt: 0,
        enemyDealt: 0,
        scenario: { objectives: statuses.map((status, i) => objective(status, true, `${matchup}-${i}`)) },
      }),
    });
    const summary = mergeReports([
      make('complete', 1, ['Completed', 'Completed']),
      make('zero', 1, ['Failed']),
      make('zero', 2, ['Active']),
      {
        matchup: 'no_data', seed: 1,
        report: report({
          outcome: 'timeout', playerDealt: 0, enemyDealt: 0,
          scenario: { objectives: [objective('Completed', false)] },
        }),
      },
    ]);

    expect(Object.keys(summary.matchups)).toEqual(['complete', 'zero', 'no_data']);
    expect(summary.matchups.complete.mandatorySetCompletion).toMatchObject({ completed: 1, sampled: 1, rate: 1 });
    expect(summary.matchups.zero.mandatorySetCompletion).toMatchObject({ completed: 0, sampled: 2, rate: 0 });
    expect(summary.matchups.no_data.mandatorySetCompletion).toMatchObject({ completed: 0, sampled: 0, rate: null });
  });
});

describe('mergeReports — doctrine phase occupancy', () => {
  it('sums every ship ledger phase_seconds per matchup with sorted keys', () => {
    const runs = [
      {
        matchup: 'm', seed: 1,
        report: report({
          outcome: 'victory', playerDealt: 1, enemyDealt: 0, deaths: [[1, 5.0]],
          phases: [{ acquire: 4.0, attack_run: 10.0 }, { escape: 2.0, acquire: 1.0 }],
        }),
      },
      {
        matchup: 'm', seed: 2,
        report: report({
          outcome: 'defeat', playerDealt: 0, enemyDealt: 1, deaths: [[1, 5.0]],
          phases: [{ attack_run: 5.0 }],
        }),
      },
    ];
    const s = mergeReports(runs).matchups.m;
    expect(s.phases).toEqual({ acquire: 5.0, attack_run: 15.0, escape: 2.0 });
    expect(Object.keys(s.phases)).toEqual(['acquire', 'attack_run', 'escape']); // sorted
  });

  it('yields an empty phases object when no ledger carries phase_seconds', () => {
    const runs = [
      { matchup: 'm', seed: 1, report: report({ outcome: 'draw', playerDealt: 0, enemyDealt: 0, deaths: [null] }) },
    ];
    expect(mergeReports(runs).matchups.m.phases).toEqual({});
  });
});

describe('formatPhases', () => {
  it('renders per-matchup occupancy as percentages ordered by share', () => {
    const summary = mergeReports([
      {
        matchup: 'a_vs_b', seed: 1,
        report: report({
          outcome: 'victory', playerDealt: 1, enemyDealt: 0, deaths: [[1, 5.0]],
          phases: [{ acquire: 25.0, attack_run: 75.0 }],
        }),
      },
    ]);
    const md = formatPhases(summary);
    expect(md).toContain('### Doctrine phase occupancy');
    expect(md).toContain('**a_vs_b**: attack_run 75%, acquire 25% (100 phase-seconds)');
  });

  it('returns an empty string when nothing has phase data', () => {
    const summary = mergeReports([
      { matchup: 'm', seed: 1, report: report({ outcome: 'draw', playerDealt: 0, enemyDealt: 0, deaths: [null] }) },
    ]);
    expect(formatPhases(summary)).toBe('');
  });
});

describe('mergeReports — station activity', () => {
  const entry = (station, human, ai, offline = 0) => ({ station, human, ai, offline });

  it('folds per-station commands across every bucket and seed, split by control source', () => {
    const runs = [
      {
        matchup: 'm', seed: 1,
        report: report({
          outcome: 'victory', playerDealt: 1, enemyDealt: 0, deaths: [[1, 5.0]],
          // Two buckets in one run — the fold sums across buckets too.
          stationBuckets: [[entry('helm', 5, 2)], [entry('helm', 3, 1), entry('weapons', 0, 4)]],
        }),
      },
      {
        matchup: 'm', seed: 2,
        report: report({
          outcome: 'defeat', playerDealt: 0, enemyDealt: 1, deaths: [[1, 5.0]],
          stationBuckets: [[entry('helm', 1, 1), entry('weapons', 0, 6)]],
        }),
      },
    ];
    const s = mergeReports(runs).matchups.m;
    // helm: human 5+3+1 = 9, ai 2+1+1 = 4; weapons: ai 4+6 = 10.
    expect(s.stationActivity.helm).toEqual({ human: 9, ai: 4, offline: 0, total: 13 });
    expect(s.stationActivity.weapons).toEqual({ human: 0, ai: 10, offline: 0, total: 10 });
    expect(Object.keys(s.stationActivity)).toEqual(['helm', 'weapons']); // sorted keys
  });

  it('yields an empty stationActivity object when no report carries the series', () => {
    const runs = [
      { matchup: 'm', seed: 1, report: report({ outcome: 'draw', playerDealt: 0, enemyDealt: 0, deaths: [null] }) },
    ];
    expect(mergeReports(runs).matchups.m.stationActivity).toEqual({});
  });
});

describe('formatStationActivity', () => {
  const entry = (station, human, ai, offline = 0) => ({ station, human, ai, offline });

  it('renders a per-station busyness table split by control source', () => {
    const summary = mergeReports([
      {
        matchup: 'a_vs_b', seed: 1,
        report: report({
          outcome: 'victory', playerDealt: 1, enemyDealt: 0, deaths: [[1, 5.0]],
          stationBuckets: [[entry('helm', 80, 40), entry('weapons', 0, 95)]],
        }),
      },
    ]);
    const md = formatStationActivity(summary);
    expect(md).toContain('### Station activity (admitted commands by control source)');
    expect(md).toContain('| Matchup | Station | Human | AI | Offline | Total |');
    expect(md).toContain('| a_vs_b | helm | 80 | 40 | 0 | 120 |');
    expect(md).toContain('| a_vs_b | weapons | 0 | 95 | 0 | 95 |');
  });

  it('returns an empty string when no matchup has station activity', () => {
    const summary = mergeReports([
      { matchup: 'm', seed: 1, report: report({ outcome: 'draw', playerDealt: 0, enemyDealt: 0, deaths: [null] }) },
    ]);
    expect(formatStationActivity(summary)).toBe('');
  });
});

describe('runFileName', () => {
  it('joins matchup and seed, sanitising filesystem-hostile characters', () => {
    expect(runFileName('destroyer_vs_harrow_patrol', 3)).toBe('destroyer_vs_harrow_patrol-seed3.json');
    expect(runFileName('a/b\\c d:e', 1)).toBe('a_b_c_d_e-seed1.json');
  });
});

describe('evaluateThresholds', () => {
  const summaryFor = (runs) => mergeReports(runs);
  const winningRun = (matchup, seed) => ({
    matchup, seed,
    report: report({ outcome: 'victory', playerDealt: 100, enemyDealt: 40, deaths: [[1, 30.0]] }),
  });

  it('records pass and fail per (matchup x metric) without throwing on failure', () => {
    const summary = summaryFor([
      winningRun('m', 1),
      { matchup: 'm', seed: 2, report: report({ outcome: 'defeat', playerDealt: 10, enemyDealt: 90, deaths: [[1, 50.0]] }) },
    ]);
    const checks = evaluateThresholds(summary, [], { min_win_rate: 0.75, max_ttk_median: 60, max_failures: 0 });
    const byMetric = Object.fromEntries(checks.map((c) => [c.metric, c]));
    expect(byMetric.min_win_rate).toMatchObject({ matchup: 'm', limit: 0.75, actual: 0.5, pass: false });
    expect(byMetric.max_ttk_median).toMatchObject({ actual: 40.0, pass: true }); // median of [30, 50]
    expect(byMetric.max_failures).toMatchObject({ actual: 0, pass: true });
  });

  it('merges a matchup-level thresholds table over the global one per key', () => {
    const summary = summaryFor([winningRun('m', 1)]);
    const matchups = [{ label: 'm', thresholds: { min_win_rate: 0.9 } }];
    const checks = evaluateThresholds(summary, matchups, { min_win_rate: 0.1, max_failures: 0 });
    const winRate = checks.find((c) => c.metric === 'min_win_rate');
    expect(winRate.limit).toBe(0.9); // the override, not the global
    expect(checks.some((c) => c.metric === 'max_failures')).toBe(true); // global keys survive
  });

  it('records pass: null when the metric has no data', () => {
    const summary = summaryFor([
      { matchup: 'm', seed: 1, report: report({ outcome: 'draw', playerDealt: 0, enemyDealt: 0, deaths: [null] }) },
    ]);
    const [check] = evaluateThresholds(summary, [], { max_ttk_median: 60 });
    expect(check).toMatchObject({ metric: 'max_ttk_median', actual: null, pass: null });
  });

  it('throws loudly on an unknown metric name or a non-numeric limit', () => {
    const summary = summaryFor([winningRun('m', 1)]);
    expect(() => evaluateThresholds(summary, [], { min_wine_rate: 0.5 })).toThrow(/unknown threshold/);
    expect(() => evaluateThresholds(summary, [], { min_win_rate: 'high' })).toThrow(/must be a number/);
  });
});

describe('formatThresholds', () => {
  it('renders a recorded (non-gating) status table, and nothing for no checks', () => {
    const md = formatThresholds([
      { matchup: 'm', metric: 'min_win_rate', limit: 0.75, actual: 0.5, pass: false },
      { matchup: 'm', metric: 'max_failures', limit: 0, actual: 0, pass: true },
      { matchup: 'm', metric: 'max_ttk_median', limit: 60, actual: null, pass: null },
    ]);
    expect(md).toContain('recorded, non-gating');
    expect(md).toContain('| m | min_win_rate | 0.75 | 0.50 | FAIL |');
    expect(md).toContain('| m | max_failures | 0 | 0.00 | PASS |');
    expect(md).toContain('| m | max_ttk_median | 60 | — | no data |');
    expect(formatThresholds([])).toBe('');
  });

  it('labels an opted-in threshold report as gating', () => {
    expect(formatThresholds([], { enforced: true })).toBe('');
    expect(formatThresholds([
      { matchup: 'm', metric: 'max_failures', limit: 0, actual: 0, pass: true },
    ], { enforced: true })).toContain('### Thresholds (gating)');
  });
});

describe('failedThresholds', () => {
  it('rejects both failed and unavailable required metrics for a gate', () => {
    const checks = [
      { matchup: 'm', metric: 'min_win_rate', pass: true },
      { matchup: 'm', metric: 'max_ttk_median', pass: false },
      { matchup: 'm', metric: 'min_damage_margin', pass: null },
    ];
    expect(failedThresholds(checks)).toEqual([checks[1], checks[2]]);
  });
});

describe('expandMatchups — thresholds carry-through', () => {
  it('keeps a [[matchup]] thresholds table on its descriptor', () => {
    const m = expandMatchups({
      matchup: [
        { name: 'x', side_a: ['destroyer'], side_b: ['ship_harrow_patrol'], thresholds: { min_win_rate: 0.5 } },
      ],
    });
    expect(m[0].thresholds).toEqual({ min_win_rate: 0.5 });
  });
});

describe('formatMarkdown', () => {
  it('renders a table row per matchup and a totals line', () => {
    const summary = mergeReports([
      { matchup: 'a_vs_b', sideA: ['a'], sideB: ['b'], seed: 1, report: report({ outcome: 'victory', playerDealt: 100, enemyDealt: 40, deaths: [[1, 5.0]] }) },
      { matchup: 'a_vs_b', sideA: ['a'], sideB: ['b'], seed: 2, error: 'exit 1' },
    ]);
    const md = formatMarkdown(summary);
    expect(md).toContain('| Matchup | Runs |');
    expect(md).toContain('| a_vs_b |');
    expect(md).toContain('Totals:');
    expect(md).toContain('1 failed');
  });

  it('renders explicit 0/N mandatory-set completion and diagnosed no-data rows', () => {
    const failedSet = (seed, status) => ({
      matchup: 'zero',
      seed,
      report: report({
        outcome: 'timeout', playerDealt: 0, enemyDealt: 0,
        scenario: { objectives: [objective(status)] },
      }),
    });
    const summary = mergeReports([
      failedSet(1, 'Failed'),
      failedSet(2, 'Active'),
      {
        matchup: 'no_data', seed: 1,
        report: report({
          outcome: 'timeout', playerDealt: 0, enemyDealt: 0,
          scenario: { objectives: [objective('Completed', false)] },
        }),
      },
    ]);
    const md = formatMarkdown(summary);
    expect(md).toContain('| Mandatory set complete |');
    expect(md).toContain('| zero | 2 | 0% | 0/0/0/2 | 0 | 0/2 (0%) |');
    expect(md).toContain('| no_data | 1 | 0% | 0/0/0/1 | 0 | 0/0 (no data; 1 no mandatory objectives) |');
  });
});

describe('formatMatrix', () => {
  it('renders a win-rate grid with side_a rows and side_b cols', () => {
    const summary = mergeReports([
      { matchup: 'courier_vs_courier', seed: 1, report: report({ outcome: 'draw', playerDealt: 0, enemyDealt: 0, deaths: [null] }) },
      { matchup: 'courier_vs_cruiser', seed: 1, report: report({ outcome: 'defeat', playerDealt: 0, enemyDealt: 1, deaths: [[1, 5.0]] }) },
      { matchup: 'cruiser_vs_courier', seed: 1, report: report({ outcome: 'victory', playerDealt: 1, enemyDealt: 0, deaths: [[1, 5.0]] }) },
      { matchup: 'cruiser_vs_cruiser', seed: 1, report: report({ outcome: 'timeout', playerDealt: 0, enemyDealt: 0, deaths: [null] }) },
    ]);
    const grid = formatMatrix(summary, ['courier', 'cruiser']);
    expect(grid).toContain('| side_a \\ side_b | courier | cruiser |');
    expect(grid).toContain('| **cruiser** | 100% | 0% |'); // cruiser beats courier, draws-as-0%-win vs cruiser
    expect(grid).toContain('| **courier** | 0% | 0% |');
  });
});
