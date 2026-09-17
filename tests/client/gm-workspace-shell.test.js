// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { mountGmWorkspaceShell } from '../../gui/gm-workspace-shell.js';
import { defaultLiveLayout, liveLayoutModel } from '../../gui/live-layout-model.js';
const source = readFileSync('server.html', 'utf8');
const observers = [];
// Every mounted shell must be disposed: it registers a document-level keydown
// listener for dock commands, and a stale one from a previous test claims the
// event (and calls preventDefault) before the live shell ever sees it.
const mountedShells = [];
afterEach(() => {
  mountedShells.splice(0).forEach(shell => shell.dispose());
  observers.forEach(observer => observer.disconnect());
  document.documentElement.classList.remove('phoenix-gm-page');
  document.head.querySelectorAll('link').forEach(link => link.remove());
  document.body.replaceChildren();
});
function mount(extra = {}, translate = id => id) {
  const parsed = new DOMParser().parseFromString(source, 'text/html');
  document.body.innerHTML = parsed.body.innerHTML;
  document.documentElement.classList.add('phoenix-gm-page');
  const selectEntity = vi.fn();
  const win = { document, Event, MutationObserver: class extends MutationObserver {
    constructor(fn) { super(fn); observers.push(this); }
  }, __phoenixGmPage: true, ...extra };
  const shell = mountGmWorkspaceShell({doc:document,win,t:translate,has:()=>false,selectEntity});
  mountedShells.push(shell);
  return {shell,selectEntity,win};
}
it('lays the desk out as the post-M5 screen and keeps the authentic iframe outside it', () => {
  mount();
  expect(document.querySelector('[data-panel="readiness"] #gm-force-start-btn')).not.toBeNull();
  // Authentic Station operation is two dock panels (issue #1504): the takeover
  // controls are a tool and the console is a document. The iframe stays out of
  // the inspector, exactly as it always has.
  expect(document.querySelector('[data-panel="station"] #gm-station-toggle')).not.toBeNull();
  expect(document.querySelector('#gm-station-tools #gm-station-pending')).not.toBeNull();
  expect(document.querySelector('#gm-station-surface #gm-station-frame')).not.toBeNull();
  expect(document.querySelector('#gm-inspector #gm-station-frame')).toBeNull();
  expect(document.querySelector('#gm-inspector #gm-station-toggle')).toBeNull();
  // With no Station taken over the console has nothing to show, so it has no
  // panel — the same rule the surface's own `hidden` always carried. Its node
  // is parked rather than detached, because the shell is the only writer of
  // that attribute and the only thing that can bring it back.
  expect(document.getElementById('gm-station-surface').closest('.workshop-dock-parked')).not.toBeNull();
  expect(document.querySelector('[data-panel="station-console"]')).toBeNull();
  // Since issue #1503 the desk is TWO regions: the dock workspace across the
  // left and centre, and the detail column.
  expect([...document.getElementById('gm-workspace').children].map(child => child.id))
    .toEqual(['gm-desk-brief', 'gm-desk-detail']);
  // The detail column is a panel frame; the dock workspace deliberately is not,
  // so it adds no padding, border, chrome or second scroll box around the dock.
  expect(document.getElementById('gm-desk-detail').classList.contains('gm-desk-region')).toBe(true);
  expect(document.getElementById('gm-desk-brief').classList.contains('gm-desk-region')).toBe(false);
  expect(document.getElementById('gm-desk-brief').classList.contains('gm-desk-dock')).toBe(true);
  expect([...document.getElementById('gm-desk-brief').children].map(child => child.id))
    .toEqual(['gm-live-layout']);
  // Right: the selected hull, then the checkpoints a live event is recovered
  // from — the restore control travels inside #gm-checkpoint.
  expect([...document.getElementById('gm-desk-detail').children].map(child => child.id))
    .toEqual(['gm-inspector', 'gm-checkpoint']);
  expect(document.querySelector('#gm-checkpoint #gm-restore-apply')).not.toBeNull();
  // Mission events, the four record surfaces, the map and the awareness panels
  // are all dock panels now.
  expect(document.getElementById('gm-desk-log')).toBeNull();
  for (const [panel, id] of [['mission', 'gm-mission-panel'], ['comms', 'gm-comms-panel'],
    ['activity', 'gm-activity'], ['journal', 'gm-journal'], ['session-history', 'gm-session-history'],
    ['map', 'gm-map-panel'], ['attention', 'gm-attention-panel'], ['workload', 'gm-workload-panel'],
    ['health', 'gm-health-panel']]) {
    expect(document.getElementById(id).closest('[data-panel]')?.dataset.panel, id).toBe(panel);
  }
  // gui/gm-widgets-panel.js owns `hidden` on the authored widget region, so a
  // scenario that authors no widget has no widget tab — and the node is parked
  // in the surface, because that owner is the only thing that can bring it back.
  expect(document.getElementById('gm-widgets').hidden).toBe(true);
  expect(document.getElementById('gm-widgets').closest('.workshop-dock-parked')).not.toBeNull();
  expect(document.querySelector('#gm-session-history #gm-session-log')).not.toBeNull();
  // The map is the surface this workspace is arranged around.
  expect(document.querySelector('[data-panel="map"]').dataset.panelKind).toBe('document');
  // The map now lives in the dock. <ph-navigation-map> restores its render loop
  // and size observer on reconnect, which is what makes that survivable.
  expect(document.getElementById('gm-map-panel').closest('#gm-live-layout')).not.toBeNull();
  // And the widget region starts hidden, so a scenario that authors no widget
  // leaves the left column exactly as it was.
  expect(document.getElementById('gm-widgets').hidden).toBe(true);
  // The #1437 technical-banner seam lives inside the QUEUE, not inside the
  // health panel: it has to sit beside the list a Game Master reads and filters,
  // which is the surface it exists to be un-hideable from.
  expect(document.querySelector('#gm-attention-panel #gm-attention-banners')).not.toBeNull();
  expect(document.querySelector('#gm-health-panel #gm-attention-banners')).toBeNull();
  expect(document.querySelector('[data-panel="manual-save"] #manual-save-panel')).not.toBeNull();
  expect(document.querySelector('[data-panel="join"] #gm-join-controls').style.position).toBe('');
});
it('keeps every operator and session status control in the fixed bar outside Live docking', () => {
  mount();
  const bar = document.querySelector('#gm-console > header');
  for (const id of ['gm-session-controls', 'gm-role-preset-label', 'gm-lethal-label',
    'gm-operator-identity', 'gm-scenario-title', 'gm-session-clock', 'gm-peer-summary', 'gm-health-pills']) {
    expect(bar.contains(document.getElementById(id)), id).toBe(true);
  }
  expect(document.getElementById('gm-live-layout').contains(bar)).toBe(false);
  expect(readFileSync('gui/gm-workspace.css', 'utf8'))
    .toMatch(/#gm-console \.gm-station-bar\s*\{[^}]*position:\s*sticky/);
});

it('moves original Live workflow nodes into one shared dock rather than cloning them', () => {
  mount();
  for (const [panel, id] of [['readiness', 'gm-start-controls'], ['join', 'gm-join-controls'],
    ['manual-save', 'manual-save-panel']]) {
    expect(document.querySelectorAll(`#${id}`)).toHaveLength(1);
    expect(document.querySelector(`[data-panel="${panel}"] #${id}`)).not.toBeNull();
  }
  expect(document.querySelector('[data-panel="roster"] #gm-roster')).not.toBeNull();
});
it('leaves ordinary host lobby and save controls in their authored surfaces', () => {
  const parsed = new DOMParser().parseFromString(source, 'text/html');
  document.body.innerHTML = parsed.body.innerHTML;
  const start = document.getElementById('gm-start-controls');
  const join = document.getElementById('gm-join-controls');
  const manual = document.getElementById('manual-save-panel');
  const parents = [start.parentNode, join.parentNode, manual.parentNode];
  const clicked = vi.fn();
  document.getElementById('gm-force-start-btn').addEventListener('click', clicked);

  const shell = mountGmWorkspaceShell({ doc: document,
    win: { document, Event, MutationObserver }, t: id => id, has: () => false,
    selectEntity: vi.fn() });

  expect([start.parentNode, join.parentNode, manual.parentNode]).toEqual(parents);
  expect(document.querySelector('#gm-live-layout .workshop-dock-canvas')).toBeNull();
  expect(join.style.position).toBe('absolute');
  expect([...document.head.querySelectorAll('link')].map(link => link.href).join('\n'))
    .not.toMatch(/(?:workshop|dock-layout)\.css/);
  document.getElementById('gm-force-start-btn').click();
  expect(clicked).toHaveBeenCalledOnce();
  shell.dispose();
});

it('restores original host controls and attributes when the GM workspace closes', () => {
  const parsed = new DOMParser().parseFromString(source, 'text/html');
  document.body.innerHTML = parsed.body.innerHTML;
  document.documentElement.classList.add('phoenix-gm-page');
  const controls = ['gm-start-controls', 'gm-join-controls', 'manual-save-panel']
    .map(id => document.getElementById(id));
  const origins = controls.map(node => ({ parent: node.parentNode, next: node.nextSibling,
    style: node.getAttribute('style'), hidden: node.getAttribute('hidden') }));
  const clicked = vi.fn();
  document.getElementById('gm-force-start-btn').addEventListener('click', clicked);
  const shell = mountGmWorkspaceShell({ doc: document,
    win: { document, Event, MutationObserver, __phoenixGmPage: true },
    t: id => id, has: () => false, selectEntity: vi.fn() });
  expect(document.querySelector('[data-panel="readiness"] #gm-start-controls')).not.toBeNull();

  shell.dispose();

  controls.forEach((node, index) => {
    expect(node.parentNode).toBe(origins[index].parent);
    expect(node.nextSibling).toBe(origins[index].next);
    expect(node.getAttribute('style')).toBe(origins[index].style);
    expect(node.getAttribute('hidden')).toBe(origins[index].hidden);
    expect(document.querySelectorAll(`#${node.id}`)).toHaveLength(1);
  });
  document.getElementById('gm-force-start-btn').click();
  expect(clicked).toHaveBeenCalledOnce();
});

it('loads only a globally neutral dock stylesheet into the host document', () => {
  const dockCss = readFileSync('gui/dock-layout.css', 'utf8');
  const shellSource = readFileSync('gui/gm-workspace-shell.js', 'utf8');
  expect(dockCss).not.toMatch(/(^|[},]\s*)(:root|html|body|\*)\s*[{,]/m);
  for (const rule of dockCss.split('\n').filter(line => line.includes('{') && !line.trim().startsWith('@'))) {
    expect(rule.trim()).toMatch(/^\.workshop-dock-root/);
  }
  expect(shellSource).toContain("new URL('./dock-layout.css', import.meta.url)");
  expect(shellSource).not.toContain("new URL('./workshop.css', import.meta.url)");
  expect(readFileSync('workshop.html', 'utf8')).toContain('gui/dock-layout.css');
});
it('uses the same Live mount for native and reports missing save capability instead of inventing one', () => {
  const parsed = new DOMParser().parseFromString(source, 'text/html');
  document.body.innerHTML = parsed.body.innerHTML;
  document.documentElement.classList.add('phoenix-gm-page');
  const win = { document, Event, MutationObserver, __phoenixGmPage: true,
    addEventListener: window.addEventListener.bind(window), removeEventListener: window.removeEventListener.bind(window) };
  const shell = mountGmWorkspaceShell({ doc: document, win, t: id => id, has: () => false,
    selectEntity: vi.fn(), native: true });
  expect(document.querySelector('[data-panel="readiness"] #gm-start-controls')).not.toBeNull();
  expect(document.getElementById('gm-native-manual-save-unavailable')?.textContent)
    .toBe('server.gm.shell.manual_save_unavailable');
  expect(win.__hostGmCheckpointCreate).toBeUndefined();
  shell.dispose();
});
it('selects roster entities through the map seam and shows authored Station ratings', () => {
  const {shell,selectEntity} = mount();
  shell.refresh({entities:[{entity_id:'ship',name:'Courier',kind:'player_ship',faction:null}]},
    {ships:[{ship_id:'ship',stations:[{name:'Helm',rating:'Backfill'}]}]});
  document.querySelector('#gm-roster-ships button').click();
  expect(selectEntity).toHaveBeenCalledWith('ship');
  shell.selection({entity_id:'ship',name:'Courier',status:{systems:[]}});
  expect(document.querySelector('#gm-roster-ships button').getAttribute('aria-pressed')).toBe('true');
  expect(document.querySelector('.gm-station-pills').textContent).toContain('Helm · Backfill');
});
it('opens, focuses, docks and floats the authentic Station console', () => {
  const { shell } = mount();
  const surface = document.getElementById('gm-station-surface');
  const controls = document.getElementById('gm-station-controls');
  // A Station row in the projection is what gives the console something to show;
  // the puppet shows the takeover controls then, taken over or not. The shell is
  // the only writer of the surface's `hidden` and the dock reads it.
  controls.hidden = false;
  shell.refresh();
  const framed = document.querySelector('[data-panel="station-console"]');
  expect(framed).not.toBeNull();
  expect(framed.dataset.panelKind).toBe('document');
  expect(surface.closest('[data-panel]')).toBe(framed);
  // The iframe travelled with it rather than being rebuilt beside it.
  expect(document.querySelectorAll('#gm-station-frame')).toHaveLength(1);
  expect(document.getElementById('gm-station-frame').closest('[data-panel]').dataset.panel)
    .toBe('station-console');

  // Float it, then dock it beside the roster: the same node each time.
  shell.setLiveLayout(liveLayoutModel.float(shell.liveLayoutState(), 'station-console',
    { x: 20, y: 30, width: 500, height: 400 }));
  expect(document.querySelector('[data-panel="station-console"].is-floating')
    .contains(document.getElementById('gm-station-frame'))).toBe(true);
  shell.setLiveLayout(liveLayoutModel.dock(shell.liveLayoutState(), 'station-console', 'roster', 'tab'));
  expect(document.getElementById('gm-station-frame').closest('[data-panel]').dataset.panel)
    .toBe('station-console');

  // Closing it is an arrangement choice, not a release: the node stays in the
  // surface and the takeover controls are untouched.
  shell.setLiveLayout(liveLayoutModel.close(shell.liveLayoutState(), 'station-console'));
  expect(document.querySelector('[data-panel="station-console"]')).toBeNull();
  expect(document.getElementById('gm-station-frame')).not.toBeNull();
  expect(document.getElementById('gm-station-controls').hidden).toBe(false);
  expect(document.querySelector('[data-panel="station"] #gm-station-toggle')).not.toBeNull();

  // Losing the Station row takes the console's panel away again.
  controls.hidden = true;
  shell.refresh();
  expect(document.querySelector('[data-panel="station-console"]')).toBeNull();
  expect(surface.closest('.workshop-dock-parked')).not.toBeNull();
});
it('keeps the Station takeover controls out of the role preset reach', () => {
  const { shell } = mount();
  // The takeover controls are their own dock panel now, not a block inside the
  // inspector, so a preset that puts the inspector away no longer takes them
  // with it — which is what docs/toml-authoring-guide.md already promised.
  expect(document.querySelector('#gm-inspector #gm-station-controls')).toBeNull();
  document.getElementById('gm-inspector').hidden = true;
  shell.refresh();
  expect(document.querySelector('[data-panel="station"] #gm-station-toggle')).not.toBeNull();
  document.getElementById('gm-inspector').hidden = false;
  shell.refresh();
  // The console follows its own surface, which the shell alone writes.
  document.getElementById('gm-station-controls').hidden = false;
  shell.refresh();
  expect(document.querySelector('[data-panel="station-console"]')).not.toBeNull();
  document.getElementById('gm-station-controls').hidden = true;
  shell.refresh();
  expect(document.querySelector('[data-panel="station-console"]')).toBeNull();
  expect(document.getElementById('gm-station-frame')).not.toBeNull();
});
it('docks the presentation, audition and Workshop source utilities', () => {
  const { shell } = mount();
  for (const [panel, id] of [['presentation', 'gm-presentation-dock'],
    ['audition', 'gm-audition-dock']]) {
    expect(document.getElementById(id).closest('[data-panel]')?.dataset.panel, id).toBe(panel);
    expect(document.querySelector(`[data-panel="${panel}"]`).dataset.panelKind).toBe('tool');
  }
  // The handoff shows itself only when a retained source pack exists, and the
  // dock reads that rather than the empty host, so it offers no empty tab.
  expect(document.querySelector('[data-panel="source-link"]')).toBeNull();
  const handoff = document.createElement('section');
  handoff.id = 'gm-workshop-source'; handoff.hidden = true;
  document.getElementById('gm-source-link-dock').append(handoff);
  shell.refresh();
  expect(document.querySelector('[data-panel="source-link"]')).toBeNull();
  handoff.hidden = false;
  shell.refresh();
  expect(document.getElementById('gm-source-link-dock').closest('[data-panel]').dataset.panel)
    .toBe('source-link');
  // They share one group by default: each is a local instrument, not a record.
  const group = document.getElementById('gm-presentation-dock').closest('.workshop-tab-stack');
  expect(group.contains(document.getElementById('gm-audition-dock'))).toBe(true);
  expect(group.contains(document.getElementById('gm-source-link-dock'))).toBe(true);
  expect(group.contains(document.getElementById('gm-roster-ships'))).toBe(false);

  // Keyboard docking moves one like any other panel, and the move is persisted
  // as placement with no payload riding along.
  shell.setLiveLayout(liveLayoutModel.dock(defaultLiveLayout(), 'audition', 'roster', 'tab'));
  expect(document.getElementById('gm-audition-dock').closest('[data-panel]').dataset.panel)
    .toBe('audition');
  shell.setLiveLayout(liveLayoutModel.float(shell.liveLayoutState(), 'source-link', { x: 12, y: 14 }));
  expect(document.querySelector('[data-panel="source-link"].is-floating')
    .contains(document.getElementById('gm-source-link-dock'))).toBe(true);
  const stored = JSON.stringify(shell.liveLayoutState());
  for (const key of ['cue', 'audio', 'route', 'view', 'draft', 'pack', 'workshop']) {
    expect(stored, key).not.toContain(key);
  }
  // Reset puts them back where the default arrangement has them.
  shell.setLiveLayout(defaultLiveLayout());
  expect(document.getElementById('gm-audition-dock').closest('.workshop-tab-stack'))
    .toBe(document.getElementById('gm-presentation-dock').closest('.workshop-tab-stack'));
});
it('opens Spawn as a floating draft and keeps a docked one in the stored layout', () => {
  const { shell } = mount();
  // A draft nobody opened is not a place: it is absent from the arrangement.
  expect(document.querySelector('[data-panel="spawn"]')).toBeNull();
  expect(shell.liveLayoutState().closed).toContain('spawn');
  expect(document.getElementById('gm-spawn-panel')).not.toBeNull();

  // The switcher opens it floating, not as a tab in somebody else's group.
  document.querySelector('[data-layout-panel="spawn"][data-layout-control="switcher"]').click();
  const framed = document.querySelector('[data-panel="spawn"]');
  expect(framed.classList.contains('is-floating')).toBe(true);
  expect(framed.contains(document.getElementById('gm-spawn-palette'))).toBe(true);
  // A floating draft is not restored — it is a form, not an arrangement.
  expect(liveLayoutModel.normalize(shell.liveLayoutState()).closed).toContain('spawn');

  // A docked one is a tool the operator keeps to hand, and it does survive.
  shell.setLiveLayout(liveLayoutModel.dock(shell.liveLayoutState(), 'spawn', 'roster', 'tab'));
  expect(document.getElementById('gm-spawn-panel').closest('[data-panel]').dataset.panel).toBe('spawn');
  const restored = liveLayoutModel.normalize(shell.liveLayoutState());
  expect(restored.closed).not.toContain('spawn');
  // Placement only: no palette choice, coordinate or correlation rides along.
  const stored = JSON.stringify(restored);
  for (const key of ['palette', 'variant', 'position', 'heading', 'correlation']) {
    expect(stored, key).not.toContain(key);
  }
});
it('confirms before the close button takes an unsent draft, and forgets it after', () => {
  const { shell, win } = mount();
  let dirty = true;
  const resets = [];
  shell.temporaryActions.register('spawn', {
    isDirty: () => dirty, reset: options => { resets.push(options); dirty = false; },
    keepOpen: () => false, focus: () => {},
  });
  document.querySelector('[data-layout-panel="spawn"][data-layout-control="switcher"]').click();
  const close = () => document.querySelector('[data-panel="spawn"] [data-layout-control="close"]');

  win.confirm = () => false;
  close().click();
  expect(document.querySelector('[data-panel="spawn"]')).not.toBeNull();
  expect(resets).toEqual([]);

  win.confirm = () => true;
  close().click();
  expect(document.querySelector('[data-panel="spawn"]')).toBeNull();
  expect(resets).toEqual([{ keepReusable: false }]);
});
it('docks an open draft by keyboard, which makes it persistent', () => {
  const { shell } = mount();
  document.querySelector('[data-layout-panel="spawn"][data-layout-control="switcher"]').click();
  const tab = document.querySelector('[data-panel="spawn"] .workshop-panel-tab');
  tab.focus();
  tab.dispatchEvent(new KeyboardEvent('keydown', {
    code: 'ArrowLeft', ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true }));
  const state = shell.liveLayoutState();
  expect(state.floats.some(entry => entry.panel === 'spawn')).toBe(false);
  expect(state.closed).not.toContain('spawn');
  // Docked is the one thing about a draft worth restoring.
  expect(liveLayoutModel.normalize(state).closed).not.toContain('spawn');
});
it('asks before a stored arrangement replaces an open draft', () => {
  const { shell, win } = mount();
  let dirty = true;
  shell.temporaryActions.register('spawn', {
    isDirty: () => dirty, reset: () => { dirty = false; }, keepOpen: () => false, focus: () => {},
  });
  document.querySelector('[data-layout-panel="spawn"][data-layout-control="switcher"]').click();
  win.confirm = () => false;
  expect(shell.setLiveLayout(defaultLiveLayout())).toBe(false);
  expect(document.querySelector('[data-panel="spawn"]')).not.toBeNull();
  win.confirm = () => true;
  expect(shell.setLiveLayout(defaultLiveLayout())).toBe(true);
  expect(document.querySelector('[data-panel="spawn"]')).toBeNull();
});
it('refuses a layout reset that would take an unsent draft away', () => {
  const { shell } = mount();
  let dirty = true;
  shell.temporaryActions.register('spawn', {
    isDirty: () => dirty, reset: () => { dirty = false; }, keepOpen: () => false, focus: () => {},
  });
  document.querySelector('[data-layout-panel="spawn"][data-layout-control="switcher"]').click();
  expect(document.querySelector('[data-panel="spawn"]')).not.toBeNull();
  expect(shell.temporaryActions.mayReset()).toBe(false);
});
it('carries no takeover draft or console runtime state in the stored layout', () => {
  const { shell } = mount();
  document.getElementById('gm-station-controls').hidden = false;
  shell.refresh();
  const stored = JSON.stringify(shell.liveLayoutState());
  // Placement only: the panel id is the only Station word in it.
  for (const key of ['src', 'ship', 'operator', 'pending', 'correlation', 'captain']) {
    expect(stored, key).not.toContain(key);
  }
  expect(stored).toContain('"station-console"');
  expect(stored.match(/console/g)).toHaveLength(1);
});
it('keeps the selected entity and the map alive across a rearrangement', () => {
  const { shell, selectEntity } = mount();
  shell.refresh({ entities: [{ entity_id: 'ship', name: 'Courier', kind: 'player_ship', faction: null }] },
    { ships: [{ ship_id: 'ship', stations: [{ name: 'Helm', rating: 'Backfill' }] }] });
  document.querySelector('#gm-roster-ships button').click();
  expect(selectEntity).toHaveBeenCalledWith('ship');
  shell.selection({ entity_id: 'ship', name: 'Courier', status: { systems: [] } });
  const map = document.getElementById('gm-entity-map');
  expect(map.closest('[data-panel]').dataset.panel).toBe('map');

  // Pull the map out of its column and float the roster; the selection seam is
  // the same nodes moved, not rebuilt, so it survives.
  let layout = liveLayoutModel.dock(defaultLiveLayout(), 'map', 'comms', 'tab');
  layout = liveLayoutModel.float(layout, 'roster', { x: 30, y: 40, width: 400, height: 300 });
  shell.setLiveLayout(layout);

  expect(document.getElementById('gm-entity-map')).toBe(map);
  expect(map.closest('[data-panel]').dataset.panel).toBe('map');
  expect(document.querySelector('#gm-roster-ships button').getAttribute('aria-pressed')).toBe('true');
  document.querySelector('#gm-roster-ships button').click();
  expect(selectEntity).toHaveBeenCalledTimes(2);
  expect(selectEntity).toHaveBeenLastCalledWith('ship');
  // And a reload restores placement without restoring a selection.
  const stored = shell.liveLayoutState();
  expect(JSON.stringify(stored)).not.toContain('ship');
});
it('drops the map tab when the role preset puts the map away', () => {
  const { shell } = mount();
  expect(document.querySelector('[data-panel="map"]')).not.toBeNull();
  // gui/gm-role-presets.js lists gm-map-panel in GM_ROLE_PRESET_PANEL_IDS.
  document.getElementById('gm-map-panel').hidden = true;
  shell.refresh();
  expect(document.querySelector('[data-panel="map"]')).toBeNull();
  // The node stays in the surface so the preset can bring it back.
  expect(document.getElementById('gm-map-panel').closest('.workshop-dock-parked')).not.toBeNull();
  expect(shell.liveLayoutState().closed).not.toContain('map');
  document.getElementById('gm-map-panel').hidden = false;
  shell.refresh();
  expect(document.querySelector('[data-panel="map"]')).not.toBeNull();
});
// Issue #1430, PRD #1418 story 6: the selected roster entity must read as
// more than the button's own `background: var(--gold)`, which a forced-
// colours browser is free to replace with its own system colour regardless
// of the author's choice — the same redundant-border pattern the attention
// queue, journal and checkpoint rows already use. jsdom's computed-style
// engine does not resolve `var()` inside a `border` shorthand (verified: even
// the file's PRE-EXISTING `.gm-roster-row { border-bottom: 1px solid
// var(--edge); }` computes as the unset `medium` there), so — like the
// sibling "applies the endpoint text scale exactly once" ratchet just above —
// this checks the actual shipped CSS TEXT rather than a jsdom layout it
// cannot produce; `tests/smoke/gm-layout.spec.js` is where a real engine
// proves the rendered edge.
it('gives the selected roster row a real border rule, not only the pressed button background', () => {
  const sheet = readFileSync('gui/gm-workspace.css', 'utf8');
  expect(sheet).toMatch(
    /\.gm-roster-row:has\(\s*>?\s*button\[aria-pressed="true"\]\s*\)\s*\{[^}]*border-left/,
  );
  // And the real markup this rule targets exists: a roster row whose direct
  // child is the pressed button, matched with the same `:has()` jsdom itself
  // supports for `querySelectorAll` (only its computed-style var() resolution
  // is the gap, proven above).
  const { shell } = mount();
  shell.refresh({ entities: [
    { entity_id: 'ship-a', name: 'Courier', kind: 'player_ship', faction: null },
    { entity_id: 'ship-b', name: 'Resolute', kind: 'player_ship', faction: null },
  ] }, { ships: [] });
  shell.selection({ entity_id: 'ship-a', name: 'Courier', status: { systems: [] } });
  const matched = [...document.querySelectorAll(
    '.gm-roster-row:has(> button[aria-pressed="true"])',
  )];
  expect(matched).toHaveLength(1);
  expect(matched[0].querySelector('button').dataset.entityId).toBe('ship-a');
});
const recordTab = panel => document.querySelector(`[role="tab"][data-layout-panel="${panel}"]`);
const recordFrame = panel => document.querySelector(`[data-panel="${panel}"]`);
it('keeps Comms, Activity, the action log and the session history as one default tab group', () => {
  mount();
  expect(recordTab('comms').getAttribute('aria-selected')).toBe('true');
  expect(recordTab('comms').getAttribute('aria-controls')).toBe(recordFrame('comms').id);
  expect(recordFrame('journal').getAttribute('role')).toBe('tabpanel');
  expect(recordFrame('journal').getAttribute('aria-labelledby')).toBe(recordTab('journal').id);
  // Arrow keys walk the group, exactly as the inspector's tabs do.
  recordTab('comms').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
  expect(recordFrame('activity').hidden).toBe(false);
  recordTab('journal').click();
  expect(recordFrame('journal').hidden).toBe(false);
  expect(recordTab('activity').getAttribute('aria-selected')).toBe('false');
  expect(recordTab('activity').tabIndex).toBe(-1);
  // Choosing a tab must never write `hidden` on the panels themselves: that
  // attribute belongs to the role preset (GM_ROLE_PRESET_PANEL_IDS).
  expect(document.getElementById('gm-comms-panel').hidden).toBe(false);
  expect(document.getElementById('gm-activity').hidden).toBe(false);
});
it('keeps every migrated record reachable by id whatever the arrangement', () => {
  const { shell } = mount();
  // Their contents are owned by modules that resolve them by id after the dock
  // mounts, so no arrangement may take one out of the document.
  const ids = ['gm-mission-panel', 'gm-comms-panel', 'gm-activity', 'gm-journal', 'gm-session-history',
    // gui/gm-station-puppet.js resolves every one of these by id at ITS mount,
    // which happens after the dock's.
    'gm-station-tools', 'gm-station-pending', 'gm-station-controls', 'gm-station-select',
    'gm-station-toggle', 'gm-station-surface', 'gm-station-frame', 'gm-station-activity',
    // The operator's own utilities mount into these AFTER the dock does.
    'gm-presentation-dock', 'gm-audition-dock', 'gm-source-link-dock'];
  for (const id of ids) expect(document.getElementById(id)).not.toBeNull();
  expect(document.querySelector('#gm-session-history #gm-session-log')).not.toBeNull();
  // Closed in a restored arrangement.
  shell.setLiveLayout({ ...defaultLiveLayout(), root: { type: 'tabs', tabs: ['roster'], active: 'roster' },
    closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal', 'session-history'],
    selected: 'roster' });
  for (const id of ids) {
    expect(document.getElementById(id), `${id} while closed`).not.toBeNull();
    expect(document.getElementById(id).closest('.workshop-dock-parked'), `${id} parked`).not.toBeNull();
  }
});
it('keeps every migrated record reachable by id in the narrow projection', () => {
  // The Live dock measures the viewport, not the surface, so a narrow GM window
  // frames exactly one panel and leaves every other node without one.
  mount({ innerWidth: 600 });
  expect(document.querySelectorAll('.workshop-dock-panel')).toHaveLength(1);
  for (const id of ['gm-mission-panel', 'gm-comms-panel', 'gm-activity', 'gm-journal', 'gm-session-history',
    'gm-station-tools', 'gm-station-controls', 'gm-station-select', 'gm-station-toggle',
    'gm-station-surface', 'gm-station-frame', 'gm-station-activity',
    'gm-presentation-dock', 'gm-audition-dock', 'gm-source-link-dock']) {
    expect(document.getElementById(id), `${id} while narrow`).not.toBeNull();
  }
  // The narrow switcher offers the Station tool like any other, and the console
  // once the projection carries a Station row.
  expect(document.querySelector('[data-layout-panel="station"][data-layout-control="switcher"]'))
    .not.toBeNull();
  expect(document.querySelector('[data-layout-panel="station-console"][data-layout-control="switcher"]'))
    .toBeNull();
});
it('keeps the record workflows working after the panels are rearranged', () => {
  const { shell } = mount();
  // Pull the journal out of its default group and float the Comms studio.
  let layout = liveLayoutModel.dock(defaultLiveLayout(), 'journal', 'roster', 'tab');
  layout = liveLayoutModel.float(layout, 'comms', { x: 40, y: 50, width: 400, height: 300 });
  shell.setLiveLayout(layout);
  // The draft field, its wrapper class and the bounded lists all travelled with
  // the original nodes rather than being rebuilt.
  expect(document.getElementById('gm-comms-text').closest('.gm-comms-draft')).not.toBeNull();
  expect(document.getElementById('gm-comms-text').closest('[data-panel]').dataset.panel).toBe('comms');
  expect(document.querySelector('[data-panel="comms"]').classList.contains('is-floating')).toBe(true);
  expect(document.getElementById('gm-journal-list')).not.toBeNull();
  expect(document.getElementById('gm-mission-events')).not.toBeNull();
  // A caller navigating to a record still reaches it in the new arrangement.
  expect(shell.showLog('gm-journal')).toBe(true);
  expect(document.getElementById('gm-journal').closest('[data-panel]').hidden).toBe(false);
  expect(shell.showLog('gm-comms-panel')).toBe(true);
  expect(document.getElementById('gm-comms-text').isConnected).toBe(true);
});
it('separates a record panel from its default group through docking', () => {
  const { shell } = mount();
  shell.setLiveLayout(liveLayoutModel.dock(defaultLiveLayout(), 'journal', 'roster', 'tab'));
  expect(document.getElementById('gm-journal').closest('[data-panel]').dataset.panel).toBe('journal');
  expect(recordTab('journal').closest('.workshop-tab-list')
    .contains(recordTab('roster'))).toBe(true);
  expect(recordTab('comms').closest('.workshop-tab-list')
    .contains(recordTab('journal'))).toBe(false);
});
it('drops the tab for a panel the role preset has put away and moves the operator off it', () => {
  const { shell } = mount();
  recordTab('activity').click();
  expect(recordFrame('activity').hidden).toBe(false);
  // What gui/gm-role-presets.js does to a panel the effective preset omits.
  document.getElementById('gm-activity').hidden = true;
  shell.refresh();
  expect(recordTab('activity')).toBeNull();
  expect(recordFrame('activity')).toBeNull();
  // The operator is left on a record that is still there, not on nothing.
  expect(recordFrame('comms').hidden).toBe(false);
  // The arrangement is unchanged: availability is read, never written.
  expect(shell.liveLayoutState().closed).not.toContain('activity');
  document.getElementById('gm-activity').hidden = false;
  shell.refresh();
  expect(recordTab('activity')).not.toBeNull();
});
it('brings a record panel to the front for a caller about to focus it', () => {
  const { shell } = mount();
  // The attention queue opens an authored Comms route (gui/gm-workspace.js).
  // A `hidden` panel has nothing to focus, so the shell reveals it first — for
  // the PANEL the caller names, not a dock id it has to know.
  expect(recordFrame('comms').hidden).toBe(false);
  expect(shell.showLog('gm-journal')).toBe(true);
  expect(recordFrame('journal').hidden).toBe(false);
  expect(shell.showLog('gm-comms-panel')).toBe(true);
  expect(recordFrame('comms').hidden).toBe(false);
  // A panel the role preset has put away is left alone: a preset hiding a
  // panel is a decision, not something a navigation may override.
  shell.showLog('gm-journal');
  document.getElementById('gm-comms-panel').hidden = true;
  expect(shell.showLog('gm-comms-panel')).toBe(false);
  expect(recordFrame('journal').hidden).toBe(false);
  document.getElementById('gm-comms-panel').hidden = false;
  // And a panel that is not a registered record is not this seam's business.
  expect(shell.showLog('gm-inspector')).toBe(false);
  // The desk really does wire it to the queue's own Comms navigation.
  const workspace = readFileSync('gui/gm-workspace.js', 'utf8');
  expect(workspace).toContain("shell.showLog('gm-comms-panel');");
});
it('states the workload of a crewed hull as a word beside its roster row', () => {
  const { shell } = mount({
    __hostGmWorkloadState: () => [
      { ship: { entity_id: 'ship' }, station_id: 'helm', level: 'engaged' },
      { ship: { entity_id: 'ship' }, station_id: 'comms', level: 'overloaded' },
      { ship: { entity_id: 'other' }, station_id: 'helm', level: 'backfill' },
    ],
  });
  shell.refresh({ entities: [
    { entity_id: 'ship', name: 'Courier', kind: 'player_ship', faction: null },
    { entity_id: 'other', name: 'Resolute', kind: 'player_ship', faction: null },
    { entity_id: 'quiet', name: 'Kestrel', kind: 'npc_ship', faction: null },
  ] }, { ships: [] });
  const words = [...document.querySelectorAll('#gm-roster-ships .gm-roster-workload')];
  // The worst claim about a PERSON wins for a hull with several Stations…
  expect(words.map(word => word.dataset.level)).toEqual(['overloaded', 'backfill']);
  expect(words[0].textContent).toBe('server.gm.workload.state.overloaded');
  // …and a hull the advisory published no rows for says nothing at all.
  expect(document.querySelectorAll('#gm-roster-ships .gm-roster-row')).toHaveLength(3);
});
it('repaints the roster on a changed workload word, not on the advisory ticking', () => {
  let rows = [{ ship: { entity_id: 'ship' }, station_id: 'helm', level: 'engaged', count: 2, sustained_secs: 4 }];
  const { shell } = mount({ __hostGmWorkloadState: () => rows });
  const projection = { entities: [
    { entity_id: 'ship', name: 'Courier', kind: 'player_ship', faction: null },
  ] };
  shell.refresh(projection, { ships: [] });
  const button = document.querySelector('#gm-roster-ships button');
  // gm_workload republishes about once a simulated second while a Station
  // holds demand: same level, longer sustain. Nothing a Game Master can read
  // has changed, so the button their pointer is down on must be the same node
  // — a replaced button eats the mouseup and loses a screen reader's place.
  rows = [{ ship: { entity_id: 'ship' }, station_id: 'helm', level: 'engaged', count: 2, sustained_secs: 5 }];
  shell.refresh(projection, { ships: [] });
  expect(document.querySelector('#gm-roster-ships button')).toBe(button);
  // The WORD changing is a real change, and does repaint.
  rows = [{ ship: { entity_id: 'ship' }, station_id: 'helm', level: 'overloaded', count: 3, sustained_secs: 6 }];
  shell.refresh(projection, { ships: [] });
  expect(document.querySelector('#gm-roster-ships button')).not.toBe(button);
  expect(document.querySelector('#gm-roster-ships .gm-roster-workload').dataset.level)
    .toBe('overloaded');
});
it('offers keyboard-operated comparison tabs without replacing existing comparison controls', () => {
  mount();
  document.getElementById('gm-tab-truth').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight'}));
  expect(document.getElementById('gm-comparison').hidden).toBe(false);
  expect(document.getElementById('gm-tab-crew').getAttribute('aria-selected')).toBe('true');
  expect(document.getElementById('gm-knowledge-select')).not.toBeNull();
});

it('refreshes Station control-source pills when ownership changes without a rating change', () => {
  const {shell} = mount();
  const entity = {entity_id:'ship',name:'Courier',kind:'player_ship',faction:null};
  const ship = {ship_id:'ship',stations:[{station_id:'helm',name:'Helm',rating:'Manual'}],
    ship_config:{station_systems:{helm:['drive']}},control_sources:{drive:'Human'}};
  shell.refresh({entities:[entity]},{ships:[ship]});
  expect(document.querySelector('.gm-station-pills').textContent).toContain('server.gm.shell.control.human');
  shell.refresh(null,{ships:[{...ship,control_sources:{drive:'Ai'}}]});
  expect(document.querySelector('.gm-station-pills').textContent).toContain('server.gm.shell.control.ai');
});
it('keeps the observing ship selector available without changing the action target', () => {
  const {shell,selectEntity} = mount();
  document.getElementById('gm-knowledge-panel').hidden = false;
  const chooser = document.getElementById('gm-knowledge-select');
  chooser.innerHTML = '<option value="crew">Crew ship</option>';
  shell.refresh();
  shell.selection({entity_id:'npc',name:'Courier',status:{systems:[]}});
  chooser.dispatchEvent(new Event('change'));
  expect(document.getElementById('gm-knowledge-scope').hidden).toBe(false);
  expect(chooser.closest('#gm-comparison')).toBeNull();
  expect(chooser.value).toBe('crew');
  expect(selectEntity).not.toHaveBeenCalled();
});
it('offers a sticky way back to the selection card after a quick action jumps the inspector', () => {
  mount();
  const inspector = document.getElementById('gm-inspector');
  const back = document.getElementById('gm-inspector-back');
  expect(inspector.firstElementChild).toBe(back);
  expect(back.hidden).toBe(true);
  // The scroll box is the right-hand REGION: on the post-M5 screen the
  // inspector and the checkpoints share one frame, and only the frame scrolls.
  const detail = document.getElementById('gm-desk-detail');
  detail.scrollTo = vi.fn();
  inspector.scrollTo = vi.fn();
  [...document.querySelectorAll('#gm-action-grid button')].find(b => b.getAttribute('aria-controls') === 'gm-objective-panel').click();
  expect(back.hidden).toBe(false);
  back.click();
  expect(back.hidden).toBe(true);
  expect(detail.scrollTo).toHaveBeenCalledWith({ top: 0 });
  expect(inspector.scrollTo).not.toHaveBeenCalled();
});
it('draws a bar pill only for a session fact a live payload carries', () => {
  const { shell } = mount();
  // Nothing has published yet: an empty health seed, no quiet advisory and no
  // checkpoint catalogue are three absent pills, not three blank frames.
  shell.refresh();
  expect(document.getElementById('gm-health-pills').children).toHaveLength(0);

  const { shell: live } = mount({
    __hostGmHealthState: () => ({
      worst: 'stale',
      projection: { peers: [{ id: 'peer:1' }], stations: [], operators: [], alerts: [], paused: false },
    }),
    __hostGmAttentionState: () => ({ occurrences: [
      { id: 'quiet:1', category: 'quiet_time', age_ms: 125000 },
      { id: 'comms:1', category: 'pending_comms', age_ms: 1000 },
    ] }),
    __hostGmCheckpointState: () => ({ rows: [
      { slotId: 'a', kind: 'manual', displayName: 'Before the ambush', captureTick: '38100' },
      { slotId: 'b', kind: 'manual', displayName: 'Storm front', captureTick: '46800' },
    ] }),
  }, (id, params) => (params ? `${id}|${JSON.stringify(params)}` : id));
  live.refresh();
  const pills = [...document.getElementById('gm-health-pills').children];
  expect(pills.map(pill => pill.dataset.pill)).toEqual(['tick', 'quiet', 'checkpoint']);
  // The tick pill says its condition as a WORD, with data-state as decoration.
  expect(pills[0].textContent)
    .toBe('server.gm.shell.pill.tick|{"state":"server.gm.health.state.stale"}');
  expect(pills[0].dataset.state).toBe('stale');
  // The quiet pill reads the occurrence's own age, as m:ss — the age the
  // QUEUE keeps (gui/gm-attention-panel.js `state()` ages its own rows),
  // which is the only one that still moves during a lull.
  expect(pills[1].textContent).toBe('server.gm.shell.pill.quiet|{"age":"2:05"}');
  // The LAST checkpoint is the highest capture tick, not the first row back.
  expect(pills[2].textContent)
    .toBe('server.gm.shell.pill.checkpoint|{"name":"Storm front","tick":"46800"}');
  // And the bar is repainted on the queue's age tick, because a lull publishes
  // nothing for the pill to ride in on.
  expect(readFileSync('gui/gm-workspace.js', 'utf8')).toContain('onAge: () => shell.refresh(),');
});
it('applies the endpoint text scale exactly once', () => {
  // gui/gm-workspace.css is loaded into server.html and into the native GM
  // document built from it (src/native_host/native_gm/document.rs), and both of
  // those carry `html { font-size: calc(var(--root-size-viewscreen) *
  // var(--a11y-text-scale, 1)) }` since issue #1427. That root rule is the ONE
  // lever: every `--text-*` rung is a `max(px, rem)`, so a rung already grows
  // with the setting. A `font-size: calc(<rung> * var(--a11y-text-scale))` here
  // would square it — the desk rendered at 56px instead of 28px at 200%.
  const commentless = /\/\*[\s\S]*?\*\//g;
  const sheet = readFileSync('gui/gm-workspace.css', 'utf8').replace(commentless, '');
  const scaledSizes = sheet
    .split(/[;{}]/)
    .map(declaration => declaration.trim())
    .filter(declaration => /^font-size\s*:/.test(declaration)
      && declaration.includes('var(--a11y-text-scale'));
  expect(scaledSizes).toEqual([]);
  // …and server.html really is where the multiplication happens.
  expect(source).toContain(
    'html { font-size: calc(var(--root-size-viewscreen) * var(--a11y-text-scale, 1)); }',
  );
});
