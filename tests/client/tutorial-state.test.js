/**
 * tests/client/tutorial-state.test.js — Contextual tutorial trigger
 * evaluation (issue #916 AC4).
 *
 * Pure-Node tests over gui/tutorial-state.js: the trigger vocabulary
 * (first_visit / control_unused / state), the shared dismissal and
 * control-completion gates, station-scoped progress keys, priority ordering,
 * the progress reducers, the storage round-trip, and hydration into the
 * sim-state singleton (module-graph regression). Definitions mirror the wire
 * shape delivered on Welcome (`ship_config.station_tutorials`), with content
 * as string ids — never English.
 */
import { describe, it, expect, vi } from 'vitest';
import {
  TUTORIAL_DISMISS_ACTION,
  TUTORIAL_PROGRESS_KEY,
  emptyTutorialProgress,
  normalizeTutorialProgress,
  scopedTutorialKey,
  progressWithDismissed,
  progressWithControlUsed,
  readPath,
  evaluateTrigger,
  eligibleOverlays,
  buildTutorialState,
  loadTutorialProgress,
  saveTutorialProgress,
  tutorialProgressAfterAction,
  hydrateTutorialProgress,
} from '../../gui/tutorial-state.js';

// ── Fixtures (wire-shaped, string-id content) ───────────────────────────────

const STATION = 'helm';

const welcome = {
  id: 'helm-welcome',
  trigger: { kind: 'first_visit' },
  title: 'entity.alliance_destroyer.station.helm.tutorial.welcome.title',
  text: 'entity.alliance_destroyer.station.helm.tutorial.welcome.text',
  anchor: 'helm-radar',
};

const joystick = {
  id: 'helm-joystick',
  trigger: { kind: 'control_unused', control: 'set_helm' },
  title: 'entity.alliance_destroyer.station.helm.tutorial.joystick.title',
  text: 'entity.alliance_destroyer.station.helm.tutorial.joystick.text',
  anchor: 'helm-joystick',
};

const boost = {
  id: 'helm-boost',
  priority: 5,
  trigger: { kind: 'state', path: 'boost_enabled', op: 'truthy', control: 'set_boost' },
  title: 'entity.alliance_destroyer.station.helm.tutorial.boost.title',
  text: 'entity.alliance_destroyer.station.helm.tutorial.boost.text',
  anchor: 'boost-btn',
};

const redAlert = {
  id: 'helm-red-alert',
  priority: 10,
  trigger: { kind: 'state', path: 'red_alert', op: 'truthy' },
  title: 'entity.alliance_destroyer.station.helm.tutorial.red_alert.title',
  text: 'entity.alliance_destroyer.station.helm.tutorial.red_alert.text',
  anchor: 'helm-radar',
};

const DEFS = [welcome, joystick, boost, redAlert];

// ── Trigger kinds ───────────────────────────────────────────────────────────

