import { describe, it, expect } from 'vitest';
import { validateFile } from '../validation.js';

describe('validateFile', () => {
  it('returns empty results for a valid world file', () => {
    const result = validateFile('assets/worlds/default.toml', {
      global: { seed: 42 },
      anchors: { start: [0.0, 0.0, 0.0] },
    });
    expect(result).toEqual([]);
  });

  it('returns error for world file missing global section', () => {
    const result = validateFile('assets/worlds/bad.toml', {
      anchors: { start: [0.0, 0.0, 0.0] },
    });
    expect(result.length).toBeGreaterThan(0);
    expect(result.some((r) => r.path === 'global')).toBe(true);
  });

  it('returns empty results for a valid entity file', () => {
    const result = validateFile('assets/entities/ship.toml', {
      tags: ['ship'],
      hull: { hull_integrity: 100 },
    });
    expect(result).toEqual([]);
  });

  it('returns error for entity file missing tags', () => {
    const result = validateFile('assets/entities/bad.toml', {
      hull: { hull_integrity: 100 },
    });
    expect(result.some((r) => r.path === 'tags')).toBe(true);
  });

  it('returns error for entity with duplicate station names', () => {
    const result = validateFile('assets/entities/ship.toml', {
      tags: ['ship'],
      stations: {
        min_players: 1,
        max_players: 1,
        1: [
          { name: 'Alpha', consoles: ['Helm'] },
          { name: 'Alpha', consoles: ['Tactical'] },
        ],
      },
    });
    expect(result.some((r) => r.path.includes('name') && r.severity === 'error')).toBe(true);
  });

  it('returns error for entity station with empty consoles', () => {
    const result = validateFile('assets/entities/ship.toml', {
      tags: ['ship'],
      stations: {
        min_players: 1,
        max_players: 1,
        1: [{ name: 'Alpha', consoles: [] }],
      },
    });
    expect(result.some((r) => r.path.includes('consoles') && r.severity === 'error')).toBe(true);
  });

  it('returns warning for entity station with unknown console name', () => {
    const result = validateFile('assets/entities/ship.toml', {
      tags: ['ship'],
      stations: {
        min_players: 1,
        max_players: 1,
        1: [{ name: 'Alpha', consoles: ['Helm', 'Hyperdrive'] }],
      },
    });
    const consoleErrors = result.filter((r) => r.path.includes('consoles'));
    expect(consoleErrors.length).toBeGreaterThan(0);
    expect(consoleErrors[0].severity).toBe('warning');
  });

  it('returns no station errors for entity with valid stations', () => {
    const result = validateFile('assets/entities/ship.toml', {
      tags: ['ship'],
      stations: {
        min_players: 1,
        max_players: 1,
        1: [{ name: 'Alpha', consoles: ['Helm'] }],
      },
    });
    const stationErrors = result.filter((r) => r.path.startsWith('stations.'));
    expect(stationErrors).toHaveLength(0);
  });

  it('returns errors for a world missing both global and anchors', () => {
    const result = validateFile('assets/worlds/bad.toml', { bad_field: 1 });
    const paths = result.map((r) => r.path).sort();
    expect(paths).toEqual(['anchors', 'global']);
    expect(result.every((r) => r.severity === 'error')).toBe(true);
  });

  it('returns empty list for a clean valid world', () => {
    const result = validateFile('assets/worlds/clean.toml', {
      global: { seed: 7 },
      anchors: { home: [0.0, 0.0, 0.0] },
    });
    expect(result).toEqual([]);
  });

  it('returns empty list for file matching neither entity nor world heuristics', () => {
    const result = validateFile('data/config.json', { version: 1 });
    expect(result).toEqual([]);
  });

  it('returns error for entity with effects but no shape section', () => {
    const result = validateFile('assets/entities/bad_region.toml', {
      tags: ['region'],
      effects: { comms_jammed: {} },
    });
    expect(result.some((r) => r.path === 'shape' && r.severity === 'error')).toBe(true);
  });

  it('returns error for entity with behaviour but no states', () => {
    const result = validateFile('assets/entities/bad_npc.toml', {
      tags: ['ship', 'npc'],
      behaviour: { initial_state: 'patrol' },
    });
    expect(result.some((r) => r.path.startsWith('behaviour') && r.severity === 'error')).toBe(true);
  });

  it('returns exactly 2 errors for a world with neither global nor anchors', () => {
    const result = validateFile('assets/worlds/bad.toml', { bad_field: 1 });
    expect(result).toHaveLength(2);
    const paths = result.map((r) => r.path).sort();
    expect(paths).toEqual(['anchors', 'global']);
  });
});
