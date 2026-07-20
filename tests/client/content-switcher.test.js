import { describe, it, expect } from 'vitest';
import { consoleSections, sectionForConsole } from '../../gui/content-switcher.js';

// Post issue #827 the switcher derives its section set from the ship's own
// station roster (uiState.shipStations) via the gui/mount-plan.js naming
// scheme — the hardcoded console-registry list is gone.

const BATTLESHIP = ['captain', 'helm', 'tactical', 'repair', 'sensors',
                    'shields', 'navigation', 'power', 'comms'];

describe('sectionForConsole', () => {
  it('derives `${id}-ui` for regular station ids', () => {
    expect(sectionForConsole('captain')).toBe('captain-ui');
    expect(sectionForConsole('helm')).toBe('helm-ui');
    expect(sectionForConsole('engineering')).toBe('engineering-ui');
    expect(sectionForConsole('pilot')).toBe('pilot-ui');
    expect(sectionForConsole('science')).toBe('science-ui');
  });

  it('applies the tactical → weapons-ui alias', () => {
    expect(sectionForConsole('tactical')).toBe('weapons-ui');
  });

  it('returns null for empty / null / undefined', () => {
    expect(sectionForConsole('')).toBeNull();
    expect(sectionForConsole(null)).toBeNull();
    expect(sectionForConsole(undefined)).toBeNull();
  });
});

describe('consoleSections', () => {
  it('covers exactly the ship stations, all false in the lobby', () => {
    const out = consoleSections('captain', false, BATTLESHIP);
    expect(Object.keys(out).sort()).toEqual([
      'captain-ui', 'comms-ui', 'helm-ui', 'navigation-ui', 'power-ui',
      'repair-ui', 'sensors-ui', 'shields-ui', 'weapons-ui',
    ]);
    expect(Object.values(out).every(v => v === false)).toBe(true);
  });

  it('all false when active console is null', () => {
    const out = consoleSections(null, true, BATTLESHIP);
    expect(Object.values(out).every(v => v === false)).toBe(true);
  });

  it('shows only the active console section in-game', () => {
    const out = consoleSections('helm', true, BATTLESHIP);
    expect(out['helm-ui']).toBe(true);
    expect(Object.entries(out).filter(([, v]) => v)).toHaveLength(1);
  });

  it('shows weapons-ui for tactical (the alias)', () => {
    const out = consoleSections('tactical', true, BATTLESHIP);
    expect(out['weapons-ui']).toBe(true);
    expect(Object.entries(out).filter(([, v]) => v)).toHaveLength(1);
  });

  it('works for cruiser/destroyer aggregate stations', () => {
    const out = consoleSections('engineering', true, ['captain', 'engineering', 'science']);
    expect(out).toEqual({
      'captain-ui': false, 'engineering-ui': true, 'science-ui': false,
    });
  });

  it('an active console not in the station list shows nothing', () => {
    const out = consoleSections('navigation', true, ['captain', 'helm']);
    expect(Object.values(out).every(v => v === false)).toBe(true);
  });

  it('is empty for an empty / missing station list', () => {
    expect(consoleSections('helm', true, [])).toEqual({});
    expect(consoleSections('helm', true, null)).toEqual({});
  });
});
