import { describe, it, expect } from 'vitest';
import { REGISTRY } from '../../gui/console-registry.js';

describe('REGISTRY', () => {
  it('contains exactly the eight HTML-panel consoles', () => {
    expect(Object.keys(REGISTRY).sort()).toEqual([
      'CaptainChair', 'Comms', 'Helm', 'Power', 'Repair', 'Sensors', 'Shields', 'Tactical',
    ]);
  });

  it('does not contain Navigation (Bevy-rendered)', () => {
    expect(REGISTRY.Navigation).toBeUndefined();
  });

  it('is frozen', () => {
    expect(Object.isFrozen(REGISTRY)).toBe(true);
  });

  it.each([
    ['CaptainChair', 'captain-ui',  'captain-iframe'],
    ['Helm',         'helm-ui',     'helm-iframe'],
    ['Tactical',     'weapons-ui',  'weapons-iframe'],
    ['Repair',       'repair-ui',   'repair-iframe'],
    ['Power',        'power-ui',    'power-iframe'],
    ['Shields',      'shields-ui',  'shields-iframe'],
    ['Sensors',      'sensors-ui',  'sensors-iframe'],
    ['Comms',        'comms-ui',    'comms-iframe'],
  ])('%s → sectionId=%s, iframeId=%s', (name, sectionId, iframeId) => {
    expect(REGISTRY[name].sectionId).toBe(sectionId);
    expect(REGISTRY[name].iframeId).toBe(iframeId);
  });
});
