// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { mountWorkshopTestPanel } from '../../gui/workshop-test-panel.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { t } from '../../gui/strings.js';
import { readFileSync } from 'node:fs';
import { getTable, setBaseCatalogue, setOverlayCatalogues, setLocale, setTable } from '../../gui/strings.js';
import { WORKSHOP_MANIFEST, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';

let panel;
afterEach(() => { panel?.dispose(); document.body.replaceChildren(); });
it('repaints a running Test trace in German with typed counts and literal ship names', async () => {
  const previous = getTable();
  setBaseCatalogue(readFileSync('assets/strings/strings.csv', 'utf8'));
  setOverlayCatalogues([{ source: 'test-de-fixture', csv: 'id,de,de_source\n'
    + 'workshop.test_heading,Probe,[Test]\n'
    + 'workshop.test_trace_identity,Takt {tick} · Folge {order},[Tick {tick} · order {order}]\n'
    + 'workshop.test_trace_count,{shown} von {total} Einträgen.,[Showing {shown} of {total} records.]\n'
    + 'workshop.test_view_gm,Spielleitung,[Game Master]\n'
    + 'workshop.test_view_launched,Gestartetes Schiff,[Launched ship]\n'
    + 'workshop.test_breakpoint_comparison_ge,Mindestens,[At least]\n'
    + 'workshop.test_running,Läuft bei Takt {tick}.,[Running at tick {tick}.]\n' }]);
  setLocale('en');
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT }]));
  const run = { running: true, tick: 2345, paused: false, multiplier: 1,
    view: { view: 'ship', entity: null }, ships: [{ entity: 'ship-2', name: 'Étoile' }],
    trace: [{ tick: 1234, order: 2, kind: 'host-call', function: 'arrive', source: {} }] };
  panel = mountWorkshopTestPanel({ root: document.body, draft: () => draft, busy: () => false,
    provider: { test: { catalog: async () => ({ worlds: [WORKSHOP_WORLD], ships: ['assets/entities/hull.toml'] }),
      capture: () => ({}), start: async () => run, status: async () => run, stop: async () => {} } } });
  await vi.waitFor(() => expect(document.getElementById('workshop-test-start').disabled).toBe(false));
  document.getElementById('workshop-test-start').click();
  await vi.waitFor(() => expect(document.querySelector('.workshop-test-trace-identity')).not.toBeNull());
  const traceFilter = document.getElementById('workshop-test-trace-filter');
  traceFilter.value = 'host-call'; traceFilter.focus();
  const comparison = document.getElementById('workshop-test-breakpoint-comparison');
  comparison.value = 'ge';
  setLocale('de'); panel.refreshLanguage();
  expect(document.getElementById('workshop-test-heading').textContent).toBe('Probe');
  expect(document.querySelector('.workshop-test-trace-identity').textContent).toBe('Takt 1.234 · Folge 2');
  expect(document.getElementById('workshop-test-trace-status').textContent).toBe('1 von 1 Einträgen.');
  expect(document.getElementById('workshop-test-status').textContent).toContain('Läuft bei Takt 2.345.');
  expect([...document.getElementById('workshop-test-view').options].map(option => option.textContent))
    .toContain('Étoile');
  expect(traceFilter.value).toBe('host-call');
  expect(comparison.value).toBe('ge');
  expect(comparison.selectedOptions[0].textContent).toBe('Mindestens');
  expect(document.activeElement).toBe(traceFilter);
  panel.dispose(); panel = null; setOverlayCatalogues([]); setLocale('en'); setTable(previous);
});
it('chooses one controlled slot and only its permitted hull for a whole-scenario Test', async () => {
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT }]));
  const lead = 'assets/entities/lead.toml', wing = 'assets/entities/wing.toml';
  const start = vi.fn(async () => ({ running: true, tick: 0 }));
  panel = mountWorkshopTestPanel({ root: document.body, draft: () => draft, busy: () => false,
    provider: { test: { catalog: async () => ({ worlds: [WORKSHOP_WORLD], ships: [lead, wing],
      slots: { [WORKSHOP_WORLD]: [
        { id: 'lead', ships: [lead], default_ship: lead },
        { id: 'wing', ships: [wing], default_ship: wing },
      ] } }), capture: () => ({}), start, status: async () => ({ running: true }), stop: async () => {} } } });
  await vi.waitFor(() => expect(document.getElementById('workshop-test-start').disabled).toBe(false));
  const slot = document.getElementById('workshop-test-slot');
  const ship = document.getElementById('workshop-test-ship');
  expect(slot.hidden).toBe(false);
  expect([...ship.options].map(row => row.value)).toEqual([lead]);
  slot.value = 'wing'; slot.dispatchEvent(new Event('change'));
  expect([...ship.options].map(row => row.value)).toEqual([wing]);
  document.getElementById('workshop-test-start').click();
  await vi.waitFor(() => expect(start).toHaveBeenCalledOnce());
  expect(start.mock.calls[0][1]).toMatchObject({ world: WORKSHOP_WORLD, slot: 'wing', ship: wing });
});
it('derives a browser pack catalog from exact text instead of native byte-array encoding', async () => {
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT }]));
  const capture = vi.fn(() => ({ bytes: Uint8Array.of(3) }));
  const catalog = vi.fn(async () => ({ worlds: [WORKSHOP_WORLD], ships: ['assets/entities/hull.toml'] }));
  const start = vi.fn(async () => ({ running: true, tick: 0 }));
  const provider = { test: { catalog, capture, start, status: async () => ({ running: true }), stop: async () => {} } };
  panel = mountWorkshopTestPanel({ root: document.body, provider, draft: () => draft, busy: () => false });
  expect(panel.node.className).toBe('workshop-test');
  expect(panel.viewNode.className).toBe('workshop-test-document');
  expect(panel.node.contains(panel.viewNode)).toBe(false);
  await vi.waitFor(() => expect(catalog).toHaveBeenCalledOnce());
  expect(catalog.mock.calls[0][0]).toEqual({ 'scenarios.toml': WORKSHOP_MANIFEST, [WORKSHOP_WORLD]: WORKSHOP_WORLD_TEXT });
  document.getElementById('workshop-test-start').click();
  await vi.waitFor(() => expect(start).toHaveBeenCalledOnce());
  expect(capture).toHaveBeenCalledExactlyOnceWith(draft);
  expect(Array.from(start.mock.calls[0][0].bytes)).toEqual([3]);
  panel.dispose();
  expect(document.querySelector('.workshop-test')).toBeNull();
  expect(document.querySelector('.workshop-test-document')).toBeNull();
  panel = null;
});

