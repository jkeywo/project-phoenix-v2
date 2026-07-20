import { describe, it, expect } from 'vitest';
import {
  planMounts,
  sectionIdFor,
  iframeIdFor,
  SECTION_ALIAS,
} from '../../gui/mount-plan.js';

describe('sectionIdFor / iframeIdFor — canonical scheme', () => {
  it('derives `${id}-ui` / `${id}-iframe` for regular station ids', () => {
    for (const id of ['captain', 'helm', 'repair', 'sensors', 'science',
                      'shields', 'navigation', 'power', 'comms',
                      'engineering', 'pilot']) {
      expect(sectionIdFor(id)).toBe(id + '-ui');
      expect(iframeIdFor(id)).toBe(id + '-iframe');
    }
  });

  it('tactical is the ONE alias: weapons-ui / weapons-iframe', () => {
    expect(sectionIdFor('tactical')).toBe('weapons-ui');
    expect(iframeIdFor('tactical')).toBe('weapons-iframe');
  });

  it('the alias table contains exactly the tactical entry', () => {
    expect(SECTION_ALIAS).toEqual({ tactical: 'weapons' });
    expect(Object.isFrozen(SECTION_ALIAS)).toBe(true);
  });

  it('unknown station ids fall back to the canonical scheme', () => {
    expect(sectionIdFor('astrometrics')).toBe('astrometrics-ui');
    expect(iframeIdFor('astrometrics')).toBe('astrometrics-iframe');
  });

  it('returns null for empty / null ids', () => {
    expect(sectionIdFor(null)).toBeNull();
    expect(sectionIdFor('')).toBeNull();
    expect(iframeIdFor(undefined)).toBeNull();
  });
});

describe('planMounts', () => {
  const shipStations = {
    stations: [
      { id: 'captain', name: 'Captain', console: 'gui/battleship/captain.html' },
      { id: 'tactical', name: 'Tactical', console: 'gui/battleship/tactical.html' },
      { id: 'helm', name: 'Helm', console: 'gui/battleship/helm.html' },
    ],
  };

  it('produces one entry per station with resolved ids and URL', () => {
    const plan = planMounts(shipStations);
    expect(plan).toHaveLength(3);
    expect(plan[0]).toEqual({
      stationId: 'captain',
      sectionId: 'captain-ui',
      iframeId: 'captain-iframe',
      url: 'gui/battleship/captain.html',
      title: 'Captain',
    });
  });

  it('applies the tactical alias in the plan', () => {
    const plan = planMounts(shipStations);
    const tac = plan.find(m => m.stationId === 'tactical');
    expect(tac.sectionId).toBe('weapons-ui');
    expect(tac.iframeId).toBe('weapons-iframe');
    expect(tac.url).toBe('gui/battleship/tactical.html');
  });

  it('skips stations with no console URL (nothing to mount)', () => {
    const plan = planMounts({
      stations: [
        { id: 'captain', console: 'gui/battleship/captain.html' },
        { id: 'ghost' },
      ],
    });
    expect(plan.map(m => m.stationId)).toEqual(['captain']);
  });

  it('skips stations with no id', () => {
    const plan = planMounts({
      stations: [{ name: 'anonymous', console: 'x.html' }],
    });
    expect(plan).toEqual([]);
  });

  it('unknown station ids mount via the canonical scheme (no registry gate)', () => {
    const plan = planMounts({
      stations: [{ id: 'astrometrics', console: 'gui/x/astrometrics.html' }],
    });
    expect(plan[0].sectionId).toBe('astrometrics-ui');
    expect(plan[0].iframeId).toBe('astrometrics-iframe');
    expect(plan[0].title).toBe('astrometrics');
  });

  it('is empty for null / missing ship stations', () => {
    expect(planMounts(null)).toEqual([]);
    expect(planMounts({})).toEqual([]);
  });
});
