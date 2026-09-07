// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createGmNpcPanel, parseNpcDoctrinePayload } from '../../gui/gm-npc-panel.js';
import { createGmConfirmationController, createGmConfirmationProfile } from '../../gui/gm-confirmation.js';

const actor = { entity_id: 'courier', name: 'Courier' };
const profile = { current: null, intent: null, choices: [{ id: 'north', label: 'North route' }, { id: 'east', label: 'East route' }] };
const payload = (changes = {}) => ({ entities: [actor], npc_doctrines: { courier: structuredClone(profile) }, npc_doctrine_results: [], ...changes });
beforeEach(() => { document.body.innerHTML = `<p id="gm-npc-target"></p><p id="gm-npc-current"></p><p id="gm-npc-intent"></p>
  <select id="gm-npc-choice"></select><p id="gm-npc-empty"></p><button id="gm-npc-apply"></button><p id="gm-npc-feedback"></p><ol id="gm-npc-results"></ol>`; });
function mount(options = {}) {
  let operator = { id: 'gm-one' }, index = 0;
  const submit = vi.fn(() => true), correlation = vi.fn(() => `npc-${++index}`), schedule = vi.fn(() => 7), cancelSchedule = vi.fn();
  const panel = createGmNpcPanel({ doc: document, getOperator: () => operator, submit, correlation, schedule, cancelSchedule, ...options });
  panel.update(payload()); panel.select(actor);
  return { panel, submit, correlation, schedule, cancelSchedule, operator: value => { operator = value; panel.refreshAdmission(); } };
}
const feedback = () => document.getElementById('gm-npc-feedback').dataset.state;
const terminal = (change = {}) => ({ action_kind: 'npc-doctrine', operator_id: 'gm-one', correlation: 'npc-1', target: 'courier', npc_doctrine: 'north', tick: 5, outcome: 'applied', ...change });
const controllers = [];
afterEach(() => { for (const controller of controllers.splice(0)) controller.destroy(); });
function mountConfirmed(mode = 'confirm-preview') {
  const confirmationProfile = createGmConfirmationProfile({ storage: { getItem: () => null, setItem: () => {} } });
  confirmationProfile.setMode('npc.directive', mode);
  const controller = createGmConfirmationController({ doc: document, profile: confirmationProfile });
  controllers.push(controller);
  return { ...mount({ t: (id, values) => values ? JSON.stringify(values) : id,
    confirmAction: request => controller.request(request) }), controller };
}

it('shows only authored compatible choices and sends captured intent without optimistic doctrine', () => {
  const { panel, submit } = mount();
  expect([...document.querySelectorAll('option')].map(row => row.value)).toEqual(['north', 'east']);
  document.getElementById('gm-npc-apply').click();
  expect(submit).toHaveBeenCalledWith({ action: 'set_npc_doctrine', operator_id: 'gm-one', correlation: 'npc-1', target: 'courier', doctrine: 'north' });
  expect(panel.state().profiles.courier.current).toBeNull();
  expect(panel.choose()).toBe(false); expect(submit).toHaveBeenCalledOnce();
  panel.update(payload({ npc_doctrines: {} }));
  expect(document.getElementById('gm-npc-apply').disabled).toBe(true);
});

it('captures confirmation intent before acceptance and allocates correlation only once after acceptance', () => {
  let confirmation;
  const { panel, correlation, submit } = mount({ t: (id, values) => values ? JSON.stringify(values) : id,
    confirmAction: request => { confirmation = request; return true; } });
  panel.choose(); expect(correlation).not.toHaveBeenCalled(); expect(panel.state().pending).toBeNull();
  expect(confirmation).toMatchObject({ category: 'npc.directive', defaultMode: 'immediate', intent: { target: 'courier', doctrine: 'north' } });
  expect(confirmation.description).toContain('Courier'); expect(confirmation.description).toContain('North route');
  document.getElementById('gm-npc-choice').value = 'east'; document.getElementById('gm-npc-choice').dispatchEvent(new Event('change'));
  expect(confirmation.accept()).toBe(true); expect(confirmation.accept()).toBe(false);
  expect(confirmation.preview()).toBe(confirmation.description); expect(confirmation.preview()).not.toContain('East route');
  expect(submit.mock.calls[0][0].doctrine).toBe('north'); expect(correlation).toHaveBeenCalledOnce();
});

it('keeps a withdrawn choice visible and requires explicit reselection before submitting', () => {
  const { panel, submit } = mount();
  const select = document.getElementById('gm-npc-choice');
  expect(select.value).toBe('north');
  panel.update(payload({ npc_doctrines: { courier: { ...profile, choices: [profile.choices[1]] } } }));
  expect(select.value).toBe('north'); expect(select.selectedOptions[0].disabled).toBe(true);
  expect(document.getElementById('gm-npc-apply').disabled).toBe(true);
  document.getElementById('gm-npc-apply').click(); expect(panel.choose()).toBe(false); expect(submit).not.toHaveBeenCalled();
  select.value = 'east'; select.dispatchEvent(new Event('change'));
  expect(document.getElementById('gm-npc-apply').disabled).toBe(false);
  document.getElementById('gm-npc-apply').click(); expect(submit.mock.calls[0][0].doctrine).toBe('east');
});

