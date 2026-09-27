// @vitest-environment jsdom
// Bounded presentation/protocol journeys. Native authority is checked by the
// companion Rust cases; these fixtures do not impersonate a running simulation.
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { setBaseCatalogue, setOverlayCatalogues, setLocale, t } from '../../gui/strings.js';
import { createGmObjectivePanel } from '../../gui/gm-objective-panel.js';
import { gameOverView } from '../../gui/game-over-view.js';
import { createFleetHealth } from '../../gui/fleet-health.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { renderCrewSpectator } from '../../gui/crew-spectator-view.js';
import { createTacticalActionRegistry } from '../../gui/stations/tactical-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { createGamepadInputRuntime } from '../../gui/gamepad-input.js';
import { createOperatorProfileSnapshot, serializeOperatorProfile, prepareOperatorProfileImport,
  applyOperatorProfile } from '../../gui/operator-profile.js';
import { makeWorld, makePeerFactory, leadOn, memberOn, settle } from './fleet-session-harness.js';

const base = readFileSync('assets/strings/strings.csv', 'utf8');
// Deliberately long representative German fixture, not shipped translation QA.
const de = `id,de,de_source
server.gm.objective.instance,Auftragsinstanz {instance},Instance {instance}
client.crew_spectator.title,Das Schiff Ihrer Besatzung wurde zerstört,Your ship has been destroyed
client.crew_spectator.choose,Ein anderes Schiff zur gemeinsamen Beobachtung auswählen,Choose a ship to follow
client.crew_spectator.empty,Keine verbleibenden Schiffe zur Beobachtung vorhanden.,No remaining ships to follow.
server.gm.health.state.recovering,Wiederherstellung läuft,[Restoring]
`;
beforeEach(() => { setBaseCatalogue(base); setOverlayCatalogues([{ source: 'journey-fixture', csv: de }]); });
afterEach(() => { setLocale('en'); setOverlayCatalogues([]); document.body.replaceChildren(); });

it('J2: scoped GM confirmation survives locale change and ends in a score-free report', () => {
  document.body.innerHTML = '<ul id="gm-objective-list"></ul><p id="gm-objective-empty"></p>'
    + '<div id="gm-objective-confirmation" hidden><p id="gm-objective-consequence"></p>'
    + '<button id="gm-objective-confirm"></button><button id="gm-objective-cancel"></button></div>'
    + '<p id="gm-objective-feedback"></p><ol id="gm-objective-results"></ol>';
  const submit = vi.fn(() => true);
  const panel = createGmObjectivePanel({ doc: document, t, submit,
    getOperator: () => ({ id: 'gm-one' }), getShipName: id => id,
    getOperatorName: id => id, correlation: () => 'journey-complete',
    schedule: () => 1, cancelSchedule() {} });
  const instance = (id, ship) => ({ id: `escort::${id}`, objective_id: 'escort', instance_id: id,
    label: 'world.alliance_convoy.objective.escort', text: 'world.alliance_convoy.objective.escort',
    status: 'Active', recipients: [ship], completion_members: [], progress: 0, available: true });
  const state = { objective_palette: [], objectives: [instance('lead', 'ship-a'), instance('wing', 'ship-b')],
    objective_results: [] };
  panel.update(state);
  panel.select({ kind: 'player_ship', entity_id: 'ship-b', name: 'Wing' });
  const complete = () => document.querySelector('[data-objective="escort::wing"] [data-verb="complete"]');
  complete().focus();
  setLocale('de'); panel.update(state);
  expect(document.activeElement).toBe(complete());
  expect(complete().getAttribute('aria-label')).toContain('Auftragsinstanz wing');
  complete().click(); panel.confirm();
  expect(submit).toHaveBeenCalledExactlyOnceWith({ operator_id: 'gm-one', correlation: 'journey-complete',
    objective: 'escort', scope: { instance: 'wing' }, verb: 'complete' });
  expect(panel.state().pending).not.toBeNull();
  panel.update({ ...state, objective_results: [{ action_kind: 'objective-control', target: 'escort',
    operator_id: 'gm-one', correlation: 'journey-complete', tick: 12, outcome: 'applied',
    objective_verb: 'complete', objective_instance_scope: { instance: 'wing' }, objective_recipients: [] }] });
  expect(panel.state().pending).toBeNull();
  expect(document.querySelector('#gm-objective-feedback').dataset.state).toBe('applied');
  const report = gameOverView({ phase: 'GameOver', shipDestroyed: true, report: [{ id: 'convoy_fate',
    heading: 'world.alliance_convoy.report.heading', outcome: 'world.alliance_convoy.report.one',
    state: 'partial', score: 1 }] });
  expect(report.outcome).toBe('reported');
  expect(report.rows[0]).not.toHaveProperty('score');
  expect(t(report.rows[0].outcomeId)).toContain('One transport');
});

