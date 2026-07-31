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
  runFileName,
  evaluateThresholds,
  formatThresholds,
} from '../../scripts/balance-runs.mjs';

// A fabricated report shaped like the real phoenix-headless JSON, minimal to
// what mergeReports consumes: outcome, sides.*.damage_dealt, damage_by_ship
// ledgers with a `death` of [tick, sim_t] | null. No simulation needed.
function report({ outcome, playerDealt, enemyDealt, deaths = [], phases = [] }) {
  const damage_by_ship = {};
  let i = 0;
  for (const d of deaths) {
    damage_by_ship[`ship-${i++}`] = { death: d }; // d is [tick, sim_t] or null
  }
  // `phases` is a list of per-ship phase_seconds objects, one ledger each.
  for (const p of phases) {
    damage_by_ship[`ship-${i++}`] = { death: null, phase_seconds: p };
  }
  return {
    outcome,
    sides: {
      player: { damage_dealt: playerDealt },
      enemy: { damage_dealt: enemyDealt },
    },
    damage_by_ship,
  };
}

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
