/**
 * Write the Live dock layout parity fixture.
 *
 * `src/native_host/panes/operator.rs` sanitizes the same stored layouts the
 * browser does, and the two must agree exactly — a native operator profile and
 * a browser one are the same file. Rather than transcribing each expectation by
 * hand every time a panel is registered, the browser model is the source of
 * truth and this writes what it produces; the Rust test asserts against it.
 *
 * The fixture pins PARITY, not correctness: the browser model's own behaviour is
 * covered by tests/client/live-layout-model.test.js.
 *
 *   node scripts/generate-live-layout-fixture.mjs [--check]
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { defaultLiveLayout, LIVE_PANELS, normalizeLiveLayout } from '../gui/live-layout-model.js';

const OUT = 'tests/fixtures/live-layout-migrations.json';

const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
/** Every registered panel but the named ones, so a panel added later is covered
 * here without anybody remembering to add it. */
const allClosedBut = (...open) => LIVE_PANELS.filter(panel => !open.includes(panel));

/** One stored layout per case, chosen to exercise a distinct rule. */
const CASES = [
  ['unknown version defaults', { version: 99 }],
  ['a version 1 tracer', {
    version: 1, root: group(['roster', 'readiness', 'join', 'manual-save'], 'roster'),
    floats: [], closed: [], selected: 'roster',
  }],
  ['a version 1 layout that closed panels', {
    version: 1, root: group(['roster', 'readiness'], 'readiness'),
    floats: [], closed: ['join', 'manual-save'], selected: 'readiness',
  }],
  ['a version 1 layout naming panels it never registered', {
    version: 1, root: group(['roster', 'comms', 'journal'], 'journal'),
    floats: [{ panel: 'activity', x: 7, y: 9, width: 300, height: 200 }],
    closed: ['readiness'], selected: 'journal',
  }],
  ['a version 1 layout with duplicates and a float', {
    version: 1, root: group(['roster', 'roster', 'unsafe'], 'unsafe'),
    floats: [{ panel: 'join', x: 30 }], closed: ['manual-save'], selected: 'unsafe',
  }],
  ['a version 2 record layout', {
    version: 2,
    root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
      group(['roster', 'mission'], 'roster'), group(['comms', 'journal'], 'journal'),
    ] },
    floats: [], closed: ['readiness', 'join', 'manual-save', 'activity', 'session-history'],
    selected: 'journal',
  }],
  ['a version 3 layout with a closed pinned panel', {
    version: 3,
    root: { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
      group(['roster', 'attention'], 'roster'), group(['map'], 'map'),
    ] },
    floats: [],
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
      'session-history', 'workload', 'widgets', 'health'],
    selected: 'roster',
  }],
  ['a version 4 layout', {
    version: 4, root: group(['roster', 'mission'], 'mission'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'comms', 'activity', 'journal', 'session-history',
      'map', 'workload', 'widgets', 'station', 'station-console'],
    selected: 'mission',
  }],
  ['a version 5 layout', {
    version: 5, root: group(['roster', 'presentation'], 'presentation'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
      'session-history', 'map', 'attention', 'workload', 'widgets', 'health', 'station',
      'station-console', 'audition', 'source-link'],
    selected: 'presentation',
  }],
  ['a docked draft, which survives, beside a floating one, which does not', {
    version: 6, root: group(['roster', 'spawn'], 'spawn'),
    floats: [{ panel: 'spawn', x: 8, y: 9, width: 300, height: 200 }],
    closed: allClosedBut('roster', 'spawn'), selected: 'spawn',
  }],
  ['a floating draft alone', {
    version: 6, root: group(['roster'], 'roster'),
    floats: [{ panel: 'spawn', x: 8, y: 9, width: 300, height: 200 }],
    closed: allClosedBut('roster', 'spawn'), selected: 'spawn',
  }],
  ['a version 6 layout', {
    version: 6, root: group(['roster', 'map'], 'map'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
      'session-history', 'attention', 'workload', 'widgets', 'health', 'station',
      'station-console', 'presentation', 'audition', 'source-link', 'spawn'],
    selected: 'map',
  }],
  ['a version 7 layout', {
    version: 7, root: group(['roster', 'journal'], 'journal'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity',
      'session-history', 'map', 'attention', 'workload', 'widgets', 'health', 'station',
      'station-console', 'presentation', 'audition', 'source-link', 'spawn', 'inspector'],
    selected: 'journal',
  }],
  ['a version 8 layout', {
    version: 8, root: group(['roster', 'inspector'], 'inspector'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
      'session-history', 'map', 'attention', 'workload', 'widgets', 'health', 'station',
      'station-console', 'presentation', 'audition', 'source-link', 'spawn', 'checkpoint',
      'restore'],
    selected: 'inspector',
  }],
  ['a version 9 layout', {
    version: 9, root: group(['roster', 'contact'], 'contact'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
      'session-history', 'map', 'attention', 'workload', 'widgets', 'health', 'station',
      'station-console', 'presentation', 'audition', 'source-link', 'spawn', 'inspector',
      'checkpoint', 'restore', 'npc', 'misclassify', 'report-policy', 'ghost'],
    selected: 'contact',
  }],
  ['a version 10 layout', {
    version: 10, root: group(['roster', 'inspector', 'system'], 'system'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
      'session-history', 'map', 'attention', 'workload', 'widgets', 'health', 'station',
      'station-console', 'presentation', 'audition', 'source-link', 'spawn', 'checkpoint',
      'restore', 'contact', 'npc', 'misclassify', 'report-policy', 'ghost', 'effect'],
    selected: 'system',
  }],
  ['a version 11 layout', {
    version: 11, root: group(['roster', 'mission', 'faction'], 'mission'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'comms', 'activity', 'journal',
      'session-history', 'map', 'attention', 'workload', 'widgets', 'health', 'station',
      'station-console', 'presentation', 'audition', 'source-link', 'spawn', 'inspector',
      'checkpoint', 'restore', 'contact', 'npc', 'misclassify', 'report-policy', 'ghost',
      'system', 'effect', 'despawn'],
    selected: 'mission',
  }],
  ['a version 12 layout', {
    version: 12, root: group(['roster', 'inspector', 'objective'], 'inspector'), floats: [],
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
      'session-history', 'map', 'attention', 'workload', 'widgets', 'health', 'station',
      'station-console', 'presentation', 'audition', 'source-link', 'spawn', 'checkpoint',
      'restore', 'contact', 'npc', 'misclassify', 'report-policy', 'ghost', 'system', 'effect',
      'despawn', 'faction'],
    selected: 'inspector',
  }],
  ['a current layout with unknown fields', {
    version: defaultLiveLayout().version,
    root: { type: 'tabs', tabs: ['comms', 'journal', 'unsafe'], active: 'journal', unsafe: 'secret' },
    floats: [{ panel: 'roster', x: 7, y: 9, width: 300, height: 200, unsafe: 'secret' }],
    closed: allClosedBut('comms', 'journal'),
    selected: 'journal', unsafe: 'secret',
  }],
  ['a current layout that closed everything', {
    version: defaultLiveLayout().version, root: group(['roster'], 'roster'), floats: [],
    closed: allClosedBut('roster'),
    selected: 'roster',
  }],
];

// The version before this one is the migration every existing profile will take,
// so it always has a case.
const previous = defaultLiveLayout().version - 1;
if (!CASES.some(([, stored]) => stored.version === previous)) {
  throw new Error(`no stored case at version ${previous}: add one before bumping the registry`);
}

const fixture = {
  note: 'Generated by scripts/generate-live-layout-fixture.mjs. Pins browser/native parity.',
  version: defaultLiveLayout().version,
  default: defaultLiveLayout(),
  cases: CASES.map(([name, stored]) => ({ name, stored, expected: normalizeLiveLayout(stored) })),
};

const text = `${JSON.stringify(fixture, null, 2)}\n`;
if (process.argv.includes('--check')) {
  const current = readFileSync(OUT, 'utf8');
  if (current !== text) {
    console.error(`${OUT} is stale — run: node scripts/generate-live-layout-fixture.mjs`);
    process.exit(1);
  }
  console.log(`${OUT} is current`);
} else {
  writeFileSync(OUT, text);
  console.log(`wrote ${OUT} (${fixture.cases.length} cases, version ${fixture.version})`);
}
