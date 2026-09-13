// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mountWorkshopAuthoring } from '../../gui/workshop-authoring.js';
import { t } from '../../gui/strings.js';

const queue = readFileSync('src/native_host/workshop/queue.js', 'utf8');
const boot = readFileSync('src/native_host/workshop/boot.js', 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const runBoot = new AsyncFunction('window', 'document', 'mountNativeWorkshop', 'applyToDom', boot.replace(/^import .*;\r?\n/gm, ''));
let dispose;
const drain = () => window.__phoenixNativeWorkshopDrain().split('\n').filter(Boolean);
afterEach(() => {
  window.dispatchEvent(new Event('pagehide'));
  for (const key of ['__phoenixNativeWorkshopSend', '__phoenixNativeWorkshopDrain', '__phoenixNativeWorkshopReply',
    '__phoenixOperatorReply', 'PhoenixOperatorStorage', 'PhoenixOperatorStorageStatus']) delete window[key];
  vi.restoreAllMocks();
});

describe('native Workshop shared boot', () => {
  it('loads durable preferences before mounting and reports live only after source startup settles', async () => {
    document.body.innerHTML = '<main id="workshop"></main>';
    window.eval(queue);
    let completeSource;
    const ready = new Promise(resolve => { completeSource = resolve; });
    const receive = vi.fn();
    dispose = vi.fn();
    const mount = vi.fn(() => ({ ready, receive, dispose }));
    const apply = vi.fn();
    const pending = runBoot(window, document, mount, apply);
    expect(drain().map(JSON.parse)).toEqual([{ type: 'NativeOperator', operation: 'load' }]);
    expect(mount).not.toHaveBeenCalled();
    const profile = '{"kind":"project-phoenix/operator-profile","version":1}';
    window.__phoenixOperatorReply({ operation: 'load', status: 'ok', profile });
    await vi.waitFor(() => expect(mount).toHaveBeenCalledOnce());
    expect(window.PhoenixOperatorStorage.getItem('phoenix-operator-profile-v1')).toBe(profile);
    expect(apply).toHaveBeenCalledWith(document);
    window.__phoenixNativeWorkshopReply({ id: 1, status: 'sources' });
    expect(receive).toHaveBeenCalledWith({ id: 1, status: 'sources' });
    expect(drain()).toEqual([]);
    completeSource();
    await pending;
    expect(drain()).toEqual(['NativeWorkshopReady']);
    window.PhoenixOperatorStorage.setItem('phoenix-operator-profile-v1', profile);
    expect(drain().map(JSON.parse)).toEqual([{ type: 'NativeOperator', operation: 'save', profile }]);
    expect(() => window.PhoenixOperatorStorage.setItem('unrelated', 'value')).toThrow();
    window.dispatchEvent(new Event('pagehide'));
    expect(dispose).toHaveBeenCalledOnce();
  });

  it('mounts with visible storage status when preference loading fails and bounds the private queue', async () => {
    window.eval(queue);
    const mount = vi.fn(() => ({ ready: Promise.resolve(), receive: vi.fn(), dispose: vi.fn() }));
    const pending = runBoot(window, document, mount, vi.fn());
    window.__phoenixNativeWorkshopDrain();
    const failure = { operation: 'load', status: 'error', error: 'Unavailable' };
    window.__phoenixOperatorReply(failure);
    await pending;
    expect(window.PhoenixOperatorStorageStatus).toEqual(failure);
    expect(window.PhoenixOperatorStorage.getItem('phoenix-operator-profile-v1')).toBeNull();
    window.__phoenixNativeWorkshopDrain();
    for (let i = 0; i < 8; i++) window.__phoenixNativeWorkshopSend(String(i));
    expect(() => window.__phoenixNativeWorkshopSend('overflow')).toThrow();
    expect(drain()).toEqual(['0', '1', '2', '3', '4', '5', '6', '7']);
    expect(() => window.__phoenixNativeWorkshopSend({})).toThrow();
  });

  it('the shared authoring surface reads native profile storage and keeps its refusal visible', async () => {
    document.body.innerHTML = '<main id="workshop"></main>';
    const getItem = vi.fn(() => null);
    window.PhoenixOperatorStorage = { getItem, setItem: vi.fn() };
    window.PhoenixOperatorStorageStatus = { status: 'error' };
    const workspace = mountWorkshopAuthoring({ root: document.getElementById('workshop'), recovery: { load: async () => null } });
    try {
      await workspace.ready;
      expect(getItem).toHaveBeenCalledWith('phoenix-operator-profile-v1');
      expect(document.querySelector('.workshop-findings').textContent).toBe(t('editor.mod.settings.storage_refused'));
    } finally { workspace.dispose(); }
  });
});