describe('evaluateTrigger', () => {
  it('first_visit is always condition-true (dismissal is the only gate)', () => {
    expect(evaluateTrigger({ kind: 'first_visit' }, {})).toBe(true);
    expect(evaluateTrigger({ kind: 'first_visit' }, null)).toBe(true);
  });

  it('control_unused requires a named control', () => {
    expect(evaluateTrigger({ kind: 'control_unused', control: 'set_helm' }, {})).toBe(true);
    expect(evaluateTrigger({ kind: 'control_unused' }, {})).toBe(false);
    expect(evaluateTrigger({ kind: 'control_unused', control: '' }, {})).toBe(false);
  });

  it('state truthy/falsy read the payload field', () => {
    const trig = { kind: 'state', path: 'boost_enabled', op: 'truthy' };
    expect(evaluateTrigger(trig, { boost_enabled: true })).toBe(true);
    expect(evaluateTrigger(trig, { boost_enabled: false })).toBe(false);
    expect(evaluateTrigger(trig, {})).toBe(false);
    const neg = { kind: 'state', path: 'lateral_is_online', op: 'falsy' };
    expect(evaluateTrigger(neg, { lateral_is_online: false })).toBe(true);
    expect(evaluateTrigger(neg, { lateral_is_online: true })).toBe(false);
    // A missing path is not "falsy" — it is unknown, and unknown fails closed.
    expect(evaluateTrigger(neg, {})).toBe(false);
  });

  it('state defaults the operator to truthy when op is absent', () => {
    expect(evaluateTrigger({ kind: 'state', path: 'red_alert' }, { red_alert: true })).toBe(true);
    expect(evaluateTrigger({ kind: 'state', path: 'red_alert' }, { red_alert: false })).toBe(false);
  });

  it('state numeric comparisons work over dot-paths', () => {
    const payload = { own_hull: { pct: 0.4 }, speed: 12 };
    expect(evaluateTrigger({ kind: 'state', path: 'own_hull.pct', op: 'lt', value: 0.5 }, payload)).toBe(true);
    expect(evaluateTrigger({ kind: 'state', path: 'own_hull.pct', op: 'gt', value: 0.5 }, payload)).toBe(false);
    expect(evaluateTrigger({ kind: 'state', path: 'speed', op: 'gte', value: 12 }, payload)).toBe(true);
    expect(evaluateTrigger({ kind: 'state', path: 'speed', op: 'lte', value: 11 }, payload)).toBe(false);
    expect(evaluateTrigger({ kind: 'state', path: 'speed', op: 'eq', value: 12 }, payload)).toBe(true);
    expect(evaluateTrigger({ kind: 'state', path: 'speed', op: 'ne', value: 12 }, payload)).toBe(false);
    expect(evaluateTrigger({ kind: 'state', path: 'speed', op: 'ne', value: 3 }, payload)).toBe(true);
  });

  it('unknown kinds and unknown ops fail closed', () => {
    expect(evaluateTrigger({ kind: 'on_mars' }, {})).toBe(false);
    expect(evaluateTrigger({ kind: 'state', path: 'speed', op: 'spaceship', value: 1 }, { speed: 2 })).toBe(false);
    expect(evaluateTrigger(null, {})).toBe(false);
    expect(evaluateTrigger({}, {})).toBe(false);
  });
});

describe('readPath', () => {
  it('reads flat, nested, and hyphenated-key paths', () => {
    expect(readPath({ a: 1 }, 'a')).toBe(1);
    expect(readPath({ a: { b: { c: 3 } } }, 'a.b.c')).toBe(3);
    expect(readPath({ systems: { 'helm-thrust': { x: 7 } } }, 'systems.helm-thrust.x')).toBe(7);
  });

  it('returns undefined on any miss without throwing', () => {
    expect(readPath({}, 'a.b')).toBeUndefined();
    expect(readPath(null, 'a')).toBeUndefined();
    expect(readPath({ a: 1 }, '')).toBeUndefined();
    expect(readPath({ a: null }, 'a.b')).toBeUndefined();
  });
});

// ── Overlay-level gates + ordering ──────────────────────────────────────────

