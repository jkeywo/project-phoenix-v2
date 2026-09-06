// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createGmSpawnPanel,
  parseGmSpawnPayload,
  placementToWire,
} from '../../gui/gm-spawn-panel.js';
import { t } from '../../gui/strings.js';

/**
 * A stand-in for `<ph-navigation-map>`: it records arming and lets a test emit
 * the same `navplace` the real chart emits, so the panel's half of the gesture
 * is exercised without a canvas. The chart's own half — press picks position,
 * drag picks heading, keyboard does both — is covered in
 * `ph-navigation-map.test.js`.
 */
function makeMap() {
  const map = document.createElement('div');
  map.id = 'gm-entity-map';
  map.armCalls = 0;
  map.cancelCalls = 0;
  map.navigationBeginPlacement = () => { map.armCalls += 1; return true; };
  map.navigationCancelPlacement = () => { map.cancelCalls += 1; return true; };
  map.emitPlacement = (detail) => map.dispatchEvent(new CustomEvent('navplace', { detail }));
  map.emitCancel = () => map.dispatchEvent(new CustomEvent('navplacecancel', { detail: null }));
  return map;
}

function mount({
  correlations = ['gm-place-1', 'gm-place-2', 'gm-place-3'],
  operator = { id: 'gm-a', name: 'Alex' },
  submitPlacement = vi.fn(() => true),
  capacity,
  timeoutMs,
  schedule = vi.fn(),
  cancelSchedule = vi.fn(),
  map = null,
} = {}) {
  const queue = [...correlations];
  const panel = createGmSpawnPanel({
    doc: document,
    win: window,
    t,
    submitPlacement,
    getOperator: () => operator,
    getOperatorName: (id) => ({ 'gm-a': 'Alex', 'gm-b': 'Blair' }[id] || id),
    getMap: () => map,
    correlation: () => queue.shift(),
    now: () => 101,
    schedule,
    cancelSchedule,
    ...(capacity == null ? {} : { capacity }),
    ...(timeoutMs == null ? {} : { timeoutMs }),
  });
  return { panel, submitPlacement, schedule, cancelSchedule, map };
}

function entry(overrides = {}) {
  return {
    id: 'raider',
    label: 'server.gm.spawn.heading',
    variants: [],
    ...overrides,
  };
}

function result(overrides = {}) {
  return {
    operator_id: 'gm-a',
    correlation: 'gm-place-1',
    outcome: 'applied',
    tick: 42,
    target: 'raider',
    ...overrides,
  };
}

const rows = () => [...document.querySelectorAll('#gm-spawn-palette .gm-spawn-entry')];
const placeButton = (id) => document.querySelector(`button[data-role="place"][data-palette-id="${id}"]`);
const logRows = () => [...document.querySelectorAll('#gm-spawn-log .gm-spawn-log-entry')];

