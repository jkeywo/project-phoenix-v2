// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createGmDirectEffectPanel,
  entityIsDamageable,
  hullPoints,
  milliHpFromInput,
  parseGmEffectResults,
  previewDirectEffect,
} from '../../gui/gm-direct-effect-panel.js';
import { t } from '../../gui/strings.js';

function mount({
  correlations = ['gm-effect-1', 'gm-effect-2', 'gm-effect-3'],
  operator = { id: 'gm-a', name: 'Alex' },
  submitDirectEffect = vi.fn(() => true),
  capacity,
  timeoutMs,
  schedule = vi.fn(),
  cancelSchedule = vi.fn(),
} = {}) {
  const queue = [...correlations];
  const panel = createGmDirectEffectPanel({
    doc: document,
    win: window,
    t,
    submitDirectEffect,
    getOperator: () => operator,
    getOperatorName: (id) => ({ 'gm-a': 'Alex', 'gm-b': 'Blair' }[id] || id),
    correlation: () => queue.shift(),
    now: () => 7,
    schedule,
    cancelSchedule,
    ...(capacity == null ? {} : { capacity }),
    ...(timeoutMs == null ? {} : { timeoutMs }),
  });
  return { panel, submitDirectEffect, schedule, cancelSchedule };
}

function entity(overrides = {}) {
  const status = {
    hull_percent: 60,
    condition_percent: null,
    destroyed: false,
    hull_current_milli_hp: 60_000,
    hull_max_milli_hp: 100_000,
    ...(overrides.status || {}),
  };
  return {
    entity_id: 'npc-1',
    name: 'Raider',
    kind: 'npc_ship',
    position: [0, 0, 0],
    faction: null,
    current_target: null,
    geometry: null,
    radar: { icon: null, colour: null, size: null, region_colour: null },
    ...overrides,
    status,
  };
}

function result(overrides = {}) {
  return {
    operator_id: 'gm-a',
    correlation: 'gm-effect-1',
    outcome: 'applied',
    tick: 42,
    target: 'npc-1',
    effect: {
      kind: 'damage',
      applied_milli_hp: 25_000,
      discarded_milli_hp: 0,
      destroyed: false,
    },
    ...overrides,
  };
}

const amountInput = () => document.getElementById('gm-effect-amount');
const damageButton = () => document.getElementById('gm-effect-damage');
const healButton = () => document.getElementById('gm-effect-heal');
const warning = () => document.getElementById('gm-effect-warning');
const logRows = () => [...document.querySelectorAll('#gm-effect-log .gm-effect-log-entry')];

function typeAmount(points) {
  const input = amountInput();
  input.value = String(points);
  input.dispatchEvent(new window.Event('input'));
}

