import { describe, expect, it } from 'vitest';
import { buildNavigationConsoleState } from '../../gui/console-state.js';
import { consoleReadState } from '../../gui/console-topology.js';

function fixture() {
  return { blackboardKinds: { 'custom-nav': 'Navigation' }, blackboards: { 'custom-nav': {
    sharing: { owner: 'custom-nav', share_own_faction: true, receive: true, can_operate: true },
    received_chart: [{ provenance: { sample_id: 'held-choice', observed_tick: 3 }, position: [4, 5], kind: { kind: 'waypoint' } }],
  } }, controlSources: { 'custom-nav': 'Human' } };
}

describe('Navigation sharing publication joins', () => {
  it('projects exact owned controls and fixed received points without changing own course', () => {
    const state = fixture(); state.navigationWaypoint = { x: 20, z: 30 };
    const payload = buildNavigationConsoleState(state, ['custom-nav']);
    expect(payload.radar_sharing).toEqual([expect.objectContaining({ owner: 'custom-nav', role: 'navigation', controllable: true })]);
    expect(payload.received_chart).toBe(state.blackboards['custom-nav'].received_chart);
    expect(payload.waypoint).toEqual({ x: 20, z: 30 });
  });
  it('keeps explicitly received foreign Navigation read-only and does not pool cohosted boards', () => {
    const source = fixture();
    const foreign = consoleReadState(source, { id: 'foreign', systems: [], feeds: [{ source: 'custom-nav', kind: 'Navigation' }] });
    expect(buildNavigationConsoleState(foreign, []).radar_sharing).toEqual([expect.objectContaining({ controllable: false })]);
    const isolated = consoleReadState(source, { id: 'isolated', systems: [], feeds: [] });
    expect(buildNavigationConsoleState(isolated).received_chart).toEqual([]);
    expect(buildNavigationConsoleState(isolated).radar_sharing).toEqual([]);
  });
  it('revokes control on delegation and refuses mismatched producer intent', () => {
    const state = fixture(); state.controlSources['custom-nav'] = 'Ai';
    expect(buildNavigationConsoleState(state, ['custom-nav']).radar_sharing[0].controllable).toBe(false);
    state.blackboards['custom-nav'].sharing.owner = 'another';
    expect(buildNavigationConsoleState(state, ['custom-nav']).radar_sharing).toEqual([]);
  });
});
