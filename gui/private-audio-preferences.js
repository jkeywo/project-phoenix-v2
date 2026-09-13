/** Portable private feedback choices. Hardware routes and occurrences never
 * enter this schema; private surfaces own only Master, Alerts and Interface. */
export const PRIVATE_AUDIO_BUSES = Object.freeze(['master', 'alerts', 'interface']);
export const PRIVATE_AUDIO_CUES = Object.freeze({
  clicks: true, refused: true, timedOut: true, applied: false, pending: false, actionable: true,
});
export const LEGACY_PRIVATE_MASTER_KEY = 'phoenix-settings-volume';

export function normalizePrivateAudio(value) {
  return {
    version: 1,
    mono: value?.mono === true,
    reducedRange: value?.reducedRange === true,
    mix: Object.fromEntries(PRIVATE_AUDIO_BUSES.map(id => {
      const bus = value?.mix?.[id];
      return [id, {
        level: typeof bus?.level === 'number' && Number.isFinite(bus.level)
          ? Math.max(0, Math.min(1, bus.level)) : 1,
        muted: bus?.muted === true,
      }];
    })),
    cues: Object.fromEntries(Object.entries(PRIVATE_AUDIO_CUES).map(([id, initial]) =>
      [id, typeof value?.cues?.[id] === 'boolean' ? value.cues[id] : initial])),
  };
}

/** Migration reads only the old client Master scalar, never a room endpoint. */
export function legacyPrivateMaster(storage) {
  try {
    const raw = storage?.getItem(LEGACY_PRIVATE_MASTER_KEY);
    if (typeof raw !== 'string' || !raw.trim()) return null;
    // Match the retired slider's parser so even its accepted numeric prefix
    // cannot migrate a quiet record back to full volume.
    const level = Number.parseFloat(raw);
    return Number.isFinite(level) ? Math.max(0, Math.min(1, level)) : null;
  } catch (_) { return null; }
}
