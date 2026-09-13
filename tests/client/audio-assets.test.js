import { expect, it } from 'vitest';
import { createBrowserAudioProvider } from '../../gui/browser-audio-provider.js';
import { defaultAudioMix } from '../../gui/audio-mix.js';
import { FakeAudioContext, settleAudio } from './audio-fixtures.js';

function endpoint(context = new FakeAudioContext({ state: 'running' })) {
  let revision = 0, sample = 0.2, watch, stopped = false, fetches = 0;
  const provider = createBrowserAudioProvider({
    contextFactory: () => context,
    fetchAudio: async () => {
      fetches++;
      const bytes = new Float32Array([sample]).buffer;
      return { ok: true, arrayBuffer: async () => bytes };
    },
    readAssetRevision: () => revision,
    watchAssetRevision: callback => { watch = callback; return () => { stopped = true; }; },
  });
  return { provider, context, replace(next) { sample = next; revision++; watch(); },
    get fetches() { return fetches; }, get stopped() { return stopped; } };
}

it('replacement/reorder/removal refresh a current loop and discard live shots, tests and auditions', async () => {
  const rig = endpoint(), { provider, context } = rig;
  provider.register('bed', { file: 'same.wav', category: 'ambience', volume: 0.5, loop: true });
  provider.register('alert', { file: 'alert.wav', category: 'alerts', volume: 1 });
  provider.loop('bed', true);
  provider.setMono(true);
  await settleAudio();
  expect(context.sample()).toBeCloseTo(0.1);
  expect(provider.cue('alert')).toBe(true);
  expect(await provider.testOutput('alert')).toBe(true);
  expect(await provider.audition({ file: 'preview.wav', category: 'effects', volume: 1 })).toBe(true);
  rig.replace(0.8);
  expect(context.sample()).toBe(0);
  expect(provider.cue('alert')).toBe(false);
  await settleAudio();
  expect(provider.snapshot().active.map(voice => voice.id)).toEqual(['bed']);
  expect(provider.snapshot().test).toBe('idle');
  expect(context.sample()).toBeCloseTo(0.4);
  const mix = defaultAudioMix(); mix.master.muted = true; provider.setMix(mix);
  rig.replace(0.3);
  context.state = 'suspended'; context.onstatechange();
  await settleAudio();
  await provider.enable();
  expect(context.sample()).toBe(0);
  mix.master.muted = false; provider.setMix(mix);
  expect(context.sample()).toBeCloseTo(0.15);
  rig.replace(0.2); // Removal resolves the same path to its base bytes.
  await settleAudio();
  expect(context.sample()).toBeCloseTo(0.1);
  expect(provider.snapshot().active.map(voice => voice.id)).toEqual(['bed']);
  provider.dispose(); expect(rig.stopped).toBe(true);
});

it('late old decode cannot replace new PCM, fail new readiness or revive pending deliberate requests', async () => {
  const context = new FakeAudioContext({ state: 'running' }), decodes = [];
  context.decodeAudioData = bytes => new Promise((resolve, reject) => {
    const sample = new Float32Array(bytes)[0];
    decodes.push({ sample, reject, finish: () => resolve({ duration: 10, sample, stereo: [sample, sample] }) });
  });
  const rig = endpoint(context), { provider } = rig;
  provider.register('bed', { file: 'same.wav', category: 'music', volume: 1, loop: true });
  provider.loop('bed', true);
  const test = provider.testOutput('bed');
  const preview = provider.audition({ file: 'same.wav', category: 'effects', volume: 1 });
  await settleAudio();
  expect(decodes).toHaveLength(2);
  rig.replace(0.8);
  await settleAudio();
  expect(decodes).toHaveLength(3);
  // Controls settle at the boundary, before either old decode completes.
  expect(await test).toBe(false); expect(await preview).toBe(false);
  decodes[2].finish(); await settleAudio();
  expect(context.sample()).toBeCloseTo(0.8);
  decodes[0].finish(); decodes[1].reject(new Error('old decode failed'));
  expect(await test).toBe(false); expect(await preview).toBe(false);
  await settleAudio();
  expect(provider.snapshot().status).toBe('playing');
  expect(provider.snapshot().active.map(voice => voice.id)).toEqual(['bed']);
  expect(context.sample()).toBeCloseTo(0.8);
  const fetches = rig.fetches;
  provider.register('another', { file: 'same.wav', category: 'alerts', volume: 0.1 });
  await settleAudio();
  expect(rig.fetches).toBe(fetches); // Old failed promise did not evict new cache entry.
  provider.dispose();
});

it('stale decode notices a host revision before a watcher or a new cue can play it', async () => {
  const context = new FakeAudioContext({ state: 'running' });
  let revision = 0, complete;
  context.decodeAudioData = () => new Promise(resolve => { complete = resolve; });
  const provider = createBrowserAudioProvider({ contextFactory: () => context,
    fetchAudio: async () => ({ arrayBuffer: async () => new ArrayBuffer(4) }),
    readAssetRevision: () => revision, watchAssetRevision: () => () => {} });
  provider.register('shot', { file: 'same.wav', category: 'effects', volume: 1 });
  await settleAudio();
  revision++;
  complete({ duration: 10, sample: 1, stereo: [1, 1] });
  await settleAudio();
  expect(provider.cue('shot')).toBe(false);
  expect(context.sample()).toBe(0);
  expect(provider.snapshot().ready).toEqual([]);
  provider.dispose();
});


it('disposing settles a deliberate preview even if its decode never completes', async () => {
  const context = new FakeAudioContext({ state: 'running' });
  context.decodeAudioData = () => new Promise(() => {});
  const { provider } = endpoint(context);
  const preview = provider.audition({ file: 'pending.wav', category: 'effects', volume: 1 });
  await settleAudio(); provider.dispose();
  expect(await preview).toBe(false);
});