it('J3: a GM disconnect/reconnect preserves identity while health reports recovery honestly', async () => {
  const world = makeWorld(), factories = { socket: world.socket, peer: makePeerFactory() };
  const root = document.createElement('section'); document.body.append(root);
  const view = createFleetHealth({ doc: document, root: () => root, t });
  const owner = await leadOn(world, factories, { onRoster: view.roster });
  let first, returned;
  try {
    first = await memberOn(world, factories, owner.code.suffix, { role: 'gm', name: 'Morgan' });
    const identity = first.member.operatorId, slot = first.member.slot;
    const credential = first.member.reconnectCredential;
    first.member.close(); await settle();
    view.health({ tick: 9, alerts: [], peers: [{ id: 'public-gm', operators: ['Morgan'], state: 'disconnected' }] });
    const region = root.querySelector('[role="status"]'); region.focus();
    expect(region.textContent).toContain('Disconnected');
    returned = await memberOn(world, factories, owner.code.suffix, { role: 'gm', reconnectCredential: credential });
    expect(returned.refusals).toEqual([]);
    expect([returned.member.operatorId, returned.member.slot]).toEqual([identity, slot]);
    setLocale('de');
    view.health({ tick: 10, alerts: [], peers: [{ id: 'public-gm', operators: ['Morgan'], state: 'recovering' }] });
    expect(region.textContent).toContain('Wiederherstellung läuft');
    expect(document.activeElement).toBe(region);
    expect(region.getAttribute('aria-live')).toBe('polite');
    expect(root.textContent).not.toContain(credential);
    view.health({ tick: 11, alerts: [], peers: [{ id: 'public-gm', operators: ['Morgan'], state: 'live' }] });
    expect(region.hidden).toBe(true);
  } finally { first?.member.close(); returned?.member.close(); owner.fleet.close(); }
});

it('J4: destruction, German camera selection, retarget and report keep private crew state', () => {
  const sim = new ClientSimState();
  sim.objectives = ['crew-private']; sim.blackboards = { crew: { secret: true } };
  const root = document.createElement('section'); document.body.append(root);
  const send = vi.fn();
  const state = { active: true, can_select: true, target: 'ally', ships: [
    { uuid: 'ally', name: 'Verbündetes Begleitschiff' }, { uuid: 'enemy', name: 'Harrow' }] };
  sim.apply({ type: 'CrewSpectatorState', data: state });
  setLocale('de'); renderCrewSpectator(root, sim.crewSpectator, { t, send });
  const target = root.querySelector('[data-ship-uuid="enemy"]'); target.focus(); target.click();
  expect(send).toHaveBeenCalledExactlyOnceWith('SelectCrewSpectatorTarget', { uuid: 'enemy' });
  expect(root.querySelector('[aria-pressed="true"]').dataset.shipUuid).toBe('ally');
  sim.apply({ type: 'CrewSpectatorState', data: { ...state, target: 'enemy' } });
  renderCrewSpectator(root, sim.crewSpectator, { t, send });
  expect(document.activeElement.dataset.shipUuid).toBe('enemy');
  sim.apply({ type: 'CrewSpectatorState', data: { ...state, target: 'ally', ships: state.ships.slice(0, 1) } });
  renderCrewSpectator(root, sim.crewSpectator, { t, send });
  expect(document.activeElement.dataset.shipUuid).toBe('ally');
  expect(sim.objectives).toEqual(['crew-private']); expect(sim.blackboards).toEqual({ crew: { secret: true } });
  sim.reset({ preserveAuthorityProjection: true });
  expect(sim.crewSpectator.target).toBe('ally');
  renderCrewSpectator(root, { ...state, ships: [], target: null }, { t, send });
  expect(document.activeElement).toBe(root);
  expect(root.querySelector('[role="status"]').textContent).toContain('Keine verbleibenden');
  expect(gameOverView({ phase: 'GameOver', shipDestroyed: true, report: [{ id: 'fate',
    heading: 'world.alliance_convoy.report.heading', outcome: 'world.alliance_convoy.report.zero', state: 'lost' }] }).outcome).toBe('reported');
});

it('J5: imported keyboard/gamepad profile and correlated feedback survive an authority transition', () => {
  const send = vi.fn(); let state = { strike_reserve: { charge: 12, enabled: false }, tactical_auto: false };
  const source = createTacticalActionRegistry({ getState: () => state, sendAction: send });
  source.setBinding('tactical.strike-boost', 0, { type: 'keyboard', code: 'KeyY' });
  const profile = createOperatorProfileSnapshot({ bindings: source.bindingProfile(), preferredGamepadSlot: 0 });
  const feedback = new ActionFeedbackLifecycle();
  const actions = createTacticalActionRegistry({ getState: () => state, sendAction: send, actionFeedback: feedback });
  const imported = prepareOperatorProfileImport(serializeOperatorProfile(profile), { registry: actions });
  expect(applyOperatorProfile(imported.profile, actions).status).toBe('applied');
  setLocale('de');
  actions.dispatchKeyboardEvent({ type: 'keydown', code: 'KeyY', cancelable: true, preventDefault() {} }, 'tactical');
  const detail = send.mock.calls.at(-1)[1];
  expect(detail.enabled).toBe(true); expect(state.strike_reserve.enabled).toBe(false);
  expect(feedback.get(detail.correlation).state).toBe('Pending');
  feedback.settle(detail.correlation, 'Applied');
  expect(feedback.get(detail.correlation).state).toBe('Applied');
  state = { ...state, strike_reserve: { charge: 12, enabled: true } };
  const pad = { index: 0, id: 'journey', mapping: 'standard', axes: [0, 0, 0, 0],
    buttons: Array.from({ length: 17 }, () => ({ pressed: false, value: 0 })) };
  const runtime = createGamepadInputRuntime({ getGamepads: () => [pad], getContext: () => 'tactical',
    getActions: () => actions.list('tactical'), activate: (id, detail) => actions.activate(id, detail) });
  runtime.select(imported.profile.gamepad.preferredSlot); runtime.poll();
  pad.buttons[3] = { pressed: true, value: 1 }; runtime.poll();
  expect(send.mock.calls.at(-1)[1].enabled).toBe(false);
  const count = send.mock.calls.length;
  pad.buttons[3] = { pressed: false, value: 0 }; runtime.poll();
  state = { ...state, tactical_auto: true };
  pad.buttons[3] = { pressed: true, value: 1 }; runtime.poll();
  expect(send).toHaveBeenCalledTimes(count);
});
