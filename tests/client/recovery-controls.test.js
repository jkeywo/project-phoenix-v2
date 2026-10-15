/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createEngineeringActionRegistry } from '../../gui/stations/engineering-actions.js';
import { RECOVERY_ACTION_IDS } from '../../gui/stations/recovery-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { buildRecoveryConsoleState } from '../../gui/console-state.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import '../../gui/components/ph-recovery-controls.js';
const candidate = { uuid: 'known-pod', name: 'Cargo pod', distance: 6, resources: ['scrap', 'machinery_parts'] };
const view = () => ({ system_id: 'actual-recovery', enabled: false, candidates: [candidate],
  destinations: [{ id: 'hold-scrap', resource: 'scrap', role: 'main', level: 0, incoming: 0, held: 0, target: 12, headroom: 24 },
    { id: 'local-scrap', resource: 'scrap', role: 'local' }], fraction: 0.2, last_moved: 1 });
afterEach(() => { delete window.activateSemanticAction; document.body.replaceChildren(); });
describe('known-contact finite recovery', () => {
  it('sends one correlated atomic selection to the actual owner, and refuses partial, unknown and local-store intent', () => {
    const state = { system_ids: ['actual-recovery'], system_families: { 'actual-recovery': 'recovery' }, systems: { 'actual-recovery': view() } };
    let sequence = 0; const send = vi.fn(), registry = createEngineeringActionRegistry({ getState: () => state, sendAction: send,
      actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'recovery-' + (++sequence) }) });
    const activate = detail => registry.activate(RECOVERY_ACTION_IDS.selection, { context: 'engineering', detail });
    activate({ target_uuid: candidate.uuid, resource: 'scrap', destination_store: 'hold-scrap' });
    const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
    expect(wire.mock.calls[0][1]).toMatchObject({ target: 'actual-recovery', payload: { type: 'SetRecoverySelection', data: { target_uuid: candidate.uuid, resource: 'scrap', destination_store: 'hold-scrap' } } });
    for (const detail of [{ target_uuid: candidate.uuid, resource: null, destination_store: null },
      { target_uuid: 'hidden-contact', resource: 'scrap', destination_store: 'hold-scrap' },
      { target_uuid: candidate.uuid, resource: 'scrap', destination_store: 'local-scrap' }]) activate(detail);
    expect(send).toHaveBeenCalledTimes(1);
    activate({ target_uuid: null, resource: null, destination_store: null }); expect(send).toHaveBeenCalledTimes(2);
    state.systems['actual-recovery'].recovery_controllable = false;
    registry.activate(RECOVERY_ACTION_IDS.enabled, { context: 'engineering', detail: { enabled: true } }); expect(send).toHaveBeenCalledTimes(2);
  });
  it('preserves focused native selects and local complete-selection drafts through distance and progress snapshots', () => {
    window.activateSemanticAction = vi.fn(); const control = document.createElement('ph-recovery-controls'); document.body.append(control); control.state = view();
    control.target.value = candidate.uuid; control.target.dispatchEvent(new Event('change'));
    control.resource.value = 'scrap'; control.resource.dispatchEvent(new Event('change'));
    const target = control.target, option = target.options[1], resource = control.resource; resource.focus();
    control.state = { ...view(), candidates: [{ ...candidate, distance: 5.9 }], fraction: 0.8 };
    expect(control.target).toBe(target); expect(target.options[1]).toBe(option); expect(control.shadowRoot.activeElement).toBe(resource);
    expect(target.value).toBe(candidate.uuid); expect(control.applySelection.disabled).toBe(false);
    expect([...control.destination.options].map(row => row.value)).toEqual(['hold-scrap']);
    expect(control.resource.options[0].textContent).toBe('Scrap'); expect(control.resource.options[1].textContent).not.toContain('machinery_parts');
    expect(control.destination.options[0].textContent).toBe('Scrap main hold');
    control.applySelection.click(); expect(window.activateSemanticAction).toHaveBeenLastCalledWith(RECOVERY_ACTION_IDS.selection,
      expect.objectContaining({ detail: { target_uuid: candidate.uuid, resource: 'scrap', destination_store: 'hold-scrap' } }));
    control.clearSelection.click(); expect(window.activateSemanticAction).toHaveBeenLastCalledWith(RECOVERY_ACTION_IDS.selection,
      expect.objectContaining({ detail: { target_uuid: null, resource: null, destination_store: null } }));
    control.state = { ...view(), recovery_auto: true }; expect([...control.shadowRoot.querySelectorAll('select,button')].every(row => row.disabled)).toBe(true);
  });
  it('derives controls only from typed owned Human publications, and keeps unknown candidates unavailable', () => {
    const state = { blackboards: { 'actual-recovery': view(), foreign: { candidates: [{ uuid: 'hidden' }] } },
      blackboardKinds: { 'actual-recovery': 'Recovery', foreign: 'Recovery' }, controlSources: { 'actual-recovery': 'Human' } };
    expect(buildRecoveryConsoleState(state, ['actual-recovery'])).toMatchObject({ system_id: 'actual-recovery', recovery_controllable: true, candidates: [candidate] });
    state.controlSources['actual-recovery'] = 'Offline'; expect(buildRecoveryConsoleState(state, ['actual-recovery']).recovery_controllable).toBe(false);
    const control = document.createElement('ph-recovery-controls'); document.body.append(control); control.state = { ...view(), candidates: [], target_uuid: 'gone' };
    expect(control.applySelection.disabled).toBe(true); expect(control.hint.textContent).toContain('Science');
  });
});
