// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { mountGmWorkspaceShell } from '../../gui/gm-workspace-shell.js';
const source = readFileSync('server.html', 'utf8');
const observers = [];
afterEach(() => { observers.forEach(observer => observer.disconnect()); document.body.replaceChildren(); });
function mount() {
  const parsed = new DOMParser().parseFromString(source, 'text/html');
  document.body.innerHTML = parsed.body.innerHTML;
  document.documentElement.classList.add('phoenix-gm-page');
  const selectEntity = vi.fn();
  const win = { document, Event, MutationObserver: class extends MutationObserver {
    constructor(fn) { super(fn); observers.push(this); }
  }, __phoenixGmPage: true };
  const shell = mountGmWorkspaceShell({doc:document,win,t:id=>id,has:()=>false,selectEntity});
  return {shell,selectEntity};
}
it('keeps shared control identities in the seven desk regions and the authentic iframe outside them', () => {
  mount();
  expect(document.querySelector('#gm-roster #gm-force-start-btn')).not.toBeNull();
  expect(document.getElementById('gm-roster-heading').nextElementSibling.id).toBe('gm-start-controls');
  expect(document.querySelector('#gm-inspector #gm-station-toggle')).not.toBeNull();
  expect(document.querySelector('#gm-station-surface #gm-station-frame')).not.toBeNull();
  expect(document.querySelector('#gm-inspector #gm-station-frame')).toBeNull();
  // Seven since #1433 added the attention queue, and it is a first-class desk
  // region rather than a strip inside another panel.
  expect([...document.querySelectorAll('#gm-workspace > section')].map(section => section.id))
    .toEqual(['gm-roster', 'gm-map-panel', 'gm-inspector', 'gm-attention-panel',
      'gm-mission-panel', 'gm-comms-panel', 'gm-activity']);
  // The #1437 technical-banner seam lives inside it and is never a filtered row.
  expect(document.querySelector('#gm-attention-panel #gm-attention-banners')).not.toBeNull();
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
  inspector.scrollTo = vi.fn();
  [...document.querySelectorAll('#gm-action-grid button')].find(b => b.getAttribute('aria-controls') === 'gm-objective-panel').click();
  expect(back.hidden).toBe(false);
  back.click();
  expect(back.hidden).toBe(true);
  expect(inspector.scrollTo).toHaveBeenCalledWith({ top: 0 });
});
