import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { GM_ACTION_NAMES, browserGmActionAdapter, nativeGmActionAdapter,
  refusingGmActionAdapter, installGmActionBindings } from '../../gui/gm-action-bindings.js';

const server = readFileSync(new URL('../../server.html', import.meta.url), 'utf8');

function liveAdapter(kind, target = {}) {
  let operator = { id: 'gm-one' };
  const send = vi.fn(() => true);
  const createCheckpoint = vi.fn(() => 'slot-one');
  const getOperator = () => operator;
  let adapter;
  if (kind === 'browser') {
    target.__hostLocalGm = getOperator;
    target.wasm_submit_gm_action = value => send(JSON.parse(value));
    // Exercise the actual classic browser preparation, not a test copy.
    const start = server.indexOf('    window.__browserGmActions = {};');
    const end = server.indexOf('    /** Local content/boot validation', start);
    if (start < 0 || end < 0) throw new Error('Browser GM preparation not found');
    new Function('window', 'localGm', server.slice(start, end))(target, getOperator);
    target.__browserGmActions.__hostGmCheckpointCreate = createCheckpoint;
    adapter = browserGmActionAdapter(target.__browserGmActions);
  } else {
    adapter = nativeGmActionAdapter({ getOperator, submitAction: send, createCheckpoint });
  }
  return { target, adapter, send, createCheckpoint, setOperator: value => { operator = value; } };
}

for (const kind of ['browser', 'native']) describe(`${kind} typed GM action bindings`, () => {
  it('submits both named Objective actions and preserves their distinct scope', () => {
    const app = liveAdapter(kind);
    const mount = installGmActionBindings(app.target, app.adapter);
    const request = { operator_id: 'gm-one', correlation: 'objective-1', objective: 'rescue', verb: 'complete' };
    expect(app.target.__hostObjectiveAction({ ...request, recipients: ['ship-a'] })).toBe(true);
    expect(app.target.__hostObjectiveInstanceAction({ ...request, scope: { instance: 'ship-b' } })).toBe(true);
    expect(app.send.mock.calls.map(([value]) => value)).toEqual([
      { ...request, action: 'objective_action', recipients: ['ship-a'] },
      { ...request, action: 'objective_instance_action', scope: { instance: 'ship-b' } },
    ]);
    mount.dispose();
  });

  it('refuses absent/spoofed operators, missing correlations and transport refusals', () => {
    const app = liveAdapter(kind);
    const mount = installGmActionBindings(app.target, app.adapter);
    const request = { operator_id: 'gm-one', correlation: 'objective-1', objective: 'rescue', verb: 'complete', scope: 'all' };
    expect(app.target.__hostObjectiveInstanceAction({ ...request, operator_id: 'other' })).toBe(false);
    expect(app.target.__hostObjectiveInstanceAction({ ...request, correlation: '' })).toBe(false);
    app.setOperator(null);
    expect(app.target.__hostObjectiveInstanceAction(request)).toBe(false);
    expect(app.send).not.toHaveBeenCalled();
    app.setOperator({ id: 'gm-one' });
    app.send.mockReturnValue(false);
    expect(app.target.__hostObjectiveInstanceAction(request)).toBe(false);
    mount.dispose();
  });

  it('retires saved action and checkpoint callbacks without deleting a newer owner', () => {
    const app = liveAdapter(kind);
    const old = installGmActionBindings(app.target, app.adapter);
    const retained = app.target.__hostObjectiveInstanceAction;
    const retainedSave = app.target.__hostGmCheckpointCreate;
    const next = liveAdapter(kind, app.target);
    const current = installGmActionBindings(next.target, next.adapter);
    const active = next.target.__hostObjectiveInstanceAction;
    const request = { operator_id: 'gm-one', correlation: 'objective-1', objective: 'rescue', verb: 'activate', scope: 'all' };
    expect(retained(request)).toBe(false);
    expect(retainedSave('old save')).toBe(false);
    old.dispose();
    expect(next.target.__hostObjectiveInstanceAction).toBe(active);
    expect(active(request)).toBe(true);
    expect(next.target.__hostGmCheckpointCreate('new save')).toBe('slot-one');
    expect(app.send).not.toHaveBeenCalled();
    expect(app.createCheckpoint).not.toHaveBeenCalled();
    current.dispose();
    expect(active(request)).toBe(false);
    expect(next.target.__hostObjectiveInstanceAction).toBeUndefined();
  });
});

describe('Workshop Test typed GM action bindings', () => {
  it('explicitly refuses every declared write, including save and pause, without a live adapter', () => {
    const target = {};
    const mount = installGmActionBindings(target, refusingGmActionAdapter(() => { throw new Error('read-only'); }));
    const retained = [];
    for (const name of GM_ACTION_NAMES) {
      retained.push(target[name]);
      expect(() => target[name]({ operator_id: 'pretend', correlation: 'test-1' }), name).toThrow('read-only');
    }
    mount.dispose();
    for (const invoke of retained) expect(invoke({})).toBe(false);
    for (const name of GM_ACTION_NAMES) expect(target[name]).toBeUndefined();
  });

  it('leaves a replacement Test owner intact and makes retained refusals inert', () => {
    const target = {};
    const old = installGmActionBindings(target, refusingGmActionAdapter(() => { throw new Error('old'); }));
    const retained = target.__hostObjectiveAction;
    const current = installGmActionBindings(target, refusingGmActionAdapter(() => { throw new Error('new'); }));
    old.dispose();
    expect(retained({})).toBe(false);
    expect(() => target.__hostObjectiveAction({})).toThrow('new');
    current.dispose();
  });
});

it('does not publish undeclared writes or presentation interest through the action installer', () => {
  const interest = vi.fn(), target = { __hostGmInspectorInterest: interest };
  const mount = installGmActionBindings(target, { arbitraryMutation: vi.fn(), __hostGmInspectorInterest: vi.fn() });
  expect(target.arbitraryMutation).toBeUndefined();
  expect(target.__hostGmInspectorInterest).toBe(interest);
  mount.dispose();
  expect(target.__hostGmInspectorInterest).toBe(interest);
});

it('keeps an unavailable native checkpoint provider absent and native-only browser writes inert', () => {
  const native = {}, browser = {}, send = vi.fn();
  const a = installGmActionBindings(native, nativeGmActionAdapter({ getOperator: () => ({ id: 'gm' }), submitAction: send }));
  const b = installGmActionBindings(browser, browserGmActionAdapter({}));
  expect(native.__hostGmCheckpointCreate).toBeUndefined();
  expect(browser.__hostBackfillShipSlot({})).toBe(false);
  expect(send).not.toHaveBeenCalled();
  a.dispose(); b.dispose();
});
