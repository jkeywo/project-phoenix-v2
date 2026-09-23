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
it('docks the desk when the page BECOMES a Game Master page, not only when it loaded as one', async () => {
  // The Host as GM landing route raises the Game Master request in place and
  // deliberately does NOT reload — `requestStandaloneGameMaster` commits the
  // flag before taking the role precisely so the press does not pull the page
  // out from under the picker it is opening. A shell that read the flag once,
  // at mount, therefore left that route with an undocked desk: the flag up,
  // `.phoenix-gm-page` on the root, and an empty #gm-live-layout.
  const parsed = new DOMParser().parseFromString(source, 'text/html');
  document.body.innerHTML = parsed.body.innerHTML;
  const win = { document, Event, MutationObserver: class extends MutationObserver {
    constructor(fn) { super(fn); observers.push(this); }
  }, __phoenixGmPage: false };
  const shell = mountGmWorkspaceShell({ doc: document, win, t: id => id, has: () => false,
    selectEntity: vi.fn() });
  mountedShells.push(shell);
  // A ship page has no dock, which is what it always had.
  expect(document.querySelector('#gm-live-layout [data-panel]')).toBeNull();
  expect(shell.liveLayoutState()).toBeNull();

  // The stored arrangement is asked for ONCE, before the route commits. It has
  // to be what gets mounted when the flag goes up, or a late dock would come
  // back as a default and silently lose the operator's own arrangement.
  const stored = liveLayoutModel.close(defaultLiveLayout(), 'checkpoint');
  shell.mountLiveLayout(stored, () => {});
  expect(document.querySelector('#gm-live-layout [data-panel]')).toBeNull();

  // Exactly what `__phoenixRequestGameMaster` does.
  win.__phoenixGmPage = true;
  document.documentElement.classList.add('phoenix-gm-page');
  await vi.waitFor(() => expect(document.querySelector('#gm-live-layout [data-panel]')).not.toBeNull());
  expect(shell.liveLayoutState().closed).toContain('checkpoint');
  // The dock's own stylesheet arrives with it rather than never at all.
  expect([...document.head.querySelectorAll('link')].some(l => l.href.includes('dock-layout.css')))
    .toBe(true);
});

it('updates compact preset buttons when authored options arrive after mount', async () => {
  const { shell } = mount();
  const select = document.getElementById('gm-role-preset-select');
  select.append(new Option('Tactical', 'tactical'));
  const button = () => document.querySelector('#gm-role-preset-select + .gm-segment [data-value="tactical"]');
  await vi.waitFor(() => expect(button()?.textContent).toBe('Tactical'));
  button().click();
  expect(select.value).toBe('tactical');
  expect(button().getAttribute('aria-pressed')).toBe('true');
  select.options[select.selectedIndex].textContent = 'Updated tactical';
  await vi.waitFor(() => expect(button()?.textContent).toBe('Updated tactical'));
  shell.dispose();
  select.options[select.selectedIndex].textContent = 'After disposal';
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(button().textContent).toBe('Updated tactical');
});

it('lays the desk out as one region: the dock workspace IS the desk', () => {
  mount();
  // Issues #1502-#1509 migrated every panel the desk held into the dock.
  expect([...document.getElementById('gm-workspace').children].map(child => child.id))
    .toEqual(['gm-desk-brief']);
  // It is deliberately not a panel frame, so it adds no padding, border, chrome
  // or second scroll box around the dock.
  expect(document.getElementById('gm-desk-brief').classList.contains('gm-desk-region')).toBe(false);
  expect(document.getElementById('gm-desk-brief').classList.contains('gm-desk-dock')).toBe(true);
  expect([...document.getElementById('gm-desk-brief').children].map(child => child.id))
    .toEqual(['gm-live-layout']);
  expect(document.getElementById('gm-desk-detail')).toBeNull();
  expect(document.getElementById('gm-desk-log')).toBeNull();
  // Restore left the checkpoint record to become a draft of its own.
  expect(document.querySelector('#gm-checkpoint #gm-restore-apply')).toBeNull();
  expect(document.getElementById('gm-restore-apply')).not.toBeNull();
});

