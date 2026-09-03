// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ACTION_FEEDBACK_STATE } from '../../gui/action-feedback.js';
import {
  GM_PAUSE_ACTION_ID,
  GM_RESUME_ACTION_ID,
} from '../../gui/gm-session-actions.js';
import {
  GM_ACTION_REFUSAL_REASON_LABELS,
  createGmSessionControls,
  parseGmSessionPayload,
} from '../../gui/gm-session-controls.js';
import { t } from '../../gui/strings.js';

function mount({
  correlations = ['gm-session-1'],
  operator = { id: 'gm-a', name: 'Alex' },
  submitSessionPaused = vi.fn(() => true),
  capacity,
  timeoutMs,
  schedule,
  cancelSchedule,
} = {}) {
  const queue = [...correlations];
  const controls = createGmSessionControls({
    doc: document,
    win: window,
    t,
    submitSessionPaused,
    getOperator: () => operator,
    getOperatorName: (id) => ({ 'gm-a': 'Alex', 'gm-b': 'Blair' }[id] || id),
    correlation: () => queue.shift(),
    now: () => 101,
    ...(capacity == null ? {} : { capacity }),
    ...(timeoutMs == null ? {} : { timeoutMs }),
    ...(schedule == null ? {} : { schedule }),
    ...(cancelSchedule == null ? {} : { cancelSchedule }),
  });
  return { controls, submitSessionPaused };
}

function result(overrides = {}) {
  return {
    operator_id: 'gm-a',
    correlation: 'gm-session-1',
    requested_active: true,
    outcome: 'applied',
    tick: 42,
    ...overrides,
  };
}