describe('GM placement panel', () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <section id="gm-spawn-panel">
        <h2 id="gm-spawn-heading"></h2>
        <ul id="gm-spawn-palette"></ul>
        <p id="gm-spawn-empty"></p>
        <div id="gm-spawn-exact-form">
          <input id="gm-spawn-x" type="number" value="0">
          <input id="gm-spawn-z" type="number" value="0">
          <input id="gm-spawn-facing" type="number" value="0">
          <button type="button" id="gm-spawn-exact"></button>
        </div>
        <p id="gm-spawn-feedback"></p>
        <ol id="gm-spawn-log"></ol>
      </section>`;
  });

  describe('projection parsing', () => {
    it('accepts a complete absolute payload as a string or an object', () => {
      const payload = {
        palette: [entry({ variants: [{ id: 'blood_eagle', label: 'server.gm.spawn.place' }] })],
        results: [result()],
      };
      expect(parseGmSpawnPayload(payload)).toEqual(payload);
      expect(parseGmSpawnPayload(JSON.stringify(payload))).toEqual(payload);
    });

    it('rejects the whole payload rather than offering an incomplete vocabulary', () => {
      expect(parseGmSpawnPayload('not json')).toBeUndefined();
      expect(parseGmSpawnPayload({ palette: [], results: null })).toBeUndefined();
      expect(parseGmSpawnPayload({
        palette: [entry(), { id: '', label: 'x', variants: [] }],
        results: [],
      })).toBeUndefined();
      expect(parseGmSpawnPayload({
        palette: [entry({ variants: [{ id: 'v' }] })],
        results: [],
      })).toBeUndefined();
      expect(parseGmSpawnPayload({
        palette: [],
        results: [result({ outcome: 'pending' })],
      })).toBeUndefined();
    });
  });

  describe('fixed-point conversion', () => {
    it('converts resolved metres and degrees to the wire once', () => {
      expect(placementToWire({ x: 120.5, z: -40.25, heading: 90 })).toEqual({
        position_mm: [120500, 0, -40250],
        heading_mdeg: 90000,
      });
    });

    it('normalises a heading and refuses anything that is not a placement', () => {
      expect(placementToWire({ x: 0, z: 0, heading: -90 }).heading_mdeg).toBe(270000);
      expect(placementToWire({ x: 0, z: 0, heading: 450 }).heading_mdeg).toBe(90000);
      expect(placementToWire({ x: Number.NaN, z: 0, heading: 0 })).toBeUndefined();
      expect(placementToWire({ x: 0, z: 0, heading: Number.POSITIVE_INFINITY })).toBeUndefined();
      // The action's own coordinate bound, applied here so an out-of-range
      // gesture never becomes a request the simulation has to reject.
      expect(placementToWire({ x: 5_000_001, z: 0, heading: 0 })).toBeUndefined();
      expect(placementToWire(null)).toBeUndefined();
    });
  });

  describe('the palette', () => {
    it('renders the authored rows with localized labels and a Place control', () => {
      const { panel } = mount();
      expect(panel.update({ palette: [entry()], results: [] })).toBe(true);
      expect(rows()).toHaveLength(1);
      expect(rows()[0].dataset.paletteId).toBe('raider');
      expect(rows()[0].querySelector('.gm-spawn-entry-label').textContent)
        .toBe(t('server.gm.spawn.heading'));
      expect(placeButton('raider').getAttribute('aria-label'))
        .toBe(t('server.gm.spawn.place_accessibility', { label: t('server.gm.spawn.heading') }));
      expect(document.getElementById('gm-spawn-empty').hidden).toBe(true);
    });

    it('says so when the scenario authors no palette at all', () => {
      const { panel } = mount();
      panel.update({ palette: [], results: [] });
      expect(rows()).toHaveLength(0);
      expect(document.getElementById('gm-spawn-empty').hidden).toBe(false);
      expect(document.getElementById('gm-spawn-empty').textContent)
        .toBe(t('server.gm.spawn.empty'));
      expect(document.getElementById('gm-spawn-exact').disabled).toBe(true);
    });

    it('offers only the authored variants, defaulting to the bare template', () => {
      const { panel } = mount();
      panel.update({
        palette: [entry({ variants: [{ id: 'blood_eagle', label: 'server.gm.spawn.place' }] })],
        results: [],
      });
      const select = document.querySelector('.gm-spawn-entry-variant');
      expect([...select.options].map((option) => option.value)).toEqual(['', 'blood_eagle']);
      expect(select.value).toBe('');
      expect(panel.state().variants).toEqual({});
    });
  });

  describe('placing', () => {
    it('arms the chart, then submits the placement the chart resolved', () => {
      const map = makeMap();
      const { panel, submitPlacement } = mount({ map });
      panel.update({ palette: [entry()], results: [] });

      placeButton('raider').click();
      expect(map.armCalls).toBe(1);
      expect(panel.state().arming).toBe('raider');

      map.emitPlacement({ x: 120.5, z: -40.25, heading: 90 });
      expect(submitPlacement).toHaveBeenCalledWith({
        palette: 'raider',
        variant: null,
        position_mm: [120500, 0, -40250],
        heading_mdeg: 90000,
        correlation: 'gm-place-1',
      });
      expect(panel.state()).toMatchObject({ arming: null, pending: 1 });
      expect(logRows()[0].dataset.outcome).toBe('pending');
    });

    it('carries the chosen authored variant and nothing else', () => {
      const map = makeMap();
      const { panel, submitPlacement } = mount({ map });
      panel.update({
        palette: [entry({ variants: [{ id: 'blood_eagle', label: 'server.gm.spawn.place' }] })],
        results: [],
      });
      const select = document.querySelector('.gm-spawn-entry-variant');
      select.value = 'blood_eagle';
      select.dispatchEvent(new Event('change'));

      placeButton('raider').click();
      map.emitPlacement({ x: 0, z: 0, heading: 0 });
      expect(submitPlacement.mock.calls[0][0]).toMatchObject({
        palette: 'raider',
        variant: 'blood_eagle',
      });
    });

    it('refuses to submit a variant the projection no longer publishes', () => {
      const map = makeMap();
      const { panel, submitPlacement } = mount({ map });
      panel.update({
        palette: [entry({ variants: [{ id: 'blood_eagle', label: 'server.gm.spawn.place' }] })],
        results: [],
      });
      const select = document.querySelector('.gm-spawn-entry-variant');
      select.value = 'blood_eagle';
      select.dispatchEvent(new Event('change'));
      // The layer authoring that variant unloads: the row survives, the variant
      // does not, and the stale choice must not ride along.
      panel.update({ palette: [entry()], results: [] });
      expect(panel.place('raider', { x: 0, z: 0, heading: 0 })).toBe(true);
      expect(submitPlacement.mock.calls[0][0].variant).toBe(null);
    });

    it('names the row the typed form will place, so it is never a guess', () => {
      const { panel } = mount();
      panel.update({ palette: [entry(), entry({ id: 'tender' })], results: [] });
      const button = document.getElementById('gm-spawn-exact');
      expect(button.dataset.paletteId).toBe('raider');
      panel.arm('tender');
      expect(button.dataset.paletteId).toBe('tender');
    });

    it('places at typed coordinates with no gesture at all', () => {
      const { panel, submitPlacement } = mount();
      panel.update({ palette: [entry()], results: [] });
      document.getElementById('gm-spawn-x').value = '250';
      document.getElementById('gm-spawn-z').value = '-125';
      document.getElementById('gm-spawn-facing').value = '45';
      document.getElementById('gm-spawn-exact').click();
      expect(submitPlacement).toHaveBeenCalledWith({
        palette: 'raider',
        variant: null,
        position_mm: [250000, 0, -125000],
        heading_mdeg: 45000,
        correlation: 'gm-place-1',
      });
    });

    it('a second press disarms rather than queueing a second placement', () => {
      const map = makeMap();
      const { panel } = mount({ map });
      panel.update({ palette: [entry()], results: [] });
      placeButton('raider').click();
      placeButton('raider').click();
      expect(map.cancelCalls).toBe(1);
      expect(panel.state().arming).toBe(null);
    });

    it('a cancelled gesture leaves nothing armed and submits nothing', () => {
      const map = makeMap();
      const { panel, submitPlacement } = mount({ map });
      panel.update({ palette: [entry()], results: [] });
      placeButton('raider').click();
      map.emitCancel();
      expect(panel.state().arming).toBe(null);
      map.emitPlacement({ x: 1, z: 1, heading: 0 });
      expect(submitPlacement).not.toHaveBeenCalled();
    });

    it('never submits without an admitted operator', () => {
      const map = makeMap();
      const { panel, submitPlacement } = mount({ map, operator: null });
      panel.update({ palette: [entry()], results: [] });
      expect(placeButton('raider').disabled).toBe(true);
      expect(panel.arm('raider')).toBe(false);
      expect(panel.place('raider', { x: 0, z: 0, heading: 0 })).toBe(false);
      expect(submitPlacement).not.toHaveBeenCalled();
    });

    it('turns a synchronous ingress refusal into an accessible terminal answer', () => {
      const { panel } = mount({ submitPlacement: vi.fn(() => false) });
      panel.update({ palette: [entry()], results: [] });
      expect(panel.place('raider', { x: 0, z: 0, heading: 0 })).toBe(true);
      expect(logRows()[0].dataset.outcome).toBe('refused');
      expect(logRows()[0].dataset.reason).toBe('ingress-rejected');
      expect(document.getElementById('gm-spawn-feedback').dataset.state).toBe('Refused');
    });
  });

  describe('the result feed', () => {
    it('settles the operator own pending placement from the absolute projection', () => {
      const { panel, cancelSchedule } = mount();
      panel.update({ palette: [entry()], results: [] });
      panel.place('raider', { x: 0, z: 0, heading: 0 });
      panel.update({ palette: [entry()], results: [result()] });

      expect(cancelSchedule).toHaveBeenCalled();
      expect(panel.state()).toMatchObject({ pending: 0, authoritative: 1 });
      const row = logRows()[0];
      expect(row.dataset.outcome).toBe('applied');
      expect(row.dataset.palette).toBe('raider');
      expect(row.textContent).toBe(t('server.gm.spawn.result_applied', {
        name: 'Alex',
        entry: t('server.gm.spawn.heading'),
        tick: '42',
        correlation: 'gm-place-1',
      }));
    });

    it('names another operator placement and localizes its refusal reason', () => {
      const { panel } = mount();
      panel.update({
        palette: [entry()],
        results: [result({
          operator_id: 'gm-b',
          correlation: 'gm-place-9',
          outcome: 'refused',
          reason: 'unknown-gm-palette-entry',
        })],
      });
      const row = logRows()[0];
      expect(row.dataset.outcome).toBe('refused');
      expect(row.textContent).toContain('Blair');
      expect(row.textContent)
        .toContain(t('server.gm.session.reason.unknown_gm_palette_entry'));
    });

    it('clears everything at the run boundary', () => {
      const map = makeMap();
      const { panel } = mount({ map });
      panel.update({ palette: [entry()], results: [result()] });
      panel.place('raider', { x: 0, z: 0, heading: 0 });
      panel.reset();
      expect(panel.state()).toMatchObject({
        palette: 0,
        arming: null,
        pending: 0,
        authoritative: 0,
      });
      expect(logRows()).toHaveLength(0);
      expect(rows()).toHaveLength(0);
    });
  });
});
