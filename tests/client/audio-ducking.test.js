import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createBrowserAudioProvider } from '../../gui/browser-audio-provider.js';
import { createHostAudio } from '../../gui/host-audio.js';
import { createRoomAudioPreferences } from '../../gui/audio-preferences.js';
import { defaultAudioMix } from '../../gui/audio-mix.js';
import { VIEWSCREEN_PRESENTATION_KEY } from '../../gui/viewscreen-presentation.js';
import { FakeAudioContext, audioFetch, memoryStorage, settleAudio, AUDIO_CONFIG } from './audio-fixtures.js';
import { roomDuckingJson } from '../../scripts/room-ducking.mjs';

const spec = JSON.parse(readFileSync(resolve('assets/audio/room-ducking.json'), 'utf8'));
async function fixture() {
  const context = new FakeAudioContext({ state: 'running' });
  const provider = createBrowserAudioProvider({ contextFactory: () => context, fetchAudio: audioFetch(0.1) });
  for (const category of ['music', 'ambience', 'effects', 'interface']) {
    provider.register(category, { file: category, volume: 1, category, loop: true });
    provider.loop(category, true);
  }
  provider.register('authored_alert', { file: 'authored_alert', volume: 1, category: 'alerts', important: true });
  await settleAudio();
  return { context, provider };
}
describe('room ducking in the production browser sample path', () => {
  it('keeps browser data equal to the native TOML envelope without another tuning source', async () => {
    expect(await roomDuckingJson(resolve('.'))).toBe(readFileSync(resolve('assets/audio/room-ducking.json'), 'utf8'));
  });
  it('defaults off and smooths only Music/Ambience under a played authored-shaped Alert', async () => {
    const { context, provider } = await fixture();
    const base = context.sample();
    provider.cue('authored_alert'); context.advance(spec.attack_seconds);
    expect(context.sample()).toBeCloseTo(base + 0.1);
    provider.stopAll();
    for (const id of ['music', 'ambience', 'effects', 'interface']) provider.loop(id, true);
    provider.setDucking(true, spec); provider.cue('authored_alert');
    expect(context.sample()).toBeCloseTo(base + 0.1);
    context.advance(spec.attack_seconds / 2);
    expect(context.sample()).toBeCloseTo(0.3 + 0.2 * (1 + spec.gain) / 2);
    context.advance(spec.attack_seconds / 2);
    expect(context.sample()).toBeCloseTo(0.3 + 0.2 * spec.gain);
    context.advance(spec.hold_seconds - spec.attack_seconds + spec.release_seconds / 2);
    expect(context.sample()).toBeCloseTo(0.3 + 0.2 * (1 + spec.gain) / 2);
    context.advance(spec.release_seconds / 2);
    expect(context.sample()).toBeCloseTo(0.5);
    provider.dispose();
  });
  it('coalesces overlapping alerts but forces complete recovery despite repeated arrivals', async () => {
    const { context, provider } = await fixture();
    provider.setDucking(true, spec); provider.cue('authored_alert');
    for (let index = 0; index < 9; index++) {
      context.advance(0.5); provider.cue('authored_alert');
      // Each ordinary alert contributes 0.1; only the two beds can attenuate.
      if (index < 7) expect(context.sample() - (index + 2) * 0.1).toBeCloseTo(0.2 + 0.2 * spec.gain);
    }
    context.advance(0.25);
    expect(context.sample() - 1).toBeCloseTo(0.4);
    provider.dispose();
  });
  it('mute, disabling, interruption and reset cannot restore a missed duck window', async () => {
    const { context, provider } = await fixture();
    provider.setDucking(true, spec);
    const mix = defaultAudioMix(); mix.music.muted = true; mix.ambience.muted = true;
    provider.setMix(mix); provider.cue('authored_alert'); context.advance(0.5);
    expect(context.sample()).toBeCloseTo(0.3);
    mix.master.muted = true; provider.setMix(mix); expect(context.sample()).toBe(0);
    provider.stopAll(); mix.master.muted = false; mix.music.muted = false; mix.ambience.muted = false;
    provider.setMix(mix); provider.loop('music', true); provider.loop('ambience', true);
    expect(context.sample()).toBeCloseTo(0.2);
    provider.cue('authored_alert'); context.advance(0.1); provider.setDucking(false);
    expect(context.sample()).toBeCloseTo(0.1 + 0.2 * spec.gain);
    context.advance(spec.release_seconds);
    expect(context.sample()).toBeCloseTo(0.3);
    provider.setDucking(true, spec); context.state = 'suspended'; context.onstatechange();
    provider.cue('authored_alert'); await context.resume();
    expect(context.sample()).toBeCloseTo(0.2);
    provider.dispose();
  });
  it('uses the real red-alert edge and persists only the room option across independent display writes', async () => {
    const context = new FakeAudioContext({ state: 'running' });
    const storage = memoryStorage();
    const audio = createHostAudio({ storage, contextFactory: () => context, fetchAudio: audioFetch(), duckingSpec: spec });
    await audio.duckingReady; audio.audioConfig(JSON.stringify(AUDIO_CONFIG)); await settleAudio();
    audio.startGameAudio(); audio.setDucking(true); audio.applyHudAudio({ red_alert: false });
    audio.setMono(true);
    audio.applyHudAudio({ red_alert: true }); context.advance(spec.attack_seconds);
    expect(audio.state().ducking).toBe(true);
    const expected = 0.5 * ((0.25 + 0.1 + 0.4) * spec.gain + 0.6);
    expect(context.sample()).toBeCloseTo(expected);
    const saved = JSON.parse(storage.getItem(VIEWSCREEN_PRESENTATION_KEY));
    saved.presentation.textScale = 1.75; storage.setItem(VIEWSCREEN_PRESENTATION_KEY, JSON.stringify(saved));
    audio.setBus('music', { level: 0.4 });
    expect(createRoomAudioPreferences(storage).read().ducking).toBe(true);
    expect(createRoomAudioPreferences(storage).read().mono).toBe(true);
    expect(JSON.parse(storage.getItem(VIEWSCREEN_PRESENTATION_KEY)).presentation.textScale).toBe(1.75);
    audio.resetMix(); expect(createRoomAudioPreferences(storage).read().ducking).toBe(false);
    expect(createRoomAudioPreferences(storage).read().mono).toBe(false);
    audio.dispose();
  });
});
