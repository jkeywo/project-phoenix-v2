/** @vitest-environment jsdom */
import { afterEach, expect, it, vi } from 'vitest';
import { personnelPayload, personnelPosting, personnelTreatment, personnelDestination, personnelTransfer } from '../../gui/personnel.js';
import { buildRepairConsoleState } from '../../gui/console-state.js';
import { renderPersonnelControls } from '../../gui/components/personnel-controls.js';
import { createEngineeringActionRegistry } from '../../gui/stations/engineering-actions.js';
import { PERSONNEL_POST_ACTION_ID, PERSONNEL_TREAT_ACTION_ID, PERSONNEL_DESTINATION_ACTION_ID, PERSONNEL_TRANSFER_ACTION_ID } from '../../gui/stations/personnel-actions.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import '../../gui/components/ph-repair-teams.js';
const roster = () => ({ teams: [0, 1].map(team_idx => ({ id: `team-${team_idx}`, label: `Team ${team_idx}`,
  team_idx, competencies: ['mechanical_repair'], officer: null, active_work_multiplier: 1 })),
  officers: [{ id: 'chief', label: 'Chief', competencies: ['mechanical_repair'],
    location: { kind: 'at_compartment', compartment: 'engineering' }, changed_tick: 7,
    repair_work_multiplier: 1.25, available_teams: [0, 1], can_release: false }] });
const payload = (owner = 'actual-repair') => personnelPayload(roster(), owner, true);
afterEach(() => { document.body.replaceChildren(); delete window.activateSemanticAction; });

it('requires exact owned Human Repair machinery, retaining delegated and feed-only readouts', () => {
  const state = { blackboards: { 'actual-repair': { personnel: roster() } },
    blackboardKinds: { 'actual-repair': 'Repair' }, controlSystemIds: ['actual-repair'],
    controlSources: { 'actual-repair': 'Human' } };
  expect(buildRepairConsoleState(state, ['actual-repair']).personnel).toMatchObject({ controllable: true, system_id: 'actual-repair' });
  for (const source of ['Ai', 'Offline', undefined]) {
    state.controlSources['actual-repair'] = source;
    expect(buildRepairConsoleState(state, ['actual-repair']).personnel.controllable).toBe(false);
  }
  state.controlSources['actual-repair'] = 'Human'; state.controlSystemIds = ['other'];
  expect(buildRepairConsoleState(state, ['actual-repair']).personnel.controllable).toBe(false);
});

it('refuses malformed, duplicate, unavailable and stale physical posting choices', () => {
  const initial = roster(); initial.officers[0].available_teams.push(8);
  expect(personnelPayload(initial, 'actual-repair', true)).toBeNull();
  const duplicate = roster(); duplicate.officers.push(duplicate.officers[0]);
  expect(personnelPayload(duplicate, 'actual-repair', true)).toBeNull();
  const request = { officer: 'chief', team_idx: 0, changed_tick: 7, control_system_id: 'actual-repair' };
  expect(personnelPosting(payload(), request)).toEqual({ officer: 'chief', team_idx: 0 });
  for (const bad of [{ ...request, changed_tick: 6 }, { ...request, team_idx: 8 },
    { ...request, team_idx: null }, { ...request, control_system_id: 'foreign-repair' }]) expect(personnelPosting(payload(), bad)).toBeNull();
});

it('routes one correlated accepted request to the actual owner without granting a delegated posting', () => {
  const repair = { system_id: 'actual-repair', repair_auto: false, personnel_controllable: true, personnel: payload() };
  const state = { system_ids: ['actual-repair'], system_families: { 'actual-repair': 'repair' }, systems: { 'actual-repair': repair } };
  let sequence = 0;
  const send = vi.fn(), registry = createEngineeringActionRegistry({ getState: () => state, sendAction: send,
    actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'personnel-' + (++sequence) }) });
  const request = { officer: 'chief', team_idx: 1, changed_tick: 7, control_system_id: 'actual-repair' };
  registry.activate(PERSONNEL_POST_ACTION_ID, { context: 'engineering', detail: request });
  const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
  expect(wire.mock.calls[0]).toEqual(['ControlSystemCorrelated', { correlation: 'personnel-1', target: 'actual-repair',
    payload: { type: 'SetDutyOfficerPosting', data: { officer: 'chief', team_idx: 1 } } }]);
  repair.personnel_controllable = false;
  registry.activate(PERSONNEL_POST_ACTION_ID, { context: 'engineering', detail: request });
  expect(send).toHaveBeenCalledTimes(1);
});

