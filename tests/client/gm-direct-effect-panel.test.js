// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createGmDirectEffectPanel,
  entityIsDamageable,
  hullPoints,
  milliHpFromInput,
  parseGmEffectResults,
  previewDirectEffect,
  scopeIsDamageable,
} from '../../gui/gm-direct-effect-panel.js';
import {
  gmEffectScopeFromKey,
  gmEffectScopeKey,
  gmEffectScopeOptions,
  gmEffectScopeRequestFields,
  gmEffectScopeTotals,
  parseGmEffectScope,
} from '../../gui/gm-effect-scope.js';
import { t } from '../../gui/strings.js';
import { createGmConfirmationProfile, createGmConfirmationController } from '../../gui/gm-confirmation.js';

function mount({
  correlations = ['gm-effect-1', 'gm-effect-2', 'gm-effect-3'],
  operator = { id: 'gm-a', name: 'Alex' },
  submitDirectEffect = vi.fn(() => true),
  capacity,
  timeoutMs,
  schedule = vi.fn(),
  cancelSchedule = vi.fn(),
  confirmAction,
  getEntity,
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
    ...(confirmAction ? { confirmAction } : {}),
    ...(getEntity ? { getEntity } : {}),
    ...(capacity == null ? {} : { capacity }),
    ...(timeoutMs == null ? {} : { timeoutMs }),
  });
  return { panel, submitDirectEffect, schedule, cancelSchedule };
}

/**
 * The per-System breakdown the `gm_entity` projection publishes for a stationed
 * hull (issue #1311): `helm` owns two Systems, `tactical` one, and `core` is
 * authored under no Station at all — the courier's ownerless bucket, which is
 * still a legitimate System-scope target.
 *
 * Each owned row carries its Station's authored display name beside the
 * authoring key, as the projection does. `station.helm.name` is a real String
 * Table row, so the expectations below pin that the Station half of the picker
 * is localised rather than reading back the raw id.
 */
