import { describe, it, expect } from 'vitest';
import { REGISTRY } from '../../gui/console-registry.js';

describe('REGISTRY', () => {
  it('contains exactly the twelve HTML-panel consoles', () => {
    expect(Object.keys(REGISTRY).sort()).toEqual([
      'captain', 'comms', 'engineering', 'helm', 'navigation', 'pilot', 'power', 'repair', 'science', 'sensors', 'shields', 'tactical',
    ]);
  });

  it('is frozen', () => {
    expect(Object.isFrozen(REGISTRY)).toBe(true);
  });

  it.each([
    ['captain',     'captain-ui',     'captain-iframe'],
    ['pilot',       'pilot-ui',       'pilot-iframe'],
    ['helm',        'helm-ui',        'helm-iframe'],
    ['tactical',    'weapons-ui',     'weapons-iframe'],
    ['repair',      'repair-ui',      'repair-iframe'],
    ['power',       'power-ui',       'power-iframe'],
    ['shields',     'shields-ui',     'shields-iframe'],
    ['sensors',     'sensors-ui',     'sensors-iframe'],
    ['science',     'science-ui',     'science-iframe'],
    ['comms',       'comms-ui',       'comms-iframe'],
    ['navigation',  'navigation-ui',  'navigation-iframe'],
    ['engineering', 'engineering-ui', 'engineering-iframe'],
  ])('%s → sectionId=%s, iframeId=%s', (name, sectionId, iframeId) => {
    expect(REGISTRY[name].sectionId).toBe(sectionId);
    expect(REGISTRY[name].iframeId).toBe(iframeId);
  });
});
