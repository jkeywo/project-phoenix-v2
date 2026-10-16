/** @vitest-environment jsdom */
import { afterEach, expect, it, vi } from 'vitest';
import { buildHelmMovement } from '../../gui/helm-movement.js';
import { createHelmActionRegistry } from '../../gui/stations/helm-actions.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import { buildHelmConsoleState, buildConsoleView } from '../../gui/console-state.js';
import { makeHelmRender } from '../../gui/stations/helm-console.js';
import '../../gui/components/ph-helm-movement.js';

const dto = () => ({
  thrust: { system: 'main-drive', available: true, intent: { mode: 'assisted', demand: { kind: 'fraction', value: .2 } } },
  steering: { system: 'rudder', available: true, intent: { mode: 'direct', rate: 0 } },
  lateral: { system: 'side-drive', available: false, intent: { mode: 'direct', force: 0 } },
});
const track = (extra = {}) => ({ contact_id: 'opaque-observation', predicted: false, selectable: true, observed_tick: 8,
  velocity: [3, 4], identity: { name: 'Measured ship' }, ...extra });
const options = (extra = {}) => ({ systemIds: ['main-drive', 'rudder', 'side-drive'], controlSources: {
  'main-drive': 'Human', rudder: 'Ai', 'side-drive': 'Human',
}, tracks: [track()], tick: 10, ...extra });
afterEach(() => { document.body.replaceChildren(); delete window.activateSemanticAction; });

it('projects only delivered dated velocity and exact independently owned axes', () => {
  const view = buildHelmMovement(dto(), options());
  expect(view.axes.map(row => [row.system_id, row.controllable, row.available])).toEqual([
    ['main-drive', true, true], ['rudder', false, true], ['side-drive', true, false],
  ]);
  expect(view.contacts).toEqual([{ contact: 'opaque-observation', name: 'Measured ship', speed: 5, observed_tick: 8, age: 2 }]);
  expect(buildHelmMovement(dto(), options({ systemIds: [] })).axes.every(row => !row.controllable)).toBe(true);
  expect(buildHelmMovement(null, options())).toBeNull();
  const malformed = dto(); malformed.steering.system = malformed.thrust.system;
  expect(buildHelmMovement(malformed, options())).toBeNull();
});

it('withholds missing/future/predicted/conflicting motion and retains original Match age', () => {
  const held = dto(); held.thrust.intent = { mode: 'assisted', demand: { kind: 'match', contact: 'held', speed: 7, observed_tick: 2 } };
  const view = buildHelmMovement(held, options({ tracks: [track({ predicted: true }), track({ selectable: false }),
    track({ observed_tick: 11 }), track({ velocity: null }), track({ velocity: [6, 8] }), track()] }));
  expect(view.contacts).toEqual([]);
  expect(view.match).toEqual({ contact: 'held', speed: 7, observed_tick: 2, age: 8, observation_available: false });
  held.thrust.intent.demand.observed_tick = 11;
  expect(buildHelmMovement(held, options())).toBeNull();
});

it('finds the actual delivered motion owner separately from instrument boards and renders mixed authority', () => {
  const state = { blackboardKinds: { 'a-radar': 'Helm', 'z-motion': 'Helm', science: 'SensorRadar' },
    blackboards: { 'a-radar': { tracks: [] }, 'z-motion': { movement: dto() }, science: { tracks: [track()] } },
    observationTick: 10, controlSources: options().controlSources };
  const projected = buildHelmConsoleState(state, options().systemIds);
  expect(projected.helm_movement.contacts[0].speed).toBe(5);
  expect(projected.thrust_system_id).toBe('main-drive');
  document.body.innerHTML = '<ph-helm-movement id="helm-movement"></ph-helm-movement><div id="stick"></div><div id="side"></div>';
  const render = makeHelmRender({ ids: { joystick: 'stick', lateral: 'side' } }); render(projected, document);
  expect(document.getElementById('helm-movement').state).toEqual(projected.helm_movement);
  expect(document.getElementById('stick').state.auto).toBe(false); expect(document.getElementById('side').state.auto).toBe(false);
  const readonly = buildHelmConsoleState(state, []); render(readonly, document);
  expect(document.getElementById('stick').state.auto).toBe(true);
});

