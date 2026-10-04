import { describe, it, expect } from 'vitest';
import { stationRatingStars, stationRatingLabel } from '../../gui/station-rating.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { buildSystemStationConsoleState } from '../../gui/console-state.js';

describe('authored station stars', () => {
  it('carries summary authority through the real composed Engineering payload', () => {
    const payload = buildSystemStationConsoleState('engineering', {
      stationSystems: { engineering: ['repair'] },
      systemConsoleFamilies: { repair: 'repair' },
      controlSources: { repair: 'Simplified' },
      blackboardKinds: { repair: 'Repair' },
      blackboards: { repair: {} },
    });
    expect(payload.systems.repair.repair_auto).toBe(true);
    expect(payload.systems.repair.repair_summary).toBe(true);
  });
  it('uses authored order and length, retaining names and the legacy default', () => {
    const ratings = ['Std', 'Guided', 'Simplified'];
    expect(ratings.map(name => stationRatingStars(ratings, name))).toEqual([3, 2, 1]);
    expect(stationRatingLabel(ratings, 'Guided')).toBe('★★ Guided');
    expect(stationRatingStars(['Std'], 'Std')).toBe(1);
    expect(stationRatingStars(ratings, 'Backfill')).toBe(0);
    expect(stationRatingStars(ratings, 'unknown')).toBeNull();
    expect(ratings[0]).toBe('Std');
  });

  it('projects depths as host state and clears removed systems on the next snapshot', () => {
    const state = new ClientSimState();
    const snapshot = { entity_states: [], system_depths: { repair: 'Simplified' }, control_sources: { repair: 'Simplified' } };
    state.apply({ type: 'SimState', data: { snapshot } });
    expect(state.systemDepths).toEqual({ repair: 'Simplified' });
    state.reset({ preserveAuthorityProjection: true });
    expect(state.systemDepths).toEqual({ repair: 'Simplified' });
    state.apply({ type: 'SimState', data: { snapshot: { entity_states: [] } } });
    expect(state.systemDepths).toEqual({});
    state.systemDepths = { repair: 'Detailed' };
    state.reset();
    expect(state.systemDepths).toEqual({});
  });
});
