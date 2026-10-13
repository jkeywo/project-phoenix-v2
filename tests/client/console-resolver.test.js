import { describe, it, expect } from 'vitest';
import { resolveConsoleUrl } from '../../gui/console-resolver.js';

const CRUISER_STATIONS = {
  consoles: [
    { id: 'captain', name: 'Captain', screen: 'gui/cruiser/captain.html' },
    { id: 'helm', name: 'Helm', screen: 'gui/cruiser/helm.html' },
    { id: 'tactical', name: 'Tactical', screen: 'gui/cruiser/tactical.html' },
    { id: 'science', name: 'Science', screen: 'gui/cruiser/science.html' },
    { id: 'engineering', name: 'Engineering', screen: 'gui/cruiser/engineering.html' },
    { id: 'comms', name: 'Comms', screen: 'gui/cruiser/comms.html' },
  ],
};

const DESTROYER_STATIONS = {
  consoles: [
    { id: 'captain', name: 'Captain', screen: 'gui/destroyer/captain.html' },
    { id: 'helm', name: 'Helm', screen: 'gui/destroyer/helm.html' },
    { id: 'tactical', name: 'Tactical', screen: 'gui/destroyer/tactical.html' },
    { id: 'engineering', name: 'Engineering', screen: 'gui/destroyer/engineering.html' },
  ],
};

const BATTLESHIP_STATIONS = {
  consoles: [
    { id: 'captain', name: 'Captain', screen: 'gui/battleship/captain.html' },
    { id: 'helm', name: 'Helm', screen: 'gui/battleship/helm.html' },
    { id: 'tactical', name: 'Tactical', screen: 'gui/battleship/tactical.html' },
    { id: 'repair', name: 'Repair', screen: 'gui/battleship/repair.html' },
    { id: 'sensors', name: 'Sensors', screen: 'gui/battleship/sensors.html' },
    { id: 'shields', name: 'Shields', screen: 'gui/battleship/shields.html' },
    { id: 'navigation', name: 'Navigation', screen: 'gui/battleship/navigation.html' },
    { id: 'power', name: 'Power', screen: 'gui/battleship/power.html' },
    { id: 'comms', name: 'Comms', screen: 'gui/battleship/comms.html' },
  ],
};

describe('resolveConsoleUrl', () => {
  it('returns station console path when station is found', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'science')).toBe('gui/cruiser/science.html');
  });

  it('returns same path for repeated lookups', () => {
    const shared = resolveConsoleUrl(CRUISER_STATIONS, 'captain');
    expect(shared).toBe('gui/cruiser/captain.html');
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'captain')).toBe(shared);
  });

  it('returns null when shipStations has no matching station', () => {
    expect(resolveConsoleUrl({ consoles: [] }, 'tactical')).toBe(null);
  });

  it('returns null when shipStations is null', () => {
    expect(resolveConsoleUrl(null, 'helm')).toBe(null);
  });

  it('returns null when station has no console field', () => {
    const noConsole = { consoles: [{ id: 'test', name: 'Test' }] };
    expect(resolveConsoleUrl(noConsole, 'test')).toBe(null);
  });
});

describe('resolveConsoleUrl - Cruiser', () => {
  it('CRUISER_STATIONS has exactly 6 stations', () => {
    expect(CRUISER_STATIONS.consoles).toHaveLength(6);
  });

  it('CRUISER_STATIONS station ids are the expected six', () => {
    const ids = CRUISER_STATIONS.consoles.map(s => s.id).sort();
    expect(ids).toEqual(['captain', 'comms', 'engineering', 'helm', 'science', 'tactical']);
  });

  it('resolves all cruiser stations to bespoke pages', () => {
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'captain')).toBe('gui/cruiser/captain.html');
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'helm')).toBe('gui/cruiser/helm.html');
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'tactical')).toBe('gui/cruiser/tactical.html');
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'science')).toBe('gui/cruiser/science.html');
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'engineering')).toBe('gui/cruiser/engineering.html');
    expect(resolveConsoleUrl(CRUISER_STATIONS, 'comms')).toBe('gui/cruiser/comms.html');
  });
});

describe('resolveConsoleUrl - Destroyer', () => {
  it('DESTROYER_STATIONS has exactly 4 stations', () => {
    expect(DESTROYER_STATIONS.consoles).toHaveLength(4);
  });

  it('DESTROYER_STATIONS station ids are the expected four', () => {
    const ids = DESTROYER_STATIONS.consoles.map(s => s.id).sort();
    expect(ids).toEqual(['captain', 'engineering', 'helm', 'tactical']);
  });

  it('resolves all destroyer stations to bespoke pages', () => {
    expect(resolveConsoleUrl(DESTROYER_STATIONS, 'captain')).toBe('gui/destroyer/captain.html');
    expect(resolveConsoleUrl(DESTROYER_STATIONS, 'helm')).toBe('gui/destroyer/helm.html');
    expect(resolveConsoleUrl(DESTROYER_STATIONS, 'tactical')).toBe('gui/destroyer/tactical.html');
    expect(resolveConsoleUrl(DESTROYER_STATIONS, 'engineering')).toBe('gui/destroyer/engineering.html');
  });

  it('returns null for a station destroyer does not have', () => {
    expect(resolveConsoleUrl(DESTROYER_STATIONS, 'science')).toBe(null);
  });
});

describe('resolveConsoleUrl - Battleship', () => {
  it('BATTLESHIP_STATIONS has exactly 9 stations', () => {
    expect(BATTLESHIP_STATIONS.consoles).toHaveLength(9);
  });

  it('BATTLESHIP_STATIONS station ids are the expected nine', () => {
    const ids = BATTLESHIP_STATIONS.consoles.map(s => s.id).sort();
    expect(ids).toEqual(['captain', 'comms', 'helm', 'navigation', 'power', 'repair', 'sensors', 'shields', 'tactical']);
  });

  it('resolves all battleship stations to ship-specific pages', () => {
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'captain')).toBe('gui/battleship/captain.html');
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'helm')).toBe('gui/battleship/helm.html');
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'tactical')).toBe('gui/battleship/tactical.html');
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'repair')).toBe('gui/battleship/repair.html');
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'sensors')).toBe('gui/battleship/sensors.html');
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'shields')).toBe('gui/battleship/shields.html');
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'navigation')).toBe('gui/battleship/navigation.html');
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'power')).toBe('gui/battleship/power.html');
    expect(resolveConsoleUrl(BATTLESHIP_STATIONS, 'comms')).toBe('gui/battleship/comms.html');
  });
});