it('passes SensorRadar velocity to Match only through the exact outer Console typed feed', () => {
  const owned = ['helm', ...options().systemIds], definition = { id: 'bridge-helm', home_station: 'helm', systems: owned, feeds: [] };
  const state = { consoleDefinitions: [definition], systemConsoleFamilies: Object.fromEntries(owned.map(id => [id, 'helm'])),
    blackboardKinds: { helm: 'Helm', science: 'SensorRadar' },
    blackboards: { helm: { movement: dto(), tracks: [] }, science: { tracks: [track()] } },
    observationTick: 10, controlSources: options().controlSources,
    asteroids: [{ uuid: 'SECRET', velocity: [50,50], forward_speed: 999 }] };
  expect(buildConsoleView('bridge-helm', state).helm_movement.contacts).toEqual([]);
  definition.feeds = [{ source: 'science', kind: 'SensorRadar' }];
  expect(buildConsoleView('bridge-helm', state).helm_movement.contacts).toMatchObject([{ contact: 'opaque-observation', speed: 5 }]);
  definition.feeds = [{ source: 'science', kind: 'TacticalRadar' }];
  expect(buildConsoleView('bridge-helm', state).helm_movement.contacts).toEqual([]);
});

it('admits semantic detail through exact current owner and correlated normal wire commands', () => {
  let view = { helm_movement: buildHelmMovement(dto(), options()), thrust_system_id: 'main-drive', steering_system_id: 'rudder', helm_auto: true };
  const wire = vi.fn(); let serial = 0;
  const registry = createHelmActionRegistry({ getState: () => view,
    sendAction: (action, payload) => dispatchConsoleAction({ action, ...payload }, wire),
    actionFeedback: { press: () => ({ correlation: 'movement-' + (++serial), inputMs: serial }), pending() {}, cancel() {}, settle() {} } });
  const activate = (id, detail) => registry.activate('helm.' + id, { context: 'helm', detail });
  expect(activate('mode', { systemId: 'main-drive', mode: 'direct' })).toMatchObject({ handled: true });
  expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ target: 'main-drive', correlation: 'movement-1', payload: { type: 'SetHelmMode', data: { mode: 'direct' } } }));
  expect(activate('desired-speed', { systemId: 'main-drive', speed: 12 })).toMatchObject({ handled: true });
  expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ target: 'main-drive', payload: { type: 'SetDesiredSpeed', data: { speed: 12 } } }));
  expect(activate('match-speed', { systemId: 'main-drive', contact: 'opaque-observation' })).toMatchObject({ handled: true });
  expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated', expect.objectContaining({ target: 'main-drive', payload: { type: 'SetMatchSpeed', data: { contact: 'opaque-observation' } } }));
  const count = wire.mock.calls.length;
  for (const [id, detail] of [['mode', { systemId: 'rudder', mode: 'direct' }], ['mode', { systemId: 'missing', mode: 'direct' }],
    ['desired-heading', { systemId: 'main-drive', heading_rad: 1 }], ['desired-speed', { systemId: 'main-drive', speed: Infinity }],
    ['match-speed', { systemId: 'main-drive', contact: 'private-world-uuid' }]]) expect(activate(id, detail).handled).toBe(false);
  view = { ...view, helm_movement: { ...view.helm_movement, contacts: [] } };
  expect(activate('match-speed', { systemId: 'main-drive', contact: 'opaque-observation' }).handled).toBe(false);
  expect(wire).toHaveBeenCalledTimes(count);
  expect(activate('match-speed', { systemId: 'main-drive', contact: null }).handled).toBe(true);
});

it('keeps native pickers and input drafts focused, isolates keys, and never toggles another operator axis', () => {
  const activate = vi.fn(); window.activateSemanticAction = activate;
  const element = document.createElement('ph-helm-movement'); document.body.append(element);
  const view = buildHelmMovement(dto(), options()); element.state = view;
  const root = element.shadowRoot, thrust = root.querySelector('[data-system-id="main-drive"]');
  expect(root.querySelector('output').textContent).toContain('20%');
  thrust.click(); expect(activate).toHaveBeenLastCalledWith('helm.mode', expect.objectContaining({ detail: { systemId: 'main-drive', mode: 'direct' } }));
  expect(root.querySelector('[data-system-id="rudder"]').disabled).toBe(true);
  const select = root.getElementById('contact'); select.value = 'opaque-observation'; select.dispatchEvent(new Event('change'));
  select.focus(); element.state = structuredClone(view); expect(root.getElementById('contact')).toBe(select); expect(root.activeElement).toBe(select); expect(select.value).toBe('opaque-observation');
  root.getElementById('match').click(); expect(activate).toHaveBeenLastCalledWith('helm.match-speed', expect.objectContaining({ detail: { systemId: 'main-drive', contact: 'opaque-observation' } }));
  const input = root.getElementById('speed'); input.value = '17'; input.focus(); element.state = structuredClone(view);
  expect(input.value).toBe('17'); expect(root.activeElement).toBe(input);
  const key = vi.fn(); document.addEventListener('keydown', key);
  input.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyW', bubbles: true, composed: true })); expect(key).not.toHaveBeenCalled(); document.removeEventListener('keydown', key);
  root.getElementById('set-speed').click(); expect(activate).toHaveBeenLastCalledWith('helm.desired-speed', expect.objectContaining({ detail: { systemId: 'main-drive', speed: 17 } }));
  element.state = { ...view, contacts: [] }; expect(root.getElementById('match').disabled).toBe(true);
  const held = dto(); held.thrust.intent = { mode: 'assisted', demand: { kind: 'match', speed: 7, contact: 'held', observed_tick: 2 } };
  element.state = buildHelmMovement(held, options({ tracks: [] }));
  expect(root.querySelector('output').textContent).toContain('7.0'); expect(root.getElementById('held').textContent).toContain('age 8');
});

