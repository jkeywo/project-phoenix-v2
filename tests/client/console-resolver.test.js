import { describe, it, expect } from 'vitest';
import { resolveConsoleUrl } from '../../gui/console-resolver.js';

const CRUISER_STATIONS = {
  stations: [
    { id: 'captain',     name: 'Captain',     console: 'gui/captain-console.html' },
    { id: 'helm',        name: 'Helm',        console: 'gui/helm-console.html' },
    { id: 'tactical',    name: 'Tactical',    console: 'gui/tactical-console.html' },
    { id: 'science',     name: 'Science',     console: 'gui/science-console.html' },
    { id: 'engineering', name: 'Engineering', console: 'gui/engineering-console.html' },
    { id: 'comms',       name: 'Comms',       console: 'gui/cruiser-comms-console.html' },
  ],
};

describe('resolveConsoleUrl', () => {
  it('returns station console path when station is found', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'science')).toBe('gui/science-console.html');
  });

  it('returns fallback when station not in shipStations', () => {
    expect(resolveConsoleUrl({ stations: [] }, 'tactical')).toBe('gui/tactical-console.html');
  });

  it('returns same path for shared console strings', () => {
    const shared = resolveConsoleUrl(CRUISER_STATIONS, 'captain');
    expect(shared).toBe('gui/captain-console.html');
    const same = resolveConsoleUrl(CRUISER_STATIONS, 'captain');
    expect(same).toBe(shared);
  });

  it('returns fallback when shipStations has no stations', () => {
    expect(resolveConsoleUrl({ stations: [] }, 'science')).toBe('gui/science-console.html');
  });

  it('returns fallback when shipStations is null', () => {
    expect(resolveConsoleUrl(null, 'helm')).toBe('gui/helm-console.html');
  });

  it('returns fallback when station has no console field', () => {
    const noConsole = { stations: [{ id: 'test', name: 'Test' }] };
    expect(resolveConsoleUrl(noConsole, 'test')).toBe('gui/test-console.html');
  });

  // ── Cruiser-specific: all 6 stations resolve through CRUISER_STATIONS ────

  it('cruiser captain resolves to captain-console.html', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'captain')).toBe('gui/captain-console.html');
  });

  it('cruiser helm resolves to helm-console.html', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'helm')).toBe('gui/helm-console.html');
  });

  it('cruiser tactical resolves to tactical-console.html', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'tactical')).toBe('gui/tactical-console.html');
  });

  it('cruiser science resolves to science-console.html', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'science')).toBe('gui/science-console.html');
  });

  it('cruiser engineering resolves to bespoke engineering-console.html', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'engineering')).toBe('gui/engineering-console.html');
  });

  it('cruiser comms resolves to bespoke cruiser-comms-console.html', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'comms')).toBe('gui/cruiser-comms-console.html');
  });

  it('CRUISER_STATIONS has exactly 6 stations', () => {
    expect(CRUISER_STATIONS.stations).toHaveLength(6);
  });

  it('CRUISER_STATIONS station ids are the expected six', () => {
    const ids = CRUISER_STATIONS.stations.map(s => s.id).sort();
    expect(ids).toEqual(['captain', 'comms', 'engineering', 'helm', 'science', 'tactical']);
  });
});