it.each(['reset', 'unauthorized', 'operator_changed', 'selection'])('retires a confirmation after %s', reason => {
  let confirmation;
  const { panel, operator, correlation, submit } = mount({ confirmAction: request => { confirmation = request; return true; } });
  panel.choose();
  if (reason === 'reset') panel.reset();
  if (reason === 'unauthorized') operator(null);
  if (reason === 'operator_changed') operator({ id: 'gm-two' });
  if (reason === 'selection') panel.select({ entity_id: 'other', name: 'Other' });
  expect(confirmation.accept()).toBe(false); expect(correlation).not.toHaveBeenCalled(); expect(submit).not.toHaveBeenCalled();
  operator({ id: 'gm-one' }); expect(confirmation.accept()).toBe(false);
});

it.each(['removed', 'withdrawn'])('submits the captured %s intent for canonical refusal after confirmation', reason => {
  let confirmation;
  const { panel, correlation, submit, cancelSchedule } = mount({ confirmAction: request => { confirmation = request; return true; } });
  panel.choose();
  const stale = reason === 'removed' ? { entities: [], npc_doctrines: {} } : { npc_doctrines: {} };
  panel.update(payload(stale));
  expect(document.getElementById('gm-npc-apply').disabled).toBe(true);
  expect(panel.state().pending).toBeNull(); expect(correlation).not.toHaveBeenCalled();
  expect(confirmation.accept()).toBe(true); expect(confirmation.accept()).toBe(false);
  expect(submit).toHaveBeenCalledExactlyOnceWith({ action: 'set_npc_doctrine', operator_id: 'gm-one', correlation: 'npc-1', target: 'courier', doctrine: 'north' });
  expect(feedback()).toBe('pending'); expect(correlation).toHaveBeenCalledOnce();
  panel.update(payload({ ...stale, npc_doctrine_results: [terminal({ outcome: 'refused', reason: 'unknown-npc-doctrine' })] }));
  expect(feedback()).toBe('refused'); expect(panel.state().pending).toBeNull(); expect(cancelSchedule).toHaveBeenCalledWith(7);
});

it('cancel retires the unsent callback without correlation, Pending or a later accept', () => {
  let confirmation;
  const { panel, correlation, submit } = mount({ confirmAction: request => { confirmation = request; return true; } });
  panel.choose(); const cancelled = confirmation; cancelled.onCancel();
  expect(cancelled.accept()).toBe(false); expect(correlation).not.toHaveBeenCalled();
  expect(submit).not.toHaveBeenCalled(); expect(panel.state().pending).toBeNull();
  expect(panel.choose()).toBe(true); expect(confirmation.accept()).toBe(true);
  expect(cancelled.accept()).toBe(false); expect(submit).toHaveBeenCalledOnce();
});

it.each(['removed', 'withdrawn'])('the shared confirmation dialog preserves a captured %s NPC intent through canonical refusal', reason => {
  const { panel, controller, correlation, submit, schedule, cancelSchedule } = mountConfirmed();
  document.getElementById('gm-npc-apply').click();
  expect(controller.isOpen()).toBe(true);
  const preview = document.querySelector('[data-confirmation-preview]');
  const description = document.querySelector('[data-confirmation-description]').textContent;
  expect(preview.hidden).toBe(false); expect(preview.textContent).toBe(description);
  expect(description).toContain('Courier'); expect(description).toContain('North route');
  expect(correlation).not.toHaveBeenCalled(); expect(submit).not.toHaveBeenCalled();
  expect(schedule).not.toHaveBeenCalled(); expect(panel.state().pending).toBeNull();

  const stale = reason === 'removed' ? { entities: [], npc_doctrines: {} }
    : { npc_doctrines: { courier: { ...profile, choices: [profile.choices[1]] } } };
  panel.update(payload(stale));
  expect(document.getElementById('gm-npc-apply').disabled).toBe(true);
  if (reason === 'withdrawn') {
    const select = document.getElementById('gm-npc-choice');
    select.value = 'east'; select.dispatchEvent(new Event('change'));
  }
  controller.refresh();
  expect(preview.textContent).toBe(description); expect(preview.textContent).not.toContain('East route');
  document.querySelector('[data-confirmation-accept]').click();
  document.querySelector('[data-confirmation-accept]').click();
  expect(controller.isOpen()).toBe(false);
  const request = { action: 'set_npc_doctrine', operator_id: 'gm-one', correlation: 'npc-1', target: 'courier', doctrine: 'north' };
  expect(submit).toHaveBeenCalledExactlyOnceWith(request); expect(correlation).toHaveBeenCalledOnce();
  expect(schedule).toHaveBeenCalledOnce(); expect(panel.state().pending).toEqual(request); expect(feedback()).toBe('pending');

  const refused = terminal({ outcome: 'refused', reason: reason === 'removed' ? 'unknown-entity' : 'unknown-npc-doctrine' });
  panel.update(payload({ ...stale, npc_doctrine_results: [{ ...refused, operator_id: 'gm-two' }] }));
  expect(panel.state().pending).toEqual(request); expect(feedback()).toBe('pending');
  panel.update(payload({ ...stale, npc_doctrine_results: [refused] }));
  expect(panel.state().pending).toBeNull(); expect(feedback()).toBe('refused'); expect(cancelSchedule).toHaveBeenCalledWith(7);
  const row = document.querySelector('#gm-npc-results li');
  expect({ ...row.dataset }).toEqual({ outcome: 'refused', target: 'courier', correlation: 'npc-1', doctrine: 'north' });
  expect(row.textContent).toContain('gm-one'); expect(submit).toHaveBeenCalledOnce();
});