it('preserves native picker focus and edits across snapshots without automatically posting', () => {
  const container = document.createElement('section'); document.body.append(container);
  const host = { sendAction: vi.fn() }, state = { personnel: payload() };
  renderPersonnelControls(container, state, host);
  const select = container.querySelector('select'); select.value = '1'; select.focus(); select.dispatchEvent(new Event('change'));
  renderPersonnelControls(container, { personnel: payload() }, host);
  expect(container.querySelector('select')).toBe(select); expect(document.activeElement).toBe(select); expect(select.value).toBe('1');
  expect(host.sendAction).not.toHaveBeenCalled();
  container.querySelector('button').click();
  expect(host.sendAction).toHaveBeenCalledWith('set_duty_officer_posting', expect.objectContaining({ officer: 'chief', team_idx: 1, changed_tick: 7 }));
  renderPersonnelControls(container, { personnel: { ...payload(), controllable: false } }, host);
  expect([...container.querySelectorAll('button,select')].every(input => input.disabled)).toBe(true);
});

it('does not turn a held activation into a posting on a different Repair owner', () => {
  const container = document.createElement('section'); document.body.append(container);
  const host = { sendAction: vi.fn() };
  renderPersonnelControls(container, { personnel: payload() }, host);
  const select = container.querySelector('select'); select.value = '0'; select.dispatchEvent(new Event('change'));
  const button = container.querySelector('button'); button.dispatchEvent(new Event('pointerdown'));
  renderPersonnelControls(container, { personnel: payload('new-repair') }, host);
  button.click(); expect(host.sendAction).not.toHaveBeenCalled();
});

it('mounts the same optional officer controls inside the actual existing Repair widget', () => {
  const widget = document.createElement('ph-repair-teams'); document.body.append(widget);
  widget.state = { teams: [], personnel: payload(), jobs: [], reports: [] };
  expect(widget.shadowRoot.querySelector('#personnel-controls select')).not.toBeNull();
  widget.state = { teams: [], jobs: [], reports: [] };
  expect(widget.shadowRoot.querySelector('#personnel-controls').hidden).toBe(true);
});

it('offers care only for an actual injured patient and eligible finite team through ordinary correlated authority', () => {
  const raw = roster(); raw.officers[0].condition = 'minor'; raw.officers[0].available_teams = []; raw.officers[0].treatment_teams = [1];
  raw.teams[1].competencies.push('medical_care');
  const view = personnelPayload(raw, 'actual-repair', true);
  const detail = { officer: 'chief', team_idx: 1, changed_tick: 7, control_system_id: 'actual-repair' };
  expect(personnelTreatment(view, detail)).toEqual({ officer: 'chief', team_idx: 1 });
  for (const bad of [{ ...detail, team_idx: 0 }, { ...detail, changed_tick: 6 }, { ...detail, control_system_id: 'other' }]) expect(personnelTreatment(view,bad)).toBeNull();
  expect(personnelTreatment({ ...view, controllable: false }, detail)).toBeNull();
  const repair = { system_id: 'actual-repair', repair_auto: false, personnel_controllable: true, personnel: view };
  const state = { system_ids: ['actual-repair'], system_families: { 'actual-repair': 'repair' }, systems: { 'actual-repair': repair } };
  const send = vi.fn(), registry = createEngineeringActionRegistry({ getState: () => state, sendAction: send,
    actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'treatment-1' }) });
  registry.activate(PERSONNEL_TREAT_ACTION_ID, { context: 'engineering', detail });
  const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
  expect(wire.mock.calls[0]).toEqual(['ControlSystemCorrelated', { correlation: 'treatment-1', target: 'actual-repair', payload: {type:'TreatDutyOfficer', data:{officer:'chief',team_idx:1}} }]);
  const container = document.createElement('section'); document.body.append(container); const owner = {sendAction:vi.fn()};
  renderPersonnelControls(container, {personnel:view}, owner);
  const picks = container.querySelectorAll('select'); expect(picks[0].disabled).toBe(true); const carePicker=container.querySelector('[data-personnel-care]');carePicker.value='1';carePicker.dispatchEvent(new Event('change'));
  const button = [...container.querySelectorAll('button')].at(-1); button.click();
  expect(owner.sendAction).toHaveBeenCalledWith('treat_duty_officer', expect.objectContaining({officer:'chief',team_idx:1}));
  button.dispatchEvent(new Event('pointerdown'));
  renderPersonnelControls(container,{personnel:{...view,system_id:'new-repair'}},owner);button.click();expect(owner.sendAction).toHaveBeenCalledTimes(1);
});

