// @vitest-environment jsdom
import cases from '../fixtures/console-payloads.json';
import { afterEach, describe, expect, it } from 'vitest';
import { buildConsoleView, withVisitingSystems } from '../../gui/console-state.js';
import { evaluateTrigger, LEGACY_TUTORIAL_PROGRESS_KEY } from '../../gui/tutorial-state.js';
import { getTable, setTable } from '../../gui/strings.js';

// Complete decoded window output captured before the object API refactor.
const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
afterEach(() => {
  Object.defineProperty(globalThis, 'localStorage', originalStorage);
  localStorage.clear();
});
function freeze(value) {
  if (value && typeof value === 'object') {
    Object.values(value).forEach(freeze);
    Object.freeze(value);
  }
  return value;
}
describe('console object construction and JSON publication', () => {
  for (const row of cases) {
    it(`preserves complete ${row.name} window output`, () => {
      const table = getTable();
      setTable(new Map(['fixture.title', 'fixture.text', 'console.radar.tactical_target', 'console.radar.science_target'].map(id => [id, id])));
      try {
        expect(JSON.parse(window.buildConsoleState(row.station, structuredClone(row.state)))).toStrictEqual(row.expected);
      } finally { setTable(table); }
    });
  }
  it('does not mutate blackboards/topology or contaminate earlier Station views', () => {
    const state = freeze(structuredClone(cases[2].state));
    const first = buildConsoleView('pilot', state);
    const snapshot = structuredClone(first);
    buildConsoleView('engineering', state);
    expect(first).toStrictEqual(snapshot);
    expect(state).toStrictEqual(cases[2].state);
    expect(first.systems.thrust).toBe(first.systems.steer);
    expect(first.system_families).toMatchObject({ thrust: 'helm', steer: 'helm', radio: 'comms' });
  });
  it('copies existing composite maps before adding visitors', () => {
    const state = structuredClone(cases[2].state);
    state.blackboards.radio.host_station = 'engineering';
    const payload = freeze({ systems: { reactor: { power: true } }, system_families: { reactor: 'power' } });
    const view = withVisitingSystems('engineering', state, payload);
    expect(view.systems).not.toBe(payload.systems);
    expect(view.system_families).not.toBe(payload.system_families);
    expect(payload.systems).toEqual({ reactor: { power: true } });
    expect(view.systems.radio).toBeTruthy();
  });
  it('evaluates tutorials after visitors, directed advice and Station damage, then adds takeover', () => {
    const state = structuredClone(cases[2].state);
    state.blackboardKinds.orders = 'Command';
    state.blackboards.orders = { directed_station: 'pilot', directed_station_ai: false, selected_stance: 'defend', stances: [{ id: 'defend', label: 'Defend' }] };
    state.stationTutorials.pilot = [{ id: 'advice', trigger: { kind: 'state', path: 'command_advice.stance_id', op: 'truthy' } }];
    const view = buildConsoleView('pilot', state);
    expect(view.tutorial.active.id).toBe('advice');
    expect(view.command_advice.stance_id).toBe('defend');
    expect(view.own_hull).toBeTruthy();
    expect(view.gm_takeover.operator_id).toBe('gm-fixture');
    state.stationTutorials.pilot = cases[2].state.stationTutorials.pilot;
    expect(buildConsoleView('pilot', state).tutorial.active.id).toBe('visitor');
    state.stationTutorials.pilot = [{ id: 'damage', trigger: { kind: 'state', path: 'own_hull', op: 'truthy' } }];
    expect(buildConsoleView('pilot', state).tutorial.active.id).toBe('damage');
    expect(buildConsoleView('engineering', state)).not.toHaveProperty('command_advice');
  });
  it('retains pre-Welcome window JSON contracts', () => {
    expect(window.buildConsoleStateInner('helm', {})).toBe('{}');
    expect(JSON.parse(window.buildConsoleState('helm', {}))).toMatchObject({ hosted_systems: [], tutorial: null, gm_takeover: null });
  });
  for (const storage of [null, { getItem() { throw new Error('unavailable'); }, setItem() { throw new Error('unavailable'); } }]) {
    it(`keeps in-memory tutorial progress with ${storage ? 'throwing' : 'absent'} storage`, () => {
      Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
      const state = structuredClone(cases[2].state);
      state.tutorialProgress.dismissed['fixture-hull/pilot/visitor'] = true;
      expect(JSON.parse(window.buildConsoleState('pilot', state)).tutorial).toBeNull();
    });
  }
  it('continues when accessing storage throws', () => {
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('privacy'); } });
    expect(JSON.parse(window.buildConsoleState('pilot', structuredClone(cases[2].state))).tutorial.active.id).toBe('visitor');
  });
  it('migrates legacy progress only through the full adapter', () => {
    localStorage.setItem(LEGACY_TUTORIAL_PROGRESS_KEY, JSON.stringify({ dismissed: { 'pilot/visitor': true }, used: {} }));
    const state = structuredClone(cases[2].state);
    expect(buildConsoleView('pilot', state).tutorial.active.id).toBe('visitor');
    expect(state.tutorialProgress.dismissed).toEqual({});
    expect(JSON.parse(window.buildConsoleState('pilot', state)).tutorial).toBeNull();
    expect(state.tutorialProgress.dismissed['fixture-hull/pilot/visitor']).toBe(true);
  });
  it('surfaces unexpected transformation errors', () => {
    const error = new Error('broken simulation view');
    const state = { blackboards: new Proxy({}, { ownKeys() { throw error; } }) };
    expect(() => buildConsoleView('pilot', state)).toThrow(error);
    expect(() => window.buildConsoleState('pilot', state)).toThrow(error);
  });
});
describe('tutorial comparisons preserve JSON numeric semantics', () => {
  for (const value of [null, NaN, Infinity, -Infinity]) {
    it(`treats ${String(value)} as null`, () => {
      expect(evaluateTrigger({ kind: 'state', path: 'value', op: 'eq', value: 0 }, { value })).toBe(true);
      expect(evaluateTrigger({ kind: 'state', path: 'value', op: 'falsy' }, { value })).toBe(true);
    });
  }
  it('keeps missing fields distinct from null', () => {
    expect(evaluateTrigger({ kind: 'state', path: 'missing', op: 'eq', value: 0 }, {})).toBe(false);
    expect(evaluateTrigger({ kind: 'state', path: 'missing', op: 'falsy' }, {})).toBe(false);
  });
});