describe('eligibleOverlays', () => {
  const calmPayload = { boost_enabled: false, red_alert: false };

  it('fresh progress on a calm console shows the intro tips in authored order', () => {
    const ids = eligibleOverlays(DEFS, emptyTutorialProgress(), calmPayload, STATION).map(d => d.id);
    expect(ids).toEqual(['helm-welcome', 'helm-joystick']);
  });

  it('a dismissed overlay never returns, whatever its kind', () => {
    let p = progressWithDismissed(emptyTutorialProgress(), scopedTutorialKey(STATION, 'helm-welcome'));
    p = progressWithDismissed(p, scopedTutorialKey(STATION, 'helm-red-alert'));
    const ids = eligibleOverlays(DEFS, p, { boost_enabled: false, red_alert: true }, STATION).map(d => d.id);
    expect(ids).toEqual(['helm-joystick']);
  });

  it('using the named control completes control_unused overlays', () => {
    const p = progressWithControlUsed(emptyTutorialProgress(), scopedTutorialKey(STATION, 'set_helm'));
    const ids = eligibleOverlays(DEFS, p, calmPayload, STATION).map(d => d.id);
    expect(ids).toEqual(['helm-welcome']);
  });

  it('a state overlay with a control also completes on use (boost tip retires after first boost)', () => {
    const hot = { boost_enabled: true, red_alert: false };
    expect(eligibleOverlays(DEFS, emptyTutorialProgress(), hot, STATION).map(d => d.id))
      .toContain('helm-boost');
    const p = progressWithControlUsed(emptyTutorialProgress(), scopedTutorialKey(STATION, 'set_boost'));
    expect(eligibleOverlays(DEFS, p, hot, STATION).map(d => d.id)).not.toContain('helm-boost');
  });

  it('progress is station-scoped: another station sharing an id or control never leaks', () => {
    // 'tactical' dismissed an overlay with the SAME id and used the SAME
    // control name — helm's overlays must be untouched.
    let p = progressWithDismissed(emptyTutorialProgress(), scopedTutorialKey('tactical', 'helm-welcome'));
    p = progressWithControlUsed(p, scopedTutorialKey('tactical', 'set_helm'));
    const ids = eligibleOverlays(DEFS, p, { boost_enabled: false, red_alert: false }, STATION).map(d => d.id);
    expect(ids).toEqual(['helm-welcome', 'helm-joystick']);
  });

  it('higher priority preempts the intro queue; equal priority keeps authored order', () => {
    const hot = { boost_enabled: true, red_alert: true };
    const ids = eligibleOverlays(DEFS, emptyTutorialProgress(), hot, STATION).map(d => d.id);
    expect(ids).toEqual(['helm-red-alert', 'helm-boost', 'helm-welcome', 'helm-joystick']);
  });

  it('tolerates null defs, malformed entries, and missing progress', () => {
    expect(eligibleOverlays(null, null, {}, STATION)).toEqual([]);
    expect(eligibleOverlays([null, {}, { id: 'x' }], undefined, {}, STATION)).toEqual([]);
  });
});

describe('buildTutorialState', () => {
  it('returns null when nothing is eligible', () => {
    const p = progressWithDismissed(
      progressWithControlUsed(emptyTutorialProgress(), scopedTutorialKey(STATION, 'set_helm')),
      scopedTutorialKey(STATION, 'helm-welcome'),
    );
    expect(buildTutorialState(DEFS, p, { boost_enabled: false, red_alert: false }, STATION)).toBeNull();
    expect(buildTutorialState([], emptyTutorialProgress(), {}, STATION)).toBeNull();
  });

  it('returns the single active overlay plus the eligible count', () => {
    const s = buildTutorialState(DEFS, emptyTutorialProgress(), { boost_enabled: false, red_alert: false }, STATION);
    expect(s.active.id).toBe('helm-welcome');
    expect(s.active.title).toBe('entity.alliance_destroyer.station.helm.tutorial.welcome.title');
    expect(s.active.anchor).toBe('helm-radar');
    expect(s.remaining).toBe(2);
  });

  it('dismissing the active overlay advances to the next one', () => {
    const p = progressWithDismissed(emptyTutorialProgress(), scopedTutorialKey(STATION, 'helm-welcome'));
    const s = buildTutorialState(DEFS, p, { boost_enabled: false, red_alert: false }, STATION);
    expect(s.active.id).toBe('helm-joystick');
    expect(s.remaining).toBe(1);
  });
});

// ── Progress reducers ───────────────────────────────────────────────────────

