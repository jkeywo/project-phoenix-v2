// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { ClientSimState } from '../../gui/sim-state.js';
import { buildSystemStationConsoleState, buildWeaponsConsoleState } from '../../gui/console-state.js';
import { familyView } from '../../gui/console-payload.js';
import { makeTacticalRender } from '../../gui/stations/tactical-console.js';
import { cooldownRemainingPercent, withWeaponCooldowns } from '../../gui/weapon-cooldown.js';
import '../../gui/components/ph-phasers-controls.js';
import '../../gui/components/ph-blasters-controls.js';

function welcome(state, { phasers = [], blasters = [], hull = 'alliance_destroyer' } = {}) {
  state.apply({ type: 'Welcome', data: {
    state: { phase: 'InProgress', players: [], complexity: {}, world: { entities: [] } },
    ship_stations: { configs: {}, min_players: 0, max_players: 0 },
    ship_config: {
      hull_id: hull,
      phaser_banks: phasers,
      blaster_banks: blasters,
      station_systems: { tactical: ['tactical-radar'] },
      system_console_families: { 'tactical-radar': 'tactical' },
    },
  } });
}

function config(id, cooldown_secs) {
  return { id, facing_deg: 0, fire_arc_deg: 90, cooldown_secs };
}

function cooling(id, remaining) {
  return { id, fire_ready: false, on_cooldown: true, cooldown_remaining: remaining,
    readiness: { ready: false, blocking_reason: 'Cooldown' } };
}

function weapons(state, banks, blasters = []) {
  state.apply({ type: 'WeaponsUpdate', data: {
    target_uuid: 'enemy', banks, blasters, tubes: [], torpedo_count: 0, phaser_mode: 'Manual',
  } });
}

function mount(keyed = false) {
  document.body.innerHTML = '<ph-phasers-controls id="phasers"></ph-phasers-controls>'
    + '<ph-blasters-controls id="blasters"></ph-blasters-controls>';
  const render = makeTacticalRender({
    weaponsView: keyed ? payload => familyView(payload, 'tactical') : payload => payload,
    ids: { radar: 'absent-radar', phasers: 'phasers', blasters: 'blasters' },
  });
  return state => render(JSON.parse(keyed
    ? buildSystemStationConsoleState('tactical', state)
    : buildWeaponsConsoleState(state)), document);
}

function fill(family, id) {
  return document.getElementById(family).shadowRoot
    .querySelector(`[data-id="${id}"] ${family === 'phasers' ? '.cooldown-fill' : '.bar-fill.cooldown'}`);
}

function fire(family, id) {
  return document.getElementById(family).shadowRoot.querySelector(`[data-id="${id}"] .btn`);
}

afterEach(() => { document.body.innerHTML = ''; });

