// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from 'vitest';
import { createGmObjectivePanel, parseGmObjectivePayload } from '../../gui/gm-objective-panel.js';
import { t } from '../../gui/strings.js';
import { createGmConfirmationController, createGmConfirmationProfile } from '../../gui/gm-confirmation.js';

beforeEach(() => {
  document.body.innerHTML = `<ul id="gm-objective-list"></ul><p id="gm-objective-empty"></p>
    <div id="gm-objective-confirmation" hidden><p id="gm-objective-consequence"></p>
    <button id="gm-objective-confirm"></button><button id="gm-objective-cancel"></button></div>
    <p id="gm-objective-feedback"></p><ol id="gm-objective-results"></ol>`;
});
const authored = (changes = {}) => ({ id: 'rescue', label: 'server.gm.objective.heading',
  text: 'server.gm.objective.scope', text_params: { ships: 'Courier', status: 'Awaiting rescue' },
  status: null, recipients: ['ship-a'], available: true, ...changes });
const payload = (changes = {}) => ({ objective_palette: [authored()], objectives: [], objective_results: [], ...changes });
const result = (changes = {}) => ({ action_kind: 'objective-control', target: 'rescue', operator_id: 'gm-a',
  correlation: 'objective-1', tick: 9, outcome: 'applied', objective_verb: 'activate', objective_recipients: ['ship-a'], ...changes });
const button = (verb) => document.querySelector(`#gm-objective-list button[data-verb="${verb}"]`);
function mount(options = {}) {
  let operator = { id: 'gm-a' };
  let sequence = 0;
  const submit = vi.fn(() => true), schedule = vi.fn(() => 7), cancelSchedule = vi.fn();
  const panel = createGmObjectivePanel({ doc: document, t, submit, schedule, cancelSchedule,
    getOperator: () => operator, getShipName: () => 'Courier', getOperatorName: () => 'Alice',
    correlation: () => `objective-${++sequence}`, ...options });
  return { panel, submit, schedule, cancelSchedule, setOperator: (value) => { operator = value; panel.refreshAdmission(); } };
}

it('uses the shared Objective policy and retains captured scope for ordinary stale admission', () => {
  const profile = createGmConfirmationProfile();
  const confirmation = createGmConfirmationController({ doc: document,
    profile: { mode: id => id === 'objective.activate' ? 'confirm-preview' : profile.mode(id) } });
  const { panel, submit } = mount({ confirmAction: confirmation.request });
  panel.update(payload());
  button('activate').click();
  expect(panel.state().pending).toBeNull();
  document.querySelector('[data-confirmation-cancel]').click();
  expect(submit).not.toHaveBeenCalled();
  button('activate').click();
  panel.update(payload({ objective_palette: [] }));
  document.querySelector('[data-confirmation-accept]').click();
  expect(submit).toHaveBeenCalledExactlyOnceWith({ operator_id: 'gm-a', correlation: 'objective-1',
    objective: 'rescue', verb: 'activate', recipients: ['ship-a'] });
  confirmation.destroy();
});

it('shows authored text and intended ships, with preview/cancel and no optimistic state change', () => {
  const { panel, submit } = mount(); panel.update(payload());
  expect(button('activate').disabled).toBe(false);
  expect(button('complete').disabled).toBe(true);
  button('activate').click();
  expect(document.querySelector('#gm-objective-consequence').textContent).toContain('Courier');
  expect(document.querySelector('#gm-objective-consequence').textContent).toContain('Awaiting rescue');
  document.querySelector('#gm-objective-cancel').click();
  expect(panel.confirm()).toBe(false); expect(submit).not.toHaveBeenCalled();
  button('activate').click(); expect(panel.confirm()).toBe(true);
  expect(submit).toHaveBeenCalledWith({ operator_id: 'gm-a', correlation: 'objective-1',
    objective: 'rescue', verb: 'activate', recipients: ['ship-a'] });
  expect(panel.state().palette[0].status).toBeNull();
  expect(panel.confirm()).toBe(false); expect(submit).toHaveBeenCalledOnce();
});