function stationedSystems() {
  return [
    {
      system_id: 'impulse-drive',
      station_id: 'helm',
      station_name: 'station.helm.name',
      name: 'Impulse Drive',
      current_milli_hp: 10_000,
      max_milli_hp: 40_000,
    },
    {
      system_id: 'manoeuvre-thrusters',
      station_id: 'helm',
      station_name: 'station.helm.name',
      name: 'Thrusters',
      current_milli_hp: 10_000,
      max_milli_hp: 20_000,
    },
    {
      system_id: 'phaser-bank',
      station_id: 'tactical',
      station_name: 'station.tactical.name',
      name: 'Phaser Bank',
      current_milli_hp: 30_000,
      max_milli_hp: 30_000,
    },
    {
      system_id: 'core',
      station_id: null,
      station_name: null,
      name: 'Core',
      current_milli_hp: 10_000,
      max_milli_hp: 10_000,
    },
  ];
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
const scopeSelect = () => document.getElementById('gm-effect-scope');
const scopeOptions = () => [...scopeSelect().options].map((option) => option.value);

function pickScope(value) {
  const select = scopeSelect();
  select.value = value;
  select.dispatchEvent(new window.Event('change'));
}
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
          <label id="gm-effect-scope-label" for="gm-effect-scope">
            <select id="gm-effect-scope"></select>
          </label>
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

  describe('shared private confirmation tracer', () => {
    function confirmations() {
      const values = new Map();
      const profile = createGmConfirmationProfile({ storage: {
        getItem: (key) => values.get(key) ?? null,
        setItem: (key, value) => values.set(key, value),
      } });
      return { profile, controller: createGmConfirmationController({ doc: document, t, profile }) };
    }

    it.each(['immediate', 'confirm', 'confirm-preview'])('ordinary damage uses %s and still waits for authority', (mode) => {
      const { profile, controller } = confirmations();
      profile.setMode('effect.damage', mode);
      const { panel, submitDirectEffect } = mount({ confirmAction: controller.request });
      panel.select(entity());
      typeAmount(5);
      damageButton().click();
      if (mode !== 'immediate') {
        expect(panel.state().pending).toBe(0);
        expect(submitDirectEffect).not.toHaveBeenCalled();
        document.querySelector('[data-confirmation-accept]').click();
      }
      expect(submitDirectEffect).toHaveBeenCalledOnce();
      expect(panel.state().pending).toBe(1);
      expect(document.getElementById('gm-effect-feedback').dataset.state).toBe('Pending');
      panel.update({ results: [result({ effect: { ...result().effect, applied_milli_hp: 5000 } })] });
      expect(panel.state().pending).toBe(0);
      expect(logRows()[0].dataset.outcome).toBe('applied');
      controller.destroy();
      panel.destroy();
    });

    it('cancels lethal damage then submits its captured amount despite a stale projection', () => {
      const { controller } = confirmations();
      let live = entity();
      const { panel, submitDirectEffect } = mount({
        confirmAction: controller.request, getEntity: () => live,
      });
      panel.select(live);
      typeAmount(1000);
      damageButton().click();
      expect(document.querySelector('#gm-action-confirmation').dataset.category).toBe('effect.lethal');
      document.querySelector('[data-confirmation-cancel]').click();
      expect(submitDirectEffect).not.toHaveBeenCalled();
      expect(panel.state().pending).toBe(0);
      damageButton().click();
      live = null;
      panel.select(null);
      controller.refresh();
      expect(document.querySelector('[data-confirmation-preview]').textContent)
        .toBe(t('settings.gm.confirmation.preview_unavailable'));
      document.querySelector('[data-confirmation-accept]').click();
      expect(submitDirectEffect).toHaveBeenCalledWith(expect.objectContaining({
        entity: 'npc-1', effect: 'damage', amount_milli_hp: 1000000,
      }));
      panel.update({ results: [result({ outcome: 'refused', reason: 'unknown-entity', effect: undefined })] });
      expect(logRows()[0].dataset.outcome).toBe('refused');
      expect(document.getElementById('gm-effect-feedback').dataset.state).toBe('Refused');
      controller.destroy();
      panel.destroy();
    });
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
        emptied: false,
        destroyed: false,
      });
      expect(previewDirectEffect(target, 'damage', 90_000)).toEqual({
        applied_milli_hp: 60_000,
        discarded_milli_hp: 30_000,
        emptied: true,
        destroyed: true,
      });
      expect(previewDirectEffect(target, 'heal', 90_000)).toEqual({
        applied_milli_hp: 40_000,
        discarded_milli_hp: 50_000,
        emptied: false,
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
      scope: 'entity',
      scope_id: null,
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
      scope: 'entity',
      scopeDamageable: false,
      amountMilliHp: 5_000,
      pending: 0,
      authoritative: 0,
    });
  });

  // -- Station- and System-scoped effects (issue #1311) --------------------
  describe('narrowed scope', () => {
    it('reads a scope both ways round without inventing a third spelling', () => {
      expect(parseGmEffectScope(undefined)).toBeNull();
      expect(parseGmEffectScope('entity')).toBeNull();
      expect(parseGmEffectScope({ station: 'helm' })).toEqual({ kind: 'station', id: 'helm' });
      expect(parseGmEffectScope({ system: 'core' })).toEqual({ kind: 'system', id: 'core' });
      for (const broken of [
        { deck: 'a' }, { station: '' }, { station: 4 }, {},
        { station: 'helm', system: 'core' }, ['station', 'helm'],
      ]) {
        expect(parseGmEffectScope(broken)).toBeUndefined();
      }

      expect(gmEffectScopeKey(null)).toBe('entity');
      expect(gmEffectScopeKey({ kind: 'station', id: 'helm' })).toBe('station:helm');
      expect(gmEffectScopeFromKey('station:helm')).toEqual({ kind: 'station', id: 'helm' });
      // Round trip through an id that itself contains the separator, which a
      // split-on-every-colon parse would quietly truncate.
      expect(gmEffectScopeFromKey('system:deck:2:core'))
        .toEqual({ kind: 'system', id: 'deck:2:core' });
      expect(gmEffectScopeFromKey('entity')).toBeNull();
      for (const broken of ['deck:a', ':helm', 'station:', 42]) {
        expect(gmEffectScopeFromKey(broken)).toBeUndefined();
      }

      expect(gmEffectScopeRequestFields(null)).toEqual({ scope: 'entity', scope_id: null });
      expect(gmEffectScopeRequestFields({ kind: 'system', id: 'core' }))
        .toEqual({ scope: 'system', scope_id: 'core' });
    });

    it('offers every Station and System the projection published, in hull order', () => {
      const target = entity({ status: { systems: stationedSystems() } });
      expect(gmEffectScopeOptions(target).map((option) => gmEffectScopeKey(option.scope)))
        .toEqual([
          'entity',
          'station:helm',
          'station:tactical',
          'system:impulse-drive',
          'system:manoeuvre-thrusters',
          'system:phaser-bank',
          'system:core',
        ]);
      // An entity with no breakdown offers only the whole hull rather than a
      // Station list the browser made up.
      expect(gmEffectScopeOptions(entity())).toHaveLength(1);
    });

    it('names a Station the way it names a System, and falls back to the id', () => {
      // One `<select>`, one convention: both halves carry the authored display
      // name the projection published, so a translated System name can never
      // sit above an untranslatable authoring key.
      const { panel } = mount();
      panel.select(entity({ status: { systems: stationedSystems() } }));
      const labelled = [...scopeSelect().options].map((option) => option.textContent);
      expect(labelled).toEqual([
        t('server.gm.effect.scope_entity'),
        t('server.gm.effect.scope_station', { name: 'Helm' }),
        t('server.gm.effect.scope_station', { name: 'Tactical' }),
        t('server.gm.effect.scope_system', { name: 'Impulse Drive' }),
        t('server.gm.effect.scope_system', { name: 'Thrusters' }),
        t('server.gm.effect.scope_system', { name: 'Phaser Bank' }),
        t('server.gm.effect.scope_system', { name: 'Core' }),
      ]);

      // A hull projected without a ship config names no owner, and the wire
      // omits the field entirely. The id then shows through rather than a
      // blank, exactly as an unnamed System's does.
      panel.select(entity({
        entity_id: 'npc-2',
        status: {
          systems: stationedSystems()
            .filter((row) => row.station_id === 'helm')
            .map(({ station_name: _dropped, ...row }) => row),
        },
      }));
      expect([...scopeSelect().options].map((option) => option.textContent)).toEqual([
        t('server.gm.effect.scope_entity'),
        t('server.gm.effect.scope_station', { name: 'helm' }),
        t('server.gm.effect.scope_system', { name: 'Impulse Drive' }),
        t('server.gm.effect.scope_system', { name: 'Thrusters' }),
      ]);
      expect(gmEffectScopeOptions(entity({
        status: { systems: [{ ...stationedSystems()[0], station_name: '' }] },
      }))[1]).toEqual({ scope: { kind: 'station', id: 'helm' }, name: 'helm' });
    });

    it('measures a scope rather than the hull it sits in', () => {
      const target = entity({ status: { systems: stationedSystems() } });
      expect(gmEffectScopeTotals(target, null)).toEqual({ current: 60_000, max: 100_000 });
      expect(gmEffectScopeTotals(target, { kind: 'station', id: 'helm' }))
        .toEqual({ current: 20_000, max: 60_000 });
      expect(gmEffectScopeTotals(target, { kind: 'system', id: 'core' }))
        .toEqual({ current: 10_000, max: 10_000 });
      expect(gmEffectScopeTotals(target, { kind: 'station', id: 'science' })).toBeNull();
      expect(gmEffectScopeTotals(target, { kind: 'system', id: 'nav-radar' })).toBeNull();
      expect(scopeIsDamageable(target, { kind: 'station', id: 'helm' })).toBe(true);
      expect(scopeIsDamageable(target, { kind: 'station', id: 'science' })).toBe(false);
    });

    it('clamps the preview to the scope but still asks the hull about lethality', () => {
      const target = entity({ status: { systems: stationedSystems() } });
      const helm = { kind: 'station', id: 'helm' };
      // 25 asked of a Station holding 20, on a hull holding 60.
      expect(previewDirectEffect(target, 'damage', 25_000, helm)).toEqual({
        applied_milli_hp: 20_000,
        discarded_milli_hp: 5_000,
        emptied: true,
        destroyed: false,
      });
      // The same press when that Station holds every point the hull has left.
      const dying = entity({
        status: {
          hull_current_milli_hp: 20_000,
          systems: stationedSystems().map((row) => (
            row.station_id === 'helm' ? row : { ...row, current_milli_hp: 0 }
          )),
        },
      });
      expect(previewDirectEffect(dying, 'damage', 25_000, helm)).toMatchObject({
        applied_milli_hp: 20_000,
        emptied: true,
        destroyed: true,
      });
      // Healing mirrors scope: helm is missing 40, the hull 40 more elsewhere.
      expect(previewDirectEffect(target, 'heal', 90_000, helm)).toMatchObject({
        applied_milli_hp: 40_000,
        discarded_milli_hp: 50_000,
        destroyed: false,
      });
    });

    it('submits the chosen Station as a scope discriminant and one id', () => {
      const { panel, submitDirectEffect } = mount();
      panel.select(entity({ status: { systems: stationedSystems() } }));
      expect(scopeOptions()).toContain('station:helm');
      pickScope('station:helm');
      typeAmount(25);
      damageButton().click();

      expect(submitDirectEffect).toHaveBeenCalledWith({
        entity: 'npc-1',
        effect: 'damage',
        amount_milli_hp: 25_000,
        correlation: 'gm-effect-1',
        scope: 'station',
        scope_id: 'helm',
      });
      expect(panel.state()).toMatchObject({ scope: 'station:helm', scopeDamageable: true });
    });

    it('shows the scope hull reading and warns about emptying it, not sinking the ship', () => {
      const { panel } = mount();
      panel.select(entity({ status: { systems: stationedSystems() } }));
      pickScope('station:helm');
      typeAmount(25);

      expect(document.getElementById('gm-effect-hull').textContent)
        .toBe(t('server.gm.effect.scope_hull', {
          scope: t('server.gm.effect.scope_station', { name: 'Helm' }),
          current: '20',
          max: '60',
        }));
      expect(warning().dataset.emptied).toBe('true');
      expect(warning().dataset.lethal).toBeUndefined();
      expect(warning().textContent).toContain(t('server.gm.effect.scope_emptied_warning', {
        scope: t('server.gm.effect.scope_station', { name: 'Helm' }),
      }));
      // The whole-hull scope is unchanged: 25 of 60 empties nothing.
      pickScope('entity');
      expect(warning().dataset.emptied).toBeUndefined();
      expect(document.getElementById('gm-effect-hull').textContent)
        .toBe(t('server.gm.effect.hull', { current: '60', max: '100' }));
    });

    it('names the narrowed scope on the accessible controls', () => {
      const { panel } = mount();
      panel.select(entity({ status: { systems: stationedSystems() } }));
      typeAmount(4);
      pickScope('system:core');

      expect(damageButton().getAttribute('aria-label'))
        .toBe(t('server.gm.effect.damage_scoped_accessibility', {
          name: 'Raider',
          scope: t('server.gm.effect.scope_system', { name: 'Core' }),
          amount: '4',
        }));
      pickScope('entity');
      expect(healButton().getAttribute('aria-label'))
        .toBe(t('server.gm.effect.heal_accessibility', { name: 'Raider', amount: '4' }));
    });

    it('leaves an unchanged picker alone while the hull numbers move', () => {
      const { panel } = mount();
      panel.select(entity({ status: { systems: stationedSystems() } }));
      pickScope('station:helm');
      const before = scopeSelect().options[1];

      // A later projection of the same ship after it took a hit: the same
      // Stations and Systems, different totals.
      panel.select(entity({
        status: {
          systems: stationedSystems().map((row) => ({ ...row, current_milli_hp: 1_000 })),
        },
      }));
      expect(panel.state().scope).toBe('station:helm');
      expect(scopeSelect().options[1]).toBe(before);
      expect(scopeSelect().value).toBe('station:helm');
      // The live numbers still move, because they live on the hull line.
      expect(document.getElementById('gm-effect-hull').textContent)
        .toBe(t('server.gm.effect.scope_hull', {
          scope: t('server.gm.effect.scope_station', { name: 'Helm' }),
          current: '2',
          max: '60',
        }));
    });

    it.each(['station:tactical', 'system:core'])('keeps unavailable %s selected without widening a press', (chosen) => {
      const { panel, submitDirectEffect } = mount();
      panel.select(entity({ status: { systems: stationedSystems() } }));
      pickScope(chosen);
      typeAmount(5);
      panel.select(entity({ status: { systems: [] } }));
      expect(panel.state()).toMatchObject({ scope: chosen, scopeDamageable: false, amountMilliHp: 5000 });
      expect(scopeSelect().value).toBe(chosen);
      expect(scopeSelect().selectedOptions[0].disabled).toBe(true);
      expect(scopeSelect().disabled).toBe(false);
      expect(damageButton().disabled).toBe(true);
      expect(healButton().disabled).toBe(true);
      damageButton().click();
      expect(panel.apply('damage')).toBe(false);
      expect(panel.apply('heal')).toBe(false);
      expect(submitDirectEffect).not.toHaveBeenCalled();
      pickScope('entity');
      expect(damageButton().disabled).toBe(false);
      damageButton().click();
      expect(submitDirectEffect).toHaveBeenCalledWith(expect.objectContaining({ scope: 'entity', scope_id: null }));
    });

    it('never submits a scope it can see names nothing damageable', () => {
      // A page with no picker in its DOM — the panel is pure over the elements
      // it is given, and the fail-closed answer has to hold without one, since
      // the picker is exactly the control that would otherwise have prevented
      // this scope from being chosen.
      scopeSelect().remove();
      const { panel, submitDirectEffect } = mount();
      panel.select(entity({ status: { systems: stationedSystems() } }));
      typeAmount(5);
      expect(panel.selectScope('station:science')).toBe(true);
      expect(panel.state()).toMatchObject({
        scope: 'station:science',
        scopeDamageable: false,
      });
      expect(document.getElementById('gm-effect-hull').textContent)
        .toBe(t('server.gm.effect.scope_undamageable', {
          scope: t('server.gm.effect.scope_station', { name: 'science' }),
        }));
      expect(damageButton().disabled).toBe(true);
      expect(panel.apply('damage')).toBe(false);
      expect(submitDirectEffect).not.toHaveBeenCalled();
    });

    it('names the scope on an authoritative result row and rejects a malformed one', () => {
      const { panel } = mount();
      panel.select(entity({ status: { systems: stationedSystems() } }));
      panel.update({ results: [result({ effect_scope: { station: 'helm' } })] });

      const row = logRows()[0];
      expect(row.dataset.scope).toBe('station:helm');
      expect(row.textContent).toContain(
        t('server.gm.activity.action.direct_effect_station', { scope: 'helm' }),
      );
      // A whole-hull result carries no scope and reads exactly as it did.
      expect(parseGmEffectResults({ results: [result()] })).toEqual([result()]);
      expect(parseGmEffectResults({ results: [result({ effect_scope: { deck: 'a' } })] }))
        .toBeUndefined();
    });
  });
});
