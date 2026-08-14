// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { formatCountdown } from '../../gui/components/ph-deadline-list.js';
import '../../gui/components/ph-deadline-list.js';

function setup() {
  document.body.innerHTML = '<ph-deadline-list id="test-panel"></ph-deadline-list>';
  return document.getElementById('test-panel');
}

function rows(host) {
  return [...host.shadowRoot.querySelectorAll('.row')].map((row) => ({
    className: row.className,
    label: row.querySelector('.label').textContent,
    clock: row.querySelector('.clock').textContent,
  }));
}

const PENDING = {
  id: 'window_opens',
  label: 'world.probe_deadlines.deadline.window_opens.label',
  remaining_secs: 92,
  state: 'pending',
};

describe('PhDeadlineList', () => {
  beforeEach(() => { document.body.innerHTML = ''; });
  afterEach(() => { document.body.innerHTML = ''; });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-deadline-list')).toBeDefined();
  });

  it('renders the empty placeholder for no deadlines, a null state, and a null list', () => {
    const el = setup();
    for (const state of [{ deadlines: [] }, null, { deadlines: null }]) {
      el.state = state;
      expect(el.shadowRoot.querySelector('.empty').textContent)
        .toBe(t('component.deadlines.empty'));
    }
  });

  it('renders a pending deadline as a resolved label and an M:SS countdown', () => {
    const el = setup();
    el.state = { deadlines: [PENDING] };
    expect(rows(el)).toEqual([
      { className: 'row', label: t(PENDING.label), clock: '1:32' },
    ]);
  });

  it('renders fired and cancelled deadlines as words, never as a stale clock', () => {
    // `remaining_secs` is 0 for a fired deadline and -1 for a cancelled one, so
    // formatting either as a countdown would read "0:00" for both and lose the
    // difference the mission actually cares about.
    const el = setup();
    el.state = {
      deadlines: [
        { ...PENDING, id: 'a', remaining_secs: 0, state: 'fired' },
        { ...PENDING, id: 'b', remaining_secs: -1, state: 'cancelled' },
      ],
    };
    expect(rows(el).map((r) => r.clock)).toEqual([
      t('component.deadlines.fired'),
      t('component.deadlines.cancelled'),
    ]);
    expect(rows(el).map((r) => r.className)).toEqual(['row spent fired', 'row spent']);
  });

  it('resolves the label through the string table rather than rendering it raw', () => {
    // No English crosses the wire: the payload carries a strings.csv id.
    const el = setup();
    el.state = { deadlines: [PENDING] };
    expect(rows(el)[0].label).not.toBe(PENDING.label);
    expect(rows(el)[0].label.length).toBeGreaterThan(0);
  });

  it('updates a row in place and drops rows that leave the payload', () => {
    const el = setup();
    el.state = { deadlines: [PENDING, { ...PENDING, id: 'other' }] };
    expect(rows(el)).toHaveLength(2);
    el.state = { deadlines: [{ ...PENDING, remaining_secs: 5 }] };
    expect(rows(el)).toEqual([
      { className: 'row', label: t(PENDING.label), clock: '0:05' },
    ]);
  });

  it('renders the heading above the list', () => {
    const el = setup();
    expect(el.shadowRoot.querySelector('.heading').textContent)
      .toBe(t('component.deadlines.heading'));
  });
});

describe('formatCountdown', () => {
  it('pads seconds and rolls over into minutes and hours', () => {
    expect(formatCountdown(0)).toBe('0:00');
    expect(formatCountdown(5)).toBe('0:05');
    expect(formatCountdown(59)).toBe('0:59');
    expect(formatCountdown(60)).toBe('1:00');
    expect(formatCountdown(3599)).toBe('59:59');
    expect(formatCountdown(3600)).toBe('1:00:00');
    expect(formatCountdown(3661)).toBe('1:01:01');
  });

  it('floors a fractional second and clamps a negative one to zero', () => {
    // The server sends whole seconds, and a negative is its "no deadline"
    // sentinel — which the component answers with a word, not a clock. Guarded
    // anyway so a payload change can never render "-1:-1".
    expect(formatCountdown(90.9)).toBe('1:30');
    expect(formatCountdown(-1)).toBe('0:00');
  });
});