it.each(['complete', 'fail'])('allows %s only on an active record and never reopens a terminal record', (verb) => {
  const { panel, submit } = mount();
  panel.update(payload({ objective_palette: [authored({ status: 'Active' })], objectives: [authored({ status: 'Active' })] }));
  expect(button('activate').disabled).toBe(true); button(verb).click();
  expect(document.querySelector('#gm-objective-consequence').textContent).toContain('cannot be reopened');
  expect(panel.confirm()).toBe(true); expect(submit.mock.calls[0][0].verb).toBe(verb);
  const status = verb === 'complete' ? 'Completed' : 'Failed';
  panel.update(payload({ objective_palette: [authored({ status })], objectives: [authored({ status })],
    objective_results: [result({ objective_verb: verb })] }));
  expect([...document.querySelectorAll('#gm-objective-list button')].every((b) => b.disabled)).toBe(true);
});

it.each([
  { status: 'Completed' }, { recipients: ['ship-b'] }, { available: false },
  { text: 'Changed Objective' }, { text_params: { ships: 'Changed ship', status: 'Changed' } },
])('invalidates a preview when its authoritative row changes: %j', (change) => {
  const { panel, submit } = mount(); panel.update(payload()); button('activate').click();
  panel.update(payload({ objective_palette: [authored(change)] }));
  expect(panel.confirm()).toBe(false); expect(submit).not.toHaveBeenCalled();
  expect(document.querySelector('#gm-objective-confirmation').hidden).toBe(true);
});

it('rejects absent admission and invalidates a preview on an operator change or removed palette', () => {
  const { panel, submit, setOperator } = mount(); panel.update(payload());
  setOperator(null); expect(button('activate').disabled).toBe(true);
  button('activate').click(); expect(panel.confirm()).toBe(false);
  setOperator({ id: 'gm-a' }); button('activate').click(); setOperator({ id: 'gm-b' });
  expect(panel.confirm()).toBe(false);
  button('activate').click(); panel.update(payload({ objective_palette: [] }));
  expect(panel.confirm()).toBe(false); expect(submit).not.toHaveBeenCalled();
});

it('settles only the exact attributed verb, Objective and scope and replaces history on reconnect', () => {
  const { panel, cancelSchedule } = mount(); panel.update(payload()); button('activate').click(); panel.confirm();
  for (const change of [{ operator_id: 'gm-b' }, { objective_verb: 'fail' }, { target: 'other' }, { objective_recipients: ['ship-b'] }]) {
    panel.update(payload({ objective_results: [result(change)] })); expect(panel.state().pending).not.toBeNull();
  }
  panel.update(payload({ objective_results: [result()] }));
  expect(panel.state().pending).toBeNull(); expect(cancelSchedule).toHaveBeenCalledWith(7);
  expect(document.querySelector('#gm-objective-feedback').dataset.state).toBe('applied');
  panel.update(payload({ objective_results: [result()] }));
  expect(document.querySelectorAll('#gm-objective-results li')).toHaveLength(1);
  panel.reset(); panel.update(payload({ objective_results: [result()] }));
  expect(panel.state().pending).toBeNull(); expect(document.querySelectorAll('#gm-objective-results li')).toHaveLength(1);
});

it('keeps feedback bounded and displays local ingress failure and timeout honestly', () => {
  const { panel, submit, schedule } = mount(); panel.update(payload());
  submit.mockReturnValueOnce(false); button('activate').click(); expect(panel.confirm()).toBe(false);
  expect(document.querySelector('#gm-objective-feedback').dataset.state).toBe('refused');
  button('activate').click(); panel.confirm(); schedule.mock.calls[0][0]();
  expect(panel.state().pending).toBeNull();
  expect(document.querySelector('#gm-objective-feedback').dataset.state).toBe('timed_out');
});

