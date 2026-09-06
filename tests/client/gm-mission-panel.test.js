// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createGmMissionPanel,
  eventIsFireable,
  parseGmMissionPayload,
} from '../../gui/gm-mission-panel.js';
import { t } from '../../gui/strings.js';

function mount({
  correlations = ['gm-fire-1', 'gm-fire-2', 'gm-fire-3'],
  operator = { id: 'gm-a', name: 'Alex' },
  submitFireEvent = vi.fn(() => true),
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
    getOperator: () => operator,
    getOperatorName: (id) => ({ 'gm-a': 'Alex', 'gm-b': 'Blair' }[id] || id),
    correlation: () => queue.shift(),
    now: () => 101,
    schedule,
    cancelSchedule,
    ...(capacity == null ? {} : { capacity }),
    ...(timeoutMs == null ? {} : { timeoutMs }),
  });
  return { panel, submitFireEvent, schedule, cancelSchedule };
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
    ...overrides,
  };
}

const rows = () => [...document.querySelectorAll('#gm-mission-events .gm-mission-event')];
const fireButton = (id) => document.querySelector(`button[data-role="fire"][data-event-id="${id}"]`);
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
        { events: [], results: [{ ...result(), outcome: 'maybe' }] },
        { events: [], results: [{ ...result(), tick: -1 }] },
        { events: [] },
        'not json',
        null,
      ]) {
        expect(parseGmMissionPayload(broken)).toBeUndefined();
      }
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
      pending: 0,
      authoritative: 0,
    });
    expect(rows()).toHaveLength(0);
    expect(logRows()).toHaveLength(0);
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
