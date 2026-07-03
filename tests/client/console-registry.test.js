import { describe, it, expect } from 'vitest';
import { REGISTRY } from '../../gui/console-registry.js';

describe('REGISTRY', () => {
  it('contains exactly the nine HTML-panel consoles', () => {
    expect(Object.keys(REGISTRY).sort()).toEqual([
      'captain', 'comms', 'helm', 'navigation', 'power', 'repair', 'sensors', 'shields', 'tactical',
    ]);
  });

  it('is frozen', () => {
    expect(Object.isFrozen(REGISTRY)).toBe(true);
  });

  it.each([
    ['captain',    'captain-ui',    'captain-iframe'],
    ['helm',       'helm-ui',       'helm-iframe'],
    ['tactical',   'weapons-ui',    'weapons-iframe'],
    ['repair',     'repair-ui',     'repair-iframe'],
    ['power',      'power-ui',      'power-iframe'],
    ['shields',    'shields-ui',    'shields-iframe'],
    ['sensors',    'sensors-ui',    'sensors-iframe'],
    ['comms',      'comms-ui',      'comms-iframe'],
    ['navigation', 'navigation-ui', 'navigation-iframe'],
  ])('%s → sectionId=%s, iframeId=%s', (name, sectionId, iframeId) => {
    expect(REGISTRY[name].sectionId).toBe(sectionId);
    expect(REGISTRY[name].iframeId).toBe(iframeId);
  });
});
