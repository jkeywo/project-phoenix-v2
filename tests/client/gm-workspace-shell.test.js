// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { mountGmWorkspaceShell } from '../../gui/gm-workspace-shell.js';
const source = readFileSync('server.html', 'utf8');
const observers = [];
afterEach(() => { observers.forEach(observer => observer.disconnect()); document.body.replaceChildren(); });
function mount(extra = {}, translate = id => id) {
  const parsed = new DOMParser().parseFromString(source, 'text/html');
  document.body.innerHTML = parsed.body.innerHTML;
  document.documentElement.classList.add('phoenix-gm-page');
  const selectEntity = vi.fn();
  const win = { document, Event, MutationObserver: class extends MutationObserver {
    constructor(fn) { super(fn); observers.push(this); }
  }, __phoenixGmPage: true, ...extra };
  const shell = mountGmWorkspaceShell({doc:document,win,t:translate,has:()=>false,selectEntity});
  return {shell,selectEntity,win};
}
it('lays the desk out as the post-M5 screen and keeps the authentic iframe outside it', () => {
  mount();
  expect(document.querySelector('#gm-roster #gm-force-start-btn')).not.toBeNull();
  expect(document.getElementById('gm-roster-heading').nextElementSibling.id).toBe('gm-start-controls');
  expect(document.querySelector('#gm-inspector #gm-station-toggle')).not.toBeNull();
  expect(document.querySelector('#gm-station-surface #gm-station-frame')).not.toBeNull();
  expect(document.querySelector('#gm-inspector #gm-station-frame')).toBeNull();
  // The canvas artboard "After M5 - facilitation + operations": six regions in
  // three columns over two rows, in reading order. Three of them are panel
  // sections in their own right; three are frames holding a stack of panels.
  expect([...document.getElementById('gm-workspace').children].map(child => child.id))
    .toEqual(['gm-desk-brief', 'gm-map-panel', 'gm-desk-detail',
      'gm-mission-panel', 'gm-desk-log', 'gm-health-panel']);
  for (const child of document.getElementById('gm-workspace').children) {
    expect(child.classList.contains('gm-desk-region')).toBe(true);
  }
  // Left: what is waiting, who is flying it, and whatever the world authored.
  expect([...document.getElementById('gm-desk-brief').children].map(child => child.id))
    .toEqual(['gm-attention-panel', 'gm-roster', 'gm-workload-panel', 'gm-widgets']);
  // Right: the selected hull, then the checkpoints a live event is recovered
  // from — the restore control travels inside #gm-checkpoint.
  expect([...document.getElementById('gm-desk-detail').children].map(child => child.id))
    .toEqual(['gm-inspector', 'gm-checkpoint']);
  expect(document.querySelector('#gm-checkpoint #gm-restore-apply')).not.toBeNull();
  // Centre-bottom: one region, three views behind a tab strip.
  expect([...document.getElementById('gm-desk-log').children].map(child => child.id))
    .toEqual(['gm-desk-log-tabs', 'gm-comms-panel', 'gm-activity', 'gm-journal']);
  // The map is never reparented: moving it would cancel <ph-navigation-map>'s
  // render loop, so it keeps the grid cell it was authored into.
  expect(document.getElementById('gm-map-panel').parentElement.id).toBe('gm-workspace');
  // And the widget region starts hidden, so a scenario that authors no widget
  // leaves the left column exactly as it was.
  expect(document.getElementById('gm-widgets').hidden).toBe(true);
  // The #1437 technical-banner seam lives inside the QUEUE, not inside the
  // health panel: it has to sit beside the list a Game Master reads and filters,
  // which is the surface it exists to be un-hideable from.
  expect(document.querySelector('#gm-attention-panel #gm-attention-banners')).not.toBeNull();
  expect(document.querySelector('#gm-health-panel #gm-attention-banners')).toBeNull();
  expect(document.querySelector('#gm-roster #manual-save-panel')).not.toBeNull();
  expect(document.querySelector('#gm-roster #gm-join-controls').style.position).toBe('');
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
it('shares one centre region between Comms, Activity and the action log', () => {
  mount();
  const region = document.getElementById('gm-desk-log');
  expect(region.dataset.logView).toBe('comms');
  expect(document.getElementById('gm-log-tab-comms').getAttribute('aria-selected')).toBe('true');
  expect(document.getElementById('gm-log-tab-comms').getAttribute('aria-controls')).toBe('gm-comms-panel');
  expect(document.getElementById('gm-journal').getAttribute('role')).toBe('tabpanel');
  // Arrow keys walk the strip, exactly as the inspector's tabs do.
  document.getElementById('gm-log-tab-comms')
    .dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
  expect(region.dataset.logView).toBe('activity');
  document.getElementById('gm-log-tab-journal').click();
  expect(region.dataset.logView).toBe('journal');
  expect(document.getElementById('gm-log-tab-activity').getAttribute('aria-selected')).toBe('false');
  expect(document.getElementById('gm-log-tab-activity').tabIndex).toBe(-1);
  // Switching views must never write `hidden` on the panels themselves: that
  // attribute belongs to the role preset (GM_ROLE_PRESET_PANEL_IDS).
  expect(document.getElementById('gm-comms-panel').hidden).toBe(false);
  expect(document.getElementById('gm-activity').hidden).toBe(false);
});
it('drops the tab for a panel the role preset has put away and moves the operator off it', () => {
  const { shell } = mount();
  document.getElementById('gm-log-tab-activity').click();
  expect(document.getElementById('gm-desk-log').dataset.logView).toBe('activity');
  // What gui/gm-role-presets.js does to a panel the effective preset omits.
  document.getElementById('gm-activity').hidden = true;
  shell.refresh();
  expect(document.getElementById('gm-log-tab-activity').hidden).toBe(true);
  expect(document.getElementById('gm-desk-log').dataset.logView).toBe('comms');
  document.getElementById('gm-activity').hidden = false;
  shell.refresh();
  expect(document.getElementById('gm-log-tab-activity').hidden).toBe(false);
});
it('brings a shared-region panel to the front for a caller about to focus it', () => {
  const { shell } = mount();
  // The attention queue opens an authored Comms route (gui/gm-workspace.js).
  // A `display: none` panel has nothing to focus, so the shell resolves the
  // view first — for the PANEL the caller names, not a view id it has to know.
  expect(document.getElementById('gm-desk-log').dataset.logView).toBe('comms');
  expect(shell.showLog('gm-journal')).toBe(true);
  expect(document.getElementById('gm-desk-log').dataset.logView).toBe('journal');
  expect(shell.showLog('gm-comms-panel')).toBe(true);
  expect(document.getElementById('gm-desk-log').dataset.logView).toBe('comms');
  // A panel the role preset has put away is left alone: a preset hiding a
  // panel is a decision, not something a navigation may override.
  shell.showLog('gm-journal');
  document.getElementById('gm-comms-panel').hidden = true;
  expect(shell.showLog('gm-comms-panel')).toBe(false);
  expect(document.getElementById('gm-desk-log').dataset.logView).toBe('journal');
  document.getElementById('gm-comms-panel').hidden = false;
  // And a panel that does not live in this region is not this seam's business.
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