describe('GM direct effect panel', () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <section id="gm-effect-panel">
        <h3 id="gm-effect-heading"></h3>
        <p id="gm-effect-empty"></p>
        <div id="gm-effect-controls" hidden>
          <p id="gm-effect-target"></p>
          <p id="gm-effect-hull"></p>
          <label id="gm-effect-amount-label" for="gm-effect-amount">
            <input id="gm-effect-amount" type="number" value="10">
          </label>
          <div id="gm-effect-actions">
            <button id="gm-effect-damage" type="button"></button>
            <button id="gm-effect-heal" type="button"></button>
          </div>
          <p id="gm-effect-warning"></p>
        </div>
        <p id="gm-effect-feedback"></p>
        <h4 id="gm-effect-log-heading"></h4>
        <ol id="gm-effect-log"></ol>
      </section>
    `;
  });

  describe('pure helpers', () => {
    it('converts between typed hull points and the wire milli-HP unit', () => {
      expect(milliHpFromInput('25')).toBe(25_000);
      expect(milliHpFromInput('0.5')).toBe(500);
      expect(milliHpFromInput('0')).toBeNull();
      expect(milliHpFromInput('-4')).toBeNull();
      expect(milliHpFromInput('')).toBeNull();
      expect(milliHpFromInput('lots')).toBeNull();
      expect(hullPoints(25_000)).toBe('25');
      expect(hullPoints(500)).toBe('0.5');
    });

    it('previews exactly what the authoritative reducer will resolve', () => {
      const target = entity();
      expect(previewDirectEffect(target, 'damage', 25_000)).toEqual({
        applied_milli_hp: 25_000,
        discarded_milli_hp: 0,
        destroyed: false,
      });
      expect(previewDirectEffect(target, 'damage', 90_000)).toEqual({
        applied_milli_hp: 60_000,
        discarded_milli_hp: 30_000,
        destroyed: true,
      });
      expect(previewDirectEffect(target, 'heal', 90_000)).toEqual({
        applied_milli_hp: 40_000,
        discarded_milli_hp: 50_000,
        destroyed: false,
      });
      expect(previewDirectEffect(entity({ status: { hull_current_milli_hp: null } }), 'damage', 1))
        .toBeNull();
    });

    it('knows which entities carry a hull an effect could touch', () => {
      expect(entityIsDamageable(entity())).toBe(true);
      expect(entityIsDamageable(entity({
        status: { hull_current_milli_hp: null, hull_max_milli_hp: null },
      }))).toBe(false);
      expect(entityIsDamageable(null)).toBe(false);
    });

    it('rejects the whole result feed rather than dropping one malformed row', () => {
      expect(parseGmEffectResults({ results: [result()] })).toEqual([result()]);
      expect(parseGmEffectResults(JSON.stringify({ results: [result()] }))).toEqual([result()]);
      for (const broken of [
        { results: [{ ...result(), outcome: 'maybe' }] },
        { results: [{ ...result(), tick: -1 }] },
        { results: [{ ...result(), effect: { ...result().effect, kind: 'vaporise' } }] },
        { results: [{ ...result(), effect: { ...result().effect, applied_milli_hp: -1 } }] },
        { entities: [] },
        'not json',
        null,
      ]) {
        expect(parseGmEffectResults(broken)).toBeUndefined();
      }
    });

    it('keeps a result with no resolved effect, which a pre-hull refusal has', () => {
      const refused = { ...result(), outcome: 'refused', reason: 'unknown-entity' };
      delete refused.effect;
      expect(parseGmEffectResults({ results: [refused] })).toEqual([refused]);
    });
  });

  it('says so when nothing is selected, and again when the selection has no hull', () => {
    const { panel } = mount();
    const empty = document.getElementById('gm-effect-empty');
    expect(empty.hidden).toBe(false);
    expect(empty.textContent).toBe(t('server.gm.effect.empty'));
    expect(document.getElementById('gm-effect-controls').hidden).toBe(true);

    panel.select(entity({ status: { hull_current_milli_hp: null, hull_max_milli_hp: null } }));
    expect(empty.textContent).toBe(t('server.gm.effect.undamageable'));
    expect(document.getElementById('gm-effect-panel').dataset.damageable).toBe('false');
  });

  it('shows the selected target with its absolute hull and enables both levers', () => {
    const { panel } = mount();
    panel.select(entity());

    expect(document.getElementById('gm-effect-controls').hidden).toBe(false);
    expect(document.getElementById('gm-effect-target').textContent)
      .toBe(t('server.gm.effect.target', { name: 'Raider' }));
    expect(document.getElementById('gm-effect-hull').textContent)
      .toBe(t('server.gm.effect.hull', { current: '60', max: '100' }));
    expect(damageButton().disabled).toBe(false);
    expect(healButton().disabled).toBe(false);
  });

  it('submits the selected identity and the typed amount as milli-HP', () => {
    const { panel, submitDirectEffect } = mount();
    panel.select(entity());
    typeAmount(25);
    damageButton().click();

    expect(submitDirectEffect).toHaveBeenCalledWith({
      entity: 'npc-1',
      effect: 'damage',
      amount_milli_hp: 25_000,
      correlation: 'gm-effect-1',
    });
    expect(logRows()[0].dataset.outcome).toBe('pending');
    expect(document.getElementById('gm-effect-feedback').dataset.state).toBe('Pending');
  });

  it('sends heal as its own effect kind rather than a negative amount', () => {
    const { panel, submitDirectEffect } = mount();
    panel.select(entity());
    typeAmount(5);
    healButton().click();

    expect(submitDirectEffect).toHaveBeenCalledWith(expect.objectContaining({ effect: 'heal' }));
  });

  it('refuses to submit an empty or nonsense amount', () => {
    const { panel, submitDirectEffect } = mount();
    panel.select(entity());
    typeAmount(0);
    expect(damageButton().disabled).toBe(true);
    damageButton().click();
    expect(submitDirectEffect).not.toHaveBeenCalled();
    expect(panel.apply('damage')).toBe(false);
  });

  it('warns before a press that would destroy the target', () => {
    const { panel } = mount();
    panel.select(entity());
    typeAmount(60);

    expect(warning().dataset.lethal).toBe('true');
    expect(warning().textContent)
      .toContain(t('server.gm.effect.lethal_warning', { name: 'Raider' }));
  });

  it('warns about the overflow a heal would discard', () => {
    const { panel } = mount();
    panel.select(entity());
    typeAmount(90);

    expect(warning().dataset.discarded).toBe('50000');
    expect(warning().textContent)
      .toContain(t('server.gm.effect.overflow_warning', { amount: '50' }));
  });

  it('settles the operator own press on the authoritative result and reports the numbers', () => {
    const { panel, cancelSchedule } = mount();
    panel.select(entity());
    typeAmount(25);
    damageButton().click();

    expect(panel.update({ results: [result()] })).toBe(true);
    expect(cancelSchedule).toHaveBeenCalled();
    expect(document.getElementById('gm-effect-feedback').dataset.state).toBe('Applied');
    const row = logRows()[0];
    expect(row.dataset.outcome).toBe('applied');
    expect(row.dataset.applied).toBe('25000');
    expect(row.textContent).toBe(t('server.gm.effect.result_applied', {
      name: 'Alex',
      entity: 'npc-1',
      amount: '25',
      tick: '42',
      correlation: 'gm-effect-1',
      reason: t('server.gm.effect.reason_unspecified'),
    }));
  });

  it('names the destroyed target and the discarded remainder on the result row', () => {
    const { panel } = mount();
    panel.update({
      results: [result({
        effect: {
          kind: 'damage',
          applied_milli_hp: 60_000,
          discarded_milli_hp: 30_000,
          destroyed: true,
        },
      })],
    });

    const row = logRows()[0];
    expect(row.dataset.destroyed).toBe('true');
    expect(row.textContent).toContain(t('server.gm.effect.result_destroyed'));
    expect(row.textContent).toContain(t('server.gm.effect.result_discarded', { amount: '30' }));
  });

  it('localises a refusal through the one shared reason table', () => {
    const { panel } = mount();
    panel.update({
      results: [result({ outcome: 'refused', reason: 'unknown-entity', effect: undefined })],
    });

    const row = logRows()[0];
    expect(row.dataset.reason).toBe('unknown-entity');
    expect(row.textContent).toContain(t('server.gm.session.reason.unknown_entity'));
  });

  it('preserves an unrecognised future refusal reason rather than hiding it', () => {
    const { panel } = mount();
    panel.update({
      results: [result({ outcome: 'refused', reason: 'from-the-future', effect: undefined })],
    });

    expect(logRows()[0].textContent)
      .toContain(t('server.gm.effect.reason_unknown', { reason: 'from-the-future' }));
  });

  it('settles a synchronous ingress refusal locally rather than hanging', () => {
    const { panel } = mount({ submitDirectEffect: vi.fn(() => false) });
    panel.select(entity());
    typeAmount(5);
    damageButton().click();

    expect(logRows()[0].dataset.outcome).toBe('refused');
    expect(logRows()[0].dataset.reason).toBe('ingress-rejected');
    expect(document.getElementById('gm-effect-feedback').dataset.state).toBe('Refused');
  });

  it('holds one press per target at a time', () => {
    const { panel, submitDirectEffect } = mount();
    panel.select(entity());
    typeAmount(5);
    damageButton().click();
    expect(damageButton().disabled).toBe(true);
    healButton().click();

    expect(submitDirectEffect).toHaveBeenCalledTimes(1);
  });

  it('is inert with no admitted GM operator', () => {
    const { panel, submitDirectEffect } = mount({ operator: null });
    panel.select(entity());
    typeAmount(5);

    expect(document.getElementById('gm-effect-panel').dataset.admitted).toBe('false');
    expect(damageButton().disabled).toBe(true);
    damageButton().click();
    expect(submitDirectEffect).not.toHaveBeenCalled();
  });

  it('clears local presses and the feed at the run boundary', () => {
    const { panel, cancelSchedule } = mount();
    panel.select(entity());
    typeAmount(5);
    damageButton().click();
    panel.reset();

    expect(cancelSchedule).toHaveBeenCalled();
    expect(logRows()).toHaveLength(0);
    expect(panel.state()).toEqual({
      selected: null,
      damageable: false,
      amountMilliHp: 5_000,
      pending: 0,
      authoritative: 0,
    });
  });
});
