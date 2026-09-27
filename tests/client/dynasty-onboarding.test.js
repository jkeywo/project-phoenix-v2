// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { parse } from 'smol-toml';
import { describe, expect, it, vi } from 'vitest';
import {
  buildTutorialState, emptyTutorialProgress, progressWithDismissed,
  tutorialProgressAfterAction, scopedTutorialKey,
} from '../../gui/tutorial-state.js';
import { applyToDom, t } from '../../gui/strings.js';
import { renderStationPanel } from '../../gui/manual-panel.js';
import { buildPowerConsoleState, buildWeaponsConsoleState } from '../../gui/console-state.js';
import '../../gui/components/ph-tutorial-overlay.js';
import { createTacticalActionRegistry } from '../../gui/stations/tactical-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { renderHelm, renderPower, renderDamageControl } from '../../gui/dynasty-cruiser/console.js';
import { createGamepadInputRuntime } from '../../gui/gamepad-input.js';
import {
  createOperatorProfileSnapshot, serializeOperatorProfile,
  prepareOperatorProfileImport, applyOperatorProfile,
} from '../../gui/operator-profile.js';

const hull = parse(readFileSync('assets/entities/dynasty_player_cruiser.toml', 'utf8'));
const stations = hull.station.filter(s => !s._remove);
const hullId = hull.hull_id;
function reservePayload(role, reserve) {
  const power = role === 'power';
  const id = power ? 'power' : 'tactical';
  const state = { blackboards: { [id]: { strike_reserve: reserve } },
    blackboardKinds: { [id]: power ? 'Power' : 'Weapons' } };
  return JSON.parse((power ? buildPowerConsoleState : buildWeaponsConsoleState)(state));
}
function journey(stationId) {
  const defs = stations.find(s => s.id === stationId).tutorial;
  let progress = emptyTutorialProgress();
  return {
    read: payload => buildTutorialState(defs, progress, payload, hullId, stationId),
    dismiss: id => { progress = progressWithDismissed(progress, scopedTutorialKey(hullId, stationId, id)); },
    used: action => {
      const folded = tutorialProgressAfterAction(progress, { action, console: stationId }, hullId);
      expect(folded.handled).toBe(false); // ordinary action still reaches admission
      progress = folded.progress;
    },
  };
}