describe('authoritative weapon cooldowns through Welcome and the shared Tactical renderer', () => {
  it.each([false, true])('moves both bars through server fractions in a keyed=%s console without granting fire', keyed => {
    const state = new ClientSimState();
    welcome(state, { phasers: [config('fore', 6)], blasters: [config('fore', 8)] });
    const render = mount(keyed);
    for (const [fraction, width] of [[1, '100%'], [0.5, '50%'], [0.25, '25%'], [0, '0%']]) {
      weapons(state, [cooling('fore', 6 * fraction)], [cooling('fore', 8 * fraction)]);
      render(state);
      expect(fill('phasers', 'fore').style.width).toBe(width);
      expect(fill('blasters', 'fore').style.width).toBe(width);
      expect(fire('phasers', 'fore').disabled).toBe(true);
      expect(fire('blasters', 'fore').disabled).toBe(true);
    }
    const ready = { id: 'fore', fire_ready: true, on_cooldown: false, cooldown_remaining: 0,
      readiness: { ready: true, blocking_reason: 'Ready' } };
    weapons(state, [ready], [ready]);
    render(state);
    expect(fire('phasers', 'fore').disabled).toBe(false);
    expect(fire('blasters', 'fore').disabled).toBe(false);
  });

  it('joins reordered banks by id, keeps weapon-family durations separate and leaves the source states untouched', () => {
    const state = new ClientSimState();
    welcome(state, { phasers: [config('aft', 12), config('fore', 6)], blasters: [config('fore', 8)] });
    const fore = Object.freeze(cooling('fore', 3));
    const aft = Object.freeze(cooling('aft', 3));
    const blaster = Object.freeze(cooling('fore', 2));
    weapons(state, [fore, aft], [blaster]);
    const render = mount(true);
    render(state);
    expect(fill('phasers', 'fore').style.width).toBe('50%');
    expect(fill('phasers', 'aft').style.width).toBe('25%');
    expect(fill('blasters', 'fore').style.width).toBe('25%');
    expect(fore).not.toHaveProperty('cooldown_secs');
    expect(blaster).not.toHaveProperty('cooldown_secs');
    weapons(state, [aft, fore], [blaster]);
    render(state);
    expect(fill('phasers', 'fore').style.width).toBe('50%');
    expect(fill('phasers', 'aft').style.width).toBe('25%');
  });

  it('uses the new Welcome durations after reconnect or a hull change, and clears missing config', () => {
    const state = new ClientSimState();
    const render = mount(true);
    welcome(state, { phasers: [config('fore', 6)], blasters: [config('fore', 8)] });
    weapons(state, [cooling('fore', 3)], [cooling('fore', 2)]);
    render(state);
    expect(fill('phasers', 'fore').style.width).toBe('50%');
    expect(fill('blasters', 'fore').style.width).toBe('25%');

    welcome(state, { phasers: [config('fore', 12)], blasters: [config('fore', 4)], hull: 'replacement' });
    weapons(state, [cooling('fore', 3)], [cooling('fore', 2)]);
    render(state);
    expect(fill('phasers', 'fore').style.width).toBe('25%');
    expect(fill('blasters', 'fore').style.width).toBe('50%');

    welcome(state);
    expect(state.phaserArcConfigs).toEqual([]);
    expect(state.blasterBankConfigs).toEqual([]);
    weapons(state, [cooling('fore', 3)], [cooling('fore', 2)]);
    render(state);
    expect(fill('phasers', 'fore').style.width).toBe('100%');
    expect(fill('blasters', 'fore').style.width).toBe('100%');
    state.reset();
    expect(state.blasterBankConfigs).toEqual([]);
  });

  it('uses the typed Weapons blackboard duration without mutating its bank state', () => {
    const state = new ClientSimState();
    const bank = Object.freeze(cooling('fore', 1.5));
    state.apply({ type: 'BlackboardUpdate', data: { updates: [
      ['weapons', { kind: 'Weapons', data: {
        banks: [bank], phaser_arcs: [config('fore', 6)], phaser_mode: 'Manual',
      } }],
    ] } });
    const render = mount();
    render(state);
    expect(fill('phasers', 'fore').style.width).toBe('25%');
    expect(bank).not.toHaveProperty('cooldown_secs');
  });
});

describe('cooldown display bounds', () => {
  it('clamps remaining time to the authored interval', () => {
    expect(cooldownRemainingPercent({ cooldown_secs: 6, cooldown_remaining: 9 })).toBe(100);
    expect(cooldownRemainingPercent({ cooldown_secs: 6, cooldown_remaining: -1 })).toBe(0);
  });

  it.each([undefined, 0, -1, NaN, Infinity, '6'])('keeps duration %s unknown instead of inferring one', duration => {
    const bank = cooling('fore', 3);
    expect(withWeaponCooldowns([bank], [config('fore', duration)])[0]).toBe(bank);
    expect(cooldownRemainingPercent({ ...bank, cooldown_secs: duration })).toBeNull();
  });

  it.each([undefined, NaN, Infinity, '3'])('keeps remaining time %s unknown instead of emitting NaN', remaining => {
    expect(cooldownRemainingPercent({ cooldown_secs: 6, cooldown_remaining: remaining })).toBeNull();
  });
});