it('lets Stop retire a browser boot before its slow launch promise settles', async () => {
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT }]));
  let finish;
  const start = vi.fn(() => new Promise(resolve => { finish = resolve; }));
  const cancelStart = vi.fn(), stop = vi.fn(async () => {});
  panel = mountWorkshopTestPanel({ root: document.body, draft: () => draft, busy: () => false,
    provider: { test: { catalog: async () => ({ worlds: [WORKSHOP_WORLD], ships: ['assets/entities/hull.toml'] }),
      start, cancelStart, stop, status: async () => ({ running: false }) } } });
  const button = document.getElementById('workshop-test-start');
  await vi.waitFor(() => expect(button.disabled).toBe(false));
  button.click();
  await vi.waitFor(() => expect(start).toHaveBeenCalledOnce());
  const stopping = document.getElementById('workshop-test-stop');
  expect(document.getElementById('workshop-test-status').textContent).toContain(t('workshop.test_starting'));
  expect(document.querySelector('.workshop-test').getAttribute('aria-busy')).toBe('true');
  expect(stopping.disabled).toBe(false);
  stopping.click();
  expect(cancelStart).toHaveBeenCalledOnce();
  expect(stop).not.toHaveBeenCalled();
  finish({ running: true, tick: 0 });
  await vi.waitFor(() => expect(stop).toHaveBeenCalledOnce());
  expect(panel.testing()).toBe(false);
  await vi.waitFor(() => expect(document.querySelector('.workshop-test').getAttribute('aria-busy')).toBe('false'));
  expect(document.getElementById('workshop-test-status').textContent).not.toContain(t('workshop.test_starting'));
});