describe('progress reducers', () => {
  it('scopedTutorialKey formats <station>/<name>', () => {
    expect(scopedTutorialKey('helm', 'helm-welcome')).toBe('helm/helm-welcome');
    expect(scopedTutorialKey('tactical', 'set_boost')).toBe('tactical/set_boost');
  });

  it('progressWithDismissed / progressWithControlUsed do not mutate their input', () => {
    const p = emptyTutorialProgress();
    const p2 = progressWithDismissed(p, 'helm/a');
    const p3 = progressWithControlUsed(p2, 'helm/set_helm');
    expect(p.dismissed).toEqual({});
    expect(p2.dismissed).toEqual({ 'helm/a': true });
    expect(p2.used).toEqual({});
    expect(p3.used).toEqual({ 'helm/set_helm': true });
    expect(p3.dismissed).toEqual({ 'helm/a': true });
  });

  it('returns the same reference when nothing changes (change detection)', () => {
    const p = progressWithDismissed(emptyTutorialProgress(), 'helm/a');
    expect(progressWithDismissed(p, 'helm/a')).toBe(p);
    const u = progressWithControlUsed(emptyTutorialProgress(), 'helm/x');
    expect(progressWithControlUsed(u, 'helm/x')).toBe(u);
    expect(progressWithDismissed(p, null)).toBe(p);
    expect(progressWithControlUsed(u, undefined)).toBe(u);
  });

  it('normalizeTutorialProgress coerces junk into a valid record', () => {
    expect(normalizeTutorialProgress(null)).toEqual({ dismissed: {}, used: {} });
    expect(normalizeTutorialProgress('nope')).toEqual({ dismissed: {}, used: {} });
    expect(normalizeTutorialProgress({ dismissed: { a: 1, b: 0 }, used: 7 }))
      .toEqual({ dismissed: { a: true }, used: {} });
  });
});

// ── Action folding ──────────────────────────────────────────────────────────

describe('tutorialProgressAfterAction', () => {
  it('tutorial_dismiss records the station-scoped overlay key and is handled locally', () => {
    const r = tutorialProgressAfterAction(
      emptyTutorialProgress(),
      { action: TUTORIAL_DISMISS_ACTION, overlay_id: 'helm-welcome', console: 'helm' },
    );
    expect(r.handled).toBe(true);
    expect(r.changed).toBe(true);
    expect(r.progress.dismissed['helm/helm-welcome']).toBe(true);
  });

  it('a repeat dismissal is handled but unchanged', () => {
    const p = progressWithDismissed(emptyTutorialProgress(), scopedTutorialKey('helm', 'helm-welcome'));
    const r = tutorialProgressAfterAction(
      p,
      { action: TUTORIAL_DISMISS_ACTION, overlay_id: 'helm-welcome', console: 'helm' },
    );
    expect(r.handled).toBe(true);
    expect(r.changed).toBe(false);
    expect(r.progress).toBe(p);
  });

  it('any other action records its station-scoped name as used and flows on', () => {
    const first = tutorialProgressAfterAction(
      emptyTutorialProgress(),
      { action: 'set_helm', console: 'helm', thrust: 1 },
    );
    expect(first.handled).toBe(false);
    expect(first.changed).toBe(true);
    expect(first.progress.used['helm/set_helm']).toBe(true);
    const second = tutorialProgressAfterAction(
      first.progress,
      { action: 'set_helm', console: 'helm', thrust: 0 },
    );
    expect(second.changed).toBe(false);
    expect(second.handled).toBe(false);
  });

  it('the same action on another console is tracked separately', () => {
    const helm = tutorialProgressAfterAction(
      emptyTutorialProgress(),
      { action: 'set_target', console: 'helm' },
    );
    const both = tutorialProgressAfterAction(
      helm.progress,
      { action: 'set_target', console: 'tactical' },
    );
    expect(both.changed).toBe(true);
    expect(both.progress.used).toEqual({ 'helm/set_target': true, 'tactical/set_target': true });
  });

  it('an envelope without a console records nothing (fail closed), but a dismiss stays handled', () => {
    const p = emptyTutorialProgress();
    const used = tutorialProgressAfterAction(p, { action: 'set_helm' });
    expect(used).toEqual({ progress: p, changed: false, handled: false });
    const dismissed = tutorialProgressAfterAction(p, { action: TUTORIAL_DISMISS_ACTION, overlay_id: 'x' });
    expect(dismissed.handled).toBe(true);
    expect(dismissed.changed).toBe(false);
    expect(dismissed.progress).toBe(p);
  });

  it('a malformed action is a no-op', () => {
    const p = emptyTutorialProgress();
    const r = tutorialProgressAfterAction(p, null);
    expect(r).toEqual({ progress: p, changed: false, handled: false });
  });
});

