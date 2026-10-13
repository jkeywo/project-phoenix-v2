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

it('shows the current hosting Station takeover on a detached Console', () => {
  const takeover = { station: 'pilot', operators: ['gm-1'] };
  const state = { consoleDefinitions: [{ ...consoles[0], home_station: null }],
    consoleHosts: { left: { host: 'pilot', rating: 'Std' } }, stationPuppets: { pilot: takeover },
    systemConsoleFamilies: { drive: 'helm' } };
  expect(buildConsoleView('left', state).gm_takeover).toEqual(takeover);
});

it('revokes removed Console publications before updates and clears derived navigation state', () => {
  const state = new ClientSimState();
  state.apply({type:'BlackboardUpdate',data:{presentation_generation:4,updates:[
    ['nav',{kind:'Navigation',data:{navigation_waypoint:{x:7,z:8}}}], ['guns',{kind:'Weapons',data:{target:'enemy'}}]
  ]}});
  const change = state.apply({type:'BlackboardUpdate',data:{presentation_generation:4,console_scope_tick:10,console_scope:['guns'],updates:[
    ['nav',{kind:'Navigation',data:{navigation_waypoint:{x:9,z:9}}}]
  ]}});
  expect(state.blackboards.nav).toBeUndefined();
  expect(state.blackboardKinds.nav).toBeUndefined();
  expect(state.blackboardPresentation.nav).toBeUndefined();
  expect(state.navigationWaypoint).toBeNull();
  expect(change.changedBlackboards.has('nav')).toBe(true);
  expect(state.blackboards.guns.target).toBe('enemy');
  state.apply({type:'BlackboardUpdate',data:{presentation_generation:4,console_scope_tick:11,console_scope:[],updates:[]}});
  expect(state.blackboards).toEqual({});
});

it('does not invent Consoles or controls from an obsolete Station-only roster', () => {
  expect(planMounts({stations:[{id:'helm',console:'gui/helm.html'}]})).toEqual([]);
  expect(buildConsoleView('helm', {stationSystems:{helm:['drive']},systemConsoleFamilies:{drive:'helm'}})).toEqual({});
  expect(heroBarModel({directStation:'helm',stations:[{id:'helm'}],activeStation:'helm'}).tabs).toEqual([]);
});

it('projects permitted getter-backed view and alert facts without leaking them to another Console', () => {
  const state = new ClientSimState();
  state.apply({type:'BlackboardUpdate',data:{updates:[['captain',{kind:'Captain',data:{red_alert:true,view_mode:{kind:'HelmRadar'}}}]]}});
  const bound = consoleReadState(state, {id:'left',systems:[],feeds:[{source:'captain',kind:'Captain'}]});
  expect(bound.currentView).toBe('HelmRadar');
  expect(bound.redAlert).toBe(true);
  const isolated = consoleReadState(state, {id:'right',systems:[],feeds:[]});
  expect(isolated.currentView).toBeUndefined();
  expect(isolated.redAlert).toBeUndefined();
});

it('refuses stale-generation scope changes before pruning newer observations, including an empty scope', () => {
  const state = new ClientSimState();
  state.apply({type:'BlackboardUpdate',data:{presentation_generation:5,updates:[['nav',{kind:'Navigation',data:{navigation_waypoint:{x:7,z:8}}}]]}});
  state.apply({type:'BlackboardUpdate',data:{presentation_generation:4,console_scope_tick:99,console_scope:[],updates:[]}});
  expect(state.blackboards.nav).toBeDefined();
  expect(state.navigationWaypoint).toEqual({x:7,z:8});
  state.apply({type:'BlackboardUpdate',data:{presentation_generation:6,console_scope_tick:0,console_scope:[],updates:[]}});
  state.apply({type:'BlackboardUpdate',data:{presentation_generation:5,console_scope_tick:100,console_scope:['nav'],updates:[['nav',{kind:'Navigation',data:{navigation_waypoint:{x:9,z:9}}}]]}});
  expect(state.blackboards).toEqual({});
  expect(state.navigationWaypoint).toBeNull();
});

it('orders scopes by host tick within a continuation and rejects unscoped updates after scoped authority', () => {
  const state = new ClientSimState();
  const publish = (generation, tick, scope, value) => state.apply({type:'BlackboardUpdate',data:{
    presentation_generation:generation,console_scope_tick:tick,console_scope:scope,
    updates:[['nav',{kind:'Navigation',data:{navigation_waypoint:value}}]]}});
  publish(3,20,['nav'],{x:2,z:2});
  publish(3,19,[],null);
  expect(state.navigationWaypoint).toEqual({x:2,z:2});
  publish(3,21,[],null);
  publish(3,20,['nav'],{x:9,z:9});
  state.apply({type:'BlackboardUpdate',data:{updates:[['nav',{kind:'Navigation',data:{navigation_waypoint:{x:8,z:8}}}]]}});
  expect(state.blackboards).toEqual({});
  publish(4,0,['nav'],{x:1,z:1});
  expect(state.navigationWaypoint).toEqual({x:1,z:1});
});

it('keeps detached and co-hosted Console attention separate and clears only authoritative snapshots', () => {
  const state = new ClientSimState();
  const snapshot = {console_importance:[{console:'right',unread:true,critical:true}],
    console_hosts:[{console:'left',host:'pilot',rating:'Std'},{console:'right',host:'pilot',rating:'Std'}]};
  state.apply({type:'SimState',data:{snapshot}});
  const model = () => heroBarModel({directStation:'pilot',stations:[{id:'pilot'}],
    consoles:[consoles[0],{...consoles[1],home_station:null}],consoleHosts:state.consoleHosts,
    consoleImportance:state.consoleImportance,activeStation:'right'});
  expect(model().tabs.find(t => t.id === 'right').importance).toEqual({unread:true,critical:true});
  expect(model().tabs.find(t => t.id === 'left').importance).toBeNull();
  state.apply({type:'SimState',data:{snapshot:{...snapshot,console_importance:[{console:'right',unread:false,critical:true}]}}});
  expect(model().tabs.find(t => t.id === 'right').importance).toEqual({unread:false,critical:true});
  state.apply({type:'SimState',data:{snapshot:{...snapshot,console_importance:[]}}});
  expect(model().tabs.find(t => t.id === 'right').importance).toBeNull();
});
