import { describe, expect, it } from 'vitest';
import { createRoomAudioPreferences, LEGACY_MASTER_VOLUME_KEY } from '../../gui/audio-preferences.js';
import { VIEWSCREEN_PRESENTATION_KEY, saveViewscreenPresentation, loadViewscreenPresentation } from '../../gui/viewscreen-presentation.js';
import { createHostAudio } from '../../gui/host-audio.js';
import { memoryStorage } from './audio-fixtures.js';

describe('room audio in the existing endpoint preference record', () => {
  it.each([0, 0.25, 1, -2, 4])('migrates legacy Master %s without louder playback and reloads mutes', value => {
    const storage = memoryStorage({ [LEGACY_MASTER_VOLUME_KEY]: String(value), 'operator': 'private', 'save': 'scenario' });
    saveViewscreenPresentation(storage, { textScale: 1.5, contrast: 'on' });
    const preferences = createRoomAudioPreferences(storage);
    const mix = preferences.read().mix;
    expect(mix.master.level).toBe(Math.max(0, Math.min(1, value)));
    expect(storage.getItem(LEGACY_MASTER_VOLUME_KEY)).toBeNull();
    mix.alerts.muted = true; mix.music.level = 0.2; preferences.save(mix);
    expect(createRoomAudioPreferences(storage).read().mix).toEqual(mix);
    expect(loadViewscreenPresentation(storage).textScale).toBe(1.5);
    expect(storage.getItem('operator')).toBe('private');
    expect(storage.getItem('save')).toBe('scenario');
  });

  it('visual and audio controllers cannot overwrite each other, including either reset', () => {
    const storage = memoryStorage();
    const audio = createHostAudio({ storage, contextFactory: () => null });
    audio.setMasterVolume(0.3); audio.setBus('alerts', { muted: true });
    saveViewscreenPresentation(storage, { textScale: 2, contrast: 'on' });
    expect(createRoomAudioPreferences(storage).read().mix.alerts.muted).toBe(true);
    audio.resetMix();
    expect(loadViewscreenPresentation(storage).textScale).toBe(2);
    audio.setMasterVolume(0.2);
    saveViewscreenPresentation(storage, {});
    expect(createRoomAudioPreferences(storage).read().mix.master.level).toBe(0.2);
    expect(JSON.parse(storage.getItem(VIEWSCREEN_PRESENTATION_KEY))).toHaveProperty('audio.version', 1);
    audio.dispose();
  });

  it('corrupt and blocked storage are visible; local mixing still works and cue history is never stored', () => {
    const corrupt = memoryStorage({ [VIEWSCREEN_PRESENTATION_KEY]: '{bad', [LEGACY_MASTER_VOLUME_KEY]: '0.2' });
    const preferences = createRoomAudioPreferences(corrupt);
    expect(preferences.read().persistence).toBe('corrupt');
    expect(preferences.read().mix.master.level).toBe(0.2);
    preferences.save(preferences.read().mix);
    expect(preferences.read().persistence).toBe('saved');
    const denied = { getItem() { throw Error('denied'); }, setItem() { throw Error('denied'); } };
    const audio = createHostAudio({ storage: denied, contextFactory: () => null });
    audio.setMasterVolume(0.4);
    expect(audio.state().persistence).toBe('unavailable');
    expect(audio.getMasterVolume()).toBe(0.4);
    audio.dispose();
    const before = new Map(corrupt.values);
    const active = createHostAudio({ storage: corrupt, contextFactory: () => null });
    active.audioConfig('{}'); active.startGameAudio(); active.audioCue('{"kind":"blaster","x":0,"y":0,"z":-5}');
    active.resetSession();
    expect(corrupt.values).toEqual(before);
    active.dispose();
  });

  it('reports malformed mix fields and retains a valid quiet Master where possible', () => {
    const storage = memoryStorage({ [VIEWSCREEN_PRESENTATION_KEY]: JSON.stringify({ audio: { version: 1,
      mix: { master: { level: 0.1, muted: true }, music: { level: 'bad' } } } }) });
    const preferences = createRoomAudioPreferences(storage);
    expect(preferences.read().persistence).toBe('corrupt');
    expect(preferences.read().mix.master).toEqual({ level: 0.1, muted: true });
  });
});
