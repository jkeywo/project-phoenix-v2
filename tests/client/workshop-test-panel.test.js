// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { mountWorkshopTestPanel } from '../../gui/workshop-test-panel.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { t } from '../../gui/strings.js';
import { WORKSHOP_MANIFEST, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';

let panel;
afterEach(() => { panel?.dispose(); document.body.replaceChildren(); });
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
  await vi.waitFor(() => expect(document.querySelectorAll('#workshop-test-trace-list li')).toHaveLength(4));
  expect([...document.querySelectorAll('.workshop-test-trace-identity')].map(node => node.textContent))
    .toEqual([t('workshop.test_trace_identity', { tick: '7', order: '0' }),
      t('workshop.test_trace_identity', { tick: '7', order: '1' }),
      t('workshop.test_trace_identity', { tick: '7', order: '2' }),
      t('workshop.test_trace_identity', { tick: '7', order: '3' })]);
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
  expect(document.querySelectorAll('#workshop-test-trace-list li')).toHaveLength(4);
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
