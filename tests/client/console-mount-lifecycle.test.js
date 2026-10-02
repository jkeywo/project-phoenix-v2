import { JSDOM } from 'jsdom';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createConsoleMounts } from '../../gui/console-mount.js';
import { ConsoleLatencyMeter, PUSH_CAUSE } from '../../gui/console-latency.js';

const SHIP = { stations: [
  { id: 'helm', console: 'gui/battleship/helm.html' },
  { id: 'pilot', console: 'gui/battleship/captain.html' },
  { id: 'tactical', console: 'gui/battleship/tactical.html' },
  { id: 'comms', console: 'gui/cruiser/comms.html' },
  { id: 'navigation', console: 'gui/cruiser/comms.html' },
] };
const cleanups = [];
afterEach(() => { cleanups.splice(0).forEach(fn => fn()); vi.restoreAllMocks(); });

function harness(overrides = {}) {
  const dom = new JSDOM('<div id="consoles"><section id="lobby-ui"></section><aside id="other"></aside></div>',
    { url: 'https://phoenix.test/' });
  const doc = dom.window.document;
  const container = doc.getElementById('consoles');
  const trace = [], loads = [], payloads = [], profiles = [];
  const live = {
    state: { epoch: 1, stationSystems: { helm: ['drive'], pilot: ['command'], tactical: ['guns'] },
      systemConsoleFamilies: { drive: 'helm', command: 'captain', guns: 'tactical' } },
    active: 'helm', bindings: { command: ['KeyX'] }, feedback: { vibration: false },
  };
  const originalAdd = dom.window.EventTarget.prototype.addEventListener;
  vi.spyOn(dom.window.EventTarget.prototype, 'addEventListener').mockImplementation(function(type, callback, options) {
    if (this.tagName === 'IFRAME' && type === 'load') loads.push({ iframe: this, callback });
    return originalAdd.call(this, type, callback, options);
  });
  const nameOf = iframe => iframe.id;
  const noteSnapshot = vi.fn((station, cause) => trace.push(`ack:${station}:${cause}`));
  const onDeclaration = vi.fn();
  const buildState = vi.fn((station, state) => JSON.stringify({ station, epoch: state.epoch }));
  const mounts = createConsoleMounts({
    doc, container, readState: () => live.state, buildState,
    readActiveStation: () => live.active,
    readBindings: () => live.bindings,
    readFeedbackPreferences: () => live.feedback,
    installLocale: iframe => trace.push(`locale:${nameOf(iframe)}`),
    applyAccessibility: iframe => trace.push(`accessibility:${nameOf(iframe)}`),
    noteSnapshot, afterBindings: () => trace.push('gamepad'), onDeclaration,
    ...overrides,
  });
  function hooks(station) {
    const iframe = mounts.frame(station);
    iframe.contentWindow.__updateConsole = (name, json) => {
      trace.push(`snapshot:${name}`); payloads.push({ iframe, name, data: JSON.parse(json) });
    };
    iframe.contentWindow.__setConsoleOverlay = value => trace.push(`overlay:${station}:${value}`);
    iframe.contentWindow.__updateSemanticActionBindings = value => {
      trace.push(`bindings:${station}`); profiles.push({ station, kind: 'bindings', value });
    };
    iframe.contentWindow.__updateActionFeedbackPreferences = value => {
      trace.push(`feedback:${station}`); profiles.push({ station, kind: 'feedback', value });
    };
    return iframe;
  }
  function mount(ship = SHIP) {
    mounts.mount(ship);
    for (const station of ship.stations) if (mounts.frame(station.id)) hooks(station.id);
  }
  function load(station) { mounts.frame(station).dispatchEvent(new dom.window.Event('load')); }
  function declare(station, data) {
    return mounts.handleDeclaration({ source: mounts.frame(station).contentWindow, data });
  }
  cleanups.push(() => { mounts.dispose(); dom.window.close(); });
  return { dom, doc, container, mounts, mount, hooks, load, declare, trace, loads, payloads, profiles,
    live, noteSnapshot, onDeclaration, buildState };
}

