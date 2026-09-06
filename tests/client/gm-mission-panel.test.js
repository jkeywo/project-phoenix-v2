// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createGmMissionPanel,
  eventIsFireable,
  eventIsPausable,
  eventIsSkippable,
  parseGmMissionPayload,
} from '../../gui/gm-mission-panel.js';
import { t } from '../../gui/strings.js';

function mount({
  correlations = ['gm-fire-1', 'gm-fire-2', 'gm-fire-3'],
  operator = { id: 'gm-a', name: 'Alex' },
  submitFireEvent = vi.fn(() => true),
  submitSetEventPaused = vi.fn(() => true),
  submitArmSkip = vi.fn(() => true),
  capacity,
  timeoutMs,
  schedule = vi.fn(),
  cancelSchedule = vi.fn(),
} = {}) {
  const queue = [...correlations];
  const panel = createGmMissionPanel({
    doc: document,
    win: window,
    t,
    submitFireEvent,
    submitSetEventPaused,
    submitArmSkip,
    getOperator: () => operator,
    getOperatorName: (id) => ({ 'gm-a': 'Alex', 'gm-b': 'Blair' }[id] || id),
    correlation: () => queue.shift(),
    now: () => 101,
    schedule,
    cancelSchedule,
    ...(capacity == null ? {} : { capacity }),
    ...(timeoutMs == null ? {} : { timeoutMs }),
  });
  return { panel, submitFireEvent, submitSetEventPaused, submitArmSkip, schedule, cancelSchedule };
}

function event(overrides = {}) {
  return {
    id: 'base-world::breach_alarm',
    label: 'server.gm.mission.heading',
    fire: true,
    pause: false,
    skip: false,
    repeatable: false,
    spent: false,
    armed: false,
    paused: false,
    skip_armed: false,
    ...overrides,
  };
}

function result(overrides = {}) {
  return {
    operator_id: 'gm-a',
    correlation: 'gm-fire-1',
    outcome: 'applied',
    tick: 42,
    target: 'base-world::breach_alarm',
    verb: 'fire',
    requested_active: true,
    ...overrides,
  };
}

const rows = () => [...document.querySelectorAll('#gm-mission-events .gm-mission-event')];
const fireButton = (id) => document.querySelector(`button[data-role="fire"][data-event-id="${id}"]`);
const pauseButton = (id) => document.querySelector(`button[data-role="pause"][data-event-id="${id}"]`);
const skipButton = (id) => document.querySelector(`button[data-role="skip"][data-event-id="${id}"]`);
const logRows = () => [...document.querySelectorAll('#gm-mission-log .gm-mission-log-entry')];

