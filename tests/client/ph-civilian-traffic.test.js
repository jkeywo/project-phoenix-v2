// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { complianceLabel, formatLeg, orderActionArgs } from '../../gui/components/ph-civilian-traffic.js';
import '../../gui/components/ph-civilian-traffic.js';

function setup() {
  document.body.innerHTML = '<ph-civilian-traffic id="test-panel"></ph-civilian-traffic>';
  return document.getElementById('test-panel');
}

function rows(host) {
  return [...host.shadowRoot.querySelectorAll('.row')].map((row) => ({
    className: row.className,
    name: row.querySelector('.name').textContent,
    leg: row.querySelector('.leg').textContent,
    state: row.querySelector('.state').textContent,
  }));
}

const HAULER = {
  uuid: 'civ-1',
  name: 'world.entity.hauler_kestrel.name',
  route: 'depot_run',
  leg: 0,
  legs: 3,
  order: '',
  order_destination: '',
  compliance: 'unordered',
  reason: '',
  order_options: [{
    id: 'storm_shelter',
    label: 'world.falling_skyway.civilian_order.storm_shelter',
    order: { verb: 'divert', route: 'storm_shelter_run' },
  }],
};

describe('PhCivilianTraffic', () => {
  beforeEach(() => { document.body.innerHTML = ''; });
  afterEach(() => { document.body.innerHTML = ''; });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-civilian-traffic')).toBeDefined();
  });

  it('renders the empty placeholder for no traffic, a null state, and a null list', () => {
    const el = setup();
    for (const state of [{ civilians: [] }, null, { civilians: null }]) {
      el.state = state;
      expect(el.shadowRoot.querySelector('.empty').textContent)
        .toBe(t('component.civilians.empty'));
    }
  });

  it('renders a craft as a resolved name, a one-based leg and its compliance word', () => {
    const el = setup();
    el.state = { civilians: [HAULER] };
    expect(rows(el)).toEqual([
      {
        className: 'row',
        name: t(HAULER.name),
        // The wire carries a zero-based cursor index; a crew counts from one.
        leg: '1/3',
        state: t('component.civilians.compliance.unordered'),
      },
    ]);
  });

  it('renders an accessible authored order and emits the existing Navigation action', () => {
    const el = setup();
    el.sendAction = vi.fn();
    el.state = { civilians: [HAULER] };

    const button = el.shadowRoot.querySelector('button[data-order-id="storm_shelter"]');
    expect(button).not.toBeNull();
    expect(button.getAttribute('aria-label')).toContain(t(HAULER.name));
    button.focus();
    el.state = { civilians: [{ ...HAULER, compliance: 'received' }] };
    expect(el.shadowRoot.activeElement).toBe(button);
    button.click();

    expect(el.sendAction).toHaveBeenCalledWith('order_civilian', {
      target: HAULER.uuid,
      verb: 'divert',
      route: 'storm_shelter_run',
    });
  });

  // The distinction the whole panel exists for: a craft that said no and
  // carried on is not the same as one that agreed and got stuck, and only the
  // second needs a crew. Rendering both the same word — or the same row style —
  // would hide exactly the case Navigation is watching for.
  it('styles a refusal and a stuck craft differently, and never identically', () => {
    const el = setup();
    el.state = {
      civilians: [
        { ...HAULER, uuid: 'a', compliance: 'refused', reason: 'civilian.compliance.reason.declined' },
        { ...HAULER, uuid: 'b', compliance: 'non_compliant', reason: 'civilian.compliance.reason.unable' },
      ],
    };
    const [refused, stuck] = rows(el);
    expect(refused.className).toBe('row refused');
    expect(stuck.className).toBe('row stuck');
    expect(refused.state).toBe(t('component.civilians.compliance.refused'));
    expect(stuck.state).toBe(t('component.civilians.compliance.non_compliant'));
    expect(refused.state).not.toBe(stuck.state);
  });

  it('shows an order still being answered as pending rather than as compliance', () => {
    const el = setup();
    for (const compliance of ['received', 'acknowledged']) {
      el.state = { civilians: [{ ...HAULER, compliance, order: 'divert' }] };
      expect(rows(el)[0].className).toBe('row pending');
    }
    el.state = { civilians: [{ ...HAULER, compliance: 'complying', order: 'divert' }] };
    expect(rows(el)[0].className).toBe('row');
  });

  it('resolves the reason id into the row title, and drops it when there is none', () => {
    const el = setup();
    el.state = { civilians: [{ ...HAULER, compliance: 'refused', reason: 'civilian.compliance.reason.declined' }] };
    expect(el.shadowRoot.querySelector('.row').title)
      .toBe(t('civilian.compliance.reason.declined'));
    el.state = { civilians: [HAULER] };
    expect(el.shadowRoot.querySelector('.row').hasAttribute('title')).toBe(false);
  });

  it('drops rows for craft that have left the world', () => {
    const el = setup();
    el.state = { civilians: [{ ...HAULER, uuid: 'a' }, { ...HAULER, uuid: 'b' }] };
    expect(rows(el)).toHaveLength(2);
    el.state = { civilians: [{ ...HAULER, uuid: 'b' }] };
    expect(rows(el)).toHaveLength(1);
  });

  it('falls back to the uuid when a craft carries no name id', () => {
    const el = setup();
    el.state = { civilians: [{ ...HAULER, name: '' }] };
    expect(rows(el)[0].name).toBe('civ-1');
  });

  describe('formatLeg', () => {
    it('is empty for a craft with no lane, and clamps past the last leg', () => {
      expect(formatLeg({ route: '', leg: 0, legs: 0 })).toBe('');
      expect(formatLeg({ route: 'depot_run', leg: 0, legs: 0 })).toBe('');
      expect(formatLeg({ route: 'depot_run', leg: 2, legs: 3 })).toBe('3/3');
      // A looping route's cursor can read past the chain before it wraps.
      expect(formatLeg({ route: 'depot_run', leg: 9, legs: 3 })).toBe('3/3');
    });
  });

  describe('complianceLabel', () => {
    it('maps every state the server publishes to a string id that resolves', () => {
      for (const state of [
        'unordered', 'received', 'acknowledged', 'complying', 'non_compliant', 'refused',
      ]) {
        const id = complianceLabel(state);
        expect(t(id)).not.toContain('⟨');
      }
      // An empty state is the resting one, not a miss.
      expect(complianceLabel('')).toBe('component.civilians.compliance.unordered');
    });
  });

  describe('orderActionArgs', () => {
    it('flattens only complete authored orders', () => {
      expect(orderActionArgs('civ-1', { verb: 'hold' }))
        .toEqual({ target: 'civ-1', verb: 'hold' });
      expect(orderActionArgs('civ-1', { verb: 'divert', route: 'lee' })).toEqual({
        target: 'civ-1', verb: 'divert', route: 'lee',
      });
      expect(orderActionArgs('civ-1', { verb: 'divert' })).toBeNull();
      expect(orderActionArgs('', { verb: 'hold' })).toBeNull();
    });
  });
});
