// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { parse } from 'smol-toml';
import { createHostAudio } from '../../gui/host-audio.js';
import { createAudioLiveEquivalents } from '../../gui/audio-live-equivalents.js';
import { FakeAudioContext, audioFetch, memoryStorage, settleAudio } from './audio-fixtures.js';

const catalog = parse(readFileSync('assets/audio/sound-cues.toml', 'utf8'));
const room = catalog.cues.filter(cue => cue.audience === 'viewscreen');
const definition = room.find(cue => cue.id === 'weapons');
const cue = (occurrence = 1, extra = {}) => JSON.stringify({ kind: 'authored', generation: 1, occurrence, definition, ...extra });
const lifecycle = (generation, extra = {}) => JSON.stringify({ generation, running: true, suspended: false, ...extra });
const cleanups = [];
afterEach(() => { cleanups.splice(0).forEach(fn => fn()); vi.useRealTimers(); });
async function setup(options = {}) {
  const context = new FakeAudioContext();
  const live = createAudioLiveEquivalents(document);
  const audio = createHostAudio({ doc: document, storage: memoryStorage(), contextFactory: () => context,
    fetchAudio: audioFetch(), fetchDucking: () => Promise.reject(), onEquivalent: value => live.update(value), ...options });
  audio.audioLifecycle(lifecycle(1));
  audio.audioConfig(JSON.stringify({ authored_sounds: room }));
  await audio.enable(); await settleAudio();
  cleanups.push(() => { audio.dispose(); live.dispose(); });
  return { audio, context };
}

describe('live authored room occurrence through the production provider', () => {
  it('drops a cold cue instead of replaying when its asset eventually decodes', async () => {
    let release; const pending = new Promise(resolve => { release = resolve; });
    const { audio, context } = await setup({ fetchAudio: () => pending });
    audio.audioCue(cue()); expect(context.sample()).toBe(0);
    release(await audioFetch()()); await settleAudio();
    audio.audioCue(cue()); expect(context.sample()).toBe(0);
    audio.audioCue(cue(2)); expect(context.sample()).toBeGreaterThan(0);
  });
  it('uses the declared category, spatial metadata and live equivalent with one current occurrence', async () => {
    const { audio, context } = await setup();
    audio.audioCue(cue());
    expect(context.sample()).toBeGreaterThan(0);
    expect(audio.state().active.filter(voice => voice.id.startsWith('authored_'))).toHaveLength(1);
    expect(document.querySelector('[data-audio-equivalent=authored]').textContent).toContain('Weapon discharge');
    expect(document.querySelector('[data-audio-equivalent=authored]').textContent).toContain('45');
    audio.audioCue(cue()); audio.audioCue(cue(0));
    expect(audio.state().active.filter(voice => voice.id.startsWith('authored_'))).toHaveLength(1);
    const before = context.sample();
    audio.setBus('effects', { level: 0.4 }); audio.setMasterVolume(0.5);
    expect(context.sample()).toBeCloseTo(before * 0.2);
    audio.audioCue(cue(2));
    expect(audio.state().active.filter(voice => voice.id.startsWith('authored_'))).toHaveLength(1);
  });
  it('consumes muted and hidden arrivals, refusing replay on unmute, page return and restore', async () => {
    const { audio, context } = await setup();
    audio.setBus('master', { muted: true }); audio.audioCue(cue());
    expect(context.sample()).toBe(0);
    expect(document.querySelector('[data-audio-equivalent=authored]')).not.toBeNull();
    audio.setBus('master', { muted: false }); audio.audioCue(cue());
    expect(context.sample()).toBe(0);
    audio.setPageActive(false); audio.audioCue(cue(2)); audio.setPageActive(true); audio.audioCue(cue(2));
    expect(context.sample()).toBe(0);
    audio.audioCue(cue(3)); expect(context.sample()).toBeGreaterThan(0);
    audio.audioLifecycle(lifecycle(2, { suspended: true }));
    expect(context.sample()).toBe(0);
    audio.audioCue(cue(4)); audio.audioLifecycle(lifecycle(1));
    expect(context.sample()).toBe(0);
    audio.audioLifecycle(lifecycle(3)); audio.audioCue(cue(5));
    expect(context.sample()).toBe(0);
    audio.audioCue(cue(6, { generation: 3 }));
    expect(context.sample()).toBeGreaterThan(0);
  });
  it('consumes a locked occurrence before enable and rejects altered or wrong-audience definitions', async () => {
    const context = new FakeAudioContext({ refuseResume: true });
    const { audio } = await setup({ contextFactory: () => context });
    audio.audioCue(cue()); expect(context.sample()).toBe(0);
    context.refuseResume = false; await audio.enable(); audio.audioCue(cue());
    expect(context.sample()).toBe(0);
    audio.audioCue(cue(2, { definition: { ...definition, volume: 1 } }));
    audio.audioCue(cue(3, { definition: catalog.cues.find(cue => cue.audience === 'gm') }));
    expect(context.sample()).toBe(0);
    audio.audioCue(cue(4)); expect(context.sample()).toBeGreaterThan(0);
  });
  it('decorative replacements clear the live label and expired labels leave no review list', async () => {
    const { audio } = await setup(); vi.useFakeTimers();
    audio.audioCue(cue());
    audio.audioCue(cue(2, { definition: room.find(cue => cue.id === 'ambient') }));
    expect(document.querySelector('[data-audio-equivalent=authored]')).toBeNull();
    audio.audioCue(cue(3)); vi.advanceTimersByTime(2001);
    expect(document.querySelector('[data-audio-equivalent=authored]')).toBeNull();
    expect(document.querySelector('#audio-live-equivalents').children).toHaveLength(0);
  });
});