it('the shared confirmation Cancel control creates no NPC correlation, pending action or delayed submission', () => {
  const { panel, controller, correlation, submit, schedule } = mountConfirmed('confirm');
  document.getElementById('gm-npc-apply').click();
  expect(controller.isOpen()).toBe(true); expect(document.querySelector('[data-confirmation-preview]').hidden).toBe(true);
  document.querySelector('[data-confirmation-cancel]').click();
  expect(controller.isOpen()).toBe(false);
  document.querySelector('[data-confirmation-accept]').click();
  controller.refresh();
  expect(correlation).not.toHaveBeenCalled(); expect(submit).not.toHaveBeenCalled();
  expect(schedule).not.toHaveBeenCalled(); expect(panel.state().pending).toBeNull();
});

it.each(['reset', 'unauthorized', 'operator_changed', 'selection'])('the shared NPC dialog cannot revive an intent retired by %s', reason => {
  const { panel, controller, operator, correlation, submit, schedule } = mountConfirmed();
  document.getElementById('gm-npc-apply').click();
  expect(controller.isOpen()).toBe(true);
  if (reason === 'reset') { panel.reset(); panel.update(payload()); panel.select(actor); }
  if (reason === 'unauthorized') operator(null);
  if (reason === 'operator_changed') operator({ id: 'gm-two' });
  if (reason === 'selection') { panel.select({ entity_id: 'other', name: 'Other' }); panel.select(actor); }
  document.querySelector('[data-confirmation-accept]').click();
  expect(controller.isOpen()).toBe(false);
  operator({ id: 'gm-one' });
  document.querySelector('[data-confirmation-accept]').click();
  expect(correlation).not.toHaveBeenCalled(); expect(submit).not.toHaveBeenCalled();
  expect(schedule).not.toHaveBeenCalled(); expect(panel.state().pending).toBeNull();
});

it('a confirmation blocked by another pending request cannot revive after that request settles', () => {
  const confirmations = [];
  const { panel, submit } = mount({ confirmAction: request => { confirmations.push(request); return true; } });
  panel.choose(); panel.choose();
  expect(confirmations[0].accept()).toBe(true); expect(confirmations[1].accept()).toBe(false);
  panel.update(payload({ npc_doctrine_results: [terminal()] }));
  expect(panel.state().pending).toBeNull(); expect(confirmations[1].accept()).toBe(false);
  expect(submit).toHaveBeenCalledOnce();
});

it.each(['applied', 'no-op', 'refused'])('matches the complete result identity and renders %s', outcome => {
  const { panel, cancelSchedule } = mount(); panel.choose();
  panel.update(payload({ npc_doctrine_results: [terminal({ npc_doctrine: 'east' }), terminal({ operator_id: 'gm-two' })] }));
  expect(feedback()).toBe('pending'); expect(panel.state().pending).not.toBeNull();
  panel.update(payload({ npc_doctrine_results: [terminal({ outcome })], npc_doctrines: { courier: { ...profile, current: 'north', intent: 'Fly north' } } }));
  expect(panel.state().pending).toBeNull(); expect(feedback()).toBe(outcome === 'no-op' ? 'no_op' : outcome);
  expect(cancelSchedule).toHaveBeenCalledWith(7); expect(document.getElementById('gm-npc-intent').textContent).toBe('Fly north');
});

it('timeout permits a new attempt and absolute reconnect/reset clears stale state', () => {
  const { panel, schedule } = mount(); panel.choose(); schedule.mock.calls[0][0]();
  expect(feedback()).toBe('timed_out'); expect(panel.choose()).toBe(true);
  panel.reset(); expect(panel.state().pending).toBeNull(); expect(panel.state().profiles).toEqual({});
});

it('rejects malformed rows atomically and supports old empty projection payloads', () => {
  const { panel } = mount();
  for (const bad of [payload({ npc_doctrines: [] }), payload({ npc_doctrine_results: [terminal({ npc_doctrine: '' })] }), payload({ npc_doctrines: { courier: { ...profile, choices: [profile.choices[0], profile.choices[0]] } } })]) {
    expect(parseNpcDoctrinePayload(bad)).toBeNull(); expect(panel.update(bad)).toBe(false); expect(panel.state().profiles.courier.choices).toHaveLength(2);
  }
  expect(panel.update({ entities: [] })).toBe(true); expect(panel.state().profiles).toEqual({});
});
