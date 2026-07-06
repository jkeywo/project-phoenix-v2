import { describe, it, expect } from 'vitest';
import { resolveConsoleUrl } from '../../gui/console-resolver.js';

const CRUISER_STATIONS = {
  stations: [
    { id: 'captain', name: 'Captain', console: 'gui/captain-console.html' },
    { id: 'helm', name: 'Helm', console: 'gui/helm-console.html' },
    { id: 'science', name: 'Science', console: 'gui/science-console.html' },
    { id: 'engineering', name: 'Engineering', console: 'gui/engineering-console.html' },
  ],
};

describe('resolveConsoleUrl', () => {
  it('returns station console path when station is found', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'science')).toBe('gui/science-console.html');
  });

  it('returns fallback when station not in shipStations', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'tactical')).toBe('gui/tactical-console.html');
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
});
