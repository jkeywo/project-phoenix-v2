/** @vitest-environment jsdom */
import { afterEach, expect, it, vi } from 'vitest';
import { personnelPayload, personnelPosting, personnelTreatment } from '../../gui/personnel.js';
import { buildRepairConsoleState } from '../../gui/console-state.js';
import { renderPersonnelControls } from '../../gui/components/personnel-controls.js';
import { createEngineeringActionRegistry } from '../../gui/stations/engineering-actions.js';
import { PERSONNEL_POST_ACTION_ID, PERSONNEL_TREAT_ACTION_ID } from '../../gui/stations/personnel-actions.js';
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
  const picks = container.querySelectorAll('select'); expect(picks[0].disabled).toBe(true); picks[1].value='1'; picks[1].dispatchEvent(new Event('change'));
  const button = [...container.querySelectorAll('button')].at(-1); button.click();
  expect(owner.sendAction).toHaveBeenCalledWith('treat_duty_officer', expect.objectContaining({officer:'chief',team_idx:1}));
  button.dispatchEvent(new Event('pointerdown'));
  renderPersonnelControls(container,{personnel:{...view,system_id:'new-repair'}},owner);button.click();expect(owner.sendAction).toHaveBeenCalledTimes(1);
});