it('renders bounded runtime order as filtered non-colour records with source navigation', async () => {
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT }]));
  const trace = [
    { tick: 7, order: 0, kind: 'host-call', function: 'arrive', source: { path: WORKSHOP_WORLD, line: 4 } },
    { tick: 7, order: 1, kind: 'flag-mutation', name: 'arrived', before: 0, after: 1,
      layer: 'assets/worlds/arrival.toml',
      source: { path: WORKSHOP_WORLD, line: 5 } },
    { tick: 7, order: 2, kind: 'callback-scheduled', function: 'anon$1', fire_tick: 12,
      source: { path: `${WORKSHOP_WORLD}#script.main` } },
    { tick: 7, order: 3, kind: 'flag-mutation', name: 'fleet_ready', before: 0, after: 1,
      source: {} },
    { tick: 7, order: 4, kind: 'recipient-diagnostic', action: 'addressed', message: 'No current ships',
      source: { path: WORKSHOP_WORLD, line: 9 } },
  ];
  const run = { running: true, starting: false, paused: false, tick: 7, multiplier: 1,
    ships: [{ entity: 'ship-2', name: 'Second ship' }], view: { view: 'ship', entity: null }, trace };
  const openSource = vi.fn();
  const provider = { test: { catalog: async () => ({ worlds: [WORKSHOP_WORLD], ships: ['assets/entities/hull.toml'] }),
    capture: () => ({}), start: async () => run, status: async () => run,
    control: vi.fn(async () => run), stop: async () => {} } };
  panel = mountWorkshopTestPanel({ root: document.body, provider, draft: () => draft,
    busy: () => false, openSource });
  await vi.waitFor(() => expect(document.getElementById('workshop-test-start').disabled).toBe(false));
  document.getElementById('workshop-test-start').click();
  await vi.waitFor(() => expect(document.querySelectorAll('#workshop-test-trace-list li')).toHaveLength(5));
  expect([...document.querySelectorAll('.workshop-test-trace-identity')].map(node => node.textContent))
    .toEqual([t('workshop.test_trace_identity', { tick: '7', order: '0' }),
      t('workshop.test_trace_identity', { tick: '7', order: '1' }),
      t('workshop.test_trace_identity', { tick: '7', order: '2' }),
      t('workshop.test_trace_identity', { tick: '7', order: '3' }),
      t('workshop.test_trace_identity', { tick: '7', order: '4' })]);
  expect(document.querySelector('[data-kind="recipient-diagnostic"] .workshop-test-trace-event').textContent)
    .toBe(t('workshop.test_trace_recipient', { action: 'addressed', message: 'No current ships' }));
  const flagEvents = [...document.querySelectorAll('[data-kind="flag-mutation"] .workshop-test-trace-event')]
    .map(node => node.textContent);
  expect(flagEvents[0])
    .toContain(t('workshop.test_trace_layer', { layer: 'assets/worlds/arrival.toml' }));
  expect(flagEvents[1]).toContain(t('workshop.test_trace_root'));
  expect(document.querySelectorAll('.workshop-test-trace-source')[3].textContent)
    .toBe(t('workshop.test_trace_source_unavailable'));
  const filter = document.getElementById('workshop-test-trace-filter');
  filter.value = 'callback'; filter.dispatchEvent(new Event('change'));
  expect([...document.querySelectorAll('#workshop-test-trace-list li')].map(node => node.dataset.kind))
    .toEqual(['callback-scheduled']);
  expect(document.getElementById('workshop-test-trace-status').getAttribute('role')).toBe('status');
  filter.value = 'all'; filter.dispatchEvent(new Event('change'));
  const view = document.getElementById('workshop-test-view'); view.value = 'ship-2';
  view.dispatchEvent(new Event('change'));
  await vi.waitFor(() => expect(provider.test.control).toHaveBeenCalled());
  panel.refresh();
  expect(document.querySelectorAll('#workshop-test-trace-list li')).toHaveLength(5);
  filter.value = 'callback'; filter.dispatchEvent(new Event('change'));
  document.querySelector('.workshop-test-trace-source').click();
  await vi.waitFor(() => expect(openSource).toHaveBeenCalledWith(`${WORKSHOP_WORLD}#script.main`, undefined));
});