const transferRoster = () => {
  const value=roster(); value.teams.forEach(team=>team.competencies.push('personnel_transport'));
  value.carried_destinations=[{craft:'actual-pod',name:'Pod',free_berths:1}];
  Object.assign(value.officers[0],{destination_request:{destination:{kind:'craft',craft:'actual-pod'},requested_tick:6},transfer_teams:[1]});
  return value;
};
it('routes destination intent and actual eligible escort through ordinary correlated Repair admission',()=>{
  const view=personnelPayload(transferRoster(),'actual-repair',true);
  const detail={officer:'chief',changed_tick:7,control_system_id:'actual-repair'};
  expect(personnelDestination(view,{...detail,destination:{kind:'craft',craft:'actual-pod'}})).toEqual({officer:'chief',destination:{kind:'craft',craft:'actual-pod'}});
  expect(personnelDestination(view,{...detail,destination:{kind:'craft',craft:'guessed-remote'}})).toBeNull();
  expect(personnelDestination(view,{...detail,changed_tick:6,destination:null})).toBeNull();
  expect(personnelTransfer(view,{...detail,team_idx:0})).toBeNull();
  expect(personnelTransfer(view,{...detail,team_idx:1})).toEqual({officer:'chief',team_idx:1});
  const repair={system_id:'actual-repair',repair_auto:false,personnel_controllable:true,personnel:view};
  const state={system_ids:['actual-repair'],system_families:{'actual-repair':'repair'},systems:{'actual-repair':repair}};
  let sequence=0;const send=vi.fn(),registry=createEngineeringActionRegistry({getState:()=>state,sendAction:send,actionFeedback:new ActionFeedbackLifecycle({correlation:()=> 'escort-'+(++sequence)})});
  registry.activate(PERSONNEL_DESTINATION_ACTION_ID,{context:'engineering',detail:{...detail,destination:{kind:'home'}}});
  registry.activate(PERSONNEL_TRANSFER_ACTION_ID,{context:'engineering',detail:{...detail,team_idx:1}});
  const wire=vi.fn();for(const [action,fields] of send.mock.calls)dispatchConsoleAction({action,...fields},wire,vi.fn());
  expect(wire.mock.calls.map(row=>row[1].payload)).toEqual([{type:'SetDutyOfficerDestination',data:{officer:'chief',destination:{kind:'home'}}},{type:'DispatchOfficerTransfer',data:{officer:'chief',team_idx:1}}]);
  repair.personnel_controllable=false;registry.activate(PERSONNEL_TRANSFER_ACTION_ID,{context:'engineering',detail:{...detail,team_idx:1}});expect(send).toHaveBeenCalledTimes(2);
});
it('keeps carried/escort readouts local and does not offer an unobserved remote berth',()=>{
  const raw=transferRoster();raw.officers[0].location={kind:'aboard_craft',craft:'actual-pod',berth:'one'};
  expect(personnelPayload(raw,'actual-repair',true)).not.toBeNull();
  const hidden=structuredClone(raw);hidden.carried_destinations=[];
  expect(personnelPayload(hidden,'actual-repair',true)).toBeNull();
  const lost=structuredClone(raw);lost.officers[0].location={kind:'lost',actor:'actual-pod'};
  expect(personnelPayload(lost,'actual-repair',true)).toBeNull();
  const escort=structuredClone(raw);escort.officers[0].location={kind:'with_transfer',actor:'actual-ship',team_idx:1,job:42};
  expect(personnelPayload(escort,'actual-repair',true)).not.toBeNull();
});
it('preserves the destination picker across publications and refuses held stale handoff activation',()=>{
  const container=document.createElement('section');document.body.append(container);const owner={sendAction:vi.fn()};
  const view=personnelPayload(transferRoster(),'actual-repair',true);renderPersonnelControls(container,{personnel:view},owner);
  const select=container.querySelector('[data-personnel-destination]');select.value='actual-pod';select.focus();
  renderPersonnelControls(container,{personnel:view},owner);expect(container.querySelector('[data-personnel-destination]')).toBe(select);expect(select.value).toBe('actual-pod');expect(document.activeElement).toBe(select);
  const buttons=[...container.querySelectorAll('button')],set=buttons[2];set.click();expect(owner.sendAction).toHaveBeenCalledWith('set_duty_officer_destination',expect.objectContaining({destination:{kind:'craft',craft:'actual-pod'}}));
  set.dispatchEvent(new Event('pointerdown'));const newer=structuredClone(view);newer.officers[0].changed_tick++;renderPersonnelControls(container,{personnel:newer},owner);set.click();expect(owner.sendAction).toHaveBeenCalledTimes(1);
});
