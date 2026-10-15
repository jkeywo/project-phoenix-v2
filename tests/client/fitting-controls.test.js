/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { registerFittingActions, FITTING_ACTION_IDS } from '../../gui/stations/fitting-actions.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import { mountSystemStatus } from '../../gui/components/ph-system-status.js';

const slot = () => ({ slot: 'beam-slot', system: 'custom-beam', installed: 'standard', desired: 'economical', phase: 'idle', progress: 0,
  can_request: true, can_dispatch: false, free_teams: [], choices: [
    { equipment: 'standard', available: 0, family: 'beam', statistics: { range: '30', damage: '40', energy: '5', active: '2', rest: '3' } },
    { equipment: 'economical', available: 1, family: 'beam', statistics: { range: '25', damage: '30', energy: '2', active: '4', rest: '1' } },
  ] });
const row = (id, fitting, extra = {}) => ({ system_id: id, display_name: id, availability: 'normal', reasons: [], delegated: false, fitting, ...extra });
const fixtures = [];
afterEach(() => { for (const widget of fixtures.splice(0)) widget.dispose(); delete window.activateSemanticAction; document.body.replaceChildren(); });

describe('permitted fitting controls', () => {
  it('routes requests to the actual capability and team commands to the reporting Repair owner', () => {
    const state = { system_status: [row('custom-beam', [slot()]), row('custom-repair', [{ ...slot(), can_request: false, can_dispatch: true, free_teams: [2] }])] };
    const send = vi.fn(); let count = 0;
    const registry = createSemanticActionRegistry({ actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'fit-' + (++count) }) });
    registerFittingActions(registry, { getState: () => state, sendAction: send });
    for (const [name, detail] of [
      ['request', { owner: 'custom-beam', slot: 'beam-slot', equipment: 'economical' }],
      ['dispatch', { owner: 'custom-repair', slot: 'beam-slot', team_idx: 2 }],
      ['cancel', { owner: 'custom-repair', slot: 'beam-slot' }],
    ]) registry.activate(FITTING_ACTION_IDS[name], { context: 'fitting', detail });
    const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
    expect(wire.mock.calls.map(call => call[1])).toEqual([
      { correlation: 'fit-1', target: 'custom-beam', payload: { type: 'RequestEquipment', data: { slot: 'beam-slot', equipment: 'economical' } } },
      { correlation: 'fit-2', target: 'custom-repair', payload: { type: 'DispatchRefit', data: { team_idx: 2, slot: 'beam-slot' } } },
      { correlation: 'fit-3', target: 'custom-repair', payload: { type: 'CancelRefit', data: { slot: 'beam-slot' } } },
    ]);
  });
  it('refuses foreign reports, delegated rows, unknown slots/equipment and unobserved team choices', () => {
    const state = { system_status: [row('custom-beam', [slot()], { delegated: true }), row('repair', [{ ...slot(), can_request: false, can_dispatch: true, free_teams: [0] }])] };
    const registry = createSemanticActionRegistry(), send = vi.fn(); registerFittingActions(registry, { getState: () => state, sendAction: send });
    const activate = (name, detail) => registry.activate(FITTING_ACTION_IDS[name], { context: 'fitting', detail });
    activate('request', { owner: 'custom-beam', slot: 'beam-slot', equipment: 'economical' });
    activate('request', { owner: 'repair', slot: 'beam-slot', equipment: 'economical' });
    activate('dispatch', { owner: 'repair', slot: 'beam-slot', team_idx: 1 });
    state.system_status[0].delegated = false;
    activate('request', { owner: 'custom-beam', slot: 'absent', equipment: 'economical' });
    activate('request', { owner: 'custom-beam', slot: 'beam-slot', equipment: 'absent' });
    expect(send).not.toHaveBeenCalled();
  });
  it('preserves a focused picker through live progress and physical status changes', () => {
    const widget = mountSystemStatus(document); fixtures.push(widget); widget.update([row('custom-beam', [slot()])]);
    document.querySelector('[data-system-id=custom-beam]').click();
    const picker = document.querySelector('[data-fitting-slot] select'); picker.value = 'standard'; picker.focus();
    widget.update([row('custom-beam', [{ ...slot(), progress: 250 }], { reasons: [{ code: 'damaged' }], availability: 'degraded' })]);
    expect(document.activeElement).toBe(picker); expect(document.querySelector('[data-fitting-slot] select')).toBe(picker); expect(picker.value).toBe('standard');
    window.activateSemanticAction = vi.fn();
    document.querySelector('[data-fitting-slot] button').click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith(FITTING_ACTION_IDS.request, expect.objectContaining({ context: 'fitting', detail: { owner: 'custom-beam', slot: 'beam-slot', equipment: 'standard' } }));
    expect(document.querySelector('[role=dialog]').textContent).toContain('Range 30');
    expect(document.querySelector('[role=dialog]').textContent).toContain('current allocation');
  });
  it('keeps delegated summaries read-only even with stale permissive flags', () => {
    const widget = mountSystemStatus(document); fixtures.push(widget); widget.update([row('custom-beam', [slot()], { delegated: true })]);
    document.querySelector('[data-delegated-summary]').click();
    const controls = [...document.querySelectorAll('[data-fitting-slot] button, [data-fitting-slot] select')];
    expect(controls.length).toBeGreaterThan(0); expect(controls.every(control => control.hidden && control.disabled)).toBe(true);
  });
  it('recovers only an observed field option into its actual local locker', () => {
    const option = { target_uuid: 'known-source', target_name: 'Derelict', slot: 'beam-slot', equipment: 'economical', destination_locker: 'support-locker' };
    const state = { system_status: [row('actual-tool', [], { equipment_recovery: { options: [option, { ...option, slot: 'reactor-slot', blocked_reason: 'major_equipment_requires_facility' }],
      selected: null, progress: 0, phase: 'idle', can_start: true, can_cancel: true } })] };
    let next = 0; const send = vi.fn(), registry = createSemanticActionRegistry({ actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'recovery-' + (++next) }) }); registerFittingActions(registry, { getState: () => state, sendAction: send });
    const activate = (name, detail) => registry.activate(FITTING_ACTION_IDS[name], { context: 'fitting', detail });
    activate('recover', { owner: 'actual-tool', target_uuid: 'hidden-source', slot: 'beam-slot', destination_locker: 'support-locker' });
    activate('recover', { owner: 'actual-tool', target_uuid: 'known-source', slot: 'reactor-slot', destination_locker: 'support-locker' });
    expect(send).not.toHaveBeenCalled();
    activate('recover', { owner: 'actual-tool', target_uuid: 'known-source', slot: 'beam-slot', destination_locker: 'support-locker' });
    activate('recover-cancel', { owner: 'actual-tool' });
    const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
    expect(wire.mock.calls[0][1]).toMatchObject({ target: 'actual-tool', payload: { type: 'RecoverEquipment', data: { target_uuid: 'known-source', slot: 'beam-slot', destination_locker: 'support-locker' } } });
    expect(wire.mock.calls[1][1]).toMatchObject({ target: 'actual-tool', payload: { type: 'CancelEquipmentRecovery', data: {} } });
    const widget = mountSystemStatus(document); fixtures.push(widget); widget.update(state.system_status); document.querySelector('[data-system-id=actual-tool]').click();
    const picker = document.querySelector('[data-equipment-recovery] select'); picker.selectedIndex = 1; picker.dispatchEvent(new Event('change')); picker.focus();
    expect(document.querySelector('[data-equipment-recovery] button').disabled).toBe(true);
    widget.update(structuredClone(state.system_status)); expect(document.activeElement).toBe(picker); expect(picker.selectedIndex).toBe(1);
    expect(document.querySelector('[data-equipment-recovery]').textContent).toContain('Major equipment requires facility work');
  });
  it('lets Escape close the status popup while a fitting picker holds focus', () => {
    const widget = mountSystemStatus(document); fixtures.push(widget); widget.update([row('custom-beam', [slot()])]);
    document.querySelector('[data-system-id=custom-beam]').click(); const picker = document.querySelector('[data-fitting-slot] select'); picker.focus();
    picker.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(document.querySelector('[role=dialog]').hidden).toBe(true);
  });
});