it('explicit per-axis Navigation follow uses only current own permitted generation and manual authority',()=>{
 const raw=dto();raw.navigation_generation=7;raw.navigation_axes=['main-drive'];
 let projected=buildHelmMovement(raw,options());const send=vi.fn();
 let serial=0;const actions=createHelmActionRegistry({getState:()=>({helm_movement:projected}),sendAction:send,actionFeedback:{press:()=>({correlation:'follow-'+(++serial),inputMs:serial}),pending(){},cancel(){},settle(){}}});
 const choose=(generation=7,systemId='main-drive')=>actions.activate('helm.follow-navigation',{context:'helm',detail:{systemId,generation,leader:'private',position:[999,999]}});
 expect(choose()).toMatchObject({handled:true});expect(send).toHaveBeenCalledWith('set_helm_follow_navigation',expect.objectContaining({control_system_id:'main-drive',generation:7}));
 const packet=send.mock.calls[0][1];const wire=vi.fn();dispatchConsoleAction({action:'set_helm_follow_navigation',...packet},wire);
 expect(wire).toHaveBeenCalledWith('ControlSystemCorrelated',expect.objectContaining({target:'main-drive',payload:{type:'SetFollowNavigation',data:{generation:7}}}));
 send.mockClear();choose(6);choose(7,'rudder');choose(null);
 projected={...projected,navigation_generation:8};choose();expect(send).not.toHaveBeenCalled();
 raw.thrust.intent={mode:'assisted',demand:{kind:'navigation',generation:7}};projected=buildHelmMovement(raw,options());
 expect(choose(null)).toMatchObject({handled:true});expect(send).toHaveBeenCalledWith('set_helm_follow_navigation',expect.objectContaining({generation:null}));
});
it('HUD captures the selected generation and rejects a newly changed course during a held click',()=>{
 const raw=dto();raw.navigation_generation=7;raw.navigation_axes=['main-drive'];const widget=document.createElement('ph-helm-movement');document.body.append(widget);
 window.activateSemanticAction=vi.fn();widget.state=buildHelmMovement(raw,options());const follow=widget.shadowRoot.querySelector('[data-follow-navigation="main-drive"]');
 expect(follow.disabled).toBe(false);follow.dispatchEvent(new Event('pointerdown'));
 raw.navigation_generation=8;widget.state=buildHelmMovement(raw,options());follow.click();expect(window.activateSemanticAction).not.toHaveBeenCalled();
 follow.click();expect(window.activateSemanticAction).toHaveBeenCalledWith('helm.follow-navigation',expect.objectContaining({detail:{systemId:'main-drive',generation:8}}));
 window.activateSemanticAction.mockClear();widget.state=buildHelmMovement(raw,options({systemIds:[]}));follow.click();expect(window.activateSemanticAction).not.toHaveBeenCalled();
});

it('keeps an older retained follow demand visibly paused when a new available course is published', () => {
  const raw = dto();
  raw.navigation_generation = 8;
  raw.navigation_axes = ['main-drive'];
  raw.thrust.intent = { mode: 'assisted', demand: { kind: 'navigation', generation: 7 } };
  const widget = document.createElement('ph-helm-movement');
  document.body.append(widget);
  widget.state = buildHelmMovement(raw, options());
  const root = widget.shadowRoot;
  const heldStatus = root.querySelector('output').textContent;
  const stop = root.querySelector('[data-follow-navigation="main-drive"]');
  expect(heldStatus).toContain('paused');
  expect(stop.disabled).toBe(false);
  expect(stop.getAttribute('aria-pressed')).toBe('true');
  raw.thrust.intent.demand.generation = 8;
  widget.state = buildHelmMovement(raw, options());
  expect(root.querySelector('output').textContent).toContain('Following');
  expect(root.querySelector('output').textContent).not.toBe(heldStatus);
});