it.each(['unknown-objective', 'objective-not-active', 'objective-scope-mismatch', 'not-game-master'])
  ('renders localized %s refusals with target, scope, operator and correlation', (reason) => {
    const { panel } = mount(); panel.update(payload({ objective_results: [result({ outcome: 'refused', reason })] }));
    const text = document.querySelector('#gm-objective-results').textContent;
    expect(text).toContain(t(`server.gm.session.reason.${reason.replaceAll('-', '_')}`));
    for (const value of ['Alice', 'rescue', 'Courier', 'objective-1']) expect(text).toContain(value);
    expect(text).not.toMatch(/server\.gm\.|⟨/);
  });

it('treats an unavailable scoped palette as disabled, never widening it to all ships', () => {
  const { panel, submit } = mount(); panel.update(payload({ objective_palette: [authored({ available: false })] }));
  expect(button('activate').disabled).toBe(true); button('activate').click(); expect(panel.confirm()).toBe(false);
  expect(submit).not.toHaveBeenCalled(); expect(document.querySelector('#gm-objective-list').textContent).toContain('Courier');
});

it.each([
  null, '{', {}, payload({ objective_palette: [authored(), authored()] }),
  payload({ objective_palette: [authored({ recipients: ['ship-a', 'ship-a'] })] }),
  payload({ objective_palette: [authored({ available: undefined })] }),
  payload({ objective_results: [result({ action_kind: 'world-despawn' })] }),
  payload({ objective_results: [result({ tick: -1 })] }),
])('rejects malformed absolute payloads without partial replacement', (bad) => {
  const { panel } = mount(); panel.update(payload());
  expect(parseGmObjectivePayload(bad)).toBeNull(); expect(panel.update(bad)).toBe(false);
  expect(panel.state().palette[0].id).toBe('rescue');
});

it('preserves keyboard focus through identical updates and cancels a preview with Escape', () => {
  const { panel } = mount(); panel.update(payload()); button('activate').focus(); panel.update(payload());
  expect(document.activeElement).toBe(button('activate')); button('activate').click();
  panel.update(payload()); // the next periodic projection replaces the opener
  document.querySelector('#gm-objective-confirmation').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  expect(panel.confirm()).toBe(false); expect(document.activeElement).toBe(button('activate'));
});

it('narrows the list to the selected ship and lists everything again for a non-ship or no selection', () => {
  const { panel } = mount({ getShipName: (id) => id === 'ship-a' ? 'Courier' : 'Raider' });
  panel.update(payload({ objective_palette: [authored(), authored({ id: 'ambush', recipients: ['ship-b'] }),
    authored({ id: 'survive', recipients: [] })] }));
  const listed = () => [...document.querySelectorAll('#gm-objective-list li')].map((row) => row.dataset.objective);
  expect(listed()).toEqual(['rescue', 'ambush', 'survive']);
  expect(document.getElementById('gm-objective-scope').hidden).toBe(true);
  panel.select({ entity_id: 'ship-b', kind: 'npc_ship', name: 'Raider' });
  expect(listed()).toEqual(['ambush', 'survive']);
  expect(document.getElementById('gm-objective-scope').hidden).toBe(false);
  expect(document.getElementById('gm-objective-scope').textContent).toContain('Raider');
  expect(panel.state().scope).toBe('ship-b');
  panel.select({ entity_id: 'ship-c', kind: 'npc_ship', name: 'Stranger' });
  panel.update(payload({ objective_palette: [authored()] }));
  expect(listed()).toEqual([]);
  expect(document.getElementById('gm-objective-empty').hidden).toBe(false);
  expect(document.getElementById('gm-objective-empty').textContent).toContain('Stranger');
  panel.select({ entity_id: 'field', kind: 'asteroid_field', name: 'Belt' });
  expect(listed()).toEqual(['rescue']);
  expect(panel.state().scope).toBeNull();
  panel.select(null);
  expect(document.getElementById('gm-objective-scope').hidden).toBe(true);
});
