import { JSDOM } from 'jsdom';
import { createConsoleMounts, stationIdForSource } from '../../gui/console-mount.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { describe, it, expect } from 'vitest';
import { planMounts } from '../../gui/mount-plan.js';
import { consoleReadState } from '../../gui/console-topology.js';
import { heroBarModel } from '../../gui/hero-bar.js';
import { buildConsoleView } from '../../gui/console-state.js';

const consoles = [
  { id: 'left', name: 'Left', home_station: 'pilot', screen: 'gui/cruiser/helm.html', systems: ['drive'], feeds: [] },
  { id: 'right', name: 'Right', home_station: 'pilot', screen: 'gui/cruiser/helm.html', systems: ['scan'], feeds: [] },
];
describe('explicit Console identity', () => {
  it('mounts two distinct instances of the same screen on one Station', () => {
    const mounts = planMounts({ stations: [{ id: 'pilot' }], consoles });
    expect(mounts).toHaveLength(2);
    expect(mounts.map(m => m.stationId)).toEqual(['left', 'right']);
    expect(new Set(mounts.map(m => m.iframeId)).size).toBe(2);
    expect(mounts[0].url).toBe(mounts[1].url);
  });
  it('keeps the historical tactical DOM alias distinct from an authored weapons Console', () => {
    const mounts = planMounts({ consoles: [{ ...consoles[0], id: 'tactical' }, { ...consoles[1], id: 'weapons' }] });
    expect(new Set(mounts.map(m => m.iframeId)).size).toBe(2);
  });
  it('routes identical screen declarations by frame identity and keeps tab switches mounted', () => {
    const dom = new JSDOM('<main></main>');
    const roster = { stations: [{ id: 'pilot' }], consoles };
    const mounts = createConsoleMounts({ doc: dom.window.document, container: dom.window.document.querySelector('main') });
    try {
      mounts.mount(roster);
      const left = mounts.frame('left'), right = mounts.frame('right');
      expect(left).not.toBe(right);
      mounts.handleDeclaration({ source: right.contentWindow, data: { type: 'console_hull', console: 'helm', entries: ['right'] } });
      expect(mounts.view('right').hull).toEqual(['right']);
      expect(mounts.view('left').hull).toEqual([]);
      mounts.handleDeclaration({ source: left.contentWindow, data: { type: 'console_tabs', console: 'helm', tabs: [], open: 'left-overlay' } });
      mounts.handleDeclaration({ source: right.contentWindow, data: { type: 'console_tabs', console: 'helm', tabs: [], open: 'right-overlay' } });
      expect(mounts.view('left').activeOverlay).toBe('left-overlay');
      expect(mounts.view('right').activeOverlay).toBe('right-overlay');
      expect(stationIdForSource(dom.window.document, roster, right.contentWindow, 'helm')).toBe('right');
      mounts.show('right', true);
      expect(right.parentElement.className).toContain('active');
      expect(left.parentElement.className).not.toContain('active');
      mounts.show('left', true);
      expect(mounts.frame('right')).toBe(right);
    } finally { mounts.dispose(); dom.window.close(); }
  });
  it('retains host projection across reconnect and clears it at a round boundary', () => {
    const state = new ClientSimState();
    const welcome = { type: 'Welcome', data: { ship_stations: { consoles }, ship_config: {}, state: { phase: 'InProgress' } } };
    state.apply(welcome);
    expect(state.consoleDefinitions).toEqual(consoles);
    state.apply({ type: 'SimState', data: { snapshot: { console_hosts: [{ console: 'left', host: 'pilot', rating: 'Ensign' }] } } });
    state.apply(welcome);
    expect(state.consoleHosts.left.host).toBe('pilot');
    state.apply({ type: 'ReturnedToLobby', data: {} });
    expect(state.consoleHosts).toEqual({});
  });
  it('hides only the Command Console whose own target has human control', () => {
    const command = { ...consoles[1], systems: ['orders'] };
    const model = heroBarModel({ directStation: 'pilot', consoles: [consoles[0], command],
      blackboards: { orders: { directed_station_ai: false }, other: { directed_station_ai: true } } });
    expect(model.tabs.map(t => t.id)).toEqual(['left']);
  });
  it('selects Console tabs without changing the occupied Station', () => {
    const model = heroBarModel({ directStation: 'pilot', stations: [{ id: 'pilot' }], consoles, activeStation: 'right',
      consoleHosts: { left: { host: 'pilot' }, right: { host: 'pilot' } } });
    expect(model.tabs.map(t => t.id)).toEqual(['left', 'right']);
    expect(model.selected).toBe('right');
  });
  it('does not merge knowledge merely because Consoles share a Station', () => {
    const state = { blackboards: { drive: { yaw: 1 }, scan: { target: 'secret' }, report: { subjects: ['known'] } },
      blackboardKinds: { drive: 'Helm', scan: 'Sensors', report: 'Dossiers' } };
    expect(Object.keys(consoleReadState(state, consoles[0]).blackboards)).toEqual(['drive']);
    const bound = { ...consoles[0], feeds: [{ source: 'report', kind: 'Dossiers' }, { source: 'scan', kind: 'Helm' }] };
    expect(Object.keys(consoleReadState(state, bound).blackboards)).toEqual(['drive', 'report']);
    expect(Object.keys(state.blackboards)).toHaveLength(3);
  });
  it('does not recover foreign targets or waypoints from legacy global caches', () => {
    const definition = { ...consoles[0], systems: ['drive'], feeds: [] };
    const state = { consoleDefinitions: [definition], systemConsoleFamilies: { drive: 'helm' },
      blackboards: { drive: { yaw: 0 }, nav: { waypoint: {} }, scan: {} },
      blackboardKinds: { drive: 'Helm', nav: 'Navigation', scan: 'Sensors' },
      sensorsTarget: 'secret', navigationWaypoint: { x: 12, z: 12 },
      asteroids: [{ uuid: 'secret', position: [12, 0, 12], tags: ['ship'], radius: 1 }] };
    const view = buildConsoleView('left', state);
    expect(view.waypoint).toBeNull();
    expect(view.blips.some(blip => blip.kind === 'science-target' || blip.kind === 'waypoint')).toBe(false);
    const unseen = buildConsoleView('left', { ...state, blackboards: {}, blackboardKinds: {} });
    expect(unseen.blips).toEqual([]);
    expect(buildConsoleView('pilot', state)).toEqual({});
  });
  it('uses the home Station for advice and the effective Console rating for automation', () => {
    const definition = { ...consoles[0], feeds: [{ source: 'orders', kind: 'Command' }] };
    const state = { consoleDefinitions: [definition], consoleHosts: { left: { host: 'other-seat', rating: 'Backfill' } },
      systemConsoleFamilies: { drive: 'helm' }, stationRatings: { pilot: 'Captain', helm: 'Captain' },
      blackboards: { drive: {}, orders: { directed_station: 'pilot', directed_station_ai: false,
        selected_stance: 'cautious', stances: [{ id: 'cautious', label: 'Cautious' }] } },
      blackboardKinds: { drive: 'Helm', orders: 'Command' } };
    const view = buildConsoleView('left', state);
    expect(view.command_advice.stance_id).toBe('cautious');
    expect(view.helm_auto).toBe(true);
    expect(state.stationRatings.helm).toBe('Captain');
  });
  it('does not turn read feeds or co-hosting into hosted controls', () => {
    const state = { consoleDefinitions: consoles, consoleHosts: { left: { host: 'pilot' }, right: { host: 'pilot' } },
      systemConsoleFamilies: { drive: 'helm', scan: 'sensors' }, stationSystems: { pilot: ['drive', 'scan'] },
      blackboards: { drive: { yaw: 1 }, scan: { host_station: 'pilot', target: 'secret' } },
      blackboardKinds: { drive: 'Helm', scan: 'Sensors' } };
    const view = buildConsoleView('left', state);
    expect(view.console_id).toBe('left');
    expect(view.hosted_systems).toEqual(['drive']);
    expect(view.system_ids).toEqual(['drive']);
  });
});

it('detached Console uses its own rating and tutorial metadata, without host Station pooling', () => {
  const tutorial = { id: 'aux-help' };
  const view = consoleReadState({ stationRatings: { pilot: 'Backfill' }, stationTutorials: { pilot: [{ id: 'seat-help' }] },
    systemConsoleFamilies: { drive: 'helm' } }, { ...consoles[0], home_station: null, visiting_rating: 'Simplified', tutorials: [tutorial] });
  expect(view.stationRatings.helm).toBe('Simplified');
  expect(view.stationTutorials.left).toEqual([tutorial]);
});