it('launches a typed state breakpoint and renders its exact held boundary with adjacent trace', async () => {
  const draft = new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
    { path: WORKSHOP_WORLD, text: WORKSHOP_WORLD_TEXT }]));
  const hit = {
    breakpoint: { condition: { kind: 'counter', name: 'arrivals', comparison: 'ge', value: 2 } },
    current: 2, tick: 9, source: { path: WORKSHOP_WORLD, line: 12 },
    adjacent_trace: [{ tick: 8, order: 1, kind: 'flag-mutation', name: 'arrivals', before: 1, after: 2, source: {} }],
  };
  const run = { running: true, starting: false, paused: true, tick: 9, multiplier: 1,
    view: { view: 'ship', entity: null }, trace: [], breakpoint_hit: hit };
  const start = vi.fn(async () => run), openSource = vi.fn();
  panel = mountWorkshopTestPanel({ root: document.body, draft: () => draft, busy: () => false, openSource,
    provider: { test: { catalog: async () => ({
      worlds: [WORKSHOP_WORLD, 'assets/worlds/arrival.toml', 'assets/worlds/unrelated.toml'],
      ships: ['assets/entities/hull.toml'], layers: { [WORKSHOP_WORLD]: ['assets/worlds/arrival.toml'] },
    }),
      capture: () => ({}), start, status: async () => run, control: async () => run, stop: async () => {} } } });
  await vi.waitFor(() => expect(document.getElementById('workshop-test-start').disabled).toBe(false));
  expect([...document.getElementById('workshop-test-breakpoint-layer').options].map(option => option.value))
    .toEqual(['', 'assets/worlds/arrival.toml']);
  document.getElementById('workshop-test-breakpoint-enabled').click();
  document.getElementById('workshop-test-breakpoint-kind').value = 'counter';
  document.getElementById('workshop-test-breakpoint-kind').dispatchEvent(new Event('change'));
  const name = document.getElementById('workshop-test-breakpoint-name'); name.value = 'arrivals';
  name.dispatchEvent(new Event('input'));
  document.getElementById('workshop-test-breakpoint-comparison').value = 'ge';
  document.getElementById('workshop-test-start').click();
  await vi.waitFor(() => expect(start).toHaveBeenCalledOnce());
  expect(start.mock.calls[0][1].breakpoint).toEqual({ condition: {
    kind: 'counter', name: 'arrivals', comparison: 'ge', value: 1,
  } });
  expect(document.getElementById('workshop-test-breakpoint-hit').hidden).toBe(false);
  expect(document.getElementById('workshop-test-breakpoint-hit-condition').textContent).toContain('arrivals');
  expect(document.querySelectorAll('#workshop-test-breakpoint-hit-trace li')).toHaveLength(1);
  expect(document.getElementById('workshop-test-breakpoint-hit-trace').textContent).toContain('arrivals');
  document.getElementById('workshop-test-breakpoint-hit-source').click();
  await vi.waitFor(() => expect(openSource).toHaveBeenCalledWith(WORKSHOP_WORLD, 12));
});
