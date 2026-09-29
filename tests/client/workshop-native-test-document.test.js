// @vitest-environment jsdom
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { ReadableStream } from 'node:stream/web';
import { mountNativeTestDocument } from '../../gui/workshop-native-test-document.js';
import { createNativeWorkshopTestView } from '../../editor/workshop-native-test-view.js';
import { createNativeWorkshopProvider } from '../../editor/workshop-provider.js';

let image, hud, gmRoot, status, gm, view;
const run = id => ({ running: true, frame_url: `${location.origin}/workshop-test-frame/${id}/view.png`,
  presentation_url: `${location.origin}/workshop-test-frame/${id}/presentation.json`, view: { view: 'ship', entity: null } });
const response = value => ({ ok: true, body: new ReadableStream({ start(controller) {
  controller.enqueue(value instanceof Uint8Array ? value : new TextEncoder().encode(JSON.stringify(value)));
  controller.close();
} }) });
const packet = sequence => ({ sequence, channels: { hud: '{"speed":1}', gm_entity: '{"entities":[]}' }, role_presets: '[]' });
beforeEach(() => {
  document.body.innerHTML = '<img id="image"><iframe id="hud"></iframe><section id="gm"></section><p id="status"></p>';
  image = document.getElementById('image'); hud = document.getElementById('hud');
  gmRoot = document.getElementById('gm'); status = document.getElementById('status');
  gm = { channel: vi.fn(), setRolePresets: vi.fn(), dispose: vi.fn() };
  window.URL.createObjectURL = vi.fn(() => 'blob:native-frame'); window.URL.revokeObjectURL = vi.fn();
});
afterEach(() => { view?.dispose(); view = null; delete window.URL.createObjectURL; delete window.URL.revokeObjectURL; });
const mount = fetcher => mountNativeTestDocument({ win: window, image, hud, gmRoot, gm, status,
  unavailable: 'Presentation unavailable', fetcher });

it('shows native ship pixels and feeds the ordinary read-only GM and role payloads without replaying unchanged channels', async () => {
  hud.contentWindow.__updateHud = vi.fn();
  view = mount(async url => response(url.endsWith('.png') ? Uint8Array.of(1, 2) : packet(1)));
  await view.update(run('first'));
  expect(image.src).toBe('blob:native-frame'); expect(gmRoot.hidden).toBe(true);
  expect(hud.contentWindow.__updateHud).toHaveBeenCalledWith('{"speed":1}');
  expect(gm.channel).toHaveBeenCalledWith('gm_entity', '{"entities":[]}');
  expect(gm.setRolePresets).toHaveBeenCalledWith('[]');
  await view.update({ ...run('first'), view: { view: 'game-master' } });
  expect(gmRoot.hidden).toBe(false); expect(image.hidden).toBe(true); expect(hud.hidden).toBe(true);
  expect(gm.channel).toHaveBeenCalledTimes(1); expect(gm.setRolePresets).toHaveBeenCalledTimes(1);
});

it('refuses foreign origins, mismatched generations and oversized presentation bodies', async () => {
  const fetcher = vi.fn(async url => response(url.endsWith('.png') ? new Uint8Array(4 * 1024 * 1024 + 1) : packet(1)));
  view = mount(fetcher);
  await view.update({ ...run('first'), frame_url: 'https://elsewhere.test/workshop-test-frame/first/view.png' });
  await view.update({ ...run('first'), presentation_url: run('second').presentation_url });
  expect(fetcher).not.toHaveBeenCalled();
  await view.update(run('first'));
  expect(image.hasAttribute('src')).toBe(false); expect(status.textContent).toBe('Presentation unavailable');
  expect(gm.channel).not.toHaveBeenCalled();
});

it('does not revive a document when pending responses finish after disposal', async () => {
  const replies = [];
  view = mount(url => new Promise(resolve => replies.push(() => resolve(response(url.endsWith('.png') ? Uint8Array.of(1) : packet(1))))));
  const pending = view.update(run('first'));
  view.dispose(); replies.forEach(reply => reply()); await pending;
  expect(image.hasAttribute('src')).toBe(false); expect(gm.channel).not.toHaveBeenCalled();
});

it('ignores older presentation packets and messages from a different window', async () => {
  let sequence = 2;
  const fetcher = vi.fn(async url => response(url.endsWith('.png') ? Uint8Array.of(1) : packet(sequence)));
  view = mount(fetcher);
  window.dispatchEvent(new MessageEvent('message', { origin: location.origin, source: null,
    data: { type: 'phoenix-native-test-view', run: run('first') } }));
  expect(fetcher).not.toHaveBeenCalled();
  await view.update(run('first')); sequence = 1; await view.update(run('first'));
  expect(window.URL.createObjectURL).toHaveBeenCalledTimes(1);
  await view.update(run('second')); expect(fetcher).toHaveBeenCalledTimes(4);
});

it('replaces the native document on a new child run and removes it on stop', () => {
  const target = document.createElement('div'); document.body.append(target);
  view = createNativeWorkshopTestView({ mount: target, title: 'Test' });
  view.update(run('first'));
  const first = target.querySelector('iframe'); expect(first.title).toBe('Test');
  view.update(run('first')); expect(target.querySelector('iframe')).toBe(first);
  view.update(run('second')); expect(first.isConnected).toBe(false);
  expect(target.querySelector('iframe')).not.toBe(first);
  view.update({ running: false }); expect(target.children).toHaveLength(0);
});

it('native provider retires its view immediately and ignores a start completing after cancellation', async () => {
  let reply;
  const update = vi.fn(), dispose = vi.fn();
  const provider = createNativeWorkshopProvider({ request: request => request.op === 'test-start'
    ? new Promise(resolve => { reply = resolve; }) : Promise.resolve({ status: 'test', run: null }),
  testView: () => ({ update, dispose }) });
  provider.test.mount(document.body, 'Test');
  const started = provider.test.start({}, {});
  await vi.waitFor(() => expect(reply).toBeTypeOf('function'));
  provider.test.cancelStart(); reply({ status: 'test', run: run('late') }); await started;
  expect(update).not.toHaveBeenCalledWith(run('late'));
  await provider.test.stop(); expect(update).toHaveBeenLastCalledWith(null);
});