// ── Persistence ─────────────────────────────────────────────────────────────

function fakeStorage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => { map.set(k, String(v)); },
    _map: map,
  };
}

describe('persistence', () => {
  it('round-trips progress through a storage object', () => {
    const storage = fakeStorage();
    const p = progressWithControlUsed(
      progressWithDismissed(emptyTutorialProgress(), scopedTutorialKey('helm', 'helm-welcome')),
      scopedTutorialKey('helm', 'set_helm'),
    );
    saveTutorialProgress(storage, p);
    expect(storage._map.has(TUTORIAL_PROGRESS_KEY)).toBe(true);
    expect(loadTutorialProgress(storage)).toEqual(p);
  });

  it('missing key, corrupt JSON, and throwing storage all yield empty progress', () => {
    expect(loadTutorialProgress(fakeStorage())).toEqual(emptyTutorialProgress());
    expect(loadTutorialProgress(fakeStorage({ [TUTORIAL_PROGRESS_KEY]: '{nope' })))
      .toEqual(emptyTutorialProgress());
    expect(loadTutorialProgress({ getItem() { throw new Error('denied'); } }))
      .toEqual(emptyTutorialProgress());
    expect(loadTutorialProgress(null)).toEqual(emptyTutorialProgress());
    // save never throws either
    expect(() => saveTutorialProgress({ setItem() { throw new Error('quota'); } }, emptyTutorialProgress()))
      .not.toThrow();
  });
});

// ── Hydration into the sim-state singleton ──────────────────────────────────

describe('hydrateTutorialProgress', () => {
  it('points sim.tutorialProgress at the stored record', () => {
    const sim = { tutorialProgress: emptyTutorialProgress() };
    const stored = { dismissed: { 'helm/helm-welcome': true }, used: { 'helm/set_helm': true } };
    const storage = fakeStorage({ [TUTORIAL_PROGRESS_KEY]: JSON.stringify(stored) });
    hydrateTutorialProgress(sim, storage);
    expect(sim.tutorialProgress).toEqual(stored);
  });

  it('a broken or absent storage yields a fresh empty record, never a throw', () => {
    const sim = {};
    hydrateTutorialProgress(sim, { getItem() { throw new Error('denied'); } });
    expect(sim.tutorialProgress).toEqual(emptyTutorialProgress());
    hydrateTutorialProgress(sim, null);
    expect(sim.tutorialProgress).toEqual(emptyTutorialProgress());
    expect(() => hydrateTutorialProgress(null, fakeStorage())).not.toThrow();
  });

  it('hydrates the REAL simState singleton at module evaluation, regardless of load order', async () => {
    // Regression for the issue #916 review finding: gui/console-state.js
    // statically imports tutorial-state.js long before client.html's
    // sim-state.js script tag runs, so hydration must not depend on
    // window.simState already existing — the explicit `import { simState }`
    // inside tutorial-state.js is what guarantees ordering. Simulate the
    // worst case: a fresh module registry where tutorial-state.js is the
    // FIRST module evaluated.
    const stored = { dismissed: { 'helm/helm-welcome': true }, used: { 'helm/set_boost': true } };
    vi.stubGlobal('localStorage', fakeStorage({ [TUTORIAL_PROGRESS_KEY]: JSON.stringify(stored) }));
    vi.resetModules();
    try {
      await import('../../gui/tutorial-state.js');
      const { simState } = await import('../../gui/sim-state.js');
      expect(simState.tutorialProgress).toEqual(stored);
      // Welcome's reset() must PRESERVE hydrated progress — a reconnect must
      // not replay dismissed tips.
      simState.reset();
      expect(simState.tutorialProgress).toEqual(stored);
    } finally {
      vi.unstubAllGlobals();
      vi.resetModules();
    }
  });
});
