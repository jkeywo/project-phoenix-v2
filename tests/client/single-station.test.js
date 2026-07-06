import { describe, it, expect } from 'vitest';
import { shouldHideTabBar, singleStationIframeUrl } from '../../gui/single-station.js';

const SHIP_STATIONS = {
  stations: [
    { id: 'captain', console: 'gui/captain-console.html' },
    { id: 'helm', console: 'gui/helm-console.html' },
    { id: 'tactical', console: 'gui/tactical-console.html' },
  ],
};

describe('shouldHideTabBar', () => {
  it('returns true when in-game with an empty consoles list', () => {
    expect(shouldHideTabBar([], true)).toBe(true);
  });

  it('returns true when in-game with exactly one console (single-station mode)', () => {
    expect(shouldHideTabBar(['helm'], true)).toBe(true);
  });

  it('returns false when in-game with two or more consoles (multi-station)', () => {
    expect(shouldHideTabBar(['helm', 'tactical'], true)).toBe(false);
  });

  it('returns false when NOT in-game with one console (lobby shows bar)', () => {
    expect(shouldHideTabBar(['helm'], false)).toBe(false);
  });

  it('returns false when NOT in-game with no consoles', () => {
    expect(shouldHideTabBar([], false)).toBe(false);
  });

  it('returns true when in-game with null consoles (treated as empty)', () => {
    expect(shouldHideTabBar(null, true)).toBe(true);
  });

  it('returns true when in-game with undefined consoles', () => {
    expect(shouldHideTabBar(undefined, true)).toBe(true);
  });

  it('returns false when NOT in-game with null consoles', () => {
    expect(shouldHideTabBar(null, false)).toBe(false);
  });
});

describe('singleStationIframeUrl', () => {
  it('returns the console path when station is found with a console field', () => {
    expect(singleStationIframeUrl(SHIP_STATIONS, 'helm')).toBe('gui/helm-console.html');
  });

  it('returns the console path for captain', () => {
    expect(singleStationIframeUrl(SHIP_STATIONS, 'captain')).toBe('gui/captain-console.html');
  });

  it('returns fallback url when station is not in shipStations', () => {
    expect(singleStationIframeUrl(SHIP_STATIONS, 'repair')).toBe('gui/repair-console.html');
  });

  it('returns fallback url when station has no console field', () => {
    const noConsole = { stations: [{ id: 'sensors' }] };
    expect(singleStationIframeUrl(noConsole, 'sensors')).toBe('gui/sensors-console.html');
  });

  it('returns null when stationId is null', () => {
    expect(singleStationIframeUrl(SHIP_STATIONS, null)).toBeNull();
  });

  it('returns null when stationId is undefined', () => {
    expect(singleStationIframeUrl(SHIP_STATIONS, undefined)).toBeNull();
  });

  it('returns null when stationId is empty string', () => {
    expect(singleStationIframeUrl(SHIP_STATIONS, '')).toBeNull();
  });

  it('returns fallback url when shipStations is null', () => {
    expect(singleStationIframeUrl(null, 'helm')).toBe('gui/helm-console.html');
  });

  it('returns fallback url when shipStations has empty stations array', () => {
    expect(singleStationIframeUrl({ stations: [] }, 'tactical')).toBe('gui/tactical-console.html');
  });

  it('returns the console path for a station with a non-default console path', () => {
    const custom = { stations: [{ id: 'helm', console: 'gui/custom-helm.html' }] };
    expect(singleStationIframeUrl(custom, 'helm')).toBe('gui/custom-helm.html');
  });
});
