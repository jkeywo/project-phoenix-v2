import { AUDIO_BUSES, defaultAudioMix, normalizeAudioMix, clampAudioLevel } from './audio-mix.js';
import {
  VIEWSCREEN_PRESENTATION_KEY, serializeViewscreenPresentation,
} from './viewscreen-presentation.js';

export const LEGACY_MASTER_VOLUME_KEY = 'phoenix-server-master-volume';

/** [ai] Audio and visual preferences share the endpoint record, but each writer
 * merges its own section against current storage to avoid stale-controller loss.
 * Only settings persist: no cue/event/restore bookkeeping is stored here. */
export function createRoomAudioPreferences(storage) {
  let status = storage ? 'saved' : 'unavailable';
  let mix = defaultAudioMix();
  let mono = false;
  let migrated = false;
  try {
    const text = storage?.getItem(VIEWSCREEN_PRESENTATION_KEY);
    let record = null;
    if (text) {
      try {
        record = JSON.parse(text);
        if (!record || typeof record !== 'object' || Array.isArray(record)) status = 'corrupt';
      }
      catch (_) { status = 'corrupt'; }
    }
    if (record?.audio?.version === 1 && record.audio.mix && typeof record.audio.mix === 'object') {
      mix = normalizeAudioMix(record.audio.mix);
      mono = record.audio.mono === true;
      if (!AUDIO_BUSES.every(id => {
        const bus = record.audio.mix[id];
        return bus && typeof bus.level === 'number' && Number.isFinite(bus.level)
          && bus.level >= 0 && bus.level <= 1 && typeof bus.muted === 'boolean';
      })) status = 'corrupt';
    } else {
      if (record?.audio) status = 'corrupt';
      const old = storage?.getItem(LEGACY_MASTER_VOLUME_KEY);
      if (old != null && old.trim() !== '' && Number.isFinite(Number(old))) {
        mix.master.level = clampAudioLevel(old);
        migrated = true;
      } else if (old != null) status = 'corrupt';
    }
  } catch (_) { status = 'unavailable'; }

  function save(next, nextMono = mono) {
    mix = normalizeAudioMix(next);
    mono = nextMono === true;
    try {
      if (!storage) throw new Error('No endpoint storage');
      let current;
      try { current = JSON.parse(storage.getItem(VIEWSCREEN_PRESENTATION_KEY)); }
      catch (_) { current = null; }
      const record = JSON.parse(serializeViewscreenPresentation(current));
      record.audio = { ...current?.audio, version: 1, mix, mono };
      storage.setItem(VIEWSCREEN_PRESENTATION_KEY, JSON.stringify(record));
      // Only retire the old value AFTER the replacement has been saved.
      storage.removeItem?.(LEGACY_MASTER_VOLUME_KEY);
      status = 'saved';
    } catch (_) { status = 'unavailable'; }
    return read();
  }
  function read() { return { mix: normalizeAudioMix(mix), mono, persistence: status }; }
  if (migrated && status === 'saved') save(mix);
  return { read, save };
}
