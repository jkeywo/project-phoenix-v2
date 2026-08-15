/**
 * tests/client/scenario-pick.test.js — PRD #1023 module 4, user story 16:
 * "immediate pending feedback and a correct message when another phone wins
 * the pick, so that the stage change never feels arbitrary".
 *
 * The three states are the contract, and each of them exists because the
 * arbiter deliberately sends no acknowledgement to the phone that asked:
 * PENDING is the missing acknowledgement, WON is the acknowledgement arriving
 * as a broadcast, LOST is the acknowledgement arriving for someone else.
 */
import { describe, it, expect } from 'vitest';
import {
  pendingPick, settlePick, pickStage, scenarioPickView,
} from '../../gui/scenario-pick.js';

const nothingLocked = { scenario_id: null, template_path: null };

describe('settlePick — a request with no acknowledgement', () => {
  it('is idle when nothing was sent', () => {
    expect(settlePick(null, nothingLocked)).toEqual({ state: 'idle', pending: null, lostTo: null });
  });

  it('stays pending while the lock has not landed', () => {
    const p = pendingPick('scenario', 'combat_test');
    const out = settlePick(p, nothingLocked);
    expect(out.state).toBe('pending');
    expect(out.pending).toBe(p);
  });

  it('is won, and cleared, when the lock is my choice', () => {
    const out = settlePick(pendingPick('scenario', 'combat_test'), { scenario_id: 'combat_test', template_path: null });
    expect(out).toEqual({ state: 'won', pending: null, lostTo: null });
  });

  it('is lost, and names the winner, when the lock is somebody else\'s', () => {
    const out = settlePick(pendingPick('scenario', 'combat_test'), { scenario_id: 'patrol', template_path: null });
    expect(out).toEqual({ state: 'lost', pending: null, lostTo: 'patrol' });
  });

  it('settles a ship request against template_path, not scenario_id', () => {
    const p = pendingPick('ship', 'assets/entities/alliance_destroyer.toml');
    // Scenario locked, ship not yet — the ship request is still in flight.
    expect(settlePick(p, { scenario_id: 'combat_test', template_path: null }).state).toBe('pending');
    expect(settlePick(p, { scenario_id: 'combat_test', template_path: 'assets/entities/alliance_destroyer.toml' }).state).toBe('won');
    const lost = settlePick(p, { scenario_id: 'combat_test', template_path: 'assets/entities/alliance_cruiser.toml' });
    expect(lost.state).toBe('lost');
    expect(lost.lostTo).toBe('assets/entities/alliance_cruiser.toml');
  });
});

describe('pickStage', () => {
  it('walks scenario → ship → locked', () => {
    expect(pickStage(nothingLocked)).toBe('scenario');
    expect(pickStage({ scenario_id: 'combat_test', template_path: null })).toBe('ship');
    expect(pickStage({ scenario_id: 'combat_test', template_path: 'assets/entities/x.toml' })).toBe('locked');
  });
});

describe('scenarioPickView', () => {
  const catalog = [{ id: 'combat_test' }, { id: 'patrol' }];

  it('accepts taps with nothing in flight', () => {
    const vm = scenarioPickView({ catalog, locked: nothingLocked });
    expect(vm.stage).toBe('scenario');
    expect(vm.accepting).toBe(true);
    expect(vm.pendingId).toBeNull();
  });

  it('marks the tapped option and stops accepting while a pick is in flight', () => {
    const vm = scenarioPickView({
      catalog, locked: nothingLocked, pending: pendingPick('scenario', 'patrol'),
    });
    expect(vm.busy).toBe(true);
    expect(vm.accepting).toBe(false);
    expect(vm.pendingId).toBe('patrol');
  });

  it('does not let a scenario request grey out the ship stage', () => {
    // The scenario lock landed (so the scenario request is answered) but the
    // caller has not settled yet. The ship buttons must still be live.
    const vm = scenarioPickView({
      catalog,
      locked: { scenario_id: 'combat_test', template_path: null },
      pending: pendingPick('scenario', 'combat_test'),
    });
    expect(vm.stage).toBe('ship');
    expect(vm.busy).toBe(false);
    expect(vm.accepting).toBe(true);
  });

  // The empty-catalogue string was wrong, not missing: it told the player to
  // wait for the host to select a scenario, when the host had already
  // broadcast and the broadcast was empty.
  it('uses the empty-catalogue string, not the waiting-for-host one', () => {
    const vm = scenarioPickView({ catalog: [], locked: nothingLocked });
    expect(vm.emptyId).toBe('client.no_scenarios');
    expect(vm.emptyId).not.toBe('client.waiting_scenario');
  });

  it('has no empty note when the catalogue has entries', () => {
    expect(scenarioPickView({ catalog, locked: nothingLocked }).emptyId).toBeNull();
  });

  it('has no empty note past the scenario stage', () => {
    const vm = scenarioPickView({ catalog: [], locked: { scenario_id: 'combat_test', template_path: null } });
    expect(vm.emptyId).toBeNull();
  });

  it('carries the race notice with the winning choice named', () => {
    const vm = scenarioPickView({
      catalog,
      locked: { scenario_id: 'patrol', template_path: null },
      notice: { choice: 'Patrol' },
    });
    expect(vm.noticeId).toBe('client.pick_taken');
    expect(vm.noticeParams).toEqual({ choice: 'Patrol' });
  });

  it('has no notice when nothing lost a race', () => {
    const vm = scenarioPickView({ catalog, locked: nothingLocked });
    expect(vm.noticeId).toBeNull();
    expect(vm.noticeParams).toBeNull();
  });
});
