import { describe, expect, it, vi } from 'vitest';
import { ActionFeedbackLifecycle, ACTION_FEEDBACK_STATE } from '../../gui/action-feedback.js';
import { GmActionFeedback } from '../../gui/gm-action-feedback.js';

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
