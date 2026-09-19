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