it('uses compact categorised window menus instead of a master tab strip', () => {
  const labels = {
    'server.gm.shell.layout.menu.session': 'Session',
    'server.gm.shell.layout.menu.crew': 'Crew',
    'server.gm.shell.layout.menu.communications': 'Communications',
    'server.gm.shell.layout.menu.world': 'World',
    'server.gm.shell.layout.menu.inspect': 'Inspect',
    'server.gm.shell.layout.menu.layout': 'Layout',
    'server.gm.shell.layout.panel.entity_fields': 'Entity fields',
  };
  mount({}, id => labels[id] || id);
  const bar = document.querySelector('.workshop-panel-switcher.is-menu-bar');
  expect([...bar.querySelectorAll(':scope > details > summary')].map(node => node.textContent))
    .toEqual(['Session', 'Crew', 'Communications', 'World', 'Inspect', 'Layout']);
  expect(bar.querySelector(':scope > [data-layout-control="switcher"]')).toBeNull();
  expect(bar.querySelector('[data-layout-panel="entity-fields"]').textContent).toBe('Entity fields');
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
it('leaves a refused restore draft open, and keeps a kept-open one', () => {
  const { shell } = mount();
  let dirty = false;
  let keep = false;
  shell.temporaryActions.register('restore', {
    isDirty: () => dirty, reset: () => { dirty = false; }, keepOpen: () => keep, focus: () => {},
  });
  document.querySelector('[data-layout-panel="restore"][data-layout-control="switcher"]').click();
  expect(document.querySelector('[data-panel="restore"]')).not.toBeNull();

  // A refusal is not a success, so nothing closes it: only `succeeded` does.
  dirty = true;
  expect(document.querySelector('[data-panel="restore"]')).not.toBeNull();

  // Keep open holds it through a landed one.
  keep = true;
  expect(shell.temporaryActions.succeeded('restore')).toBe(false);
  expect(document.querySelector('[data-panel="restore"]')).not.toBeNull();

  keep = false;
  expect(shell.temporaryActions.succeeded('restore')).toBe(true);
  expect(document.querySelector('[data-panel="restore"]')).toBeNull();
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
it('offers keyboard-operated comparison tabs without replacing existing comparison controls', () => {
  mount();
  document.getElementById('gm-tab-truth').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight'}));
  expect(document.getElementById('gm-comparison').hidden).toBe(false);
  expect(document.getElementById('gm-tab-crew').getAttribute('aria-selected')).toBe('true');
  expect(document.getElementById('gm-knowledge-select')).not.toBeNull();
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
it('opens the draft a quick action points into rather than pointing at nothing', () => {
  const { shell } = mount();
  const opened = [];
  shell.temporaryActions.register('effect', {
    isDirty: () => false, reset: options => opened.push(options),
    keepOpen: () => false, focus: () => {},
  });
  // A draft that is closed is not a panel the operator put away: a draft
  // nobody opened is simply not there, so the shortcut opens it.
  expect(document.querySelector('[data-panel="effect"]')).toBeNull();
  [...document.querySelectorAll('#gm-action-grid button')]
    .find(button => button.getAttribute('aria-controls') === 'gm-effect-amount').click();
  expect(document.querySelector('[data-panel="effect"]')).not.toBeNull();
  expect(opened).toEqual([{ keepReusable: false }]);
  // Inspector tools now open on demand as popups too.
  shell.setLiveLayout(liveLayoutModel.select(shell.liveLayoutState(), 'contact'));
  expect(document.getElementById('gm-system-select').closest('[data-panel]')).toBeNull();
  [...document.querySelectorAll('#gm-action-grid button')]
    .find(button => button.getAttribute('aria-controls') === 'gm-system-select').click();
  expect(document.getElementById('gm-system-select').closest('[data-panel]').hidden).toBe(false);
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



const get = id => document.getElementById(id);
function openPanel(panel) {
  document.querySelector('.workshop-panel-switcher [data-layout-panel="' + panel + '"]').click();
}
it('opens exactly Entity Tree, Map, Inspector and Activity by default', () => {
  const { shell } = mount();
  expect([...document.querySelectorAll('.workshop-dock-panel[data-panel]')].map(node => node.dataset.panel).sort())
    .toEqual(['activity', 'inspector', 'map', 'roster']);
  expect(shell.liveLayoutState().closed).toContain('manual-save');
  expect(get('gm-station-frame').getAttribute('src')).toBeNull();
});
it('shows critical warnings in the fixed header while Session remains closed', () => {
  const { shell } = mount();
  expect(get('gm-attention-banners').closest('.gm-station-bar')).not.toBeNull();
  shell.revealBanners();
  expect(shell.liveLayoutState().closed).toContain('readiness');
});
it('composes readiness, join requests, GM presence and peer health in Session', () => {
  mount(); openPanel('readiness');
  const session = document.querySelector('[data-panel="readiness"]');
  for (const id of ['gm-start-controls', 'gm-join-controls', 'gm-roster-operators', 'gm-health-panel']) {
    expect(session.contains(get(id)), id).toBe(true);
  }
  for (const retired of ['join', 'health', 'station', 'objective', 'journal', 'session-history']) {
    expect(document.querySelector('.workshop-panel-switcher [data-layout-panel="' + retired + '"]')).toBeNull();
  }
});
it('retains original controls and listeners when saved layout mounts again', () => {
  const { shell } = mount();
  const ready = get('gm-ready-btn'), manual = get('manual-save-panel'), health = get('gm-health-panel');
  const clicked = vi.fn(); ready.addEventListener('click', clicked);
  shell.mountLiveLayout(defaultLiveLayout());
  openPanel('readiness'); get('gm-ready-btn').click();
  expect(clicked).toHaveBeenCalledOnce();
  expect(get('gm-ready-btn')).toBe(ready);
  expect(get('manual-save-panel')).toBe(manual);
  expect(get('gm-health-panel')).toBe(health);
});
it('merges mission events and objectives, preserving both presenters', () => {
  mount(); openPanel('mission');
  const frame = get('gm-objective-panel').closest('[data-panel]');
  expect(frame.dataset.panel).toBe('mission');
  expect(frame.contains(get('gm-mission-panel'))).toBe(true);
});
it('keeps three distinct Activity sources selectable within one pane', () => {
  mount();
  const activity = get('gm-activity-dock');
  const filter = activity.querySelector('select');
  filter.value = 'gm-journal'; filter.dispatchEvent(new Event('change'));
  expect(get('gm-journal').hidden).toBe(false);
  expect(get('gm-activity').hidden).toBe(true);
  expect(get('gm-session-history').hidden).toBe(true);
  filter.value = 'all'; filter.dispatchEvent(new Event('change'));
  expect(get('gm-session-history').hidden).toBe(false);
});
it('opens detail tools as dockable popups rather than initial tabs', () => {
  const { shell } = mount();
  for (const panel of ['contact', 'npc', 'system', 'despawn', 'faction', 'entity-fields']) {
    expect(shell.liveLayoutState().closed).toContain(panel);
    openPanel(panel);
    expect(document.querySelector('[data-panel="' + panel + '"]').classList.contains('is-floating')).toBe(true);
  }
});
it('closes manual save only on confirmed success and honours Keep open', () => {
  const { shell } = mount(); openPanel('manual-save');
  expect(shell.liveLayoutState().closed).not.toContain('manual-save');
  get('manual-save-panel').dispatchEvent(new Event('gm-save-confirmed', { bubbles: true }));
  expect(shell.liveLayoutState().closed).toContain('manual-save');
  openPanel('manual-save');
  document.querySelector('[data-popup-keep="manual-save"]').checked = true;
  get('manual-save-panel').dispatchEvent(new Event('gm-save-confirmed', { bubbles: true }));
  expect(shell.liveLayoutState().closed).not.toContain('manual-save');
});
it('places explicit takeover and feedback alongside the authentic station frame', () => {
  const { shell } = mount(); get('gm-station-controls').hidden = false; shell.refresh();
  openPanel('station-console');
  const surface = get('gm-station-surface');
  for (const id of ['gm-station-frame', 'gm-station-toggle', 'gm-station-status', 'gm-station-select']) {
    expect(surface.contains(get(id)), id).toBe(true);
  }
  expect(get('gm-inspector').contains(surface)).toBe(false);
});
it('uses compact icon chrome without a Float button', () => {
  mount();
  const stack = document.querySelector('[data-panel="roster"]').closest('.workshop-tab-stack');
  expect(stack.querySelector('[data-layout-control="float"]')).toBeNull();
  expect(stack.querySelector('[data-layout-control="close"]').textContent).toBe('×');
  expect(stack.querySelector('.workshop-dock-panel > .workshop-panel-header')).toBeNull();
});
it('selects entities through the shared selection seam and stations in observation mode', () => {
  const station = vi.fn();
  const { shell, selectEntity } = mount({ __hostGmFocusStation: station });
  const entity = { entity_id: 'ship', name: 'Resolute', kind: 'player_ship', faction: null, status: { systems: [] } };
  shell.refresh({ entities: [entity] }, { ships: [{ ship_id: 'ship', stations: [{ station_id: 'helm', name: 'Helm', rating: 'Backfill' }] }] });
  const button = text => [...get('gm-roster-ships').querySelectorAll('button')].find(node => node.textContent === text);
  button('Resolute').click(); expect(selectEntity).toHaveBeenCalledWith('ship');
  button('Helm · Backfill').click(); expect(station).toHaveBeenCalledWith('ship', 'helm');
});
it('lets an empty player-slot tree node fill with AI before Start', () => {
  const submitted = vi.fn(() => true);
  const { shell } = mount({ __hostLocalGm: () => ({ id: 'gm-1' }), __hostBackfillShipSlot: submitted });
  shell.metadata({ gms: [], ship_slots: [{ id: 'wing', label: 'Wing slot', state: 'empty', can_backfill: true }] });
  [...get('gm-roster-ships').querySelectorAll('button')].find(node => node.textContent === 'Wing slot').click();
  get('gm-tree-inspection').querySelector('button').click();
  expect(submitted).toHaveBeenCalledWith(expect.objectContaining({ operator_id: 'gm-1', slot: 'wing' }));
});
it('does not rebuild unchanged tree rows on every projection', () => {
  const { shell } = mount();
  const payload = { entities: [{ entity_id: 'ship', name: 'Resolute', faction: null, kind: 'player_ship' }] };
  shell.refresh(payload);
  const row = [...get('gm-roster-ships').querySelectorAll('button')].find(node => node.textContent === 'Resolute');
  shell.refresh({ entities: [{ ...payload.entities[0], position: [5, 0, 0] }] });
  expect([...get('gm-roster-ships').querySelectorAll('button')].find(node => node.textContent === 'Resolute')).toBe(row);
});
