import { describe, expect, it } from 'vitest';
import { buildSensorsConsoleState, buildWeaponsConsoleState } from '../../gui/console-state.js';
import { consoleReadState } from '../../gui/console-topology.js';

function state() {
  return { blackboardKinds: { sensor: 'SensorRadar', tactical: 'TacticalRadar' },
    blackboards: {
      sensor: { tracks: [], sharing: { owner: 'sensor', share_own_faction: true, receive: true, received_samples: 2, can_operate: true } },
      tactical: { tracks: [], sharing: { owner: 'tactical', share_own_faction: false, receive: true, received_samples: 0, can_operate: true } },
    }, controlSources: { sensor: 'Human', tactical: 'Human' } };
}

describe('deliberate feed control projections', () => {
  it('exposes exact current owners and keeps received typed feeds read-only', () => {
    const source = state();
    const own = buildSensorsConsoleState(source, ['sensor']).radar_sharing;
    expect(own).toEqual([expect.objectContaining({ owner: 'sensor', role: 'sensor_radar', controllable: true, share_own_faction: true })]);
    const fed = consoleReadState(source, { id: 'weapons', systems: ['tactical'], feeds: [{ source: 'sensor', kind: 'SensorRadar' }] });
    expect(buildSensorsConsoleState(fed, ['tactical']).radar_sharing).toEqual([
      expect.objectContaining({ owner: 'sensor', controllable: false }),
    ]);
    expect(buildWeaponsConsoleState(fed, ['tactical']).radar_sharing).toEqual([
      expect.objectContaining({ owner: 'tactical', role: 'tactical_radar', controllable: true }),
    ]);
  });
  it('cannot pool cohosted unpermitted feeds or substitute another owner in the payload', () => {
    const source = state();
    const isolated = consoleReadState(source, { id: 'science', systems: ['sensor'], feeds: [] });
    expect(buildWeaponsConsoleState(isolated, ['sensor']).radar_sharing).toEqual([]);
    source.blackboards.sensor.sharing.owner = 'tactical';
    expect(buildSensorsConsoleState(source, ['sensor']).radar_sharing).toEqual([]);
  });
  it('handoff to ordinary AI removes human control while preserving the shared intent display', () => {
    const source = state(); source.controlSources.sensor = 'Ai';
    const rows = buildSensorsConsoleState(source, ['sensor']).radar_sharing;
    expect(rows).toEqual([expect.objectContaining({ owner: 'sensor', share_own_faction: true, controllable: false })]);
  });
});
