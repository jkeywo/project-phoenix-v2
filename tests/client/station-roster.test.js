import { describe, it, expect } from 'vitest';
import { buildStationRoster } from '../../gui/station-roster.js';

const DEFS = [
  { id: 'captain', name: 'Captain', short_code: 'CPT', rank: 'Commander', ratings: ['Std'] },
  { id: 'helm', name: 'Helm', short_code: 'HLM', rank: 'Lt', ratings: ['Std', 'Simplified'] },
];

describe('buildStationRoster', () => {
  it('maps station defs to roster rows with empty holders', () => {
    const out = buildStationRoster([], DEFS);
    expect(out.maxPlayers).toBe(2);
    expect(out.stations).toEqual([
      { id: 'captain', name: 'Captain', short_code: 'CPT', rank: 'Commander',
        holder_name: null, holder_token: null, ratings: ['Std'] },
      { id: 'helm', name: 'Helm', short_code: 'HLM', rank: 'Lt',
        holder_name: null, holder_token: null, ratings: ['Std', 'Simplified'] },
    ]);
    expect(out.allFilled).toBe(false);
  });

  it('resolves the holder from a string station id', () => {
    const players = [{ token: 't1', name: 'Ada', connected: true, ready: false, station: 'helm' }];
    const out = buildStationRoster(players, DEFS);
    const helm = out.stations.find(st => st.id === 'helm');
    expect(helm.holder_name).toBe('Ada');
    expect(helm.holder_token).toBe('t1');
  });

  it('resolves the holder from an object station { id }', () => {
    const players = [{ token: 't1', name: 'Ada', connected: true, station: { id: 'captain' } }];
    const out = buildStationRoster(players, DEFS);
    expect(out.stations.find(st => st.id === 'captain').holder_token).toBe('t1');
  });

  it('ignores disconnected holders', () => {
    const players = [{ token: 't1', name: 'Ada', connected: false, station: 'helm' }];
    const out = buildStationRoster(players, DEFS);
    expect(out.stations.find(st => st.id === 'helm').holder_name).toBeNull();
  });

  it('allFilled is true only when every station has a holder', () => {
    const players = [
      { token: 't1', name: 'Ada', connected: true, ready: false, station: 'helm' },
      { token: 't2', name: 'Bob', connected: true, ready: false, station: 'captain' },
    ];
    expect(buildStationRoster(players, DEFS).allFilled).toBe(true);
    expect(buildStationRoster(players.slice(0, 1), DEFS).allFilled).toBe(false);
  });

  it('allFilled is false with zero stations', () => {
    expect(buildStationRoster([], []).allFilled).toBe(false);
  });

  it('allReady is true when every player is ready, and vacuously with no players', () => {
    const ready = [{ token: 't1', name: 'Ada', connected: true, ready: true, station: 'helm' }];
    expect(buildStationRoster(ready, DEFS).allReady).toBe(true);
    expect(buildStationRoster([], DEFS).allReady).toBe(true);
    const mixed = [...ready, { token: 't2', name: 'Bob', connected: true, ready: false }];
    expect(buildStationRoster(mixed, DEFS).allReady).toBe(false);
  });

  it('defaults missing def fields to empty strings / arrays', () => {
    const out = buildStationRoster([], [{ id: 'x' }]);
    expect(out.stations[0]).toEqual({
      id: 'x', name: '', short_code: '', rank: '',
      holder_name: null, holder_token: null, ratings: [],
    });
  });
});
