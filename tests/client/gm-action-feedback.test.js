import { describe, expect, it, vi } from 'vitest';
import { ActionFeedbackLifecycle, ACTION_FEEDBACK_STATE } from '../../gui/action-feedback.js';
import { GmActionFeedback } from '../../gui/gm-action-feedback.js';
import { createGmFeedbackPresentation, isGmResultEnvelope } from '../../gui/gm-feedback-presentation.js';
import { LOCAL_INGRESS_REFUSAL } from '../../gui/gm-action-reasons.js';

function setup(capacity = 2) {
  let sequence = 0;
  const callbacks = [];
  const lifecycle = new ActionFeedbackLifecycle({ correlation: () => `request-${++sequence}`, now: () => 0 });
  const onLocalTerminal = vi.fn();
  const cancelSchedule = vi.fn();
  const feed = new GmActionFeedback({ lifecycle: () => lifecycle, capacity, timeoutMs: 10,
    schedule: (fn) => { callbacks.push(fn); return callbacks.length; }, cancelSchedule, onLocalTerminal });
  function press(operatorId = 'me', extra = {}) {
    feed.makeRoom();
    const { correlation } = lifecycle.press('test');
    const meta = { correlation, operatorId, ...extra };
    feed.track(meta);
    feed.submitted(meta, true);
    return meta;
  }
  return { feed, lifecycle, callbacks, onLocalTerminal, cancelSchedule, press };
}

const result = (meta, extra = {}) => ({ operator_id: meta.operatorId, correlation: meta.correlation,
  outcome: 'applied', tick: 1, ...extra });

describe('GM action request feedback', () => {
  it('settles exact operators throughout the full feed before limiting display history', () => {
    const { feed, lifecycle, press } = setup(1);
    const meta = press();
    feed.replace([result(meta, { operator_id: 'other' })]);
    expect(feed.size).toBe(1);
    feed.replace([result(meta), result(meta, { correlation: 'other-request' })]);
    expect(feed.size).toBe(0);
    expect(lifecycle.get(meta.correlation).state).toBe(ACTION_FEEDBACK_STATE.APPLIED);
    expect([...feed.entries()].map(entry => entry.value.correlation)).toEqual(['other-request']);
  });

  it('orders authoritative, unmatched local terminal and pending entries without accumulating projections', () => {
    const { feed, callbacks, press } = setup();
    const timedOut = press();
    callbacks[0]();
    const pending = press();
    expect([...feed.entries()].map(entry => entry.kind)).toEqual(['local', 'pending']);
    feed.replace([result(timedOut)]);
    expect([...feed.entries()].map(entry => entry.value.correlation)).toEqual([timedOut.correlation, pending.correlation]);
    feed.replace([]);
    expect([...feed.entries()].map(entry => entry.value.correlation)).toEqual([pending.correlation]);
  });

  it('bounds local terminals and expires the oldest pending request on capacity pressure', () => {
    const { feed, press, onLocalTerminal } = setup(1);
    const first = press();
    const second = press();
    expect(onLocalTerminal).toHaveBeenCalledWith(first, 'timed-out', null);
    feed.finishLocal(second.correlation, 'refused', 'ingress');
    expect([...feed.entries()].map(entry => entry.value.correlation)).toEqual([second.correlation]);
  });

  it.each([true, false])('cleans timers on reset/disposal and ignores cancelled callbacks (cancel=%s)', cancel => {
    const { feed, callbacks, cancelSchedule, onLocalTerminal, press } = setup();
    const old = press();
    feed.reset(cancel);
    // Reuse of a correlation after a run boundary must not make the old timer current.
    const replacement = { ...old };
    feed.track(replacement);
    callbacks[0]();
    expect(feed.get(old.correlation)).toBe(replacement);
    expect(cancelSchedule).toHaveBeenCalledWith(1);
    expect(onLocalTerminal).not.toHaveBeenCalled();
  });

  it('keeps local refusal and authoritative settlement distinct', () => {
    const { feed, lifecycle, onLocalTerminal } = setup();
    const meta = { ...lifecycle.press('test'), operatorId: 'me' };
    feed.track(meta);
    feed.submitted(meta, false, 'ingress');
    expect(onLocalTerminal).toHaveBeenCalledWith(meta, 'refused', 'ingress');
    expect(lifecycle.get(meta.correlation).state).toBe(ACTION_FEEDBACK_STATE.REFUSED);
    expect([...feed.entries()][0].value.reason).toBe('ingress');
  });

  it('lets the owning panel select its result source without consuming another request', () => {
    const { feed, press } = setup();
    const ghost = press('me', { ghost: true });
    feed.replace([result(ghost)], { accepts: meta => !meta.ghost });
    expect(feed.size).toBe(1);
    const match = feed.settle(result(ghost), meta => meta.ghost);
    expect(match.meta).toBe(ghost);
    expect(feed.size).toBe(0);
  });
});


describe('request-shaped GM feedback', () => {
  it('refuses capacity pressure and retains exact request shapes', () => {
    const feed = new GmActionFeedback({ capacity: 1, timeoutMs: 10, schedule: () => 1, cancelSchedule: () => {} });
    const request = { operator_id: 'gm', correlation: 'one', target: 'ship' };
    expect(feed.begin(request)).toBe(true);
    expect(feed.begin({ ...request, correlation: 'two' })).toBe(false);
    expect([...feed.requests()]).toEqual([request]);
    expect(feed.settle({ ...request, outcome: 'applied', target: 'other' }, (meta, row) => meta.request.target === row.target)).toBeNull();
    expect(feed.firstRequest).toEqual(request);
    expect(feed.settle({ ...request, outcome: 'applied' }).meta.request).toEqual(request);
    expect(feed.firstRequest).toBeNull();
  });
  it('ignores old callbacks after reset and correlation reuse without a semantic lifecycle', () => {
    const callbacks = [], terminal = vi.fn();
    const feed = new GmActionFeedback({ capacity: 1, timeoutMs: 10, schedule: fn => callbacks.push(fn), cancelSchedule: () => {}, onLocalTerminal: terminal });
    const request = { operator_id: 'gm', correlation: 'same' };
    feed.begin(request); feed.reset(); feed.begin({ ...request });
    callbacks[0](); expect(terminal).not.toHaveBeenCalled(); expect(feed.firstRequest).toEqual(request);
    callbacks[1](); expect(feed.firstRequest).toBeNull(); expect(terminal).toHaveBeenCalledOnce();
  });
});


