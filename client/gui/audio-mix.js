/** Local presentation policy; never a simulation input. [ai] One gain law for
 * room and future private providers, with authored sound levels left intact. */
export const AUDIO_CATEGORIES = Object.freeze(['music', 'ambience', 'effects', 'alerts', 'interface']);
export const AUDIO_BUSES = Object.freeze(['master', ...AUDIO_CATEGORIES]);
export const clampAudioLevel = value => Number.isFinite(Number(value))
  ? Math.max(0, Math.min(1, Number(value))) : 0;

export function defaultAudioMix() {
  return Object.fromEntries(AUDIO_BUSES.map(id => [id, { level: 1, muted: false }]));
}

export function normalizeAudioMix(value) {
  const mix = defaultAudioMix();
  for (const id of AUDIO_BUSES) {
    const entry = value?.[id];
    if (entry && typeof entry.level === 'number' && Number.isFinite(entry.level)) {
      mix[id].level = clampAudioLevel(entry.level);
    }
    mix[id].muted = entry?.muted === true;
  }
  return mix;
}

export function audioBusGain(mix, id) {
  const bus = mix[id];
  return bus?.muted ? 0 : clampAudioLevel(bus?.level);
}

export function audioSoundGain(mix, category, authored) {
  return clampAudioLevel(authored) * audioBusGain(mix, category) * audioBusGain(mix, 'master');
}