describe('authored Dynasty onboarding', () => {
  it('exposes authored Dock and recovery gear on their human Stations', () => {
    const helmDoc = new DOMParser().parseFromString(readFileSync('gui/dynasty-cruiser/helm.html', 'utf8'), 'text/html');
    const damageDoc = new DOMParser().parseFromString(readFileSync('gui/dynasty-cruiser/damage-control.html', 'utf8'), 'text/html');
    applyToDom(helmDoc);
    applyToDom(damageDoc);
    expect(helmDoc.querySelector('script[type="module"]').textContent).toContain('HELM_DOCK_ACTION_ID');
    expect(damageDoc.querySelector('script[type="module"]').textContent).toContain("'engineering'");
    renderHelm({
      system_ids: ['dock'], system_families: { dock: 'helm' },
      systems: { dock: { dock: { system_id: 'dock', available: true, available_target_name: 'Berth' },
        tow_load: { active: true, target_name: 'Freighter' } } },
    }, helmDoc);
    expect(helmDoc.getElementById('dock-panel').hidden).toBe(false);
    expect(helmDoc.getElementById('dock-btn').dataset.systemId).toBe('dock');
    expect(helmDoc.getElementById('dock-status').textContent).toContain('Berth');
    expect(helmDoc.getElementById('tow-load-panel').hidden).toBe(false);
    renderDamageControl({
      system_ids: ['tractor', 'umbilical'],
      system_families: { tractor: 'tractor', umbilical: 'umbilical' },
      systems: { tractor: { system_id: 'tractor', engaged: true },
        umbilical: { system_id: 'umbilical', running: true } },
    }, damageDoc);
    expect(damageDoc.getElementById('tractor-panel').hidden).toBe(false);
    expect(damageDoc.getElementById('tractor-btn').classList.contains('engaged')).toBe(true);
    expect(damageDoc.getElementById('umbilical-panel').hidden).toBe(false);
    expect(damageDoc.getElementById('umbilical-btn').classList.contains('engaged')).toBe(true);
  });

  it.each([
    ['command', 'captain'], ['gunnery', 'tactical'], ['damage-control', 'engineering'],
  ])('%s uses its shared action context for keyboard and gamepad input', (station, context) => {
    const html = readFileSync(`gui/dynasty-cruiser/${station}.html`, 'utf8');
    expect(html).toContain(`getActionContext:()=> '${context}'`);
  });

  it('shows battery and reserve together on Power so allocation costs remain readable', () => {
    const doc = new DOMParser().parseFromString(readFileSync('gui/dynasty-cruiser/power.html', 'utf8'), 'text/html');
    const power = { battery_charge: 30, battery_max: 60,
      strike_reserve: { charge: 8, capacity: 20, charging: true } };
    renderPower({ system_ids: ['power-reactor'],
      system_families: { 'power-reactor': 'power' },
      systems: { 'power-reactor': power } }, doc);
    expect(doc.getElementById('battery-bar').state.level_pct).toBe(50);
    expect(doc.getElementById('strike-reserve').state).toEqual(power.strike_reserve);
  });

  it('renders all six authored manual overviews as prose and preserves literal mod overviews', () => {
    for (const station of stations) {
      const panel = renderStationPanel(document, { overview: station.manual_overview, sections: [] });
      expect(panel.textContent).toBe(t(station.manual_overview));
      expect(panel.textContent).not.toContain('dynasty.onboarding.');
      expect(panel.textContent.length).toBeGreaterThan(50);
    }
    const prose = 'Keep a course through the outer belt.';
    expect(renderStationPanel(document, { overview: prose }).textContent).toBe(prose);
  });
  it.each(stations.map(s => [s.id]))('%s teaches its role and the shared cycle without waiting for other crew', role => {
    const guide = journey(role);
    const expected = ['welcome', 'cycle', 'backfill', 'inputs'];
    for (const suffix of expected) {
      const lesson = guide.read({});
      expect(lesson.active.id).toBe(`${role}-${suffix}`);
      expect(t(lesson.active.title)).not.toContain('⟨');
      expect(t(lesson.active.text)).not.toContain('⟨');
      guide.dismiss(lesson.active.id);
    }
    expect(guide.read({})).toBeNull();
    // Another operator/Station has independent tutorial progress.
    expect(journey(role).read({}).active.id).toBe(`${role}-welcome`);
  });

  it('follows real reserve projections from charging through boost and depletion to re-enable', () => {
    const power = journey('power');
    const gun = journey('gunnery');
    expect(power.read(reservePayload('power', { charging: true })).active.id).toBe('power-charging');
    expect(gun.read(reservePayload('gunnery', { charge: 12, enabled: false })).active.id).toBe('gunnery-ready');
    gun.used('set_strike_boost');
    expect(gun.read({ strike_reserve: { charge: 12, enabled: true } }).active.id).toBe('gunnery-enabled');
    const depleted = { charge: 1, enabled: false, depleted: true };
    expect(gun.read({ strike_reserve: depleted }).active.id).toBe('gunnery-depleted');
    expect(power.read(reservePayload('power', depleted)).active.id).toBe('power-depleted');
    // State changes, including ordinary Backfill changes, drive the lessons.
    // No local tutorial event changes charge or authorises a weapon action.
    expect(gun.read({ strike_reserve: { charge: 12, enabled: true, depleted: false } }).active.id)
      .toBe('gunnery-enabled');
    expect(depleted).toEqual({ charge: 1, enabled: false, depleted: true });
  });

  it.each(stations.map(s => [s.id, s.console]))('%s retains a translated reopenable guide and valid lesson anchors', (role, file) => {
    const doc = new DOMParser().parseFromString(readFileSync(file, 'utf8'), 'text/html');
    applyToDom(doc);
    const guide = doc.getElementById('role-guidance');
    expect(guide.tagName).toBe('DETAILS');
    expect(guide.querySelector('summary').textContent).toContain('Role');
    expect(guide.querySelector('ol').children).toHaveLength(4);
    guide.open = true;
    expect(guide.textContent).toContain('Backfill');
    guide.open = false;
    guide.open = true;
    expect(guide.open).toBe(true);
    for (const lesson of stations.find(s => s.id === role).tutorial) {
      if (lesson.anchor) expect(doc.getElementById(lesson.anchor), lesson.id).not.toBeNull();
    }
  });

  it('shows depletion text and dismisses through the shared tutorial action without a combat command', () => {
    const guide = journey('gunnery');
    const element = document.createElement('ph-tutorial-overlay');
    const send = vi.fn();
    element.sendAction = send;
    document.body.append(element);
    element.state = guide.read({ strike_reserve: { charge: 1, enabled: false, depleted: true } });
    expect(element.shadowRoot.getElementById('text').textContent).toContain('fired normally');
    element.shadowRoot.getElementById('dismiss').click();
    expect(send).toHaveBeenCalledExactlyOnceWith('tutorial_dismiss', { overlay_id: 'gunnery-depleted' });
    element.remove();
  });

  it('keeps boost on the ordinary action path after keyboard profile import and gamepad selection', () => {
    const send = vi.fn();
    let state = { strike_reserve: { charge: 12, enabled: false }, tactical_auto: false };
    const source = createTacticalActionRegistry({ getState: () => state, sendAction: send });
    source.setBinding('tactical.strike-boost', 0, { type: 'keyboard', code: 'KeyY' });
    const profile = createOperatorProfileSnapshot({ bindings: source.bindingProfile() });
    const actions = createTacticalActionRegistry({ getState: () => state, sendAction: send,
      actionFeedback: new ActionFeedbackLifecycle() });
    const imported = prepareOperatorProfileImport(serializeOperatorProfile(profile), { registry: actions });
    expect(imported.status).toBe('imported');
    expect(applyOperatorProfile(imported.profile, actions).status).toBe('applied');
    const key = code => ({ type: 'keydown', code, cancelable: true, preventDefault: vi.fn() });
    expect(actions.dispatchKeyboardEvent(key('KeyV'), 'tactical').claimed).toBe(false);
    expect(actions.dispatchKeyboardEvent(key('KeyY'), 'tactical').handled).toBe(true);
    expect(send).toHaveBeenLastCalledWith('set_strike_boost', expect.objectContaining({ enabled: true }));
    expect(state.strike_reserve.enabled).toBe(false); // host acknowledgement owns state

    const pad = { index: 0, id: 'journey-pad', mapping: 'standard', axes: [0, 0, 0, 0],
      buttons: Array.from({ length: 17 }, () => ({ pressed: false, value: 0 })) };
    const runtime = createGamepadInputRuntime({ getGamepads: () => [pad],
      getContext: () => 'tactical', getActions: () => actions.list('tactical'),
      activate: (id, detail) => actions.activate(id, detail) });
    runtime.select(0);
    runtime.poll();
    state = { ...state, strike_reserve: { charge: 12, enabled: true } };
    pad.buttons[3] = { pressed: true, value: 1 };
    runtime.poll();
    expect(send).toHaveBeenLastCalledWith('set_strike_boost', expect.objectContaining({ enabled: false }));
    const sent = send.mock.calls.length;
    pad.buttons[3] = { pressed: false, value: 0 };
    runtime.poll();
    state = { ...state, tactical_auto: true };
    pad.buttons[3] = { pressed: true, value: 1 };
    runtime.poll();
    expect(send).toHaveBeenCalledTimes(sent); // Backfill's system stays authoritative
  });
});