describe('GM mission panel', () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <section id="gm-mission-panel">
        <h2 id="gm-mission-heading"></h2>
        <ul id="gm-mission-events"></ul>
        <p id="gm-mission-empty"></p>
        <p id="gm-mission-feedback"></p>
        <h3 id="gm-mission-log-heading"></h3>
        <ol id="gm-mission-log"></ol>
      </section>
    `;
  });

  describe('parseGmMissionPayload', () => {
    it('accepts the exact absolute projection shape, from a string or an object', () => {
      const payload = { events: [event()], results: [result()] };
      expect(parseGmMissionPayload(payload)).toEqual(payload);
      expect(parseGmMissionPayload(JSON.stringify(payload))).toEqual(payload);
    });

    it('rejects the whole payload rather than dropping one malformed event', () => {
      for (const broken of [
        { events: [event({ id: '' })], results: [] },
        { events: [event({ label: 123 })], results: [] },
        { events: [{ ...event(), spent: 'yes' }], results: [] },
        { events: [{ ...event(), skip_armed: 'yes' }], results: [] },
        { events: [], results: [{ ...result(), outcome: 'maybe' }] },
        { events: [], results: [{ ...result(), tick: -1 }] },
        // A lever only one side knows is not degraded to a Fire.
        { events: [], results: [{ ...result(), lever: 'pause' }] },
        { events: [] },
        'not json',
        null,
      ]) {
        expect(parseGmMissionPayload(broken)).toBeUndefined();
      }
    });

    it('rejects a result payload that names neither lever it could have been', () => {
      const withoutEither = { ...result() };
      delete withoutEither.verb;
      expect(parseGmMissionPayload({ events: [], results: [withoutEither] })).toBeUndefined();
      expect(parseGmMissionPayload({
        events: [], results: [result({ verb: 'skip' })],
      })).toBeUndefined();
      const withoutPaused = { ...event() };
      delete withoutPaused.paused;
      expect(parseGmMissionPayload({ events: [withoutPaused], results: [] })).toBeUndefined();
    });
  });

  it('renders one row per controllable event with its String Table label', () => {
    const { panel } = mount();
    expect(panel.update({
      events: [event(), event({ id: 'base-world::sweep', repeatable: true })],
      results: [],
    })).toBe(true);

    expect(rows()).toHaveLength(2);
    expect(rows()[0].dataset.eventId).toBe('base-world::breach_alarm');
    expect(rows()[0].querySelector('.gm-mission-event-label').textContent)
      .toBe(t('server.gm.mission.heading'));
    expect(rows()[0].querySelector('.gm-mission-event-state').textContent)
      .toBe(t('server.gm.mission.state_ready'));
    expect(rows()[1].dataset.repeatable).toBe('true');
    expect(document.getElementById('gm-mission-empty').hidden).toBe(true);
  });

  it('says so, accessibly, when the scenario authors no GM events', () => {
    const { panel } = mount();
    panel.update({ events: [], results: [] });
    expect(rows()).toHaveLength(0);
    const empty = document.getElementById('gm-mission-empty');
    expect(empty.hidden).toBe(false);
    expect(empty.textContent).toBe(t('server.gm.mission.empty'));
    expect(empty.getAttribute('aria-live')).toBe('polite');
  });

  it('fires exactly the qualified id the projection published', () => {
    const { panel, submitFireEvent } = mount();
    panel.update({ events: [event()], results: [] });

    fireButton('base-world::breach_alarm').click();

    expect(submitFireEvent).toHaveBeenCalledWith({
      event: 'base-world::breach_alarm',
      correlation: 'gm-fire-1',
    });
    expect(panel.state().pending).toBe(1);
    expect(logRows()).toHaveLength(1);
    expect(logRows()[0].dataset.outcome).toBe('pending');
  });

  it('announces the press by the authored label the Fire button shows', () => {
    const { panel } = mount();
    panel.update({ events: [event()], results: [] });

    fireButton('base-world::breach_alarm').click();

    const feedback = document.getElementById('gm-mission-feedback');
    const spoken = t('server.gm.mission.fire_accessibility', {
      label: t('server.gm.mission.heading'),
    });
    expect(feedback.dataset.event).toBe('base-world::breach_alarm');
    expect(feedback.textContent).toContain(spoken);
    expect(feedback.textContent).not.toContain('base-world::breach_alarm');
    expect(fireButton('base-world::breach_alarm').getAttribute('aria-label')).toBe(spoken);

    // The authoritative answer speaks the same name the pending press did.
    panel.update({ events: [event({ spent: true })], results: [result()] });
    expect(feedback.textContent).toContain(spoken);
    expect(feedback.textContent).not.toContain('base-world::breach_alarm');
  });

  it('falls back to the qualified id when the event is no longer listed', () => {
    const { panel } = mount();
    panel.update({ events: [event()], results: [] });
    fireButton('base-world::breach_alarm').click();

    // The layer carrying the event is unloaded under the pending press.
    panel.update({ events: [], results: [result()] });

    const feedback = document.getElementById('gm-mission-feedback');
    expect(feedback.textContent).toContain(t('server.gm.mission.fire_accessibility', {
      label: 'base-world::breach_alarm',
    }));
  });

  it('settles the exact local press from the authoritative result feed', () => {
    const { panel, cancelSchedule } = mount();
    panel.update({ events: [event()], results: [] });
    fireButton('base-world::breach_alarm').click();

    panel.update({
      events: [event({ spent: true })],
      results: [result()],
    });

    expect(panel.state().pending).toBe(0);
    expect(cancelSchedule).toHaveBeenCalled();
    expect(logRows()).toHaveLength(1);
    expect(logRows()[0].dataset.outcome).toBe('applied');
    expect(logRows()[0].textContent).toContain('Alex');
    expect(logRows()[0].textContent).toContain('base-world::breach_alarm');
  });

  it('disables Fire on a spent one-shot event and keeps it on a repeatable one', () => {
    const { panel } = mount();
    panel.update({
      events: [
        event({ spent: true }),
        event({ id: 'base-world::sweep', repeatable: true }),
      ],
      results: [],
    });

    expect(fireButton('base-world::breach_alarm').disabled).toBe(true);
    expect(fireButton('base-world::breach_alarm').getAttribute('aria-disabled')).toBe('true');
    expect(rows()[0].querySelector('.gm-mission-event-state').textContent)
      .toBe(t('server.gm.mission.state_spent'));
    expect(fireButton('base-world::sweep').disabled).toBe(false);
  });

  it('never submits a Fire for a spent event even when the control is driven directly', () => {
    const { panel, submitFireEvent } = mount();
    panel.update({ events: [event({ spent: true })], results: [] });

    expect(panel.fire('base-world::breach_alarm')).toBe(false);
    expect(panel.fire('base-world::not-authored')).toBe(false);
    expect(submitFireEvent).not.toHaveBeenCalled();
  });

  it('lists an event that declares no Fire control without offering one', () => {
    const { panel } = mount();
    panel.update({ events: [event({ fire: false })], results: [] });

    expect(rows()).toHaveLength(1);
    expect(fireButton('base-world::breach_alarm')).toBeNull();
    expect(rows()[0].querySelector('.gm-mission-event-state').textContent)
      .toBe(t('server.gm.mission.state_unavailable'));
    expect(eventIsFireable(event({ fire: false }))).toBe(false);
  });

  it('reports an armed Fire while its handler has not run yet', () => {
    const { panel } = mount();
    panel.update({ events: [event({ armed: true })], results: [] });
    expect(rows()[0].dataset.armed).toBe('true');
    expect(rows()[0].querySelector('.gm-mission-event-state').textContent)
      .toBe(t('server.gm.mission.state_armed'));
  });

  it('presents a localized refusal rather than the wire reason', () => {
    const { panel } = mount();
    panel.update({
      events: [event()],
      results: [result({
        outcome: 'refused',
        reason: 'unknown-gm-event',
        operator_id: 'gm-b',
        correlation: 'other-gm-1',
      })],
    });

    expect(logRows()).toHaveLength(1);
    expect(logRows()[0].textContent)
      .toContain(t('server.gm.session.reason.unknown_gm_event'));
    expect(logRows()[0].textContent).not.toContain('unknown-gm-event');
    expect(logRows()[0].textContent).toContain('Blair');
  });

  it('turns a synchronous ingress refusal into an accessible terminal answer', () => {
    const { panel } = mount({ submitFireEvent: vi.fn(() => false) });
    panel.update({ events: [event()], results: [] });

    fireButton('base-world::breach_alarm').click();

    expect(panel.state().pending).toBe(0);
    expect(logRows()).toHaveLength(1);
    expect(logRows()[0].dataset.outcome).toBe('refused');
    expect(logRows()[0].textContent)
      .toContain(t('server.gm.session.reason.ingress_rejected'));
  });

  it('times out a press that never receives an authoritative answer', () => {
    let fire = null;
    const { panel } = mount({ schedule: (fn) => { fire = fn; return 7; } });
    panel.update({ events: [event()], results: [] });
    fireButton('base-world::breach_alarm').click();

    expect(panel.state().pending).toBe(1);
    fire();
    expect(panel.state().pending).toBe(0);
    expect(logRows()[0].dataset.outcome).toBe('timed-out');
  });

  it('refuses every control without an admitted GM identity', () => {
    const { panel, submitFireEvent } = mount({ operator: null });
    panel.update({ events: [event()], results: [] });

    expect(panel.refreshAdmission()).toBe(false);
    expect(document.getElementById('gm-mission-panel').dataset.admitted).toBe('false');
    expect(fireButton('base-world::breach_alarm').disabled).toBe(true);
    expect(panel.fire('base-world::breach_alarm')).toBe(false);
    expect(submitFireEvent).not.toHaveBeenCalled();
  });

  it('clears local presses and the authored list at an explicit run boundary', () => {
    const { panel } = mount();
    panel.update({ events: [event()], results: [] });
    fireButton('base-world::breach_alarm').click();
    expect(panel.state().pending).toBe(1);

    panel.reset();

    expect(panel.state()).toEqual({
      events: 0,
      fireable: 0,
      pausable: 0,
      paused: 0,
      skippable: 0,
      armedSkips: 0,
      pending: 0,
      authoritative: 0,
    });
    expect(rows()).toHaveLength(0);
    expect(logRows()).toHaveLength(0);
  });

  // ── Pause / Resume (issue #1303) ────────────────────────────────────────

  it('offers the toggle only on an event that declares Pause', () => {
    const { panel } = mount();
    panel.update({
      events: [
        event({ pause: true }),
        event({ id: 'base-world::sweep', pause: false }),
      ],
      results: [],
    });

    expect(pauseButton('base-world::breach_alarm')).not.toBeNull();
    expect(pauseButton('base-world::sweep')).toBeNull();
    expect(eventIsPausable(event({ pause: true }))).toBe(true);
    expect(eventIsPausable(event({ pause: false }))).toBe(false);
    expect(panel.state()).toMatchObject({ events: 2, pausable: 1, paused: 0 });

    // And the control is driven only by the declaration, however hard it is
    // pushed: this is the absent-control refusal on the operator's side of the
    // wire, so no request is minted at all.
    const { panel: other, submitSetEventPaused } = mount();
    other.update({ events: [event({ pause: false })], results: [] });
    expect(other.setPaused('base-world::breach_alarm', true)).toBe(false);
    expect(other.setPaused('base-world::not-authored', true)).toBe(false);
    expect(submitSetEventPaused).not.toHaveBeenCalled();
  });

  it('asks for the absolute state, and reads Resume back from the projection', () => {
    const { panel, submitSetEventPaused } = mount();
    panel.update({ events: [event({ pause: true })], results: [] });

    const control = pauseButton('base-world::breach_alarm');
    expect(control.textContent).toBe(t('server.gm.mission.pause'));
    expect(control.getAttribute('aria-label')).toBe(
      t('server.gm.mission.pause_accessibility', { label: t('server.gm.mission.heading') }),
    );
    control.click();

    expect(submitSetEventPaused).toHaveBeenCalledWith({
      event: 'base-world::breach_alarm',
      active: true,
      correlation: 'gm-fire-1',
    });

    // The authoritative projection is what moves the toggle. A reconnecting GM
    // reads exactly this, which is why the button's position is never local
    // state: the row simply renders what the simulation says.
    panel.update({
      events: [event({ pause: true, paused: true })],
      results: [result({ verb: 'pause', requested_active: true })],
    });
    const resumed = pauseButton('base-world::breach_alarm');
    expect(resumed.textContent).toBe(t('server.gm.mission.resume'));
    expect(resumed.getAttribute('aria-label')).toBe(
      t('server.gm.mission.resume_accessibility', { label: t('server.gm.mission.heading') }),
    );
    expect(rows()[0].dataset.paused).toBe('true');
    expect(rows()[0].querySelector('.gm-mission-event-state').textContent)
      .toBe(t('server.gm.mission.state_paused'));
    expect(panel.state()).toMatchObject({ paused: 1, pending: 0 });

    resumed.click();
    expect(submitSetEventPaused).toHaveBeenLastCalledWith({
      event: 'base-world::breach_alarm',
      active: false,
      correlation: 'gm-fire-2',
    });
  });

  it('keeps Fire available on a paused event, and holds the two levers apart', () => {
    const { panel, submitFireEvent, submitSetEventPaused } = mount();
    panel.update({ events: [event({ pause: true, paused: true })], results: [] });

    // A pending Pause must not disable Fire, and vice versa: they are two
    // independent authoritative requests about one event.
    pauseButton('base-world::breach_alarm').click();
    expect(fireButton('base-world::breach_alarm').disabled).toBe(false);
    fireButton('base-world::breach_alarm').click();

    expect(submitSetEventPaused).toHaveBeenCalledTimes(1);
    expect(submitFireEvent).toHaveBeenCalledTimes(1);
    expect(panel.state().pending).toBe(2);
    expect(pauseButton('base-world::breach_alarm').disabled).toBe(true);
    expect(fireButton('base-world::breach_alarm').disabled).toBe(true);
  });

  it('says which lever each result was, rather than reporting a Resume as a fire', () => {
    const { panel } = mount();
    panel.update({
      events: [event({ pause: true })],
      results: [
        result({ correlation: 'r-1', verb: 'fire' }),
        result({ correlation: 'r-2', verb: 'pause', requested_active: true }),
        result({ correlation: 'r-3', verb: 'pause', requested_active: false, outcome: 'no-op' }),
      ],
    });

    const texts = logRows().map((row) => row.textContent);
    expect(texts[0]).toContain(t('server.gm.mission.verb_fire'));
    expect(texts[1]).toContain(t('server.gm.mission.verb_pause'));
    expect(texts[2]).toContain(t('server.gm.mission.verb_resume'));
    expect(logRows().map((row) => row.dataset.verb)).toEqual(['fire', 'pause', 'pause']);
    expect(texts[1]).not.toContain(t('server.gm.mission.verb_fire'));
  });

  it('shows a reconnecting GM the paused state without any local history', () => {
    // A GM that joins mid-mission mounts a brand-new panel and is handed the
    // absolute projection. Nothing here accumulated the Pause, so what the row
    // shows is exactly what the simulation says — the same reading a live GM
    // has, which is what makes reconnect and restore the same code path.
    const { panel } = mount();
    const projection = {
      events: [
        event({ pause: true, paused: true }),
        event({ id: 'base-world::sweep', pause: true, paused: false }),
      ],
      results: [result({ verb: 'pause', requested_active: true })],
    };
    expect(panel.update(projection)).toBe(true);

    expect(pauseButton('base-world::breach_alarm').textContent)
      .toBe(t('server.gm.mission.resume'));
    expect(pauseButton('base-world::sweep').textContent).toBe(t('server.gm.mission.pause'));
    expect(rows().map((row) => row.dataset.paused)).toEqual(['true', 'false']);
    expect(panel.state()).toMatchObject({ events: 2, pausable: 2, paused: 1, pending: 0 });
    expect(logRows()[0].textContent).toContain(t('server.gm.mission.verb_pause'));
  });

  it('announces a Pause press under the lever it pressed', () => {
    const { panel } = mount();
    panel.update({ events: [event({ pause: true })], results: [] });
    pauseButton('base-world::breach_alarm').click();

    const feedback = document.getElementById('gm-mission-feedback');
    expect(feedback.dataset.verb).toBe('pause');
    expect(feedback.dataset.lever).toBe('pause');
    expect(feedback.textContent).toContain(t('server.gm.mission.pause_accessibility', {
      label: t('server.gm.mission.heading'),
    }));
    expect(feedback.textContent).not.toContain(t('server.gm.mission.fire'));
  });

  // ── Skip-next (issue #1304) ──────────────────────────────────────────────

  it('offers Skip only on events that declare it, beside an untouched Fire', () => {
    const { panel } = mount();
    panel.update({
      events: [
        event({ skip: true }),
        event({ id: 'base-world::sweep' }),
        event({ id: 'base-world::silent', fire: false, skip: true }),
      ],
      results: [],
    });

    expect(skipButton('base-world::breach_alarm')).not.toBeNull();
    expect(fireButton('base-world::breach_alarm')).not.toBeNull();
    expect(skipButton('base-world::sweep')).toBeNull();
    // A lever is a lever: an event may declare Skip and no Fire.
    expect(skipButton('base-world::silent')).not.toBeNull();
    expect(fireButton('base-world::silent')).toBeNull();
    expect(panel.state()).toMatchObject({ events: 3, fireable: 2, skippable: 2 });
    expect(eventIsSkippable(event({ skip: true }))).toBe(true);
    expect(eventIsSkippable(event())).toBe(false);
    expect(eventIsSkippable(event({ skip: true, spent: true }))).toBe(false);
  });

  it('arms exactly the qualified id the projection published, on its own seam', () => {
    const { panel, submitArmSkip, submitFireEvent } = mount();
    panel.update({ events: [event({ skip: true })], results: [] });

    skipButton('base-world::breach_alarm').click();

    expect(submitArmSkip).toHaveBeenCalledWith({
      event: 'base-world::breach_alarm',
      correlation: 'gm-fire-1',
    });
    expect(submitFireEvent).not.toHaveBeenCalled();
    expect(logRows()).toHaveLength(1);
    expect(logRows()[0].dataset.lever).toBe('skip');
    expect(logRows()[0].textContent)
      .toBe(t('server.gm.mission.skip_result_pending', {
        name: 'Alex',
        event: 'base-world::breach_alarm',
        correlation: 'gm-fire-1',
      }));
  });

  it('announces the arm by its own accessible name, not the Fire one', () => {
    const { panel } = mount();
    panel.update({ events: [event({ skip: true })], results: [] });

    skipButton('base-world::breach_alarm').click();

    const spoken = t('server.gm.mission.skip_accessibility', {
      label: t('server.gm.mission.heading'),
    });
    const feedback = document.getElementById('gm-mission-feedback');
    expect(feedback.dataset.lever).toBe('skip');
    expect(feedback.textContent).toContain(spoken);
    expect(skipButton('base-world::breach_alarm').getAttribute('aria-label')).toBe(spoken);
  });

  it('reports an armed Skip in its own state span and stays pressable', () => {
    const { panel } = mount();
    panel.update({ events: [event({ skip: true, skip_armed: true })], results: [] });

    expect(rows()[0].dataset.skipArmed).toBe('true');
    expect(rows()[0].querySelector('.gm-mission-event-skip-state').textContent)
      .toBe(t('server.gm.mission.state_skip_armed'));
    // The Fire lifecycle span is untouched by the other lever's state.
    expect(rows()[0].querySelector('.gm-mission-event-state').textContent)
      .toBe(t('server.gm.mission.state_ready'));
    // Re-arming is a deterministic No-op the simulation reports, and the
    // acceptance criterion asks for that answer to be VISIBLE.
    expect(skipButton('base-world::breach_alarm').disabled).toBe(false);
    expect(panel.state().armedSkips).toBe(1);
  });

  it('renders an authoritative Skip result with its own sentence', () => {
    const { panel } = mount();
    panel.update({
      events: [event({ skip: true, skip_armed: true })],
      results: [
        result({ correlation: 'gm-fire-1', verb: undefined, lever: 'skip-next' }),
        result({
          correlation: 'gm-fire-2', outcome: 'no-op', verb: undefined, lever: 'skip-next',
        }),
        result({ correlation: 'gm-fire-3' }),
      ],
    });

    expect(logRows()).toHaveLength(3);
    expect(logRows()[0].dataset.lever).toBe('skip');
    expect(logRows()[0].textContent).toBe(t('server.gm.mission.skip_result_applied', {
      name: 'Alex',
      event: 'base-world::breach_alarm',
      tick: '42',
      correlation: 'gm-fire-1',
    }));
    expect(logRows()[1].textContent).toBe(t('server.gm.mission.skip_result_no_op', {
      name: 'Alex',
      event: 'base-world::breach_alarm',
      tick: '42',
      correlation: 'gm-fire-2',
    }));
    // A Fire in the same feed keeps the sentence it always had.
    expect(logRows()[2].dataset.lever).toBe('fire');
    expect(logRows()[2].textContent).toBe(t('server.gm.mission.result_applied', {
      name: 'Alex',
      verb: t('server.gm.mission.verb_fire'),
      event: 'base-world::breach_alarm',
      tick: '42',
      correlation: 'gm-fire-3',
    }));
  });

  it('keeps an in-flight Skip from disabling the Fire beside it', () => {
    const { panel, submitFireEvent } = mount();
    panel.update({ events: [event({ skip: true })], results: [] });

    skipButton('base-world::breach_alarm').click();

    expect(skipButton('base-world::breach_alarm').disabled).toBe(true);
    expect(fireButton('base-world::breach_alarm').disabled).toBe(false);
    fireButton('base-world::breach_alarm').click();
    expect(submitFireEvent).toHaveBeenCalledWith({
      event: 'base-world::breach_alarm',
      correlation: 'gm-fire-2',
    });
    expect(panel.state().pending).toBe(2);
  });

  it('never submits a Skip an event does not declare, however it is driven', () => {
    const { panel, submitArmSkip } = mount();
    panel.update({
      events: [event(), event({ id: 'base-world::sweep', skip: true, spent: true })],
      results: [],
    });

    expect(panel.armSkip('base-world::breach_alarm')).toBe(false);
    expect(panel.armSkip('base-world::sweep')).toBe(false);
    expect(panel.armSkip('base-world::not-authored')).toBe(false);
    expect(submitArmSkip).not.toHaveBeenCalled();
  });

  it('shows a reconnecting GM exactly the armed Skip a live one sees', () => {
    // The projection is absolute, so a GM that reconnects mid-mission is
    // handed the same page: nothing here accumulates, and nothing is inferred
    // from a press this browser happened to make.
    const { panel } = mount();
    panel.update({
      events: [event({ skip: true, skip_armed: true })],
      results: [result({ verb: undefined, lever: 'skip-next' })],
    });

    expect(rows()[0].dataset.skipArmed).toBe('true');
    expect(panel.state()).toMatchObject({ armedSkips: 1, pending: 0, authoritative: 1 });

    // And the moment the world consumes it, the same absolute push says so.
    panel.update({
      events: [event({ skip: true, skip_armed: false })],
      results: [result({ verb: undefined, lever: 'skip-next' })],
    });
    expect(rows()[0].dataset.skipArmed).toBe('false');
    expect(rows()[0].querySelector('.gm-mission-event-skip-state').textContent)
      .toBe(t('server.gm.mission.state_skip_ready'));
    expect(panel.state().armedSkips).toBe(0);
  });

  it('holds Skip apart from Fire and Pause: three independent levers on one event', () => {
    const { panel, submitFireEvent, submitSetEventPaused, submitArmSkip } = mount();
    panel.update({ events: [event({ pause: true, skip: true })], results: [] });

    fireButton('base-world::breach_alarm').click();
    pauseButton('base-world::breach_alarm').click();
    skipButton('base-world::breach_alarm').click();

    expect(submitFireEvent).toHaveBeenCalledTimes(1);
    expect(submitSetEventPaused).toHaveBeenCalledTimes(1);
    expect(submitArmSkip).toHaveBeenCalledTimes(1);
    expect(panel.state().pending).toBe(3);
    expect(logRows().map((row) => row.dataset.lever)).toEqual(['fire', 'pause', 'skip']);
  });

  it('never settles a same-correlation result attributed to another operator', () => {
    const { panel } = mount();
    panel.update({ events: [event()], results: [] });
    fireButton('base-world::breach_alarm').click();

    panel.update({
      events: [event()],
      results: [result({ operator_id: 'gm-b', correlation: 'gm-fire-1' })],
    });

    expect(panel.state().pending).toBe(1);
  });
});