describe('GM session controls', () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <section id="gm-session-controls">
        <h2 id="gm-session-heading"></h2>
        <button id="gm-session-pause"></button>
        <button id="gm-session-resume"></button>
        <p id="gm-session-state"></p>
        <p id="gm-session-feedback"></p>
        <h3 id="gm-session-log-heading"></h3>
        <ol id="gm-session-log" aria-labelledby="gm-session-log-heading"></ol>
      </section>
    `;
  });

  it('mounts a labelled accessible region, controls, live status, and bounded log', () => {
    const { controls } = mount();
    const region = document.getElementById('gm-session-controls');
    const pause = document.getElementById('gm-session-pause');
    const resume = document.getElementById('gm-session-resume');
    const state = document.getElementById('gm-session-state');
    const feedback = document.getElementById('gm-session-feedback');
    const log = document.getElementById('gm-session-log');

    expect(region.getAttribute('role')).toBe('region');
    expect(region.getAttribute('aria-labelledby')).toBe('gm-session-heading');
    expect(pause.textContent).toBe(t('semantic_action.gm.session_pause.label'));
    expect(pause.getAttribute('aria-label'))
      .toBe(t('semantic_action.gm.session_pause.accessibility'));
    expect(resume.getAttribute('aria-describedby')).toBe('gm-session-state gm-session-feedback');
    expect(state.getAttribute('role')).toBe('status');
    expect(feedback.getAttribute('aria-live')).toBe('polite');
    expect(log.getAttribute('role')).toBe('log');
    expect(log.getAttribute('aria-relevant')).toBe('additions text');
    expect(pause.disabled).toBe(false);
    expect(resume.disabled).toBe(false);
    controls.destroy();
  });

  it('shows Pending without optimistically changing the authoritative state, then applies projection', () => {
    const { controls, submitSessionPaused } = mount();
    const pause = document.getElementById('gm-session-pause');
    const resume = document.getElementById('gm-session-resume');
    const state = document.getElementById('gm-session-state');
    const feedback = document.getElementById('gm-session-feedback');
    const log = document.getElementById('gm-session-log');
    controls.update({ paused: false, results: [] });

    expect(pause.getAttribute('aria-pressed')).toBe('false');
    expect(resume.getAttribute('aria-pressed')).toBe('true');
    pause.click();

    expect(submitSessionPaused).toHaveBeenCalledWith(true, 'gm-session-1');
    expect(state.dataset.paused).toBe('false');
    expect(state.textContent).toBe(t('server.gm.session.state_running'));
    expect(pause.getAttribute('aria-pressed')).toBe('false');
    expect(resume.getAttribute('aria-pressed')).toBe('true');
    expect(feedback.dataset.state).toBe(ACTION_FEEDBACK_STATE.PENDING);
    expect(log.children).toHaveLength(1);
    expect(log.firstElementChild.dataset.outcome).toBe('pending');
    expect(log.firstElementChild.textContent).toContain('Pending');

    controls.update({ paused: true, results: [result()] });
    expect(state.dataset.paused).toBe('true');
    expect(state.textContent).toBe(t('server.gm.session.state_paused'));
    expect(pause.getAttribute('aria-pressed')).toBe('true');
    expect(resume.getAttribute('aria-pressed')).toBe('false');
    expect(feedback.dataset.state).toBe(ACTION_FEEDBACK_STATE.APPLIED);
    expect(log.firstElementChild.dataset.outcome).toBe('applied');
    expect(log.firstElementChild.dataset.operatorId).toBe('gm-a');
    expect(log.firstElementChild.dataset.tick).toBe('42');
    expect(log.firstElementChild.textContent).toContain('Alex');
    expect(log.firstElementChild.textContent).toContain('gm-session-1');
    controls.destroy();
  });

  it('also keeps a paused projection unchanged while resume is Pending', () => {
    const { controls, submitSessionPaused } = mount();
    const state = document.getElementById('gm-session-state');
    controls.update({ paused: true, results: [] });

    document.getElementById('gm-session-resume').click();

    expect(submitSessionPaused).toHaveBeenCalledWith(false, 'gm-session-1');
    expect(state.dataset.paused).toBe('true');
    expect(state.textContent).toBe(t('server.gm.session.state_paused'));
    controls.update({
      paused: false,
      results: [result({ requested_active: false, outcome: 'applied' })],
    });
    expect(state.dataset.paused).toBe('false');
    controls.destroy();
  });

  it('settles an exact no-op feed result as successful shared feedback', () => {
    const { controls } = mount();
    controls.update({ paused: true, results: [] });
    document.getElementById('gm-session-pause').click();

    controls.update({ paused: true, results: [result({ outcome: 'no-op', tick: 43 })] });

    const row = document.getElementById('gm-session-log').firstElementChild;
    expect(controls.state().paused).toBe(true);
    expect(row.dataset.outcome).toBe('no-op');
    expect(row.textContent).toContain('No-op');
    expect(document.getElementById('gm-session-feedback').dataset.state)
      .toBe(ACTION_FEEDBACK_STATE.APPLIED);
    controls.destroy();
  });

  it('retains authoritative running state and exact reason when a request is refused', () => {
    const { controls } = mount();
    controls.update({ paused: false, results: [] });
    document.getElementById('gm-session-pause').click();

    controls.update({
      paused: false,
      results: [result({ outcome: 'refused', reason: 'operator not admitted', tick: 44 })],
    });

    const row = document.getElementById('gm-session-log').firstElementChild;
    expect(controls.state().paused).toBe(false);
    expect(document.getElementById('gm-session-state').dataset.paused).toBe('false');
    expect(row.dataset.outcome).toBe('refused');
    expect(row.textContent).toContain('operator not admitted');
    expect(document.getElementById('gm-session-feedback').dataset.state)
      .toBe(ACTION_FEEDBACK_STATE.REFUSED);
    controls.destroy();
  });

  it('keys results by operator plus correlation and settles only the local operator request', () => {
    const { controls } = mount({ correlations: ['shared-correlation'] });
    controls.update({ paused: false, results: [] });
    document.getElementById('gm-session-pause').click();

    controls.update({
      paused: false,
      results: [
        result({
          operator_id: 'gm-b', correlation: 'shared-correlation',
          outcome: 'refused', reason: 'remote refusal',
        }),
        result({ correlation: 'shared-correlation', outcome: 'no-op' }),
      ],
    });

    const rows = [...document.querySelectorAll('#gm-session-log > li')];
    expect(rows).toHaveLength(2);
    expect(rows.map((row) => [row.dataset.operatorId, row.dataset.outcome])).toEqual([
      ['gm-b', 'refused'],
      ['gm-a', 'no-op'],
    ]);
    expect(document.getElementById('gm-session-feedback').dataset.state)
      .toBe(ACTION_FEEDBACK_STATE.APPLIED);
    controls.destroy();
  });

  it('leaves document dispatch to the shared host registry listener', () => {
    const { controls, submitSessionPaused } = mount({
      correlations: ['gm-key-pause', 'gm-key-resume'],
    });
    document.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyP', bubbles: true }));
    document.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyR', bubbles: true }));

    expect(submitSessionPaused).not.toHaveBeenCalled();
    const event = (code) => ({ type: 'keydown', code, preventDefault: vi.fn() });
    controls.actions.dispatchKeyboardEvent(event('KeyP'), 'gm');
    controls.actions.dispatchKeyboardEvent(event('KeyR'), 'gm');

    expect(submitSessionPaused.mock.calls).toEqual([
      [true, 'gm-key-pause'],
      [false, 'gm-key-resume'],
    ]);
    controls.destroy();
  });

  it('presents a synchronous WASM ingress rejection as accessible Refused', async () => {
    const transitions = [];
    const recordTransition = (event) => {
      if (event.detail.actionId === GM_PAUSE_ACTION_ID) transitions.push(event.detail.state);
    };
    window.addEventListener('phoenix-action-feedback', recordTransition);
    const { controls, submitSessionPaused } = mount({
      submitSessionPaused: vi.fn(() => false),
    });
    controls.update({ paused: false, results: [] });

    document.getElementById('gm-session-pause').click();
    await Promise.resolve();

    const feedback = document.getElementById('gm-session-feedback');
    const row = document.getElementById('gm-session-log').firstElementChild;
    expect(submitSessionPaused).toHaveBeenCalledWith(true, 'gm-session-1');
    expect(feedback.dataset.state).toBe(ACTION_FEEDBACK_STATE.REFUSED);
    expect(feedback.textContent).toContain(t('action_feedback.refused'));
    expect(row.dataset.outcome).toBe('refused');
    expect(row.dataset.reason).toBe('ingress-rejected');
    expect(row.textContent).toContain(t('server.gm.session.reason.ingress_rejected'));
    expect(controls.state().pending).toBe(0);
    expect(controls.state().paused).toBe(false);
    expect(transitions).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.REFUSED,
    ]);
    window.removeEventListener('phoenix-action-feedback', recordTransition);
    controls.destroy();
  });

  it('times out and bounds the separate local Pending metadata deterministically', () => {
    const timers = [];
    const cancelled = [];
    const schedule = vi.fn((callback) => {
      const handle = timers.length;
      timers.push(callback);
      return handle;
    });
    const { controls } = mount({
      correlations: ['gm-one', 'gm-two', 'gm-three'],
      capacity: 2,
      timeoutMs: 25,
      schedule,
      cancelSchedule: (handle) => cancelled.push(handle),
    });
    controls.update({ paused: false, results: [] });

    controls.activate(GM_PAUSE_ACTION_ID);
    controls.activate(GM_RESUME_ACTION_ID);
    controls.activate(GM_PAUSE_ACTION_ID);

    expect(controls.state().pending).toBe(2);
    expect([...document.querySelectorAll('#gm-session-log > li')]
      .map((row) => [row.dataset.correlation, row.dataset.outcome])).toEqual([
      ['gm-one', 'timed-out'],
      ['gm-two', 'pending'],
      ['gm-three', 'pending'],
    ]);
    expect(cancelled).toContain(0);

    timers[1]();
    expect(controls.state().pending).toBe(1);
    expect(document.getElementById('gm-session-feedback').dataset.state)
      .toBe(ACTION_FEEDBACK_STATE.TIMED_OUT);
    expect([...document.querySelectorAll('#gm-session-log > li')]
      .find((row) => row.dataset.correlation === 'gm-two').dataset.outcome)
      .toBe('timed-out');
    controls.destroy();
  });

  it('resets Pending metadata and terminal rows only through the explicit run boundary', () => {
    const cancelled = [];
    const { controls } = mount({
      cancelSchedule: (handle) => cancelled.push(handle),
      schedule: () => 'pending-timer',
    });
    controls.update({ paused: false, results: [result({ correlation: 'old-run' })] });
    controls.activate(GM_PAUSE_ACTION_ID);
    expect(controls.state()).toMatchObject({ paused: false, pending: 1, authoritative: 1 });

    controls.reset();

    expect(controls.state()).toMatchObject({ paused: null, pending: 0, authoritative: 0, entries: 0 });
    expect(cancelled).toEqual(['pending-timer']);
    expect(document.getElementById('gm-session-state').textContent)
      .toBe(t('server.gm.session.state_waiting'));
    expect(document.getElementById('gm-session-log').children).toHaveLength(0);
    controls.destroy();
  });

  it('reconciles terminal rows and DOM order from every absolute projection', () => {
    const { controls } = mount({ correlations: ['still-live'], capacity: 3 });
    controls.activate(GM_PAUSE_ACTION_ID);
    controls.update({
      paused: false,
      results: [
        result({ operator_id: 'gm-b', correlation: 'b', tick: 2 }),
        result({ correlation: 'a', tick: 1, outcome: 'no-op' }),
      ],
    });
    expect([...document.querySelectorAll('#gm-session-log > li')]
      .map((row) => row.dataset.correlation)).toEqual(['b', 'a', 'still-live']);

    controls.update({
      paused: true,
      results: [result({ operator_id: 'gm-b', correlation: 'new-only', tick: 3 })],
    });
    expect([...document.querySelectorAll('#gm-session-log > li')]
      .map((row) => row.dataset.correlation)).toEqual(['new-only', 'still-live']);
    controls.destroy();
  });

  it('localises every Rust refusal token while retaining its diagnostic identity', () => {
    const reasons = Object.keys(GM_ACTION_REFUSAL_REASON_LABELS);
    const { controls } = mount({ capacity: reasons.length });
    controls.update({
      paused: false,
      results: reasons.map((reason, index) => result({
        operator_id: 'gm-b',
        correlation: `reason-${index}`,
        outcome: 'refused',
        reason,
        tick: index,
      })),
    });

    const rows = [...document.querySelectorAll('#gm-session-log > li')];
    expect(rows).toHaveLength(reasons.length);
    for (const [index, reason] of reasons.entries()) {
      expect(rows[index].dataset.reason).toBe(reason);
      expect(rows[index].textContent).toContain(t(GM_ACTION_REFUSAL_REASON_LABELS[reason]));
      expect(rows[index].textContent).not.toContain(reason);
    }
    expect(reasons).toContain('wrong-phase');
    controls.destroy();
  });

  it('locally refuses controls when no admitted GM is present', () => {
    const submitSessionPaused = vi.fn(() => true);
    const { controls } = mount({ operator: null, submitSessionPaused });
    const pause = document.getElementById('gm-session-pause');
    const resume = document.getElementById('gm-session-resume');

    expect(pause.disabled).toBe(true);
    expect(resume.getAttribute('aria-disabled')).toBe('true');
    expect(controls.activate(GM_PAUSE_ACTION_ID)).toMatchObject({
      claimed: true, actionId: GM_PAUSE_ACTION_ID, handled: false,
    });
    expect(controls.activate(GM_RESUME_ACTION_ID)).toMatchObject({
      claimed: true, actionId: GM_RESUME_ACTION_ID, handled: false,
    });
    expect(submitSessionPaused).not.toHaveBeenCalled();
    expect(controls.state().pending).toBe(0);
    controls.destroy();
  });

  it('bounds the authoritative result feed and rejects malformed projections atomically', () => {
    const { controls } = mount({ capacity: 2 });
    expect(controls.update({
      paused: false,
      results: [
        result({ correlation: 'one', tick: 1 }),
        result({ correlation: 'two', tick: 2, operator_id: 'gm-b', outcome: 'no-op' }),
        result({ correlation: 'three', tick: 3, outcome: 'refused' }),
      ],
    })).toBe(true);
    expect([...document.querySelectorAll('#gm-session-log > li')]
      .map((row) => row.dataset.correlation)).toEqual(['two', 'three']);

    expect(controls.update({ paused: true, results: [{ outcome: 'applied' }] })).toBe(false);
    expect(controls.state().paused).toBe(false);
    expect([...document.querySelectorAll('#gm-session-log > li')]
      .map((row) => row.dataset.correlation)).toEqual(['two', 'three']);
    controls.destroy();
  });
});

describe('GM session Host Channel payload parser', () => {
  it('accepts the authoritative schema as object or JSON and preserves exact outcomes', () => {
    const payload = { paused: true, results: [result({ outcome: 'no-op' })] };
    expect(parseGmSessionPayload(payload)).toEqual(payload);
    expect(parseGmSessionPayload(JSON.stringify(payload))).toEqual(payload);
  });

  it('refuses partial, untyped, or unknown-outcome projections', () => {
    expect(parseGmSessionPayload({ paused: true })).toBeUndefined();
    expect(parseGmSessionPayload({ paused: 'true', results: [] })).toBeUndefined();
    expect(parseGmSessionPayload({
      paused: true,
      results: [result({ outcome: 'pending' })],
    })).toBeUndefined();
    expect(parseGmSessionPayload('not json')).toBeUndefined();
  });
});