const dirtyAll = () => ({ changedSystems: new Set(['drive', 'command', 'guns']) });

describe('parent Console lifetime', () => {
  it('seeds each load in order and clears only that Station overlay', () => {
    const h = harness(); h.mount();
    h.mounts.selectOverlay('helm', 'map'); h.trace.length = 0;
    h.load('helm');
    expect(h.trace).toEqual([
      'locale:helm-iframe', 'snapshot:helm', 'ack:helm:iframe-load', 'overlay:helm:null',
      'accessibility:helm-iframe', 'bindings:helm', 'feedback:helm', 'gamepad',
    ]);
    expect(h.mounts.view('helm').activeOverlay).toBeNull();
  });

  it('reloads from live state and preferences without remounting or erasing declarations', () => {
    const h = harness(); h.mount();
    h.declare('helm', { type: 'console_tabs', console: 'helm', tabs: [{ id: 'map' }] });
    h.declare('helm', { type: 'console_hull', console: 'helm', entries: [{ id: 'drive' }] });
    const before = h.mounts.frame('helm');
    h.load('helm');
    h.live.state = { ...h.live.state, epoch: 2 };
    h.live.bindings = { command: ['KeyY'] }; h.live.feedback = { vibration: true };
    h.load('helm');
    expect(h.payloads.map(row => row.data.epoch)).toEqual([1, 2]);
    expect(h.profiles.slice(-2)).toEqual([
      { station: 'helm', kind: 'bindings', value: h.live.bindings },
      { station: 'helm', kind: 'feedback', value: h.live.feedback },
    ]);
    expect(h.mounts.frame('helm')).toBe(before);
    expect(h.mounts.view('helm')).toEqual({ tabs: [{ id: 'map' }], hull: [{ id: 'drive' }], activeOverlay: null });
  });

  it('keeps two Stations sharing a document in distinct declaration slots', () => {
    const h = harness(); h.mount();
    for (const station of ['comms', 'navigation']) {
      h.declare(station, { type: 'console_tabs', console: 'comms', tabs: [{ id: station }] });
      h.declare(station, { type: 'console_hull', console: 'comms', entries: [{ id: station }] });
    }
    for (const station of ['comms', 'navigation']) {
      expect(h.mounts.view(station)).toEqual({ tabs: [{ id: station }], hull: [{ id: station }], activeOverlay: null });
    }
    expect(h.onDeclaration.mock.calls.map(([row]) => row.stationId)).toEqual(['comms', 'comms', 'navigation', 'navigation']);
  });

  it('preserves claimed-name fallback for absent and unmatched sources', () => {
    const h = harness(); h.mount();
    for (const source of [null, h.dom.window]) {
      expect(h.mounts.handleDeclaration({ source, data: { type: 'console_hull', console: 'external', entries: [1] } })).toBe(true);
      expect(h.mounts.view('external').hull).toEqual([1]);
    }
    expect(h.mounts.handleDeclaration({ data: { type: 'console_tabs' } })).toBe(true);
    expect(h.mounts.handleDeclaration({ data: { type: 'console_action' } })).toBe(false);
    expect(h.mounts.handleDeclaration(null)).toBe(false);
    expect(h.onDeclaration).toHaveBeenCalledTimes(2);
  });

  it('settles selection from declarations without a background close clearing another Station', () => {
    const h = harness(); h.mount();
    h.mounts.selectOverlay('helm', 'map');
    h.declare('navigation', { type: 'console_tabs', console: 'comms', tabs: [{ id: 'nav' }], open: null });
    h.load('navigation');
    expect(h.mounts.view('helm').activeOverlay).toBe('map');
    h.declare('helm', { type: 'console_tabs', tabs: [{ id: 'intel' }], open: 'intel' });
    expect(h.mounts.view('helm').activeOverlay).toBe('intel');
    expect(h.mounts.view('navigation').activeOverlay).toBeNull();
    h.declare('helm', { type: 'console_tabs', tabs: null, open: null });
    h.declare('helm', { type: 'console_hull', entries: null });
    expect(h.mounts.view('helm')).toEqual({ tabs: [], hull: [], activeOverlay: null });
  });

  it('sets one overlay, closing its previous Station before opening another', () => {
    const h = harness(); h.mount();
    h.mounts.selectOverlay('helm', 'map');
    h.mounts.selectOverlay('navigation', 'chart');
    h.mounts.selectOverlay(null, null);
    expect(h.trace).toEqual(['overlay:helm:map', 'overlay:helm:null', 'overlay:navigation:chart', 'overlay:navigation:null']);
  });

  it('publishes host changes only to active or always-push Stations using authoritative families', () => {
    const h = harness(); h.mount();
    h.mounts.publishChanges(dirtyAll());
    expect(h.payloads.map(row => row.name)).toEqual(['helm', 'pilot']);
    expect(h.noteSnapshot.mock.calls).toEqual([['helm', PUSH_CAUSE.SERVER_MESSAGE], ['pilot', PUSH_CAUSE.SERVER_MESSAGE]]);
    h.payloads.length = 0; h.live.active = 'tactical';
    h.mounts.publishChanges(dirtyAll());
    expect(h.payloads.map(row => row.name)).toEqual(['pilot', 'tactical']);
    expect(h.mounts.frame('tactical').id).toBe('weapons-iframe');
  });

  it('keeps dirty publication before Welcome replacement and seeds new frames from current state', () => {
    const h = harness(); h.mount();
    const old = h.mounts.frame('helm');
    h.live.state.epoch = 2;
    h.mounts.publishChanges(dirtyAll());
    h.mount(); h.load('helm');
    expect(h.payloads[0]).toMatchObject({ iframe: old, data: { epoch: 2 } });
    expect(h.payloads.at(-1)).toMatchObject({ iframe: h.mounts.frame('helm'), data: { epoch: 2 } });
    expect(h.noteSnapshot.mock.calls.at(-1)).toEqual(['helm', PUSH_CAUSE.IFRAME_LOAD]);
  });

  it('does not acknowledge a pending action on local render, tutorial or iframe reload', () => {
    const h = harness(); h.mount();
    const meter = new ConsoleLatencyMeter(); meter.setEnabled(true);
    h.noteSnapshot.mockImplementation((station, cause) => meter.noteAck(station, cause));
    meter.noteDispatch('helm_input', 'helm', Date.now());
    h.mounts.refresh('helm', PUSH_CAUSE.RENDER);
    h.mounts.refresh('helm', PUSH_CAUSE.TUTORIAL);
    h.load('helm');
    expect(meter.drain().samples).toEqual([]);
    h.mounts.publishChanges(dirtyAll());
    expect(meter.drain().samples).toHaveLength(1);
  });

  it('refreshes explicit hidden Stations locally and measures attempted publication with no hook', () => {
    const h = harness(); h.mount();
    h.mounts.refresh('tactical', PUSH_CAUSE.RENDER);
    delete h.mounts.frame('helm').contentWindow.__updateConsole;
    h.mounts.refresh('helm', PUSH_CAUSE.SERVER_MESSAGE);
    h.mounts.refresh('unmounted', PUSH_CAUSE.TUTORIAL);
    expect(h.payloads.map(row => row.name)).toEqual(['tactical']);
    expect(h.noteSnapshot.mock.calls).toEqual([
      ['tactical', PUSH_CAUSE.RENDER], ['helm', PUSH_CAUSE.SERVER_MESSAGE], ['unmounted', PUSH_CAUSE.TUTORIAL],
    ]);
  });

  it('fans current bindings and feedback to inactive frames, skipping the whole operation without a registry', () => {
    const h = harness(); h.mount();
    h.mounts.refreshBindings();
    expect(h.profiles.filter(row => row.kind === 'bindings').map(row => row.station)).toEqual(SHIP.stations.map(row => row.id));
    expect(h.profiles.filter(row => row.kind === 'feedback')).toHaveLength(SHIP.stations.length);
    h.trace.length = 0; h.profiles.length = 0; h.live.bindings = null;
    h.mounts.refreshBindings();
    expect(h.trace).toEqual([]); expect(h.profiles).toEqual([]);
    h.load('helm');
    expect(h.trace).toEqual(['locale:helm-iframe', 'snapshot:helm', 'ack:helm:iframe-load', 'accessibility:helm-iframe']);
  });

  it('preserves iframe identity and local context across tab and phase visibility changes', () => {
    const h = harness(); h.mount();
    const helm = h.mounts.frame('helm'); helm.dataset.draft = 'half-typed';
    h.mounts.show('helm', true); h.mounts.show('navigation', true); h.mounts.show('helm', false);
    expect(helm.parentElement.className).toBe('console-section');
    h.mounts.show('helm', true);
    expect(h.mounts.frame('helm')).toBe(helm);
    expect(helm.dataset.draft).toBe('half-typed');
    expect(helm.parentElement.className).toBe('console-section active');
    expect(h.doc.querySelectorAll('.console-section.active')).toHaveLength(1);
  });

  it('detaches obsolete loads and makes an already-queued callback inert after replacement', () => {
    const h = harness(); h.mount();
    const old = h.mounts.frame('helm');
    const obsolete = h.loads.find(row => row.iframe === old).callback;
    const removal = vi.spyOn(old, 'removeEventListener');
    h.declare('helm', { type: 'console_tabs', tabs: [{ id: 'map' }] });
    h.mount();
    expect(removal).toHaveBeenCalledWith('load', obsolete);
    // Cached declarations retain their existing policy until a fresh report.
    expect(h.mounts.view('helm').tabs).toEqual([{ id: 'map' }]);
    h.mounts.selectOverlay('helm', 'replacement-overlay'); h.trace.length = 0;
    old.dispatchEvent(new h.dom.window.Event('load'));
    obsolete();
    expect(h.trace).toEqual([]);
    expect(h.payloads).toEqual([]);
    expect(h.mounts.view('helm').activeOverlay).toBe('replacement-overlay');
    h.load('helm');
    expect(h.payloads[0].iframe).toBe(h.mounts.frame('helm'));
    expect(h.mounts.view('helm').activeOverlay).toBeNull();
  });

  it('disposes owned nodes and listeners, preserves other container children, and rejects late work', () => {
    const h = harness(); h.mount();
    const old = h.mounts.frame('helm');
    const obsolete = h.loads.find(row => row.iframe === old).callback;
    h.mounts.dispose(); h.mounts.dispose();
    expect(h.container.querySelectorAll('.console-section')).toHaveLength(0);
    expect(h.doc.getElementById('lobby-ui')).not.toBeNull();
    expect(h.doc.getElementById('other')).not.toBeNull();
    expect(h.mounts.frame('helm')).toBeNull();
    obsolete(); h.mounts.refresh('helm', PUSH_CAUSE.RENDER); h.mounts.publishChanges(dirtyAll());
    h.mounts.refreshBindings(); h.mounts.mount(SHIP); h.mounts.show('helm', true); h.mounts.selectOverlay('helm', 'map');
    expect(h.mounts.handleDeclaration({ data: { type: 'console_tabs', console: 'helm', open: 'map' } })).toBe(false);
    expect(h.trace).toEqual([]);
    expect(h.mounts.view('helm')).toEqual({ tabs: [], hull: [], activeOverlay: null });
  });

  it('tolerates missing builders, frame hooks and cross-origin hook access', () => {
    const h = harness({ buildState: undefined }); h.mount();
    h.mounts.refresh('helm', PUSH_CAUSE.RENDER);
    expect(h.payloads[0].data).toEqual({});
    const iframe = h.mounts.frame('helm');
    Object.defineProperty(iframe.contentWindow, '__updateConsole', { get() { throw new Error('cross-origin'); } });
    Object.defineProperty(iframe.contentWindow, '__updateSemanticActionBindings', { get() { throw new Error('cross-origin'); } });
    Object.defineProperty(iframe.contentWindow, '__setConsoleOverlay', { get() { throw new Error('cross-origin'); } });
    expect(() => { h.load('helm'); h.mounts.selectOverlay('helm', 'map'); }).not.toThrow();
    expect(h.trace.at(-1)).toBe('gamepad');
  });
});