describe('shared GM submission and presentation', () => {
  it.each([() => false, () => { throw new Error('ingress'); }])('settles ingress refusal after sending with metadata already tracked', send => {
    const { feed, lifecycle, onLocalTerminal } = setup();
    const meta = { ...lifecycle.press('test'), operatorId: 'me' };
    const observed = [];
    expect(feed.submit(meta, () => {
      observed.push(feed.get(meta.correlation) === meta);
      expect(lifecycle.get(meta.correlation).state).toBe(ACTION_FEEDBACK_STATE.PRESSED);
      return send();
    }, 'ingress')).toBe(false);
    expect(observed).toEqual([true]);
    expect(onLocalTerminal).toHaveBeenCalledWith(meta, 'refused', 'ingress');
    expect(lifecycle.get(meta.correlation).state).toBe(ACTION_FEEDBACK_STATE.REFUSED);
  });

  it('does not create a timer or resurrect metadata after a synchronous run reset', () => {
    const { feed, lifecycle, callbacks } = setup();
    const meta = { ...lifecycle.press('test'), operatorId: 'me' };
    expect(feed.submit(meta, () => { feed.reset(); return true; })).toBe(true);
    expect(feed.size).toBe(0);
    expect(callbacks).toHaveLength(0);
    expect(lifecycle.get(meta.correlation)).toBeNull();
  });

  function presentation(extra = {}) {
    const doc = { createElement: () => ({ dataset: {} }) };
    const log = { children: [], replaceChildren() { this.children = []; }, appendChild(row) { this.children.push(row); } };
    const t = (id, params) => params ? `${id}:${params.reason}` : id;
    const view = createGmFeedbackPresentation({ doc, log, rowClass: 'gm-test-entry', t,
      feed: { *entries() {} }, reasonPrefix: 'server.gm.effect',
      ...extra });
    return { view, log };
  }

  it('uses safe identity fallbacks and retains unknown refusal diagnostics', () => {
    const { view } = presentation({ getOperator: () => { throw new Error('gone'); }, getOperatorName: () => '' });
    expect(view.operator()).toBeNull();
    expect(view.operatorName('gm')).toBe('gm');
    expect(view.refusalText(null)).toBe('server.gm.effect.reason_unspecified');
    expect(view.refusalText(LOCAL_INGRESS_REFUSAL)).toBe('server.gm.session.reason.ingress_rejected');
    expect(view.refusalText('future')).toBe('server.gm.effect.reason_unknown:future');
    expect(presentation().view.operatorName('gm')).toBe('gm');
    expect(presentation({ getOperatorName: () => { throw new Error('gone'); } }).view.operatorName('gm')).toBe('gm');
    expect(presentation({ getOperator: () => ({ id: '' }) }).view.operator()).toBeNull();
  });

  it('preserves duplicate result rows, feed order and unique-key diagnostics', () => {
    const result = { operator_id: 'gm', correlation: 'same' };
    const local = { operatorId: 'gm', correlation: 'local', outcome: 'refused', reason: 'ingress' };
    const pending = { operatorId: 'gm', correlation: 'pending' };
    const entries = [{ kind: 'result', value: result }, { kind: 'result', value: result },
      { kind: 'local', value: local }, { kind: 'pending', value: pending }];
    const calls = [], keys = new Set();
    let view;
    const built = presentation({ feed: { entries: () => entries }, beforeRender: () => keys.clear(),
      onAppend: row => keys.add(row.dataset.entryKey),
      paintResult: value => { calls.push('result'); view.appendRow(value.operator_id, value.correlation); },
      paintLocal: (value, outcome, reason) => { calls.push([outcome, reason]); view.appendRow(value.operatorId, value.correlation); },
    });
    view = built.view;
    view.renderLog();
    expect(calls).toEqual(['result', 'result', ['refused', 'ingress'], ['pending', null]]);
    expect(built.log.children.map(row => row.dataset.correlation)).toEqual(['same', 'same', 'local', 'pending']);
    expect(built.log.children[0]).toEqual({ className: 'gm-test-entry', dataset: {
      entryKey: '["gm","same"]', operatorId: 'gm', correlation: 'same',
    } });
    expect(keys.size).toBe(3);
    entries.length = 0; view.renderLog();
    expect(built.log.children).toHaveLength(0);
    expect(keys.size).toBe(0);
  });

  it('checks common identity without accepting malformed result envelopes', () => {
    const valid = { operator_id: 'gm', correlation: 'press', outcome: 'no-op', tick: 0 };
    expect(isGmResultEnvelope(valid)).toBe(true);
    for (const invalid of [null, {}, { ...valid, operator_id: '' }, { ...valid, correlation: '' },
      { ...valid, outcome: 'future' }, { ...valid, tick: -1 }, { ...valid, tick: 1.5 }, { ...valid, reason: 1 }]) {
      expect(isGmResultEnvelope(invalid)).toBe(false);
    }
  });
});
